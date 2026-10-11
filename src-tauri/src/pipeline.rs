//! Audio → speech segments → Finnish text → English text.
//!
//! Three threads, connected by channels:
//!
//! * **segmenter** — runs Silero VAD over the incoming 16 kHz audio and cuts it into utterances
//!   at pauses. While someone is still talking it also emits *draft* jobs so the UI can show a
//!   live preview.
//! * **asr** — Whisper. Drafts are dropped whenever a newer job for the same utterance is queued,
//!   so a slow machine degrades to "fewer previews", never to a growing backlog.
//! * **mt** — Marian translation, with the same "latest draft wins" rule.

use crate::asr::{self, Transcriber, SAMPLE_RATE};
use crate::audio::{self, Capture};
use crate::models;
use crate::mt::Translator;
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Runtime};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams};

/// Silero works on 32 ms frames at 16 kHz.
const FRAME: usize = 512;
const FRAME_MS: f32 = 32.0;
/// Run VAD every 8 frames (256 ms) over a 768 ms window; whisper.cpp resets the VAD's LSTM
/// state on every call, so the extra context lets it settle before the frames we keep.
const HOP: usize = FRAME * 8;
const VAD_CONTEXT: usize = FRAME * 16;
const PRE_ROLL: usize = SAMPLE_RATE * 3 / 10;
const DRAFT_EVERY: usize = SAMPLE_RATE * 6 / 5;
const MIN_SPEECH_MS: f32 = 300.0;

/// Set once when the app is quitting: workers stop taking jobs and Whisper aborts mid-run.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::Relaxed)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Silence that ends an utterance.
    pub pause_ms: u32,
    /// Utterances are force-split after this long, at the quietest point.
    pub max_utterance_secs: f32,
    pub live_preview: bool,
    pub vad_threshold: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { pause_ms: 600, max_utterance_secs: 12.0, live_preview: true, vad_threshold: 0.5 }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtteranceEvent {
    id: u64,
    is_final: bool,
    source: String,
    translation: String,
    started_at: u64,
    audio_ms: u64,
    asr_ms: u64,
    mt_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LevelEvent {
    level: f32,
    speaking: bool,
}

pub type Models = (Arc<Mutex<Transcriber>>, Arc<Mutex<Translator>>);

/// Loaded models, kept across sessions so switching audio source is instant.
#[derive(Default)]
pub struct Engine {
    /// Model id, the GPU it was loaded onto (`None` = CPU), and the model.
    asr: Mutex<Option<(String, Option<i32>, Arc<Mutex<Transcriber>>)>>,
    mt: Mutex<Option<Arc<Mutex<Translator>>>>,
}

impl Engine {
    /// Load (or reuse) the requested ASR model on the processor `device` prefers (see
    /// [`asr::resolve_device`]), and the translator. Blocking; can take seconds.
    /// Loads can't be interrupted midway, but once `still_wanted` turns false the remaining
    /// models are skipped and `None` is returned. Whatever already loaded stays cached.
    pub fn prepare(&self, root: &Path, asr_id: &str, device: &str, still_wanted: &dyn Fn() -> bool) -> Result<Option<Models>> {
        let gpu = asr::resolve_device(device);
        let asr = {
            let mut slot = self.asr.lock();
            match slot.as_ref() {
                Some((id, on, t)) if id == asr_id && *on == gpu => t.clone(),
                _ => {
                    // Free the old model before loading a new multi-GB one.
                    *slot = None;
                    let path = models::weights_path(root, asr_id)?;
                    let load = |gpu| -> Result<Transcriber> {
                        let mut t = Transcriber::load(&path, gpu)?;
                        // First inference compiles GPU kernels; get that out of the way now.
                        t.transcribe(&vec![0.0; SAMPLE_RATE], true)?;
                        Ok(t)
                    };
                    // A GPU that's listed can still fail (out of video memory, driver trouble);
                    // run on the CPU rather than not at all. Cached under the requested GPU, so
                    // later starts don't retry the failing load.
                    let t = match load(gpu) {
                        Err(e) if gpu.is_some() => {
                            log::warn!("loading {asr_id} on GPU {gpu:?} failed, using the CPU: {e:#}");
                            load(None)?
                        }
                        loaded => loaded?,
                    };
                    let t = Arc::new(Mutex::new(t));
                    *slot = Some((asr_id.to_string(), gpu, t.clone()));
                    t
                }
            }
        };
        if !still_wanted() {
            return Ok(None);
        }
        let mt = {
            let mut slot = self.mt.lock();
            match slot.as_ref() {
                Some(t) => t.clone(),
                None => {
                    let spec = models::spec(models::MT)?;
                    anyhow::ensure!(spec.is_installed(root), "{} is not downloaded yet", spec.name);
                    let t = Arc::new(Mutex::new(Translator::load(&spec.dir(root))?));
                    *slot = Some(t.clone());
                    t
                }
            }
        };
        Ok(Some((asr, mt)))
    }

    /// Whether `id` is currently loaded (and so possibly in use by a session).
    pub fn holds(&self, id: &str) -> bool {
        match models::spec(id).map(|s| s.kind) {
            Ok(models::ModelKind::Asr) => self.asr.lock().as_ref().is_some_and(|(loaded, ..)| loaded == id),
            Ok(models::ModelKind::Mt) => self.mt.lock().is_some(),
            _ => false,
        }
    }

    /// Drop the cached copy of `id`, if loaded. Running workers keep their own reference until
    /// they stop.
    pub fn evict(&self, id: &str) {
        match models::spec(id).map(|s| s.kind) {
            Ok(models::ModelKind::Asr) => {
                let mut slot = self.asr.lock();
                if slot.as_ref().is_some_and(|(loaded, ..)| loaded == id) {
                    *slot = None;
                }
            }
            Ok(models::ModelKind::Mt) => *self.mt.lock() = None,
            _ => {}
        }
    }

    /// Stop all inference and free the models. Must run before the process exits: ggml's Metal
    /// device is a C++ static whose destructor asserts that every GPU buffer was released.
    ///
    /// Worker threads (including ones from already-stopped sessions) hold their own references,
    /// so wait for them to let go. Returns false if they didn't within `timeout`.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        SHUTTING_DOWN.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + timeout;
        // `prepare` holds the lock while a model loads; don't wait on it forever.
        let asr = self.asr.try_lock_for(timeout).and_then(|mut slot| slot.take());
        let mt = self.mt.try_lock_for(deadline.saturating_duration_since(Instant::now())).and_then(|mut slot| slot.take());
        let mut released = true;
        if let Some((.., t)) = asr {
            released &= wait_until_unique(t, deadline);
        }
        if let Some(t) = mt {
            released &= wait_until_unique(t, deadline);
        }
        released
    }
}

