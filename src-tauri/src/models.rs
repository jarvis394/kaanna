//! Model catalog and downloader. Everything runs locally; this is the only network access the
//! app makes. Downloads are pinned to exact Hugging Face commits and verified by SHA-256.

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Asr,
    Mt,
    Vad,
}

pub struct ModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: Option<&'static str>,
}

pub struct ModelSpec {
    pub id: &'static str,
    pub kind: ModelKind,
    pub name: &'static str,
    pub description: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [ModelFile],
}

impl ModelSpec {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn dir(&self, root: &Path) -> PathBuf {
        root.join(self.id)
    }

    /// Cheap check (sizes only); full hashes are verified once, at download time.
    pub fn is_installed(&self, root: &Path) -> bool {
        let dir = self.dir(root);
        self.files.iter().all(|f| {
            std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.len() == f.size)
        })
    }
}

pub const DEFAULT_ASR: &str = "asr-fi-large-v3-q5";
pub const MT: &str = "mt-opus-fi-en";
pub const VAD: &str = "vad-silero";

pub const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "asr-fi-large-v3-q5",
        kind: ModelKind::Asr,
        name: "Whisper Large v3 Finnish · Q5",
        description: "Finnish-NLP fine-tune, 5-bit quantised (community conversion). Best speed/quality on Apple Silicon.",
        repo: "cerkku/whisper-large-finnish-v3-ggml",
        revision: "d660543b0e6cb492b7e03255d5bc02a7565dede5",
        files: &[ModelFile {
            name: "ggml-whisper-large-finnish-v3-q5_0.bin",
            size: 1_081_140_203,
            sha256: Some("7c7a5cc5064ca96dc73cfa164c9044d87ee843b8782fc69040b6330569f12b9f"),
        }],
    },
    ModelSpec {
        id: "asr-fi-large-v3",
        kind: ModelKind::Asr,
        name: "Whisper Large v3 Finnish · F16",
        description: "Official Finnish-NLP whisper.cpp conversion, full precision. Most accurate, needs ~4 GB RAM.",
        repo: "Finnish-NLP/Finnish-finetuned-whisper-models-ggml-format",
        revision: "c58924b6deb4438756b3d38ecd67d65bdf20298d",
        files: &[ModelFile {
            name: "ggml-model-fi-large-v3.bin",
            size: 3_095_033_483,
            sha256: Some("89536e66c7b521189b55cd010c71faf50c447d281a38cf3f6ba616a5f4b78d2e"),
        }],
    },
    ModelSpec {
        id: "asr-fi-medium",
        kind: ModelKind::Asr,
        name: "Whisper Medium Finnish",
        description: "Official Finnish-NLP medium fine-tune. Roughly twice as fast, somewhat less accurate.",
        repo: "Finnish-NLP/Finnish-finetuned-whisper-models-ggml-format",
        revision: "c58924b6deb4438756b3d38ecd67d65bdf20298d",
        files: &[ModelFile {
            name: "ggml-model-fi-medium.bin",
            size: 1_533_763_059,
            sha256: Some("a0d1126381bebc9155e6a06a36ae4bf8e87abd02eacf73fff1593719a7d6c739"),
        }],
    },
    ModelSpec {
        id: MT,
        kind: ModelKind::Mt,
        name: "OPUS-MT Finnish → English (big)",
        description: "Helsinki-NLP transformer-big Marian model.",
        repo: "Helsinki-NLP/opus-mt-tc-big-fi-en",
        revision: "956e13710061738d21f677e4ed16a56546511c0f",
        files: &[
            ModelFile { name: "config.json", size: 1_076, sha256: None },
            ModelFile { name: "vocab.json", size: 1_458_135, sha256: None },
            ModelFile {
                name: "source.spm",
                size: 831_865,
                sha256: Some("82fcb74f79c5f5b5bf0fe73f7dfb6a646efa50ffb4b2a0cd564a1193c78a032e"),
            },
            ModelFile {
                name: "model.safetensors",
                size: 471_295_780,
                sha256: Some("78b81c43b0752c41facfb4a7f2f007e3085f38289d3ddf0e37ac45b9c05ee19b"),
            },
        ],
    },
    ModelSpec {
        id: VAD,
        kind: ModelKind::Vad,
        name: "Silero VAD v6",
        description: "Voice activity detection, used to split speech into sentences.",
        repo: "ggml-org/whisper-vad",
        revision: "9ffd54a1e1ee413ddf265af9913beaf518d1639b",
        files: &[ModelFile {
            name: "ggml-silero-v6.2.0.bin",
            size: 885_098,
            sha256: Some("2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987"),
        }],
    },
];

