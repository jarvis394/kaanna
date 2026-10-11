//! System-wide audio capture on macOS via ScreenCaptureKit (macOS 13+).
//!
//! Requires the "Screen & System Audio Recording" permission. No virtual audio driver needed.

use super::resample::{downmix_interleaved, Resampler};
use super::{Capture, SampleSink};
use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use screencapturekit::prelude::*;

/// kAudioFormatFlagIsNonInterleaved
const NON_INTERLEAVED: u32 = 1 << 5;

struct AudioHandler {
    state: Mutex<HandlerState>,
}

struct HandlerState {
    sink: SampleSink,
    resampler: Option<(u32, Resampler)>,
    mono: Vec<f32>,
    out: Vec<f32>,
}

impl SCStreamOutputTrait for AudioHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if of_type != SCStreamOutputType::Audio {
            return;
        }
        let Some(fmt) = sample.format_description() else { return };
        if !fmt.audio_is_float() {
            log::warn!("unexpected non-float system audio");
            return;
        }
        let rate = fmt.audio_sample_rate().unwrap_or(48_000.0) as u32;
        let channels = fmt.audio_channel_count().unwrap_or(1) as usize;
        let non_interleaved = fmt.audio_format_flags().unwrap_or(0) & NON_INTERLEAVED != 0;
        let Ok(list) = sample.audio_buffer_list() else { return };

        let mut st = self.state.lock();
        let st = &mut *st;
        st.mono.clear();
        let buffers: Vec<Vec<f32>> = (&list).into_iter().map(|b| to_f32(b.data())).collect();
        if non_interleaved && buffers.len() > 1 {
            // One buffer per channel: average them.
            let n = buffers.iter().map(Vec::len).min().unwrap_or(0);
            let scale = 1.0 / buffers.len() as f32;
            st.mono.extend((0..n).map(|i| buffers.iter().map(|b| b[i]).sum::<f32>() * scale));
        } else if let Some(b) = buffers.first() {
            downmix_interleaved(b, if non_interleaved { 1 } else { channels }, &mut st.mono);
        }

        if st.resampler.as_ref().is_none_or(|(r, _)| *r != rate) {
            st.resampler = Some((rate, Resampler::new(rate, crate::asr::SAMPLE_RATE as u32)));
        }
        st.out.clear();
        if let Some((_, r)) = st.resampler.as_mut() {
            r.process(&st.mono, &mut st.out);
        }
        if !st.out.is_empty() {
            (st.sink)(&st.out);
        }
    }
}

fn to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Video frames we don't want; registering a handler stops ScreenCaptureKit from logging
/// "stream output NOT found" for every dropped frame.
struct DiscardFrames;
impl SCStreamOutputTrait for DiscardFrames {
    fn did_output_sample_buffer(&self, _: CMSampleBuffer, _: SCStreamOutputType) {}
}

pub fn start(sink: SampleSink) -> Result<Capture> {
    let content = SCShareableContent::get().map_err(|e| {
        anyhow!(
            "Can't access system audio ({e}). Allow Käännä under System Settings → Privacy & \
             Security → Screen & System Audio Recording, then restart the app."
        )
    })?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .context("no display found to attach the audio capture to")?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build()
        .map_err(|e| anyhow!("creating capture filter: {e}"))?;

    // We only want audio, so ask for the tiniest, slowest video stream possible.
    let config = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_minimum_frame_interval(&CMTime::new(1, 1))
        .with_captures_audio(true)
        .with_excludes_current_process_audio(true)
        .with_sample_rate(48_000)
        .with_channel_count(2);

    let mut stream = SCStream::new(&filter, &config).map_err(|e| anyhow!("creating capture stream: {e}"))?;
    let handler = AudioHandler {
        state: Mutex::new(HandlerState { sink, resampler: None, mono: Vec::new(), out: Vec::new() }),
    };
    stream
        .add_output_handler(handler, SCStreamOutputType::Audio)
        .map_err(|e| anyhow!("adding audio output: {e}"))?;
    stream
        .add_output_handler(DiscardFrames, SCStreamOutputType::Screen)
        .map_err(|e| anyhow!("adding screen output: {e}"))?;
    stream
        .start_capture()
        .map_err(|e| anyhow!("starting system audio capture: {e}"))?;

    Ok(Capture::new(move || {
        if let Err(e) = stream.stop_capture() {
            log::warn!("stopping system audio capture: {e}");
        }
    }))
}
