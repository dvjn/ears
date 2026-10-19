use enigo::{Enigo, Keyboard, Settings as EnigoSettings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_store::StoreExt;

use crate::{
    live,
    state::{AppState, RecordingState, Settings},
    tray::{set_tray_last_result, set_tray_recording},
};

const STORE_FILE: &str = "settings.json";

#[derive(Serialize)]
pub struct AppStatus {
    pub state: String,
    pub last_result: Option<String>,
}

/// Settings as exposed to the webview. The API key is deliberately *not*
/// included — the renderer never needs the secret (the backend reads it from
/// state for every request), and returning it would turn any script injection
/// into immediate credential theft. `has_api_key` lets the UI show whether a
/// key is already stored without revealing it.
#[derive(Serialize)]
pub struct SettingsView {
    pub base_url: String,
    pub model: String,
    pub language: Option<String>,
    pub max_duration_secs: u32,
    pub silence_stop_secs: u32,
    pub auto_type: bool,
    pub auto_copy: bool,
    pub history_limit: usize,
    pub has_api_key: bool,
}

impl From<&Settings> for SettingsView {
    fn from(s: &Settings) -> Self {
        SettingsView {
            base_url: s.base_url.clone(),
            model: s.model.clone(),
            language: s.language.clone(),
            max_duration_secs: s.max_duration_secs,
            silence_stop_secs: s.silence_stop_secs,
            auto_type: s.auto_type,
            auto_copy: s.auto_copy,
            history_limit: s.history_limit,
            has_api_key: !s.api_key.is_empty(),
        }
    }
}

// ---- Settings persistence --------------------------------------------------

pub fn load_settings(app: &AppHandle, state: &AppState) {
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    let mut settings = state.settings.lock().unwrap();
    if let Some(v) = store
        .get("base_url")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
    {
        settings.base_url = v;
    }
    if let Some(v) = store
        .get("api_key")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
    {
        settings.api_key = v;
    }
    if let Some(v) = store
        .get("model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
    {
        settings.model = v;
    }
    if let Some(v) = store.get("language") {
        settings.language = v.as_str().map(|s| s.to_string());
    }
    if let Some(v) = store.get("max_duration_secs").and_then(|v| v.as_u64()) {
        settings.max_duration_secs = v as u32;
    }
    if let Some(v) = store.get("silence_stop_secs").and_then(|v| v.as_u64()) {
        settings.silence_stop_secs = v as u32;
    }
    if let Some(v) = store.get("auto_type").and_then(|v| v.as_bool()) {
        settings.auto_type = v;
    }
    if let Some(v) = store.get("auto_copy").and_then(|v| v.as_bool()) {
        settings.auto_copy = v;
    }
    if let Some(v) = store.get("history_limit").and_then(|v| v.as_u64()) {
        settings.history_limit = (v as usize).clamp(1, 9999);
    }
}

pub fn load_history(app: &AppHandle, state: &AppState) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let path = dir.join("history.json");
    let Ok(data) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(history) = serde_json::from_str::<Vec<String>>(&data) else {
        return;
    };
    *state.history.lock().unwrap() = history;
}

fn save_history(app: &AppHandle, history: &[String]) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("history.json");
    if let Ok(data) = serde_json::to_string(history) {
        let _ = std::fs::write(path, data);
    }
}

fn persist_settings(app: &AppHandle, settings: &Settings) {
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    store.set("base_url", settings.base_url.clone());
    store.set("api_key", settings.api_key.clone());
    store.set("model", settings.model.clone());
    store.set("language", serde_json::json!(settings.language));
    store.set("max_duration_secs", settings.max_duration_secs);
    store.set("silence_stop_secs", settings.silence_stop_secs);
    store.set("auto_type", settings.auto_type);
    store.set("auto_copy", settings.auto_copy);
    store.set("history_limit", settings.history_limit);
    let _ = store.save();
}

// ---- Tauri commands --------------------------------------------------------

