// Talking to the transcription provider outside of a recording: connection
// checks and model lists, for either ears-stt or any OpenAI-compatible API.

use serde::Serialize;
use std::time::Duration;

use crate::live::{http_client, validate_base_url};
use crate::state::ProviderKind;

#[derive(Serialize, Default)]
pub struct ProviderStatus {
    /// One line for the UI, e.g. "ears-stt 0.2.0 · default whisper-medium.en".
    pub message: String,
    pub models: Vec<String>,
    /// ears-stt's live transcription websocket is available.
    pub streaming: bool,
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

/// Whether a model id from an OpenAI-style listing transcribes audio. Hosted
/// APIs list every model they have, so narrow the picker; ears-stt lists only
/// its own.
fn fits(id: &str) -> bool {
    ["whisper", "transcribe"].iter().any(|k| id.contains(k))
}

/// Model ids the provider offers for transcription.
pub async fn list_models(
    kind: ProviderKind,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, String> {
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
    let mut models: Vec<String> = if kind == ProviderKind::Openai {
        let fitting: Vec<String> = all.iter().filter(|id| fits(id)).cloned().collect();
        if fitting.is_empty() {
            all
        } else {
            fitting
        }
    } else {
        all
    };
    models.sort();
    Ok(models)
}

/// Check that a provider is reachable and authorized, and describe it.
pub async fn check(
    kind: ProviderKind,
    base_url: &str,
    api_key: &str,
) -> Result<ProviderStatus, String> {
    validate_base_url(base_url)?;
    let mut status = ProviderStatus::default();

    if kind == ProviderKind::Ears {
        let service = "ears-stt";
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
        status.streaming = health["streaming"].as_bool().unwrap_or(false);
        status.message = format!(
            "{service} {} · default {}",
            health["version"].as_str().unwrap_or("?"),
            health["model"].as_str().unwrap_or("?")
        );
    }

    status.models = list_models(kind, base_url, api_key).await?;
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
    fn transcription_models() {
        assert!(fits("whisper-1"));
        assert!(fits("gpt-4o-transcribe"));
        assert!(!fits("gpt-4o"));
    }
}
