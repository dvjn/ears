use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::menu::MenuItem;

use crate::live::LiveSession;

pub enum RecordingState {
    Idle,
    LiveRecording {
        session: LiveSession,
    },
    Transcribing,
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
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

fn default_history_limit() -> usize { 10 }
fn default_auto_copy() -> bool { true }
fn default_silence_stop_secs() -> u32 { 3 }
fn default_max_duration_secs() -> u32 { 120 }
fn default_language() -> Option<String> { Some("en".to_string()) }

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            api_key: String::new(),
            model: String::new(),
            language: Some("en".to_string()),
            max_duration_secs: 120,
            silence_stop_secs: 3,
            auto_type: false,
            auto_copy: true,
            history_limit: 10,
        }
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
