use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::menu::MenuItem;
use whisper_rs::WhisperContext;

use crate::audio::ActiveRecording;

pub enum RecordingState {
    Idle,
    Recording {
        #[allow(dead_code)]
        start: Instant,
        session: ActiveRecording,
    },
    Transcribing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub model_name: String,
    pub language: Option<String>,
    pub max_duration_secs: u32,
    #[serde(default)]
    pub auto_type: bool,
    #[serde(default = "default_auto_copy")]
    pub auto_copy: bool,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
}

fn default_history_limit() -> usize { 10 }
fn default_auto_copy() -> bool { true }

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_name: "base.en".to_string(),
            language: Some("en".to_string()),
            max_duration_secs: 120,
            auto_type: false,
            auto_copy: true,
            history_limit: 10,
        }
    }
}

pub struct AppState {
    pub recording: Mutex<RecordingState>,
    pub whisper_ctx: Arc<Mutex<Option<WhisperContext>>>,
    pub loaded_model: Mutex<Option<String>>,
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
            whisper_ctx: Arc::new(Mutex::new(None)),
            loaded_model: Mutex::new(None),
            settings: Mutex::new(Settings::default()),
            last_result: Mutex::new(None),
            history: Mutex::new(Vec::new()),
            last_result_menu_item: Mutex::new(None),
            start_stop_menu_item: Mutex::new(None),
        }
    }
}
