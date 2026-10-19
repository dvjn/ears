// On-screen text extraction, used as optional context for the post-processing
// clean-up pass.
//
// Captures the primary monitor and runs the `ocrs` OCR engine over it. The two
// model files (~10MB total) are downloaded once on first use and cached in the
// app data dir, so the shipped binary stays small and users who never turn on
// the screenshot option never pay for them.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use tauri::{AppHandle, Manager};

use crate::state::CaptureMethod;

const DETECTION_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten";
const RECOGNITION_URL: &str =
    "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten";

/// Capture the primary screen and return the text found on it. Best-effort: the
/// caller logs and ignores any error (dictation still works without context).
pub async fn screen_text(app: &AppHandle, method: CaptureMethod) -> Result<String> {
    let models_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| anyhow!("no app data dir: {e}"))?
        .join("ocrs-models");

    let detection = ensure_model(&models_dir, "text-detection.rten", DETECTION_URL).await?;
    let recognition = ensure_model(&models_dir, "text-recognition.rten", RECOGNITION_URL).await?;

    // The screenshot portal may wait on a permission dialog or never answer;
    // screen context is optional, so don't hold up the dictation for it.
    let rgb = tokio::time::timeout(Duration::from_secs(8), capture_screen(method))
        .await
        .map_err(|_| anyhow!("screen capture timed out"))??;

    // OCR is CPU-bound; keep it off the async runtime.
    tokio::task::spawn_blocking(move || run_ocr(rgb, &detection, &recognition))
        .await
        .map_err(|e| anyhow!("ocr task join: {e}"))?
}

fn run_ocr(rgb: image::RgbImage, detection: &Path, recognition: &Path) -> Result<String> {
    let (w, h) = rgb.dimensions();

    let detection_model = Model::load_file(detection).context("load detection model")?;
    let recognition_model = Model::load_file(recognition).context("load recognition model")?;
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })?;

    let source = ImageSource::from_bytes(rgb.as_raw(), (w, h))?;
    let input = engine.prepare_input(source)?;
    engine.get_text(&input)
}

#[cfg(target_os = "linux")]
async fn capture_screen(method: CaptureMethod) -> Result<image::RgbImage> {
    let use_grim = match method {
        CaptureMethod::Grim => true,
        CaptureMethod::Portal => false,
        CaptureMethod::Auto => has_grim(),
    };
    if use_grim {
        capture_grim().await
    } else {
        capture_portal().await
    }
}

#[cfg(target_os = "linux")]
fn has_grim() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("grim").is_file()))
}

/// grim writes a PNG of all outputs to stdout. wlroots compositors only.
#[cfg(target_os = "linux")]
async fn capture_grim() -> Result<image::RgbImage> {
    let out = tokio::process::Command::new("grim")
        .args(["-t", "png", "-"])
        .kill_on_drop(true)
        .output()
        .await
        .context("running grim")?;
    if !out.status.success() {
        return Err(anyhow!(
            "grim failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let img = tokio::task::spawn_blocking(move || image::load_from_memory(&out.stdout))
        .await
        .map_err(|e| anyhow!("decode task join: {e}"))?
        .context("decode screenshot")?;
    Ok(img.to_rgb8())
}

/// Screenshot through the desktop's screenshot portal, which works on most
/// Wayland desktops. The portal writes a file for us; read and remove it.
#[cfg(target_os = "linux")]
async fn capture_portal() -> Result<image::RgbImage> {
    use ashpd::desktop::screenshot::Screenshot;

    let response = Screenshot::request()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .context("screenshot portal request")?
        .response()
        .context("screenshot portal response")?;
    let path = reqwest::Url::parse(response.uri().as_str())
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .ok_or_else(|| anyhow!("screenshot portal returned a non-file uri"))?;
    let img = tokio::task::spawn_blocking({
        let path = path.clone();
        move || image::open(&path)
    })
    .await
    .map_err(|e| anyhow!("decode task join: {e}"))?
    .context("decode screenshot")?;
    let _ = std::fs::remove_file(&path);
    Ok(img.to_rgb8())
}

#[cfg(not(target_os = "linux"))]
async fn capture_screen(_method: CaptureMethod) -> Result<image::RgbImage> {
    tokio::task::spawn_blocking(|| {
        let monitors = xcap::Monitor::all().map_err(|e| anyhow!("enumerate monitors: {e}"))?;
        let monitor = monitors
            .iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .or(monitors.first())
            .ok_or_else(|| anyhow!("no monitors found"))?;
        let rgba = monitor.capture_image().context("screen capture failed")?;
        Ok(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
    })
    .await
    .map_err(|e| anyhow!("capture task join: {e}"))?
}

/// Return the path to a cached model, downloading it on first use. Writes via a
/// temp file then renames, so an interrupted download never leaves a corrupt
/// model cached.
async fn ensure_model(dir: &Path, name: &str, url: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).context("create models dir")?;

    let bytes = download_client()
        .get(url)
        .send()
        .await
        .with_context(|| format!("download {name}"))?
        .error_for_status()?
        .bytes()
        .await?;

    let tmp = dir.join(format!("{name}.part"));
    std::fs::write(&tmp, &bytes).context("write model")?;
    std::fs::rename(&tmp, &path).context("finalize model")?;
    Ok(path)
}

/// Dedicated client with a generous timeout: the shared transcription client's
/// 30s limit is too tight for a cold ~10MB model download.
fn download_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build download client")
    })
}
