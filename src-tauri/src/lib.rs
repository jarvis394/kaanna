mod asr;
mod audio;
mod models;
mod mt;
mod pipeline;
mod spm;

use parking_lot::{Mutex, RwLock};
use pipeline::{Engine, Session, Settings};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

struct AppState {
    models_dir: PathBuf,
    engine: Arc<Engine>,
    session: Mutex<Option<Session>>,
    settings: Arc<RwLock<Settings>>,
    downloads: Mutex<HashSet<String>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    id: String,
    received: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase", tag = "state", content = "message")]
enum Status {
    Idle,
    Loading(String),
    Listening,
}

#[tauri::command]
fn list_sources() -> Vec<audio::SourceInfo> {
    audio::list_sources()
}

#[tauri::command]
fn list_models(state: State<'_, AppState>) -> Vec<models::ModelStatus> {
    models::statuses(&state.models_dir)
}

#[tauri::command]
fn default_asr_model() -> &'static str {
    models::DEFAULT_ASR
}

#[tauri::command]
async fn download_model(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if !state.downloads.lock().insert(id.clone()) {
        return Ok(()); // already in progress
    }
    let root = state.models_dir.clone();
    let result = models::download(&root, &id, |received, total| {
        let _ = app.emit("download-progress", DownloadProgress { id: id.clone(), received, total });
    })
    .await;
    state.downloads.lock().remove(&id);
    result.map_err(err)
}

#[tauri::command]
async fn start(app: AppHandle, state: State<'_, AppState>, source_id: String, asr_model: String) -> CmdResult<()> {
    // Tear down the previous session first so two captures never run at once.
    state.session.lock().take();

    let _ = app.emit("status", Status::Loading("Loading models…".into()));
    let (engine, root) = (state.engine.clone(), state.models_dir.clone());
    let loaded = tauri::async_runtime::spawn_blocking(move || engine.prepare(&root, &asr_model))
        .await
        .map_err(|e| e.to_string())?;
    let (asr, mt) = match loaded {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("status", Status::Idle);
            return Err(err(e));
        }
    };

    let session = pipeline::start(app.clone(), &state.models_dir, &source_id, asr, mt, state.settings.clone());
    match session {
        Ok(s) => {
            *state.session.lock() = Some(s);
            let _ = app.emit("status", Status::Listening);
            Ok(())
        }
        Err(e) => {
            let _ = app.emit("status", Status::Idle);
            Err(err(e))
        }
    }
}

#[tauri::command]
fn stop(app: AppHandle, state: State<'_, AppState>) {
    state.session.lock().take();
    let _ = app.emit("status", Status::Idle);
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.read().clone()
}

#[tauri::command]
fn set_settings(state: State<'_, AppState>, settings: Settings) {
    *state.settings.write() = settings;
}

/// Release capture and models before the process exits (see `Engine::shutdown`).
fn shutdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.session.lock().take();
    if !state.engine.shutdown(Duration::from_secs(5)) {
        // A worker is stuck holding a model. Exiting normally would run ggml's static
        // destructors and crash on the leaked GPU buffers, so skip them.
        log::warn!("models still in use at exit; skipping native teardown");
        // SAFETY: we're quitting; nothing after this needs to run.
        unsafe { libc::_exit(0) };
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,whisper_rs=warn"))
        .init();
    whisper_rs::install_logging_hooks();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let models_dir = app.path().app_data_dir()?.join("models");
            std::fs::create_dir_all(&models_dir)?;
            log::info!("models directory: {}", models_dir.display());
            app.manage(AppState {
                models_dir,
                engine: Arc::new(Engine::default()),
                session: Mutex::new(None),
                settings: Arc::new(RwLock::new(Settings::default())),
                downloads: Mutex::new(HashSet::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sources,
            list_models,
            default_asr_model,
            download_model,
            start,
            stop,
            get_settings,
            set_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                shutdown(app);
            }
        });
}

/// End-to-end checks against the real models in the app data directory.
/// Run with `cargo test --release -- --ignored --nocapture`.
#[cfg(test)]
mod e2e {
    use super::*;
    use std::time::Instant;

