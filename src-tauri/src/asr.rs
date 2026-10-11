//! Finnish speech recognition with whisper.cpp (Metal on macOS, Vulkan on Windows).

use anyhow::{Context, Result};
use serde::Serialize;
use std::ffi::CStr;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

pub const SAMPLE_RATE: usize = 16_000;

pub struct Transcriber {
    // The state borrows model weights from the context, so keep it alive alongside.
    _ctx: WhisperContext,
    state: WhisperState,
}

pub struct Transcript {
    pub text: String,
    pub no_speech_prob: f32,
}

/// A GPU Whisper can run on. `index` counts GPUs only, in the order whisper.cpp numbers them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub index: i32,
    pub name: String,
    pub integrated: bool,
    /// Video memory in bytes; for integrated GPUs this is shared system memory.
    pub memory: u64,
}

/// GPUs available for inference. Empty on macOS, where there is one GPU and Metal picks it,
/// and on builds without a GPU backend.
pub fn list_gpus() -> Vec<GpuInfo> {
    if cfg!(target_os = "macos") {
        return Vec::new();
    }
    use whisper_rs::whisper_rs_sys as sys;
    let mut gpus = Vec::new();
    // SAFETY: ggml's device registry is built once, thread-safely, on first use and never
    // changes afterwards; the returned strings live as long as the registry.
    unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            let kind = sys::ggml_backend_dev_type(dev);
            let integrated = kind == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU;
            if kind != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU && !integrated {
                continue;
            }
            let name = CStr::from_ptr(sys::ggml_backend_dev_description(dev)).to_string_lossy().trim().to_string();
            let (mut free, mut total) = (0usize, 0usize);
            sys::ggml_backend_dev_memory(dev, &mut free, &mut total);
            gpus.push(GpuInfo { index: gpus.len() as i32, name, integrated, memory: total as u64 });
        }
    }
    gpus
}

/// Resolve a processor preference to a GPU index, or `None` for the CPU.
///
/// `pref` is `"auto"`, `"cpu"` or `"gpu:<index>"`. Auto prefers a dedicated GPU, then an
/// integrated one. A GPU that has disappeared (e.g. an unplugged eGPU) falls back to auto.
pub fn resolve_device(pref: &str) -> Option<i32> {
    if pref == "cpu" {
        return None;
    }
    if cfg!(target_os = "macos") {
        return Some(0);
    }
    let gpus = list_gpus();
    let chosen = pref.strip_prefix("gpu:").and_then(|i| i.parse::<i32>().ok());
    if let Some(i) = chosen.filter(|i| gpus.iter().any(|g| g.index == *i)) {
        return Some(i);
    }
    gpus.iter().find(|g| !g.integrated).or(gpus.first()).map(|g| g.index)
}

impl Transcriber {
    /// Load `model` onto GPU `gpu` (an index from [`list_gpus`]), or onto the CPU if `None`.
    pub fn load(model: &Path, gpu: Option<i32>) -> Result<Self> {
        let params = WhisperContextParameters {
            use_gpu: gpu.is_some(),
            gpu_device: gpu.unwrap_or(0),
            // Flash attention is slower here with a trimmed audio_ctx, and combined with
            // single-segment decoding it makes whisper.cpp emit garbage for this model.
            flash_attn: false,
            ..Default::default()
        };
        let ctx = WhisperContext::new_with_params(model, params)
            .with_context(|| format!("loading whisper model {}", model.display()))?;
        let state = ctx.create_state().context("creating whisper state")?;
        Ok(Self { _ctx: ctx, state })
    }

    /// Transcribe one utterance of 16 kHz mono audio.
    ///
    /// `draft` trades accuracy for latency; it's used for the live preview of an utterance
    /// that is still being spoken, which gets re-transcribed properly once it ends.
    pub fn transcribe(&mut self, pcm: &[f32], draft: bool) -> Result<Transcript> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("fi"));
        params.set_translate(false);
        params.set_n_threads(threads());
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if draft {
            params.set_temperature_inc(0.0);
        }
        // Bail out of a running inference when the app quits, so models can be freed promptly.
        params.set_abort_callback_safe(crate::pipeline::is_shutting_down);
        // Whisper's encoder always looks at a 30 s window. For short utterances, shrinking the
        // audio context makes encoding several times faster at a small accuracy cost.
        params.set_audio_ctx(audio_ctx_for(pcm.len()));

        // whisper.cpp refuses input shorter than 1 s; pad with a little silence.
        let min = SAMPLE_RATE + SAMPLE_RATE / 4;
        let padded;
        let pcm = if pcm.len() < min {
            padded = [pcm, &vec![0.0; min - pcm.len()]].concat();
            &padded[..]
        } else {
            pcm
        };

        self.state.full(params, pcm).context("whisper inference failed")?;

        let mut text = String::new();
        let mut no_speech_prob: f32 = 0.0;
        for segment in self.state.as_iter() {
            no_speech_prob = no_speech_prob.max(segment.no_speech_probability());
            if let Ok(s) = segment.to_str_lossy() {
                text.push_str(&s);
            }
        }
        Ok(Transcript { text: text.trim().to_string(), no_speech_prob })
    }
}

/// CPU threads for inference. ggml scales with physical cores and slows down on hyperthreads,
/// so assume two logical cores per physical one. Matters most where Whisper runs on the CPU.
fn threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(8, |n| n.get());
    (logical / 2).clamp(4, 8) as i32
}

/// Encoder frames needed for `samples` of audio (1500 frames = 30 s), with headroom.
///
/// Going much below the actual audio length makes Whisper hallucinate, and very small contexts
/// degrade quality, so this rounds up generously and never drops below ~10 s.
fn audio_ctx_for(samples: usize) -> i32 {
    let secs = samples as f32 / SAMPLE_RATE as f32;
    let frames = ((secs + 2.0) * 50.0).ceil() as i32;
    let frames = frames.clamp(512, 1500);
    // Keep it a multiple of 64 for nicer GPU kernels.
    ((frames + 63) / 64 * 64).min(1500)
}

/// Phrases Whisper is known to hallucinate on silence/music (it learned them from subtitle
/// credits in its training data). Matched case-insensitively against the whole utterance.
const HALLUCINATIONS: &[&str] = &[
    "kiitos katsomisesta",
    "kiitos kun katsoit",
    "kiitos, että katsoit",
    "tekstitys",
    "suomennos",
    "tilaa kanava",
    "thanks for watching",
    "subtitles by",
];

pub fn looks_like_hallucination(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return true;
    }
    if t.contains("www.") || t.contains(".com") || t.contains(".fi/") {
        return true;
    }
    HALLUCINATIONS.iter().any(|h| t.starts_with(h))
}
