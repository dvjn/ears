// Model catalog and on-demand download of whisper.cpp ggml model files.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const WHISPER_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
const VAD_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin";
const VAD_FILE: &str = "ggml-silero-v5.1.2.bin";

/// API model ids. Each maps to `ggml-<id without "whisper-">.bin`.
pub const MODELS: &[&str] = &[
    "whisper-tiny",
    "whisper-tiny.en",
    "whisper-base",
    "whisper-base.en",
    "whisper-small",
    "whisper-small.en",
    "whisper-small.en-q5_1",
    "whisper-medium",
    "whisper-medium.en",
    "whisper-medium.en-q5_0",
    "whisper-large-v3",
    "whisper-large-v3-turbo",
    "whisper-large-v3-turbo-q5_0",
    "whisper-large-v3-turbo-q8_0",
];

pub fn is_known(id: &str) -> bool {
    MODELS.contains(&id)
}

/// Model size without prefix or quantization suffix: "whisper-small.en-q5_1"
/// -> "small.en".
pub fn size(id: &str) -> &str {
    let name = id.trim_start_matches("whisper-");
    match name.rsplit_once("-q") {
        Some((base, _)) => base,
        None => name,
    }
}

fn file_name(id: &str) -> String {
    format!("ggml-{}.bin", id.trim_start_matches("whisper-"))
}

/// Path of a model file, downloading it first if it isn't there yet.
pub async fn ensure_model(dir: &Path, id: &str) -> Result<PathBuf> {
    let name = file_name(id);
    ensure(dir, &name, &format!("{WHISPER_BASE_URL}/{name}")).await
}

pub async fn ensure_vad(dir: &Path) -> Result<PathBuf> {
    ensure(dir, VAD_FILE, VAD_URL).await
}

async fn ensure(dir: &Path, name: &str, url: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    if path.exists() {
        return Ok(path);
    }
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;

    eprintln!("downloading {url}");
    let resp = reqwest::get(url).await?.error_for_status()?;
    let part = dir.join(format!("{name}.part"));
    let mut file = tokio::fs::File::create(&part).await?;
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, &path)
        .await
        .map_err(|e| anyhow!("saving {}: {e}", path.display()))?;
    eprintln!("saved {}", path.display());
    Ok(path)
}