    fn models_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap();
        PathBuf::from(home).join("Library/Application Support/dev.kaanna.app/models")
    }

    /// 16-bit PCM WAV → f32 samples.
    fn read_wav(path: &str) -> Vec<f32> {
        let b = std::fs::read(path).unwrap();
        let mut i = 12;
        while i + 8 <= b.len() {
            let id = &b[i..i + 4];
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if id == b"data" {
                return b[i + 8..i + 8 + len]
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                    .collect();
            }
            i += 8 + len + (len & 1);
        }
        panic!("no data chunk");
    }

    #[test]
    #[ignore]
    fn download_translate_and_transcribe() {
        let root = models_dir();
        tauri::async_runtime::block_on(models::download(&root, models::MT, |r, t| {
            if r == t {
                eprintln!("mt model ready ({t} bytes)");
            }
        }))
        .unwrap();

        let t0 = Instant::now();
        let mut mt = mt::Translator::load(&models::spec(models::MT).unwrap().dir(&root)).unwrap();
        eprintln!("mt load {:?}", t0.elapsed());
        for s in ["Hyvää huomenta! Mitä sinulle kuuluu?", "Kiitos, minulle kuuluu oikein hyvää."] {
            let t0 = Instant::now();
            let en = mt.translate(s).unwrap();
            eprintln!("mt {:?}: {s} → {en}", t0.elapsed());
        }

        let pcm = read_wav("tests/fixtures/fi_sample.wav");
        let id = std::env::var("ASR").unwrap_or_else(|_| models::DEFAULT_ASR.into());
        let mut asr = asr::Transcriber::load(&models::weights_path(&root, &id).unwrap()).unwrap();
        asr.transcribe(&vec![0.0; 16000], true).unwrap(); // warm-up
        for draft in [false, true] {
            let t0 = Instant::now();
            let t = asr.transcribe(&pcm, draft).unwrap();
            let dt = t0.elapsed();
            let t1 = Instant::now();
            let en = mt.translate(&t.text).unwrap();
            eprintln!(
                "[{id} draft={draft}] {:.1}s audio: asr {dt:?}, mt {:?}\n  fi: {}\n  en: {en}",
                pcm.len() as f32 / 16000.0,
                t1.elapsed(),
                t.text
            );
            assert!(t.text.to_lowercase().contains("huomenta"));
        }
    }

    /// Streams the sample through VAD segmentation in real time, like a live capture would.
    #[test]
    #[ignore]
    fn live_pipeline_segments_and_translates() {
        use tauri::Listener;
        let root = models_dir();
        let app = tauri::test::mock_app();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        app.listen("utterance", move |e| {
            let _ = tx.send(e.payload().to_string());
        });

        let engine = Engine::default();
        let (asr, mt) = engine.prepare(&root, models::DEFAULT_ASR).unwrap();
        let settings = Arc::new(RwLock::new(Settings::default()));
        let (audio, _stop) =
            pipeline::spawn_workers(app.handle().clone(), &root, asr, mt, settings).unwrap();

        // Two passes of the sample with a pause in between, in 20 ms chunks at real time.
        let pcm = read_wav("tests/fixtures/fi_sample.wav");
        let silence = vec![0.0f32; 16_000 * 2];
        let started = Instant::now();
        for part in [&pcm[..], &silence[..], &pcm[..], &silence[..]] {
            for chunk in part.chunks(320) {
                audio.send(chunk.to_vec()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        drop(audio);

        let mut finals = 0;
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
            let v: serde_json::Value = serde_json::from_str(&ev).unwrap();
            eprintln!(
                "[{:>5.1}s] {} #{}: {}",
                started.elapsed().as_secs_f32(),
                if v["isFinal"].as_bool().unwrap() { "FINAL" } else { "draft" },
                v["id"],
                v["translation"]
            );
            if v["isFinal"].as_bool().unwrap() {
                finals += 1;
            }
        }
        assert!(finals >= 2, "expected at least one final utterance per pass");
    }

    /// Quitting mid-inference must free the Metal buffers before exit, otherwise ggml's static
    /// destructor aborts the process (GGML_ASSERT in ggml-metal-device.m). A failure here shows up
    /// as the test binary crashing *after* the test reports "ok".
    #[test]
    #[ignore]
    fn shutdown_releases_models_mid_inference() {
        let engine = Engine::default();
        let (asr, mt) = engine.prepare(&models_dir(), models::DEFAULT_ASR).unwrap();
        drop(mt);
        let worker = std::thread::spawn(move || {
            let noise: Vec<f32> = (0..16_000 * 28).map(|i| ((i * 7919) % 200) as f32 / 1000.0 - 0.1).collect();
            let _ = asr.lock().transcribe(&noise, false);
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        let t0 = Instant::now();
        assert!(engine.shutdown(Duration::from_secs(5)), "models still referenced");
        eprintln!("released in {:?}", t0.elapsed());
        worker.join().unwrap();
    }
}
