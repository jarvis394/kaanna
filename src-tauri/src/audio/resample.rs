//! Streaming downmix + resample to 16 kHz mono for Whisper.
//!
//! Speech recognition is forgiving, so a box-filter (anti-aliasing) followed by linear
//! interpolation is plenty and avoids a DSP dependency.

pub struct Resampler {
    step: f64,
    pos: f64,
    filtered: Vec<f32>,
    window: std::collections::VecDeque<f32>,
    window_len: usize,
    window_sum: f32,
}

impl Resampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Self {
        let step = f64::from(input_rate) / f64::from(output_rate);
        Self {
            step,
            pos: 0.0,
            filtered: Vec::new(),
            window: Default::default(),
            window_len: step.round().max(1.0) as usize,
            window_sum: 0.0,
        }
    }

    /// Push mono samples, appending resampled output to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.window_len == 1 && (self.step - 1.0).abs() < f64::EPSILON {
            out.extend_from_slice(input);
            return;
        }
        for &x in input {
            self.window.push_back(x);
            self.window_sum += x;
            if self.window.len() > self.window_len {
                self.window_sum -= self.window.pop_front().unwrap_or(0.0);
            }
            self.filtered.push(self.window_sum / self.window.len() as f32);
        }
        while self.pos + 1.0 < self.filtered.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(self.filtered[i] * (1.0 - frac) + self.filtered[i + 1] * frac);
            self.pos += self.step;
        }
        let consumed = (self.pos as usize).min(self.filtered.len());
        self.filtered.drain(..consumed);
        self.pos -= consumed as f64;
        // Guard against float drift in long sessions.
        if self.window_sum.abs() > 1e3 {
            self.window_sum = self.window.iter().sum();
        }
    }
}

/// Average interleaved frames down to mono.
pub fn downmix_interleaved(data: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels <= 1 {
        out.extend_from_slice(data);
        return;
    }
    let scale = 1.0 / channels as f32;
    out.extend(data.chunks_exact(channels).map(|f| f.iter().sum::<f32>() * scale));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_48k_to_16k_at_the_right_rate() {
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = Vec::new();
        // Feed 1 s in uneven chunks the way audio callbacks do.
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        for chunk in tone.chunks(477) {
            r.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 16_000).abs() <= 2, "{}", out.len());
    }

    #[test]
    fn passthrough_at_16k() {
        let mut r = Resampler::new(16_000, 16_000);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }
}
