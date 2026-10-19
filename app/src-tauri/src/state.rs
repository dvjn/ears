use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::menu::MenuItem;

use crate::live::LiveSession;

pub enum RecordingState {
    Idle,
    LiveRecording { session: Box<LiveSession> },
    Transcribing,
}

/// Which kind of server a provider points at. `Ears` is ears-stt (adds a
/// health check and streaming); `Openai` is any OpenAI-compatible API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Ears,
    Openai,
}

pub const EARS_STT_URL: &str = "http://localhost:13278/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Transcription provider.
    #[serde(default)]
    pub stt_provider: ProviderKind,
    #[serde(default = "default_stt_url")]
    pub stt_base_url: String,
    #[serde(default)]
    pub stt_api_key: String,
    #[serde(default)]
    pub stt_model: String,

    /// Microphone id (see mics.rs); `None` uses the system default.
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default = "default_language")]
    pub language: Option<String>,
    #[serde(default = "default_max_duration_secs")]
    pub max_duration_secs: u32,
    /// Auto-stop after this many seconds of silence (no new transcribed text).
    /// 0 disables silence-based auto-stop.
    #[serde(default = "default_silence_stop_secs")]
    pub silence_stop_secs: u32,
    #[serde(default)]
    pub auto_type: bool,
    #[serde(default = "default_auto_copy")]
    pub auto_copy: bool,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
}

fn default_stt_url() -> String {
    EARS_STT_URL.to_string()
}
fn default_history_limit() -> usize {
    10
}
fn default_auto_copy() -> bool {
    true
}
fn default_silence_stop_secs() -> u32 {
    3
}
fn default_max_duration_secs() -> u32 {
    120
}
fn default_language() -> Option<String> {
    Some("en".to_string())
}

impl Default for Settings {
    fn default() -> Self {
        // Deserializing an empty object applies every serde default above.
        serde_json::from_value(serde_json::json!({})).expect("settings defaults")
    }
}

pub struct AppState {
    pub recording: Mutex<RecordingState>,
    pub settings: Mutex<Settings>,
    pub last_result: Mutex<Option<String>>,
    pub history: Mutex<Vec<String>>,
    pub last_result_menu_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    pub start_stop_menu_item: Mutex<Option<MenuItem<tauri::Wry>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            recording: Mutex::new(RecordingState::Idle),
            settings: Mutex::new(Settings::default()),
            last_result: Mutex::new(None),
            history: Mutex::new(Vec::new()),
            last_result_menu_item: Mutex::new(None),
            start_stop_menu_item: Mutex::new(None),
        }
    }
}
