//! Capture from a regular input device (microphone, BlackHole/Loopback, USB interface…) via cpal.

use super::resample::{downmix_interleaved, Resampler};
use super::{Capture, SampleSink, SourceInfo, SourceKind};
use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};
use std::sync::mpsc;

pub const ID_PREFIX: &str = "input:";

pub fn list() -> Vec<SourceInfo> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| d.id().ok());
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|d| {
            let id = d.id().ok()?;
            let name = d.description().ok()?.name().to_string();
            Some(SourceInfo {
                is_default: default_id.as_ref() == Some(&id),
                id: format!("{ID_PREFIX}{id}"),
                name,
                kind: SourceKind::Input,
            })
        })
        .collect()
}

pub fn start(device_id: &str, sink: SampleSink) -> Result<Capture> {
    let id: cpal::DeviceId = device_id.parse().map_err(|e| anyhow!("bad device id: {e}"))?;
    run(sink, move || {
        let device = cpal::default_host()
            .device_by_id(&id)
            .context("audio device not found — was it unplugged?")?;
        let config = device.default_input_config().context("reading device format")?;
        Ok((device, config))
    })
}

/// Windows: record what's playing on the default output device. WASAPI switches to loopback
/// capture when an input stream is opened on an output device; no virtual driver needed.
///
/// Loopback delivers nothing while nothing plays, but the segmenter needs to hear the pause to
/// end a sentence, so gaps are filled with silence.
#[cfg(windows)]
pub fn start_loopback(sink: SampleSink) -> Result<Capture> {
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const TICK: Duration = Duration::from_millis(20);
    const GAP: Duration = Duration::from_millis(100);
    let sink = Arc::new(Mutex::new(sink));
    let last = Arc::new(Mutex::new(Instant::now()));
    let feed: SampleSink = {
        let (sink, last) = (sink.clone(), last.clone());
        Box::new(move |samples: &[f32]| {
            *last.lock() = Instant::now();
            (sink.lock())(samples);
        })
    };
    let capture = run(feed, || {
        let device = cpal::default_host()
            .default_output_device()
            .context("no audio output device")?;
        let config = device.default_output_config().context("reading output format")?;
        Ok((device, config))
    })?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let silence = vec![0.0f32; crate::asr::SAMPLE_RATE * TICK.as_millis() as usize / 1000];
        std::thread::Builder::new().name("loopback-silence".into()).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                if last.lock().elapsed() >= GAP {
                    (sink.lock())(&silence);
                }
            }
        })?;
    }
    Ok(Capture::new(move || {
        stop.store(true, Ordering::Relaxed);
        drop(capture);
    }))
}

/// Open a device and stream it into `sink` until the returned Capture is dropped.
fn run<F>(sink: SampleSink, open: F) -> Result<Capture>
where
    F: FnOnce() -> Result<(cpal::Device, cpal::SupportedStreamConfig)> + Send + 'static,
{
    // cpal streams aren't Send on every platform, so the stream lives on its own thread for
    // as long as the capture handle does.
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("audio-input".into())
        .spawn(move || {
            let stream = match open().and_then(|(device, config)| build_stream(&device, config, sink)) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            // Blocks until the Capture is dropped (sender disconnects).
            let _ = stop_rx.recv();
            drop(stream);
        })?;
    ready_rx.recv().context("audio thread exited")??;
    Ok(Capture::new(move || drop(stop_tx)))
}

fn build_stream(device: &cpal::Device, supported: cpal::SupportedStreamConfig, sink: SampleSink) -> Result<cpal::Stream> {
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(device, &config, sink)?,
        SampleFormat::I16 => build::<i16>(device, &config, sink)?,
        SampleFormat::I32 => build::<i32>(device, &config, sink)?,
        SampleFormat::U16 => build::<u16>(device, &config, sink)?,
        f => return Err(anyhow!("unsupported sample format {f:?}")),
    };
    stream.play().context("starting audio stream")?;
    Ok(stream)
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, mut sink: SampleSink) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let mut resampler = Resampler::new(config.sample_rate, crate::asr::SAMPLE_RATE as u32);
    let (mut as_f32, mut mono, mut out) = (Vec::new(), Vec::new(), Vec::new());
    let stream = device.build_input_stream(
        *config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            as_f32.clear();
            as_f32.extend(data.iter().map(|&s| <f32 as cpal::FromSample<T>>::from_sample_(s)));
            mono.clear();
            downmix_interleaved(&as_f32, channels, &mut mono);
            out.clear();
            resampler.process(&mono, &mut out);
            if !out.is_empty() {
                sink(&out);
            }
        },
        |err| log::error!("audio input error: {err}"),
        None,
    )?;
    Ok(stream)
}
