//! Finnish → English machine translation with an OPUS-MT Marian model on candle.

use crate::spm::{Unigram, WORD_BOUNDARY};
use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::marian::{Config, MTModel};
use std::collections::HashMap;
use std::path::Path;

const MAX_POSITIONS: usize = 512;

pub struct Translator {
    model: MTModel,
    cfg: Config,
    spm: Unigram,
    vocab: HashMap<String, u32>,
    pieces: Vec<String>,
    unk_id: u32,
    device: Device,
}

impl Translator {
    pub fn load(dir: &Path) -> Result<Self> {
        let cfg: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)
            .context("parsing Marian config.json")?;
        let vocab: HashMap<String, u32> =
            serde_json::from_slice(&std::fs::read(dir.join("vocab.json"))?)
                .context("parsing Marian vocab.json")?;
        let mut pieces = vec![String::new(); vocab.values().max().map_or(0, |m| *m as usize + 1)];
        for (piece, &id) in &vocab {
            pieces[id as usize] = piece.clone();
        }
        let unk_id = *vocab.get("<unk>").context("vocab has no <unk>")?;
        let spm = Unigram::from_file(&dir.join("source.spm"))?;

        let device = Device::Cpu;
        // SAFETY: the file is only mmapped for the lifetime of loading; candle copies tensors
        // out when converting from the stored f16 to f32.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DType::F32, &device)?
        };
        let model = MTModel::new(&cfg, vb).context("building Marian model")?;
        Ok(Self { model, cfg, spm, vocab, pieces, unk_id, device })
    }

    /// Translate a chunk of Finnish text. Sentences are translated one at a time, which keeps
    /// Marian well inside the lengths it was trained on.
    pub fn translate(&mut self, text: &str) -> Result<String> {
        let mut out = Vec::new();
        for sentence in split_sentences(text) {
            let t = self.translate_sentence(sentence)?;
            if !t.is_empty() {
                out.push(t);
            }
        }
        Ok(out.join(" "))
    }

    fn translate_sentence(&mut self, text: &str) -> Result<String> {
        let eos = self.cfg.eos_token_id;
        let pad = self.cfg.pad_token_id as usize;

        let mut ids: Vec<u32> = self
            .spm
            .encode(text)
            .iter()
            .map(|p| self.vocab.get(p).copied().unwrap_or(self.unk_id))
            .collect();
        if ids.is_empty() {
            return Ok(String::new());
        }
        ids.truncate(MAX_POSITIONS - 1);
        ids.push(eos);

        self.model.reset_kv_cache();
        let input = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let encoded = self.model.encoder().forward(&input, 0)?;

        let mut tokens = vec![self.cfg.decoder_start_token_id];
        let max_len = (ids.len() * 2 + 16).min(MAX_POSITIONS);
        for step in 0..max_len {
            // First step feeds the start token; afterwards the KV cache holds the prefix.
            let ctx = if step == 0 { tokens.len() } else { 1 };
            let start = tokens.len() - ctx;
            let input = Tensor::new(&tokens[start..], &self.device)?.unsqueeze(0)?;
            let logits = self.model.decode(&input, &encoded, start)?.squeeze(0)?;
            let mut logits: Vec<f32> = logits.get(logits.dim(0)? - 1)?.to_vec1()?;
            // `bad_words_ids` in the generation config: never emit <pad>.
            logits[pad] = f32::NEG_INFINITY;
            let next = argmax(&logits);
            if next == eos || next == self.cfg.forced_eos_token_id {
                break;
            }
            tokens.push(next);
        }
        self.model.reset_kv_cache();
        Ok(self.detokenize(&tokens[1..]))
    }

    fn detokenize(&self, ids: &[u32]) -> String {
        let mut s = String::new();
        for &id in ids {
            if id == self.unk_id {
                continue;
            }
            if let Some(p) = self.pieces.get(id as usize) {
                s.push_str(p);
            }
        }
        s.replace(WORD_BOUNDARY, " ").trim().to_string()
    }
}

fn argmax(v: &[f32]) -> u32 {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if x > v[best] {
            best = i;
        }
    }
    best as u32
}

/// Split after sentence-final punctuation followed by whitespace.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev_terminal = false;
    for (i, c) in text.char_indices() {
        if prev_terminal && c.is_whitespace() {
            let s = text[start..i].trim();
            if !s.is_empty() {
                out.push(s);
            }
            start = i;
        }
        prev_terminal = matches!(c, '.' | '!' | '?' | '…');
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_sentences;

    #[test]
    fn splits_on_terminal_punctuation() {
        assert_eq!(
            split_sentences("Hei! Mitä kuuluu? Hyvää kiitos.  Entä sinulle"),
            vec!["Hei!", "Mitä kuuluu?", "Hyvää kiitos.", "Entä sinulle"]
        );
        assert_eq!(split_sentences("klo 12.30 alkaen"), vec!["klo 12.30 alkaen"]);
    }
}
