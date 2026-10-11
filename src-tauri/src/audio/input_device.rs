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
    // cpal streams aren't Send on every platform, so the stream lives on its own thread for
    // as long as the capture handle does.
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("audio-input".into())
        .spawn(move || {
            let stream = match build_stream(&id, sink) {
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

fn build_stream(id: &cpal::DeviceId, sink: SampleSink) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .device_by_id(id)
        .context("audio device not found — was it unplugged?")?;
    let supported = device.default_input_config().context("reading device format")?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, sink)?,
        SampleFormat::I16 => build::<i16>(&device, &config, sink)?,
        SampleFormat::I32 => build::<i32>(&device, &config, sink)?,
        SampleFormat::U16 => build::<u16>(&device, &config, sink)?,
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
