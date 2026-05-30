use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: &'static str,
    pub filename: &'static str,
    pub size_mb: u32,
}

pub static MODELS: &[ModelInfo] = &[
    ModelInfo { name: "tiny.en",   filename: "ggml-tiny.en.bin",   size_mb: 75  },
    ModelInfo { name: "base.en",   filename: "ggml-base.en.bin",   size_mb: 142 },
    ModelInfo { name: "small.en",  filename: "ggml-small.en.bin",  size_mb: 466 },
    ModelInfo { name: "medium.en", filename: "ggml-medium.en.bin", size_mb: 1457 },
    ModelInfo { name: "tiny",      filename: "ggml-tiny.bin",      size_mb: 75  },
    ModelInfo { name: "base",      filename: "ggml-base.bin",      size_mb: 142 },
    ModelInfo { name: "small",     filename: "ggml-small.bin",     size_mb: 466 },
    ModelInfo { name: "medium",    filename: "ggml-medium.bin",    size_mb: 1457 },
];

pub fn model_dir(app: &AppHandle) -> Result<PathBuf> {
    let dir = app.path().app_data_dir()?.join("models");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn model_path(app: &AppHandle, filename: &str) -> Result<PathBuf> {
    Ok(model_dir(app)?.join(filename))
}

pub fn is_downloaded(app: &AppHandle, model: &ModelInfo) -> bool {
    model_path(app, model.filename)
        .map(|p| p.exists())
        .unwrap_or(false)
}

pub fn find_model(name: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.name == name)
}

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub model: String,
    pub downloaded: u64,
    pub total: u64,
    pub done: bool,
}

pub async fn download_model(app: AppHandle, model_name: String) -> Result<PathBuf> {
    let model = find_model(&model_name)
        .ok_or_else(|| anyhow::anyhow!("unknown model: {model_name}"))?;

    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        model.filename
    );

    let dir = model_dir(&app)?;
    let tmp_path = dir.join(format!("{}.tmp", model.filename));
    let final_path = dir.join(model.filename);

    if final_path.exists() {
        return Ok(final_path);
    }

    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(&tmp_path).await?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                model: model_name.clone(),
                downloaded,
                total,
                done: false,
            },
        );
    }

    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp_path, &final_path).await?;

    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            model: model_name,
            downloaded,
            total,
            done: true,
        },
    );

    Ok(final_path)
}

pub fn delete_model(app: &AppHandle, model_name: &str) -> Result<()> {
    let model = find_model(model_name)
        .ok_or_else(|| anyhow::anyhow!("unknown model: {model_name}"))?;
    let path = model_path(app, model.filename)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
