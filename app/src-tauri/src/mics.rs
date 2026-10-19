// Microphone listing and selection.
//
// cpal picks its PulseAudio host when a pulse server (or pipewire-pulse) is
// running, and plain ALSA otherwise. Pulse sources have real names and stable
// ids; ALSA only offers raw PCM names, so there we offer just the default.
//
// Ids are cpal device ids ("pulseaudio:<source name>"). One that doesn't
// resolve, like a mic that is unplugged or an id saved by an older version,
// records from the default.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

#[derive(Serialize)]
pub struct Mic {
    pub id: String,
    pub label: String,
}

pub fn list() -> Vec<Mic> {
    let host = cpal::default_host();
    #[cfg(target_os = "linux")]
    if host.id() == cpal::HostId::Alsa {
        return Vec::new();
    }
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut mics: Vec<Mic> = devices
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            // A sink's monitor records what is playing, not a microphone.
            if id.ends_with(".monitor") {
                return None;
            }
            Some(Mic {
                id,
                label: d.to_string(),
            })
        })
        .collect();
    mics.sort_by(|a, b| a.label.cmp(&b.label));
    mics
}

/// The device to record from for a saved mic id (`None` = system default).
pub fn open(id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let saved = id
        .and_then(|id| id.parse().ok())
        .and_then(|id| host.device_by_id(&id));
    if saved.is_none() {
        if let Some(id) = id {
            eprintln!("[live] microphone {id:?} not found, using the default");
        }
    }
    saved
        .or_else(|| host.default_input_device())
        .ok_or_else(|| anyhow!("no microphone available"))
}
