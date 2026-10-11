//! Audio sources. Every source delivers 16 kHz mono f32 to a sink callback.

mod input_device;
mod resample;
#[cfg(target_os = "macos")]
mod system_audio;

use anyhow::{bail, Result};
use serde::Serialize;

#[cfg(any(target_os = "macos", windows))]
const SYSTEM_ID: &str = "system";

pub type SampleSink = Box<dyn FnMut(&[f32]) + Send + 'static>;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// System-audio capture: ScreenCaptureKit on macOS, WASAPI loopback on Windows.
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    System,
    Input,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub is_default: bool,
}

/// A running capture. Dropping it stops the audio.
pub struct Capture {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl Capture {
    fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        Self { stop: Some(Box::new(stop)) }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

pub fn list_sources() -> Vec<SourceInfo> {
    let mut sources = Vec::new();
    #[cfg(any(target_os = "macos", windows))]
    sources.push(SourceInfo {
        id: SYSTEM_ID.into(),
        name: "System audio".into(),
        kind: SourceKind::System,
        is_default: false,
    });
    sources.extend(input_device::list());
    sources
}

pub fn start(source_id: &str, sink: SampleSink) -> Result<Capture> {
    #[cfg(target_os = "macos")]
    if source_id == SYSTEM_ID {
        return system_audio::start(sink);
    }
    #[cfg(windows)]
    if source_id == SYSTEM_ID {
        return input_device::start_loopback(sink);
    }
    if let Some(id) = source_id.strip_prefix(input_device::ID_PREFIX) {
        return input_device::start(id, sink);
    }
    bail!("unknown audio source {source_id}")
}
