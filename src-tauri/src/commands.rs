use enigo::{Enigo, Keyboard, Settings as EnigoSettings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_store::StoreExt;

use crate::{
    audio,
    models,
    state::{AppState, RecordingState, Settings},
    transcribe,
    tray::{set_tray_last_result, set_tray_recording},
};

const STORE_FILE: &str = "settings.json";


#[derive(Serialize)]
pub struct ModelStatus {
    pub name: &'static str,
    pub filename: &'static str,
    pub size_mb: u32,
    pub downloaded: bool,
}

#[derive(Serialize)]
pub struct AppStatus {
    pub state: String,
    pub last_result: Option<String>,
}

// ---- Settings persistence --------------------------------------------------

pub fn load_settings(app: &AppHandle, state: &AppState) {
    let Ok(store) = app.store(STORE_FILE) else { return };
    let mut settings = state.settings.lock().unwrap();
    if let Some(v) = store.get("model_name").and_then(|v| v.as_str().map(|s| s.to_string())) {
        settings.model_name = v;
    }
    if let Some(v) = store.get("language") {
        settings.language = v.as_str().map(|s| s.to_string());
    }
    if let Some(v) = store.get("max_duration_secs").and_then(|v| v.as_u64()) {
        settings.max_duration_secs = v as u32;
    }
    if let Some(v) = store.get("type_at_cursor").and_then(|v| v.as_bool()) {
        settings.type_at_cursor = v;
    }
    if let Some(v) = store.get("history_limit").and_then(|v| v.as_u64()) {
        settings.history_limit = (v as usize).clamp(1, 9999);
    }
}

pub fn load_history(app: &AppHandle, state: &AppState) {
    let Ok(dir) = app.path().app_data_dir() else { return };
    let path = dir.join("history.json");
    let Ok(data) = std::fs::read_to_string(path) else { return };
    let Ok(history) = serde_json::from_str::<Vec<String>>(&data) else { return };
    *state.history.lock().unwrap() = history;
}

fn save_history(app: &AppHandle, history: &[String]) {
    let Ok(dir) = app.path().app_data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("history.json");
    if let Ok(data) = serde_json::to_string(history) {
        let _ = std::fs::write(path, data);
    }
}

fn persist_settings(app: &AppHandle, settings: &Settings) {
    let Ok(store) = app.store(STORE_FILE) else { return };
    store.set("model_name", settings.model_name.clone());
    store.set("language", serde_json::json!(settings.language));
    store.set("max_duration_secs", settings.max_duration_secs);
    store.set("type_at_cursor", settings.type_at_cursor);
    store.set("history_limit", settings.history_limit);
    let _ = store.save();
}

// ---- Tauri commands --------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
    persist_settings(&app, &settings);
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

#[tauri::command]
pub fn list_models(app: AppHandle) -> Vec<ModelStatus> {
    models::MODELS
        .iter()
        .map(|m| ModelStatus {
            name: m.name,
            filename: m.filename,
            size_mb: m.size_mb,
            downloaded: models::is_downloaded(&app, m),
        })
        .collect()
}

#[tauri::command]
pub async fn download_model(app: AppHandle, model_name: String) -> Result<(), String> {
    models::download_model(app, model_name)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_model(app: AppHandle, state: State<AppState>, model_name: String) -> Result<(), String> {
    models::delete_model(&app, &model_name).map_err(|e| e.to_string())?;
    if state.loaded_model.lock().unwrap().as_deref() == Some(&model_name) {
        *state.whisper_ctx.lock().unwrap() = None;
        *state.loaded_model.lock().unwrap() = None;
    }
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> AppStatus {
    let status = match &*state.recording.lock().unwrap() {
        RecordingState::Idle => "idle".to_string(),
        RecordingState::Recording { .. } => "recording".to_string(),
        RecordingState::Transcribing => "transcribing".to_string(),
    };
    AppStatus {
        state: status,
        last_result: state.last_result.lock().unwrap().clone(),
    }
}

#[tauri::command]
pub fn get_history(state: State<AppState>) -> Vec<String> {
    state.history.lock().unwrap().clone()
}

#[tauri::command]
pub fn remove_history_item(app: AppHandle, state: State<AppState>, index: usize) {
    let mut history = state.history.lock().unwrap();
    if index < history.len() {
        history.remove(index);
        save_history(&app, &history);
    }
}

#[tauri::command]
pub async fn cmd_toggle_recording(app: AppHandle) -> Result<(), String> {
    toggle_recording(app).await.map_err(|e| e.to_string())
}

// ---- Core toggle -----------------------------------------------------------

pub async fn toggle_recording(app: AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();

    // Atomically read and transition to prevent TOCTOU race between concurrent callers.
    // The Transcribing sentinel blocks any concurrent caller from also entering "start".
    let action = {
        let mut guard = state.recording.lock().unwrap();
        match &*guard {
            RecordingState::Idle => {
                *guard = RecordingState::Transcribing;
                "start"
            }
            RecordingState::Recording { .. } => "stop",
            RecordingState::Transcribing => "noop",
        }
    };

    match action {
        "start" => {
            let model_name = state.settings.lock().unwrap().model_name.clone();

            let model_info = match models::find_model(&model_name) {
                Some(m) => m,
                None => {
                    *state.recording.lock().unwrap() = RecordingState::Idle;
                    return Err(anyhow::anyhow!("unknown model: {model_name}"));
                }
            };
            let model_path = match models::model_path(&app, model_info.filename) {
                Ok(p) => p,
                Err(e) => {
                    *state.recording.lock().unwrap() = RecordingState::Idle;
                    return Err(e);
                }
            };

            if !model_path.exists() {
                *state.recording.lock().unwrap() = RecordingState::Idle;
                let _ = app.notification()
                    .builder()
                    .title("ears - no model")
                    .body(format!("Model '{model_name}' is not downloaded. Open settings to download it."))
                    .show();
                return Ok(());
            }

            let session = match audio::start_recording() {
                Ok(s) => s,
                Err(e) => {
                    *state.recording.lock().unwrap() = RecordingState::Idle;
                    return Err(e);
                }
            };
            let max_secs = state.settings.lock().unwrap().max_duration_secs;

            *state.recording.lock().unwrap() = RecordingState::Recording {
                start: std::time::Instant::now(),
                session,
            };

            set_tray_recording(&app, true);
            let _ = app.emit("recording-state-changed", "recording");


            let app_clone = app.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(max_secs as u64)).await;
                let s = app_clone.state::<AppState>();
                let is_recording = matches!(&*s.recording.lock().unwrap(), RecordingState::Recording { .. });
                if is_recording {
                    if let Err(e) = do_stop_and_transcribe(app_clone).await {
                        eprintln!("auto-stop error: {e}");
                    }
                }
            });
        }
        "stop" => {
            do_stop_and_transcribe(app).await?;
        }
        _ => {}
    }

    Ok(())
}