pub fn spec(id: &str) -> Result<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id).with_context(|| format!("unknown model {id}"))
}

/// Path of the single weights file for a one-file model (ASR, VAD).
pub fn weights_path(root: &Path, id: &str) -> Result<PathBuf> {
    let spec = spec(id)?;
    if !spec.is_installed(root) {
        bail!("{} is not downloaded yet", spec.name);
    }
    Ok(spec.dir(root).join(spec.files[0].name))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: &'static str,
    pub kind: ModelKind,
    pub name: &'static str,
    pub description: &'static str,
    pub size: u64,
    pub installed: bool,
}

pub fn statuses(root: &Path) -> Vec<ModelStatus> {
    CATALOG
        .iter()
        .map(|m| ModelStatus {
            id: m.id,
            kind: m.kind,
            name: m.name,
            description: m.description,
            size: m.size(),
            installed: m.is_installed(root),
        })
        .collect()
}

/// Download every missing file of a model, calling `progress(received, total)` periodically.
/// Partial downloads (`*.part`) are resumed.
pub async fn download(root: &Path, id: &str, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let spec = spec(id)?;
    let dir = spec.dir(root);
    tokio::fs::create_dir_all(&dir).await?;
    let total = spec.size();
    let client = reqwest::Client::builder()
        .user_agent(concat!("kaanna/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()?;

    let mut done_before = 0u64;
    for file in spec.files {
        let dest = dir.join(file.name);
        if std::fs::metadata(&dest).is_ok_and(|m| m.len() == file.size) {
            done_before += file.size;
            progress(done_before, total);
            continue;
        }
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            spec.repo, spec.revision, file.name
        );
        download_file(&client, &url, &dest, file, |n| progress(done_before + n, total))
            .await
            .with_context(|| format!("downloading {}", file.name))?;
        done_before += file.size;
    }
    progress(total, total);
    Ok(())
}

/// Delete a model's files, including any partial download.
pub async fn delete(root: &Path, id: &str) -> Result<()> {
    let dir = spec(id)?.dir(root);
    match tokio::fs::remove_dir_all(&dir).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).context("deleting model files"),
        _ => Ok(()),
    }
}

/// Remove the `*.part` files left by an interrupted download.
pub async fn discard_partial(root: &Path, id: &str) {
    let Ok(spec) = spec(id) else { return };
    let dir = spec.dir(root);
    for file in spec.files {
        tokio::fs::remove_file(part_path(&dir.join(file.name))).await.ok();
    }
}

fn part_path(dest: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", dest.display()))
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    file: &ModelFile,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let part = part_path(dest);
    let mut hasher = Sha256::new();

    // Resume: re-hash what we already have, then ask for the rest.
    let mut have = 0u64;
    if let Ok(mut f) = tokio::fs::File::open(&part).await {
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            have += n as u64;
        }
    }
    if have > file.size {
        tokio::fs::remove_file(&part).await?;
        hasher = Sha256::new();
        have = 0;
    }

    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await?.error_for_status()?;
    let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if have > 0 && !resumed {
        // Server ignored the range; start over.
        hasher = Sha256::new();
        have = 0;
    }
    let mut out = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await?;

    let mut stream = resp.bytes_stream();
    let mut last_report = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        out.write_all(&chunk).await?;
        have += chunk.len() as u64;
        if last_report.elapsed() > Duration::from_millis(200) {
            progress(have);
            last_report = Instant::now();
        }
    }
    out.flush().await?;
    drop(out);

    if have != file.size {
        bail!("size mismatch: got {have} bytes, expected {}", file.size);
    }
    if let Some(expected) = file.sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            tokio::fs::remove_file(&part).await.ok();
            bail!("checksum mismatch (expected {expected}, got {actual})");
        }
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}