/// Drop `arc` once we hold the last reference; false if other holders remain at `deadline`.
fn wait_until_unique<T>(mut arc: Arc<T>, deadline: Instant) -> bool {
    loop {
        match Arc::try_unwrap(arc) {
            Ok(inner) => {
                drop(inner);
                return true;
            }
            Err(shared) if Instant::now() < deadline => {
                arc = shared;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return false,
        }
    }
}

/// A running capture + pipeline. Dropping it stops everything.
pub struct Session {
    _capture: Capture,
    stop: Arc<AtomicBool>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

struct Job {
    id: u64,
    is_final: bool,
    started_at: u64,
    audio: Arc<Vec<f32>>,
}

struct Text {
    id: u64,
    is_final: bool,
    started_at: u64,
    audio_ms: u64,
    asr_ms: u64,
    source: String,
}

pub fn start<R: Runtime>(
    app: AppHandle<R>,
    root: &Path,
    source_id: &str,
    asr: Arc<Mutex<Transcriber>>,
    mt: Arc<Mutex<Translator>>,
    settings: Arc<RwLock<Settings>>,
) -> Result<Session> {
    let (audio_tx, stop) = spawn_workers(app, root, asr, mt, settings)?;
    let capture = audio::start(
        source_id,
        Box::new(move |samples: &[f32]| {
            let _ = audio_tx.send(samples.to_vec());
        }),
    )?;
    Ok(Session { _capture: capture, stop })
}

/// Start the segmenter/ASR/MT threads. Feed 16 kHz mono audio into the returned sender;
/// dropping it (or setting the flag) shuts the pipeline down.
pub(crate) fn spawn_workers<R: Runtime>(
    app: AppHandle<R>,
    root: &Path,
    asr: Arc<Mutex<Transcriber>>,
    mt: Arc<Mutex<Translator>>,
    settings: Arc<RwLock<Settings>>,
) -> Result<(Sender<Vec<f32>>, Arc<AtomicBool>)> {
    let vad_path = models::weights_path(root, models::VAD)?;
    let mut vad_params = WhisperVadContextParams::new();
    vad_params.set_n_threads(1);
    vad_params.set_use_gpu(false);
    let vad = WhisperVadContext::new(&vad_path.to_string_lossy(), vad_params)
        .context("loading VAD model")?;

    let stop = Arc::new(AtomicBool::new(false));
    let asr_busy = Arc::new(AtomicBool::new(false));
    let (audio_tx, audio_rx) = crossbeam_channel::unbounded::<Vec<f32>>();
    let (job_tx, job_rx) = crossbeam_channel::unbounded::<Job>();
    let (text_tx, text_rx) = crossbeam_channel::unbounded::<Text>();

    {
        let (app, stop, busy) = (app.clone(), stop.clone(), asr_busy.clone());
        std::thread::Builder::new().name("segmenter".into()).spawn(move || {
            let mut seg = Segmenter::new(vad, settings, job_tx, busy, app);
            seg.run(audio_rx, &stop);
        })?;
    }
    {
        let (app, stop, busy) = (app.clone(), stop.clone(), asr_busy.clone());
        std::thread::Builder::new()
            .name("asr".into())
            .spawn(move || asr_worker(asr, job_rx, text_tx, busy, &stop, &app))?;
    }
    {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("mt".into())
            .spawn(move || mt_worker(mt, text_rx, &stop, &app))?;
    }
    Ok((audio_tx, stop))
}

struct Utterance {
    id: u64,
    started_at: u64,
    audio: Vec<f32>,
    /// Number of pre-roll samples at the start of `audio` (not covered by `probs`).
    pre_roll: usize,
    probs: Vec<f32>,
    voiced_frames: usize,
    silent_frames: usize,
    last_draft_len: usize,
}

struct Segmenter<R: Runtime> {
    vad: WhisperVadContext,
    settings: Arc<RwLock<Settings>>,
    jobs: Sender<Job>,
    asr_busy: Arc<AtomicBool>,
    app: AppHandle<R>,
    pending: Vec<f32>,
    /// Most recent audio, for VAD context and pre-roll.
    recent: VecDeque<f32>,
    current: Option<Utterance>,
    next_id: u64,
    level_acc: (f32, usize),
    last_level: Instant,
}

impl<R: Runtime> Segmenter<R> {
    fn new(
        vad: WhisperVadContext,
        settings: Arc<RwLock<Settings>>,
        jobs: Sender<Job>,
        asr_busy: Arc<AtomicBool>,
        app: AppHandle<R>,
    ) -> Self {
        let next_id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        Self {
            vad,
            settings,
            jobs,
            asr_busy,
            app,
            pending: Vec::new(),
            recent: VecDeque::with_capacity(VAD_CONTEXT + HOP),
            current: None,
            next_id,
            level_acc: (0.0, 0),
            last_level: Instant::now(),
        }
    }

    fn run(&mut self, audio: Receiver<Vec<f32>>, stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) && !is_shutting_down() {
            match audio.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => self.push(&chunk),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Flush whatever was being said when capture stopped.
        if let Some(u) = self.current.take() {
            self.finish(u);
        }
    }

    fn push(&mut self, chunk: &[f32]) {
        self.report_level(chunk);
        self.pending.extend_from_slice(chunk);
        while self.pending.len() >= HOP {
            let hop: Vec<f32> = self.pending.drain(..HOP).collect();
            let probs = self.detect(&hop);
            let settings = self.settings.read().clone();
            for (frame, p) in hop.chunks_exact(FRAME).zip(probs) {
                self.on_frame(frame, p, &settings);
                self.recent.extend(frame);
                let excess = self.recent.len().saturating_sub(VAD_CONTEXT.max(PRE_ROLL));
                self.recent.drain(..excess);
            }
        }
    }

    /// Speech probability for each frame of `hop`.
    fn detect(&mut self, hop: &[f32]) -> Vec<f32> {
        let mut window: Vec<f32> = self.recent.iter().copied().collect();
        window.extend_from_slice(hop);
        let frames = hop.len() / FRAME;
        if let Err(e) = self.vad.detect_speech(&window) {
            log::warn!("VAD failed: {e}");
            return vec![0.0; frames];
        }
        let probs = self.vad.probabilities();
        let tail = &probs[probs.len().saturating_sub(frames)..];
        let mut out = tail.to_vec();
        out.resize(frames, 0.0);
        out
    }

    fn on_frame(&mut self, frame: &[f32], p: f32, s: &Settings) {
        let Some(u) = self.current.as_mut() else {
            if p >= s.vad_threshold {
                let pre: Vec<f32> = self.recent.iter().skip(self.recent.len().saturating_sub(PRE_ROLL)).copied().collect();
                let id = self.next_id;
                self.next_id += 1;
                let started_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                let pre_roll = pre.len();
                let mut audio = pre;
                audio.extend_from_slice(frame);
                self.current = Some(Utterance {
                    id,
                    started_at,
                    audio,
                    pre_roll,
                    probs: vec![p],
                    voiced_frames: 1,
                    silent_frames: 0,
                    last_draft_len: 0,
                });
            }
            return;
        };

        u.audio.extend_from_slice(frame);
        u.probs.push(p);
        // Hysteresis: once speaking, require a clearly lower probability to count as silence.
        if p >= s.vad_threshold - 0.15 {
            u.voiced_frames += 1;
            u.silent_frames = 0;
        } else {
            u.silent_frames += 1;
        }

        let max_len = (s.max_utterance_secs.max(3.0) * SAMPLE_RATE as f32) as usize;
        if u.silent_frames as f32 * FRAME_MS >= s.pause_ms as f32 {
            // Keep a bit of the trailing silence; Whisper likes a clean ending.
            let keep = u.audio.len() - (u.silent_frames.saturating_sub(6) * FRAME);
            let mut u = self.current.take().expect("checked above");
            u.audio.truncate(keep);
            self.finish(u);
        } else if u.audio.len() >= max_len {
            self.split_long();
        } else if s.live_preview
            && u.audio.len() - u.last_draft_len >= DRAFT_EVERY
            && !self.asr_busy.load(Ordering::Relaxed)
        {
            u.last_draft_len = u.audio.len();
            let _ = self.jobs.send(Job {
                id: u.id,
                is_final: false,
                started_at: u.started_at,
                audio: Arc::new(u.audio.clone()),
            });
        }
    }

    /// Cut an over-long utterance at its quietest frame in the last few seconds and keep going
    /// with the remainder.
    fn split_long(&mut self) {
        let Some(mut u) = self.current.take() else { return };
        let window = (3000.0 / FRAME_MS) as usize;
        let from = u.probs.len().saturating_sub(window);
        let (cut_frame, _) = u.probs[from..]
            .iter()
            .enumerate()
            .fold((0, f32::MAX), |best, (i, &p)| if p < best.1 { (i, p) } else { best });
        let cut_frame = from + cut_frame;
        let cut = u.pre_roll + cut_frame * FRAME;

        let tail_audio = u.audio.split_off(cut);
        let tail_probs = u.probs.split_off(cut_frame);
        let id = self.next_id;
        self.next_id += 1;
        let started_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let voiced = tail_probs.iter().filter(|&&p| p >= 0.35).count();
        self.finish(u);
        self.current = Some(Utterance {
            id,
            started_at,
            audio: tail_audio,
            pre_roll: 0,
            probs: tail_probs,
            voiced_frames: voiced,
            silent_frames: 0,
            last_draft_len: 0,
        });
    }

    fn finish(&mut self, u: Utterance) {
        if (u.voiced_frames as f32) * FRAME_MS < MIN_SPEECH_MS {
            let _ = self.app.emit("utterance-dropped", u.id);
            return;
        }
        let _ = self.jobs.send(Job { id: u.id, is_final: true, started_at: u.started_at, audio: Arc::new(u.audio) });
    }

    fn report_level(&mut self, chunk: &[f32]) {
        self.level_acc.0 += chunk.iter().map(|x| x * x).sum::<f32>();
        self.level_acc.1 += chunk.len();
        if self.last_level.elapsed() < Duration::from_millis(60) || self.level_acc.1 == 0 {
            return;
        }
        let rms = (self.level_acc.0 / self.level_acc.1 as f32).sqrt();
        let db = 20.0 * rms.max(1e-6).log10();
        let level = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let _ = self.app.emit("audio-level", LevelEvent { level, speaking: self.current.is_some() });
        self.level_acc = (0.0, 0);
        self.last_level = Instant::now();
    }
}

/// Pull everything queued and drop drafts that a newer job for the same utterance supersedes.
fn drain_latest<T>(first: T, rx: &Receiver<T>, key: impl Fn(&T) -> (u64, bool)) -> Vec<T> {
    let mut items: Vec<T> = std::iter::once(first).chain(rx.try_iter()).collect();
    let mut keep = vec![true; items.len()];
    for i in 0..items.len() {
        let (id, is_final) = key(&items[i]);
        if !is_final && items[i + 1..].iter().any(|later| key(later).0 == id) {
            keep[i] = false;
        }
    }
    let mut k = keep.into_iter();
    items.retain(|_| k.next().unwrap_or(true));
    items
}

fn asr_worker<R: Runtime>(
    asr: Arc<Mutex<Transcriber>>,
    jobs: Receiver<Job>,
    out: Sender<Text>,
    busy: Arc<AtomicBool>,
    stop: &AtomicBool,
    app: &AppHandle<R>,
) {
    while let Ok(first) = jobs.recv() {
        for job in drain_latest(first, &jobs, |j| (j.id, j.is_final)) {
            if is_shutting_down() {
                return;
            }
            // After stop, still finish final utterances but skip drafts.
            if stop.load(Ordering::Relaxed) && !job.is_final {
                continue;
            }
            busy.store(true, Ordering::Relaxed);
            let t0 = Instant::now();
            let result = asr.lock().transcribe(&job.audio, !job.is_final);
            busy.store(false, Ordering::Relaxed);
            let audio_ms = (job.audio.len() * 1000 / SAMPLE_RATE) as u64;
            match result {
                Ok(t) => {
                    let rejected = asr::looks_like_hallucination(&t.text)
                        || (t.no_speech_prob > 0.8 && t.text.chars().count() < 40);
                    log::info!(
                        "asr {}#{} {audio_ms}ms audio in {}ms, no_speech={:.2}{}: {}",
                        if job.is_final { "final" } else { "draft" },
                        job.id,
                        t0.elapsed().as_millis(),
                        t.no_speech_prob,
                        if rejected { " (rejected)" } else { "" },
                        t.text
                    );
                    if rejected {
                        if job.is_final {
                            let _ = app.emit("utterance-dropped", job.id);
                        }
                        continue;
                    }
                    let _ = out.send(Text {
                        id: job.id,
                        is_final: job.is_final,
                        started_at: job.started_at,
                        audio_ms,
                        asr_ms: t0.elapsed().as_millis() as u64,
                        source: t.text,
                    });
                }
                Err(e) => {
                    log::error!("{e:#}");
                    let _ = app.emit("pipeline-error", format!("{e:#}"));
                }
            }
        }
    }
}

fn mt_worker<R: Runtime>(mt: Arc<Mutex<Translator>>, texts: Receiver<Text>, stop: &AtomicBool, app: &AppHandle<R>) {
    while let Ok(first) = texts.recv() {
        for t in drain_latest(first, &texts, |t| (t.id, t.is_final)) {
            if is_shutting_down() {
                return;
            }
            if stop.load(Ordering::Relaxed) && !t.is_final {
                continue;
            }
            let t0 = Instant::now();
            let translation = match mt.lock().translate(&t.source) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("translation failed: {e:#}");
                    let _ = app.emit("pipeline-error", format!("Translation failed: {e:#}"));
                    String::new()
                }
            };
            log::info!("mt #{} in {}ms: {translation}", t.id, t0.elapsed().as_millis());
            let _ = app.emit(
                "utterance",
                UtteranceEvent {
                    id: t.id,
                    is_final: t.is_final,
                    source: t.source,
                    translation,
                    started_at: t.started_at,
                    audio_ms: t.audio_ms,
                    asr_ms: t.asr_ms,
                    mt_ms: t0.elapsed().as_millis() as u64,
                },
            );
        }
    }
}
