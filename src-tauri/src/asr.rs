//! Finnish speech recognition with whisper.cpp (Metal-accelerated on Apple Silicon).

use anyhow::{Context, Result};
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

impl Transcriber {
    pub fn load(model: &Path) -> Result<Self> {
        let params = WhisperContextParameters {
            use_gpu: true,
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
        params.set_n_threads(4);
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
