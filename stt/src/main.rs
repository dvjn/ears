// ears-stt: OpenAI-compatible speech-to-text server backed by whisper.cpp.
//
//   GET  /health                           identity + capabilities (no auth)
//   GET  /v1/models                        available model ids
//   POST /v1/audio/transcriptions          OpenAI batch transcription
//   WS   /v1/audio/transcriptions/stream   live transcription (ears-specific)

mod audio;
mod config;
mod models;
mod notify;
mod streaming;
mod whisper;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use config::Config;
use whisper::{Model, Options};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_UPLOAD: usize = 100 * 1024 * 1024;

struct AppState {
    cfg: Config,
    vad_model: Option<PathBuf>,
    models: Mutex<HashMap<String, Arc<Model>>>,
}

impl AppState {
    /// The loaded model for an id, loading (and downloading) it on first use.
    async fn model(&self, id: &str) -> Result<Arc<Model>, ApiError> {
        let mut models = self.models.lock().await;
        if let Some(m) = models.get(id) {
            return Ok(m.clone());
        }
        let path = models::ensure_model(&self.cfg.models_dir, id)
            .await
            .map_err(ApiError::server)?;
        eprintln!("loading {id}");
        let vad = self.vad_model.clone();
        let gpu = self.cfg.use_gpu;
        let size = models::size(id).to_string();
        let model =
            tokio::task::spawn_blocking(move || Model::load(&path, &size, vad.as_deref(), gpu))
                .await
                .map_err(ApiError::server)?
                .map_err(ApiError::server)?;
        let model = Arc::new(model);
        models.insert(id.to_string(), model.clone());
        eprintln!("loaded {id}");
        Ok(model)
    }

    fn resolve(&self, id: Option<&str>) -> Result<String, ApiError> {
        match id.filter(|s| !s.is_empty()) {
            None | Some("whisper-1") => Ok(self.cfg.model.clone()),
            Some(id) if models::is_known(id) => Ok(id.to_string()),
            Some(id) => Err(ApiError::bad_request(
                "model_not_found",
                format!("Model '{id}' not found. Use GET /v1/models to list available models."),
            )),
        }
    }

    fn key_ok(&self, token: Option<&str>) -> bool {
        let key = self.cfg.api_key.as_bytes();
        key.is_empty() || token.is_some_and(|t| constant_time_eq(t.as_bytes(), key))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// OpenAI-style error body.
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    fn server(e: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "server_error",
            message: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let kind = if self.status.is_server_error() {
            "server_error"
        } else {
            "invalid_request_error"
        };
        let body = json!({"error": {"message": self.message, "type": kind, "code": self.code}});
        (self.status, Json(body)).into_response()
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

async fn require_key(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    if state.key_ok(bearer(req.headers())) {
        return next.run(req).await;
    }
    let mut resp = ApiError {
        status: StatusCode::UNAUTHORIZED,
        code: "invalid_api_key",
        message: "Invalid API key".into(),
    }
    .into_response();
    resp.headers_mut()
        .insert(header::WWW_AUTHENTICATE, "Bearer".parse().unwrap());
    resp
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "service": "ears-stt",
        "version": VERSION,
        "model": state.cfg.model,
        "streaming": true,
    }))
}

async fn list_models() -> Json<serde_json::Value> {
    let data: Vec<_> = models::MODELS
        .iter()
        .map(|id| json!({"id": id, "object": "model", "created": 1677532384, "owned_by": "ears-stt"}))
        .collect();
    Json(json!({"object": "list", "data": data}))
}

