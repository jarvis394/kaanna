//! Minimal SentencePiece *unigram* encoder.
//!
//! Marian (OPUS-MT) checkpoints ship a raw `source.spm` protobuf. Rather than
//! pull in the C++ sentencepiece library we read the piece table straight out
//! of the protobuf and run the unigram Viterbi segmentation ourselves — it's
//! the only part of sentencepiece the encoder side needs.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

pub const WORD_BOUNDARY: char = '\u{2581}'; // ▁

/// Penalty applied to out-of-vocabulary characters, same constant as sentencepiece.
const UNK_PENALTY: f32 = 10.0;

pub struct Unigram {
    scores: HashMap<String, f32>,
    max_piece_chars: usize,
    unk_score: f32,
}

impl Unigram {
    pub fn from_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_proto(&bytes)
    }

    fn from_proto(buf: &[u8]) -> Result<Self> {
        let mut scores = HashMap::new();
        let mut max_piece_chars = 1;
        let mut min_score = f32::MAX;

        let mut r = Reader::new(buf);
        while !r.done() {
            let (field, wire) = r.key()?;
            // ModelProto.pieces = 1 (repeated SentencePiece)
            if field == 1 && wire == 2 {
                let (piece, score, kind) = parse_piece(r.bytes()?)?;
                // 1 = NORMAL, 4 = USER_DEFINED. Control/unknown/byte pieces are never emitted
                // by segmentation.
                if kind == 1 || kind == 4 {
                    max_piece_chars = max_piece_chars.max(piece.chars().count());
                    min_score = min_score.min(score);
                    scores.insert(piece, score);
                }
            } else {
                r.skip(wire)?;
            }
        }
        if scores.is_empty() {
            bail!("sentencepiece model contains no pieces");
        }
        Ok(Self {
            scores,
            max_piece_chars,
            unk_score: min_score - UNK_PENALTY,
        })
    }

    /// Normalise and segment `text` into sentencepiece pieces (e.g. `▁Hyvää`, `▁huomen`, `ta`).
    pub fn encode(&self, text: &str) -> Vec<String> {
        let normalized: String = text.nfkc().collect();
        let mut pieces = Vec::new();
        for word in normalized.split_whitespace() {
            let word: Vec<char> = std::iter::once(WORD_BOUNDARY).chain(word.chars()).collect();
            self.viterbi(&word, &mut pieces);
        }
        pieces
    }

    fn viterbi(&self, chars: &[char], out: &mut Vec<String>) {
        let n = chars.len();
        // best[i] = (score, start of last piece) for the prefix chars[..i]
        let mut best: Vec<(f32, usize)> = vec![(f32::NEG_INFINITY, 0); n + 1];
        best[0] = (0.0, 0);
        let mut candidate = String::new();
        for end in 1..=n {
            let lo = end.saturating_sub(self.max_piece_chars);
            for start in lo..end {
                let base = best[start].0;
                if base == f32::NEG_INFINITY {
                    continue;
                }
                candidate.clear();
                candidate.extend(&chars[start..end]);
                let score = match self.scores.get(&candidate) {
                    Some(s) => *s,
                    // A single unknown character is always allowed, so a path always exists.
                    None if end - start == 1 => self.unk_score,
                    None => continue,
                };
                if base + score > best[end].0 {
                    best[end] = (base + score, start);
                }
            }
        }
        let mut spans = Vec::new();
        let mut end = n;
        while end > 0 {
            let start = best[end].1;
            spans.push((start, end));
            end = start;
        }
        out.extend(spans.into_iter().rev().map(|(s, e)| chars[s..e].iter().collect()));
    }
}

fn parse_piece(buf: &[u8]) -> Result<(String, f32, u64)> {
    let mut r = Reader::new(buf);
    let (mut piece, mut score, mut kind) = (String::new(), 0.0, 1);
    while !r.done() {
        match r.key()? {
            (1, 2) => piece = String::from_utf8(r.bytes()?.to_vec())?,
            (2, 5) => score = f32::from_le_bytes(r.fixed32()?),
            (3, 0) => kind = r.varint()?,
            (_, wire) => r.skip(wire)?,
        }
    }
    Ok((piece, score, kind))
}

/// Just enough of the protobuf wire format to walk a sentencepiece ModelProto.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> Result<u64> {
        let mut out = 0u64;
        for shift in (0..64).step_by(7) {
            let Some(&b) = self.buf.get(self.pos) else {
                bail!("truncated varint")
            };
            self.pos += 1;
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(out);
            }
        }
        bail!("varint too long")
    }

    fn key(&mut self) -> Result<(u64, u8)> {
        let k = self.varint()?;
        Ok((k >> 3, (k & 7) as u8))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len());
        let Some(end) = end else { bail!("truncated field") };
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.varint()? as usize;
        self.take(n)
    }

    fn fixed32(&mut self) -> Result<[u8; 4]> {
        Ok(self.take(4)?.try_into()?)
    }

    fn skip(&mut self, wire: u8) -> Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => {
                self.take(8)?;
            }
            2 => {
                self.bytes()?;
            }
            5 => {
                self.take(4)?;
            }
            w => bail!("unsupported protobuf wire type {w}"),
        }
        Ok(())
    }
}
