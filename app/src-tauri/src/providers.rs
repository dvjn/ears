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

#[derive(Serialize)]
pub struct ProviderStatus {
    /// What the server turned out to be.
    pub kind: ProviderKind,
    pub models: Vec<String>,
    /// ears-stt's live transcription websocket is available.
    pub streaming: bool,
    /// Listed models that are cleanup-tuned (fixed prompt, no screen context).
    pub cleanup_models: Vec<String>,
}

/// The host (and port) of a base URL, for messages.
pub fn host(base_url: &str) -> &str {
    let rest = base_url.split_once("://").map_or(base_url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
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
    let resp = http_client()
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .bearer_auth(api_key)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| unreachable(base_url, &e))?;
    let code = resp.status();
    if code == reqwest::StatusCode::UNAUTHORIZED || code == reqwest::StatusCode::FORBIDDEN {
        return Err(if api_key.is_empty() {
            "This server needs an API key".to_string()
        } else {
            "The server rejected the API key".to_string()
        });
    }
    if !code.is_success() {
        return Err(format!("{} answered {code}", host(base_url)));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|_| "This is not an OpenAI-compatible server".to_string())?;
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

fn unreachable(base_url: &str, e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("{} did not answer in time", host(base_url))
    } else {
        format!("Can't reach {}", host(base_url))
    }
}

/// What `/health` says the server is: an ears service, or (no ears health
/// endpoint) some other OpenAI-compatible API.
enum Health {
    Ears { service: String, streaming: bool },
    Other,
}

async fn health(base_url: &str) -> Result<Health, String> {
    let resp = http_client()
        .get(format!("{}/health", root(base_url)))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| unreachable(base_url, &e))?;
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return Ok(Health::Other);
    };
    match json["service"].as_str() {
        Some(service) if service.starts_with("ears-") => Ok(Health::Ears {
            service: service.to_string(),
            streaming: json["streaming"].as_bool().unwrap_or(false),
        }),
        _ => Ok(Health::Other),
    }
}

/// Check that a server is reachable and authorized, detect whether it is an
/// ears service, and list its models for the role.
pub async fn check(role: Role, base_url: &str, api_key: &str) -> Result<ProviderStatus, String> {
    validate_base_url(base_url).map_err(|_| "Enter an http:// or https:// URL".to_string())?;
    let (kind, streaming) = match health(base_url).await? {
        Health::Ears { service, streaming } => {
            let want = match role {
                Role::Stt => "ears-stt",
                Role::Llm => "ears-llm",
            };
            if service != want {
                return Err(match role {
                    Role::Stt => "This server can't transcribe speech".to_string(),
                    Role::Llm => "This server can't clean up text".to_string(),
                });
            }
            (ProviderKind::Ears, streaming)
        }
        Health::Other => (ProviderKind::Openai, false),
    };
    let (models, cleanup_models) = list_models(kind, role, base_url, api_key).await?;
    Ok(ProviderStatus {
        kind,
        models,
        streaming,
        cleanup_models,
    })
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
    fn host_of_url() {
        assert_eq!(host("http://localhost:13278/v1"), "localhost:13278");
        assert_eq!(host("https://api.openai.com"), "api.openai.com");
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