#[tauri::command]
pub async fn list_provider_models(
    state: State<'_, AppState>,
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    // The renderer no longer holds the saved key (see `SettingsView`); an empty
    // `api_key` means "use the stored one". A non-empty value is a key the user
    // just typed but hasn't saved yet.
    live::validate_base_url(&base_url)?;
    let api_key = if api_key.is_empty() {
        state.settings.lock().unwrap().api_key.clone()
    } else {
        api_key
    };
    let resp = live::http_client()
        .get(format!("{base_url}/models"))
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut models: Vec<String> = json["data"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
        .filter(|id| id.contains("whisper"))
        .collect();
    models.sort();
    Ok(models)
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> SettingsView {
    SettingsView::from(&*state.settings.lock().unwrap())
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<AppState>,
    settings: Settings,
) -> Result<(), String> {
    let mut settings = settings;
    live::validate_base_url(&settings.base_url)?;
    {
        let cur = state.settings.lock().unwrap();
        // An empty incoming key means "keep the current one" — the renderer is
        // never given the saved key to send back (see `SettingsView`).
        if settings.api_key.is_empty() {
            settings.api_key = cur.api_key.clone();
        }
        // Likewise, an empty model means "keep current": a transient /models
        // fetch failure leaves the picker empty, and we must not let that wipe
        // the user's saved model.
        if settings.model.is_empty() {
            settings.model = cur.model.clone();
        }
    }
    persist_settings(&app, &settings);
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> AppStatus {
    let status = match &*state.recording.lock().unwrap() {
        RecordingState::Idle => "idle".to_string(),
        RecordingState::LiveRecording { .. } => "recording".to_string(),
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
pub fn clear_history(app: AppHandle, state: State<AppState>) {
    let mut history = state.history.lock().unwrap();
    history.clear();
    save_history(&app, &history);
}

#[tauri::command]
pub async fn cmd_toggle_recording(app: AppHandle) -> Result<(), String> {
    toggle_recording(app).await.map_err(|e| e.to_string())
}

// ---- Live overlay window ---------------------------------------------------

const LIVE_LABEL: &str = "live";

// Window operations must run on the GTK/main thread on Linux, so these are
// marshalled via run_on_main_thread (the commands run on a tokio worker).
fn show_live_overlay(app: &AppHandle) {
    // Clear any text from a previous session before it becomes visible.
    let _ = app.emit("live-reset", ());
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(win) = handle.get_webview_window(LIVE_LABEL) else {
            return;
        };
        let _ = win.show();

        // Pin to bottom-center of the current monitor (best-effort: some Wayland
        // compositors ignore programmatic positioning).
        if let Ok(Some(monitor)) = win.current_monitor() {
            let scale = monitor.scale_factor();
            let screen = monitor.size().to_logical::<f64>(scale);
            if let Ok(size) = win.outer_size() {
                let w = size.to_logical::<f64>(scale);
                let x = (screen.width - w.width) / 2.0;
                let y = screen.height - w.height - 90.0;
                let _ = win.set_position(LogicalPosition::new(x.max(0.0), y.max(0.0)));
            }
        }
    });
}

fn hide_live_overlay(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window(LIVE_LABEL) {
            let _ = win.hide();
        }
    });
}

// ---- Core toggle -----------------------------------------------------------

pub async fn toggle_recording(app: AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();

    enum Action {
        Start,
        Stop,
        Noop,
    }

    let action = {
        let mut guard = state.recording.lock().unwrap();
        match &*guard {
            RecordingState::Idle => {
                *guard = RecordingState::Transcribing; // reserve; replaced in start_live
                Action::Start
            }
            RecordingState::LiveRecording { .. } => Action::Stop,
            RecordingState::Transcribing => Action::Noop,
        }
    };

    match action {
        Action::Start => start_live(app).await?,
        Action::Stop => do_stop_live(app).await?,
        Action::Noop => {}
    }

    Ok(())
}

fn api_config(state: &AppState) -> live::ApiConfig {
    let s = state.settings.lock().unwrap();
    live::ApiConfig {
        base_url: s.base_url.clone(),
        api_key: s.api_key.clone(),
        model: s.model.clone(),
        language: s.language.clone(),
    }
}

async fn start_live(app: AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();

    let config = api_config(&state);
    if config.api_key.is_empty() {
        *state.recording.lock().unwrap() = RecordingState::Idle;
        let _ = app.emit("error-no-api-key", ());
        return Ok(());
    }

    let session = match live::start_live(app.clone(), config) {
        Ok(s) => s,
        Err(e) => {
            *state.recording.lock().unwrap() = RecordingState::Idle;
            let _ = app.emit("transcription-error", format!("Live start failed: {e}"));
            return Err(e);
        }
    };
    let (max_secs, silence_secs) = {
        let s = state.settings.lock().unwrap();
        (s.max_duration_secs, s.silence_stop_secs)
    };

    *state.recording.lock().unwrap() = RecordingState::LiveRecording { session };

    show_live_overlay(&app);
    set_tray_recording(&app, true);
    let _ = app.emit("recording-state-changed", "recording");

    // Watcher: auto-stop on max duration or on a stretch of silence.
    let app_clone = app.clone();
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let s = app_clone.state::<AppState>();
            let should_stop = {
                let guard = s.recording.lock().unwrap();
                match &*guard {
                    RecordingState::LiveRecording { session } => {
                        let max_hit = max_secs > 0
                            && start.elapsed() >= std::time::Duration::from_secs(max_secs as u64);
                        let silence_hit = silence_secs > 0
                            && session
                                .silent_for(std::time::Duration::from_secs(silence_secs as u64));
                        max_hit || silence_hit
                    }
                    _ => return, // no longer live; stop watching
                }
            };
            if should_stop {
                if let Err(e) = do_stop_live(app_clone).await {
                    eprintln!("live auto-stop error: {e}");
                }
                return;
            }
        }
    });
    Ok(())
}

