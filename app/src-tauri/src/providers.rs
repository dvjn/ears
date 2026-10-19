// Talking to providers outside of a recording: connection checks and model
// lists, for either an ears service or any OpenAI-compatible API.

use serde::Serialize;
use std::time::Duration;

use crate::live::{http_client, validate_base_url};
use crate::state::ProviderKind;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Stt,
    Llm,
}

impl Role {
    pub fn parse(s: &str) -> Result<Role, String> {
        match s {
            "stt" => Ok(Role::Stt),
            "llm" => Ok(Role::Llm),
            _ => Err(format!("unknown provider role: {s}")),
        }
    }
}

#[derive(Serialize, Default)]
pub struct ProviderStatus {
    /// One line for the UI, e.g. "ears-stt 0.2.0 · default whisper-medium.en".
    pub message: String,
    pub models: Vec<String>,
    /// ears-stt's live transcription websocket is available.
    pub streaming: bool,
    /// Listed models that are cleanup-tuned (fixed prompt, no screen context).
    pub cleanup_models: Vec<String>,
}

/// The server root of an OpenAI-style base URL (".../v1" -> "...").
fn root(base_url: &str) -> &str {
    let base = base_url.trim_end_matches('/');
    base.strip_suffix("/v1").unwrap_or(base)
}

/// ws(s):// URL of ears-stt's streaming endpoint for a base URL.
pub fn stream_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}/audio/transcriptions/stream")
}

/// Whether a model id from an OpenAI-style listing fits the role. Hosted APIs
/// list every model they have, so narrow the picker; ears services list only
/// relevant ones.
fn fits(role: Role, id: &str) -> bool {
    let audio = ["whisper", "transcribe"];
    let not_chat = [
        "whisper",
        "transcribe",
        "tts",
        "embed",
        "dall-e",
        "image",
        "moderation",
        "audio",
        "realtime",
        "search",
        "sora",
    ];
    match role {
        Role::Stt => audio.iter().any(|k| id.contains(k)),
        Role::Llm => !not_chat.iter().any(|k| id.contains(k)),
    }
}

/// Model ids for the role, plus those ears-llm marks as cleanup-tuned.
pub async fn list_models(
    kind: ProviderKind,
    role: Role,
    base_url: &str,
    api_key: &str,
) -> Result<(Vec<String>, Vec<String>), String> {
    let json: serde_json::Value = http_client()
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .bearer_auth(api_key)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let data = json["data"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let all: Vec<String> = data
        .iter()
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    let cleanup: Vec<String> = data
        .iter()
        .filter(|m| m["ears"]["kind"] == "cleanup")
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    let mut models: Vec<String> = if kind == ProviderKind::Openai {
        let fitting: Vec<String> = all.iter().filter(|id| fits(role, id)).cloned().collect();
        if fitting.is_empty() {
            all
        } else {
            fitting
        }
    } else {
        all
    };
    models.sort();
    Ok((models, cleanup))
}

/// Check that a provider is reachable and authorized, and describe it.
pub async fn check(
    kind: ProviderKind,
    role: Role,
    base_url: &str,
    api_key: &str,
) -> Result<ProviderStatus, String> {
    validate_base_url(base_url)?;
    let mut status = ProviderStatus::default();

    if kind == ProviderKind::Ears {
        let service = match role {
            Role::Stt => "ears-stt",
            Role::Llm => "ears-llm",
        };
        let health: serde_json::Value = http_client()
            .get(format!("{}/health", root(base_url)))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| format!("can't reach {service}: {e}"))?
            .json()
            .await
            .map_err(|_| format!("not an {service} server"))?;
        if health["service"] != service {
            return Err(format!("not an {service} server"));
        }
        let version = health["version"].as_str().unwrap_or("?");
        status.streaming = health["streaming"].as_bool().unwrap_or(false);
        status.message = match role {
            Role::Stt => format!(
                "{service} {version} · default {}",
                health["model"].as_str().unwrap_or("?")
            ),
            Role::Llm => format!(
                "{service} {version} · loaded {}",
                health["loaded"].as_str().unwrap_or("none")
            ),
        };
    }

    (status.models, status.cleanup_models) = list_models(kind, role, base_url, api_key).await?;
    if status.message.is_empty() {
        status.message = format!("connected · {} models", status.models.len());
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_url_swaps_scheme() {
        assert_eq!(
            stream_url("http://localhost:13278/v1"),
            "ws://localhost:13278/v1/audio/transcriptions/stream"
        );
        assert_eq!(
            stream_url("https://gpu.example/v1/"),
            "wss://gpu.example/v1/audio/transcriptions/stream"
        );
    }

    #[test]
    fn root_strips_v1() {
        assert_eq!(root("http://h:1/v1"), "http://h:1");
        assert_eq!(root("http://h:1/v1/"), "http://h:1");
        assert_eq!(root("http://h:1"), "http://h:1");
    }

    #[test]
    fn model_roles() {
        assert!(fits(Role::Stt, "whisper-1"));
        assert!(fits(Role::Stt, "gpt-4o-transcribe"));
        assert!(!fits(Role::Stt, "gpt-4o"));
        assert!(fits(Role::Llm, "gpt-4o-mini"));
        assert!(!fits(Role::Llm, "tts-1"));
    }
}