async fn transcribe(
    State(state): State<Arc<AppState>>,
    mut form: Multipart,
) -> Result<Response, ApiError> {
    let mut file = None;
    let mut fields = HashMap::new();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request("invalid_request", e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request("invalid_request", e.to_string()))?;
        if name == "file" {
            file = Some(data.to_vec());
        } else {
            fields.insert(name, String::from_utf8_lossy(&data).into_owned());
        }
    }
    let file = file.ok_or_else(|| ApiError::bad_request("missing_file", "'file' is required"))?;

    let format = fields
        .get("response_format")
        .cloned()
        .unwrap_or_else(|| "json".into());
    if !matches!(format.as_str(), "json" | "text") {
        return Err(ApiError::bad_request(
            "unsupported_value",
            format!("Unsupported response_format '{format}'"),
        ));
    }
    let id = state.resolve(fields.get("model").map(String::as_str))?;
    let temperature = fields
        .get("temperature")
        .and_then(|t| t.parse().ok())
        .unwrap_or(0.0);

    let samples = audio::decode(file)
        .await
        .map_err(|e| ApiError::bad_request("invalid_audio", e.to_string()))?;
    let model = state.model(&id).await?;
    let duration = samples.len() as f32 / audio::SAMPLE_RATE as f32;

    let text = tokio::task::spawn_blocking(move || {
        let opts = Options {
            language: fields.get("language").map(String::as_str),
            prompt: fields.get("prompt").map(String::as_str),
            temperature,
            word_timestamps: false,
        };
        model.transcribe(&samples, &opts).map(|segments| {
            segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>()
                .trim()
                .to_string()
        })
    })
    .await
    .map_err(ApiError::server)?
    .map_err(ApiError::server)?;

    eprintln!(
        "transcribed {duration:.2}s with {id} ({} chars)",
        text.len()
    );
    Ok(if format == "text" {
        text.into_response()
    } else {
        Json(json!({"text": text})).into_response()
    })
}

#[derive(Deserialize)]
struct StreamParams {
    model: Option<String>,
    language: Option<String>,
    api_key: Option<String>,
}

async fn stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StreamParams>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = bearer(&headers).or(params.api_key.as_deref());
    if !state.key_ok(token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = run_stream(state, params, socket).await {
            eprintln!("stream error: {e}");
        }
    })
}

/// Client sends 16 kHz mono s16le PCM as binary frames and {"type":"close"}
/// when done. Server sends {"type":"commit"|"partial","text"} as text firms
/// up, then {"type":"final"} and closes.
async fn run_stream(state: Arc<AppState>, params: StreamParams, mut ws: WebSocket) -> Result<()> {
    let send = |v: serde_json::Value| Message::Text(v.to_string().into());

    let model = match state.resolve(params.model.as_deref()) {
        Ok(id) => state.model(&id).await,
        Err(e) => Err(e),
    };
    let model = match model {
        Ok(m) => m,
        Err(e) => {
            ws.send(send(json!({"type": "error", "message": e.message})))
                .await?;
            return Ok(());
        }
    };

    let proc = Arc::new(std::sync::Mutex::new(streaming::OnlineProcessor::new(
        model,
        params.language.filter(|l| !l.is_empty()),
    )));
    let mut pending = 0usize;

    while let Some(msg) = ws.recv().await {
        match msg? {
            Message::Binary(data) => {
                let pcm: Vec<f32> = data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| i16::from_le_bytes(*s) as f32 / 32768.0)
                    .collect();
                pending += pcm.len();
                proc.lock().unwrap().insert_audio(&pcm);
                if pending >= streaming::MIN_CHUNK {
                    pending = 0;
                    let p = proc.clone();
                    let (commit, partial) =
                        tokio::task::spawn_blocking(move || p.lock().unwrap().process()).await??;
                    if !commit.is_empty() {
                        ws.send(send(json!({"type": "commit", "text": commit})))
                            .await?;
                    }
                    ws.send(send(json!({"type": "partial", "text": partial})))
                        .await?;
                }
            }
            Message::Text(text) => {
                let close = serde_json::from_str::<serde_json::Value>(&text)
                    .is_ok_and(|v| v["type"] == "close");
                if close {
                    break;
                }
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }

    let p = proc.clone();
    let tail = tokio::task::spawn_blocking(move || p.lock().unwrap().finish()).await??;
    if !tail.is_empty() {
        ws.send(send(json!({"type": "commit", "text": tail})))
            .await?;
    }
    ws.send(send(json!({"type": "final"}))).await?;
    ws.send(Message::Close(None)).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env()?;

    let vad_model = if cfg.vad {
        Some(models::ensure_vad(&cfg.models_dir).await?)
    } else {
        None
    };
    let state = Arc::new(AppState {
        cfg,
        vad_model,
        models: Mutex::new(HashMap::new()),
    });
    // Load the default model before accepting requests.
    state
        .model(&state.cfg.model)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;

    let api = Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/audio/transcriptions", post(transcribe))
        .layer(middleware::from_fn_with_state(state.clone(), require_key));
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/audio/transcriptions/stream", get(stream))
        .merge(api)
        .layer(DefaultBodyLimit::max(MAX_UPLOAD))
        .with_state(state.clone());

    let addr = format!("{}:{}", state.cfg.host, state.cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    notify::ready();
    eprintln!("ears-stt {VERSION} listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