async fn do_stop_live(app: AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();

    let session = {
        let mut guard = state.recording.lock().unwrap();
        match std::mem::replace(&mut *guard, RecordingState::Transcribing) {
            RecordingState::LiveRecording { session } => session,
            other => {
                *guard = other;
                return Ok(());
            }
        }
    };

    set_tray_recording(&app, false);
    let _ = app.emit("recording-state-changed", "transcribing");

    // Finish any pending utterances and collect the final transcript.
    let result = session.shutdown().await;
    hide_live_overlay(&app);

    let text = match result {
        live::FinalTranscript::Text(t) => t,
        other => {
            *state.recording.lock().unwrap() = RecordingState::Idle;
            let _ = app.emit("recording-state-changed", "idle");
            let msg = match other {
                live::FinalTranscript::NoSpeech => "No speech detected.".to_string(),
                live::FinalTranscript::NoText => "No speech transcribed.".to_string(),
                live::FinalTranscript::Error(e) => format!("Transcription failed: {e}"),
                live::FinalTranscript::Text(_) => unreachable!(),
            };
            let _ = app.emit("transcription-error", msg);
            return Ok(());
        }
    };

    finalize_transcription(&app, text).await;
    Ok(())
}

/// Post-processing after a live dictation completes: copy / type / notify /
/// history, then emit the final result and return to idle.
async fn finalize_transcription(app: &AppHandle, text: String) {
    let state = app.state::<AppState>();

    let (auto_copy, auto_type) = {
        let s = state.settings.lock().unwrap();
        (s.auto_copy, s.auto_type)
    };

    if auto_copy {
        if let Err(e) = app.clipboard().write_text(&text) {
            eprintln!("clipboard write failed: {e}");
        }
    }

    if auto_type {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let text_to_type = text.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(mut enigo) = Enigo::new(&EnigoSettings::default()) {
                let _ = enigo.text(&text_to_type);
            }
        })
        .await
        .ok();
    }

    if !auto_type {
        let preview: String = text.chars().take(60).collect();
        let body = if text.chars().count() > 60 {
            format!("{preview}...")
        } else {
            preview
        };
        let title = if auto_copy {
            "ears - copied"
        } else {
            "ears - transcribed"
        };
        let _ = app.notification().builder().title(title).body(body).show();
    }

    {
        let limit = state.settings.lock().unwrap().history_limit;
        let mut history = state.history.lock().unwrap();
        history.insert(0, text.clone());
        history.truncate(limit);
        save_history(app, &history);
    }

    *state.last_result.lock().unwrap() = Some(text.clone());
    set_tray_last_result(app, &text);

    let _ = app.emit("transcription-done", &text);
    let _ = app.emit("recording-state-changed", "idle");
    *state.recording.lock().unwrap() = RecordingState::Idle;
}
