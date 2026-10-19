use enigo::{Enigo, Keyboard, Settings as EnigoSettings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_store::StoreExt;

use crate::{
    live, providers,
    state::{AppState, ProviderKind, RecordingState, Settings},
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
/// into immediate credential theft. `has_stt_api_key` lets the UI show whether
/// a key is stored without revealing it.
#[derive(Serialize)]
pub struct SettingsView {
    #[serde(flatten)]
    pub settings: Settings,
    pub has_stt_api_key: bool,
}

impl From<&Settings> for SettingsView {
    fn from(s: &Settings) -> Self {
        let mut settings = s.clone();
        settings.stt_api_key.clear();
        SettingsView {
            settings,
            has_stt_api_key: !s.stt_api_key.is_empty(),
        }
    }
}

// ---- Settings persistence --------------------------------------------------

pub fn load_settings(app: &AppHandle, state: &AppState) {
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    let settings = match store.get("settings") {
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => {
            let migrated = migrate_flat_settings(|k| store.get(k));
            persist_settings(app, &migrated);
            migrated
        }
    };
    *state.settings.lock().unwrap() = settings;
}

/// Settings from before the provider choice were stored as flat keys. They
/// pointed at some OpenAI-compatible server, so the provider becomes `Openai`
/// with those values.
fn migrate_flat_settings(get: impl Fn(&str) -> Option<serde_json::Value>) -> Settings {
    let mut s = Settings::default();
    let str_of = |k: &str| get(k).and_then(|v| v.as_str().map(str::to_string));
    let Some(base_url) = str_of("base_url") else {
        return s; // fresh install
    };
    let api_key = str_of("api_key").unwrap_or_default();

    s.stt_provider = ProviderKind::Openai;
    s.stt_base_url = base_url;
    s.stt_api_key = api_key;
    s.stt_model = str_of("model").unwrap_or_default();
    if let Some(v) = get("input_device") {
        s.input_device = v.as_str().map(str::to_string);
    }
    if let Some(v) = get("language") {
        s.language = v.as_str().map(str::to_string);
    }
    if let Some(v) = get("max_duration_secs").and_then(|v| v.as_u64()) {
        s.max_duration_secs = v as u32;
    }
    if let Some(v) = get("silence_stop_secs").and_then(|v| v.as_u64()) {
        s.silence_stop_secs = v as u32;
    }
    if let Some(v) = get("auto_type").and_then(|v| v.as_bool()) {
        s.auto_type = v;
    }
    if let Some(v) = get("auto_copy").and_then(|v| v.as_bool()) {
        s.auto_copy = v;
    }
    if let Some(v) = get("history_limit").and_then(|v| v.as_u64()) {
        s.history_limit = (v as usize).clamp(1, 9999);
    }
    s
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
    store.clear();
    store.set("settings", serde_json::json!(settings));
    let _ = store.save();
}

// ---- Tauri commands --------------------------------------------------------

/// Check the transcription provider and list its models. An empty `api_key`
/// means "use the stored key"; a non-empty one is a key the user just typed.
#[tauri::command]
pub async fn check_provider(
    state: State<'_, AppState>,
    kind: ProviderKind,
    base_url: String,
    api_key: String,
) -> Result<providers::ProviderStatus, String> {
    let api_key = if api_key.is_empty() {
        state.settings.lock().unwrap().stt_api_key.clone()
    } else {
        api_key
    };
    providers::check(kind, base_url.trim(), &api_key).await
}

#[tauri::command]
pub async fn list_input_devices() -> Vec<crate::mics::Mic> {
    tokio::task::spawn_blocking(crate::mics::list)
        .await
        .unwrap_or_default()
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
    live::validate_base_url(&settings.stt_base_url)?;
    {
        let cur = state.settings.lock().unwrap();
        // An empty incoming key means "keep the current one" — the renderer is
        // never given the saved key to send back (see `SettingsView`).
        if settings.stt_api_key.is_empty() {
            settings.stt_api_key = cur.stt_api_key.clone();
        }
        // Likewise, an empty model means "keep current": a transient /models
        // fetch failure leaves the picker empty, and that must not wipe the
        // saved model.
        if settings.stt_model.is_empty() {
            settings.stt_model = cur.stt_model.clone();
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
        base_url: s.stt_base_url.trim().to_string(),
        api_key: s.stt_api_key.clone(),
        model: s.stt_model.clone(),
        language: s.language.clone(),
        stream: s.stt_provider == ProviderKind::Ears,
        input_device: s.input_device.clone(),
    }
}

async fn start_live(app: AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();

    let config = api_config(&state);
    // ears-stt on localhost may run without a key; any other provider needs one.
    let keyless_ok = config.stream;
    if config.api_key.is_empty() && !keyless_ok {
        *state.recording.lock().unwrap() = RecordingState::Idle;
        let _ = app.emit("error-no-api-key", ());
        return Ok(());
    }

    let session = match live::start_live(app.clone(), config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[live] opening the microphone failed: {e}");
            *state.recording.lock().unwrap() = RecordingState::Idle;
            ask_for_mic(&app, &format!("Couldn't open the microphone: {e}"));
            return Ok(());
        }
    };
    let id = session.id();
    let (max_secs, silence_secs) = {
        let s = state.settings.lock().unwrap();
        (s.max_duration_secs, s.silence_stop_secs)
    };

    *state.recording.lock().unwrap() = RecordingState::LiveRecording {
        session: Box::new(session),
    };

    show_live_overlay(&app);
    set_tray_recording(&app, true);
    let _ = app.emit("recording-state-changed", "recording");

    // Watcher: auto-stop on max duration or on a stretch of silence, and hand
    // a microphone that never sent audio back to the user.
    let app_clone = app.clone();
    tokio::spawn(async move {
        enum Check {
            Continue,
            Stop,
            MicFailed,
        }
        let start = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let s = app_clone.state::<AppState>();
            let check = {
                let mut guard = s.recording.lock().unwrap();
                let RecordingState::LiveRecording { session } = &*guard else {
                    return; // no longer live; stop watching
                };
                if session.id() != id {
                    return; // a newer recording has its own watcher
                }
                if session.mic_failed() {
                    *guard = RecordingState::Idle;
                    Check::MicFailed
                } else if (max_secs > 0
                    && start.elapsed() >= std::time::Duration::from_secs(max_secs as u64))
                    || (silence_secs > 0
                        && session.silent_for(std::time::Duration::from_secs(silence_secs as u64)))
                {
                    Check::Stop
                } else {
                    Check::Continue
                }
            };
            match check {
                Check::Continue => {}
                Check::Stop => {
                    if let Err(e) = do_stop_live(app_clone).await {
                        eprintln!("live auto-stop error: {e}");
                    }
                    return;
                }
                Check::MicFailed => {
                    ask_for_mic(
                        &app_clone,
                        "No audio from the microphone. Reconnect it or pick another one.",
                    );
                    return;
                }
            }
        }
    });
    Ok(())
}

/// The microphone failed: end any recording UI and open the main window with
/// a microphone picker so the user can switch and try again.
fn ask_for_mic(app: &AppHandle, message: &str) {
    hide_live_overlay(app);
    set_tray_recording(app, false);
    let _ = app.emit("recording-state-changed", "idle");
    let _ = app.emit("mic-failed", message);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window("main") {
            let _ = win.show();
            let _ = win.set_focus();
        }
    });
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
    let result = (*session).shutdown().await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn migrates_shared_provider_settings() {
        let old = json!({
            "base_url": "http://localhost:10300/v1",
            "api_key": "k",
            "model": "whisper-medium.en",
            "auto_type": true,
            "history_limit": 50,
        });
        let s = migrate_flat_settings(|k| old.get(k).cloned());
        assert_eq!(s.stt_provider, ProviderKind::Openai);
        assert_eq!(s.stt_base_url, "http://localhost:10300/v1");
        assert_eq!(s.stt_api_key, "k");
        assert_eq!(s.stt_model, "whisper-medium.en");
        assert!(s.auto_type);
        assert_eq!(s.history_limit, 50);
    }

    #[test]
    fn fresh_install_defaults_to_ears_stt() {
        let s = migrate_flat_settings(|_| None);
        assert_eq!(s.stt_provider, ProviderKind::Ears);
        assert_eq!(s.stt_base_url, crate::state::EARS_STT_URL);
    }

    #[test]
    fn view_hides_keys() {
        let s = Settings {
            stt_api_key: "secret".into(),
            ..Settings::default()
        };
        let v = serde_json::to_value(SettingsView::from(&s)).unwrap();
        assert_eq!(v["stt_api_key"], "");
        assert_eq!(v["has_stt_api_key"], true);
    }
}