async fn do_stop_and_transcribe(app: AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();

    let (session, sample_rate) = {
        let mut guard = state.recording.lock().unwrap();
        match std::mem::replace(&mut *guard, RecordingState::Transcribing) {
            RecordingState::Recording { session, .. } => {
                let sr = session.sample_rate;
                (session, sr)
            }
            other => {
                *guard = other;
                return Ok(());
            }
        }
    };

    set_tray_recording(&app, false);
    let _ = app.emit("recording-state-changed", "transcribing");


    // join() is blocking — run it off the async executor
    let raw_samples = tokio::task::spawn_blocking(move || audio::stop_and_collect(session)).await?;

    if raw_samples.is_empty() {
        *state.recording.lock().unwrap() = RecordingState::Idle;
        let _ = app.emit("recording-state-changed", "idle");
        let _ = app.notification().builder().title("ears").body("No audio recorded.").show();
        return Ok(());
    }

    let samples_16k = tokio::task::spawn_blocking(move || audio::resample(raw_samples, sample_rate))
        .await??;

    let model_name = state.settings.lock().unwrap().model_name.clone();
    let language = state.settings.lock().unwrap().language.clone();

    let model_info = models::find_model(&model_name)
        .ok_or_else(|| anyhow::anyhow!("unknown model: {model_name}"))?;
    let model_path = models::model_path(&app, model_info.filename)?;

    let need_reload = state.loaded_model.lock().unwrap().as_deref() != Some(&model_name);
    if need_reload {
        let path = model_path.clone();
        let ctx = tokio::task::spawn_blocking(move || transcribe::load_model(&path)).await??;
        *state.whisper_ctx.lock().unwrap() = Some(ctx);
        *state.loaded_model.lock().unwrap() = Some(model_name.clone());
    }

    let ctx_arc = state.inner().whisper_ctx.clone();
    let text = tokio::task::spawn_blocking(move || {
        let guard = ctx_arc.lock().unwrap();
        let ctx = guard.as_ref().ok_or_else(|| anyhow::anyhow!("no whisper context"))?;
        transcribe::transcribe(ctx, &samples_16k, language.as_deref())
    })
    .await??;

    // Copy to clipboard via the plugin's system clipboard (works with hidden window)
    if let Err(e) = app.clipboard().write_text(&text) {
        eprintln!("clipboard write failed: {e}");
    }

    // Type transcribed text directly at the cursor position
    if state.settings.lock().unwrap().type_at_cursor {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let text_to_type = text.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(mut enigo) = Enigo::new(&EnigoSettings::default()) {
                let _ = enigo.text(&text_to_type);
            }
        }).await.ok();
    }

    // Notification — skip when typing, since the text appears directly at cursor
    if !state.settings.lock().unwrap().type_at_cursor {
        let preview: String = text.chars().take(60).collect();
        let body = if text.chars().count() > 60 { format!("{preview}...") } else { preview };
        let _ = app.notification().builder().title("ears - copied").body(body).show();
    }

    // Update history
    {
        let limit = state.settings.lock().unwrap().history_limit;
        let mut history = state.history.lock().unwrap();
        history.insert(0, text.clone());
        history.truncate(limit);
        save_history(&app, &history);
    }

    *state.last_result.lock().unwrap() = Some(text.clone());
    set_tray_last_result(&app, &text);

    let _ = app.emit("transcription-done", &text);
    let _ = app.emit("recording-state-changed", "idle");
    *state.recording.lock().unwrap() = RecordingState::Idle;

    Ok(())
}
