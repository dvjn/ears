// Live preview over ears-stt's streaming websocket.
//
// Audio goes up as 16 kHz mono s16le frames; the server answers with
// "commit" (text that won't change) and "partial" (the volatile tail) messages,
// which are forwarded to the UI as the same `live-commit` / `live-partial`
// events the batch preview uses. The final transcript still comes from one
// batch pass over the whole recording, so the preview only has to be close.

use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::live::{ApiConfig, CaptionPayload};

/// Converts mono audio from the device rate to 16 kHz as it arrives. Linear
/// interpolation is plenty for a preview; the final pass resamples properly.
pub struct LinearResampler {
    step: f64, // source samples per output sample
    pos: f64,  // read position in `buf`
    buf: Vec<f32>,
}

impl LinearResampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            step: from_rate as f64 / to_rate as f64,
            pos: 0.0,
            buf: Vec::new(),
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.buf.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        while self.pos + 1.0 < self.buf.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(self.buf[i] * (1.0 - frac) + self.buf[i + 1] * frac);
            self.pos += self.step;
        }
        let consumed = (self.pos as usize).min(self.buf.len());
        self.buf.drain(..consumed);
        self.pos -= consumed as f64;
        out
    }
}

/// Stream 16 kHz audio from `audio` to ears-stt until the channel closes, then
/// wait briefly for the server's final words. Sets `connected` once the
/// websocket is up so the caller can skip its per-utterance batch preview;
/// on any failure it stays false and the batch preview carries on.
pub async fn run(
    app: AppHandle,
    config: ApiConfig,
    mut audio: UnboundedReceiver<Vec<f32>>,
    connected: Arc<AtomicBool>,
    committed: Arc<Mutex<String>>,
) {
    let mut url = crate::providers::stream_url(&config.base_url);
    let mut query = vec![format!("model={}", config.model)];
    if let Some(lang) = &config.language {
        query.push(format!("language={lang}"));
    }
    url = format!("{url}?{}", query.join("&"));

    let mut request = match url.into_client_request() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[stream] bad url: {e}");
            return;
        }
    };
    if !config.api_key.is_empty() {
        if let Ok(v) = format!("Bearer {}", config.api_key).parse() {
            request.headers_mut().insert("Authorization", v);
        }
    }

    let connect = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(request),
    );
    let ws = match connect.await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => {
            eprintln!("[stream] connect failed, using batch preview: {e}");
            return;
        }
        Err(_) => {
            eprintln!("[stream] connect timed out, using batch preview");
            return;
        }
    };
    connected.store(true, Ordering::Release);
    eprintln!("[stream] connected");
    let (mut tx, mut rx) = ws.split();

    let reader = {
        let app = app.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = rx.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                let text = v["text"].as_str().unwrap_or_default().to_string();
                match v["type"].as_str() {
                    Some("commit") => {
                        let full = {
                            let mut c = committed.lock().unwrap();
                            c.push_str(&text);
                            c.trim().to_string()
                        };
                        let _ = app.emit("live-commit", CaptionPayload { text: full });
                    }
                    Some("partial") => {
                        let text = text.trim().to_string();
                        let _ = app.emit("live-partial", CaptionPayload { text });
                    }
                    Some("final") => break,
                    Some("error") => {
                        let msg = v["message"].as_str().unwrap_or("stream error").to_string();
                        let _ = app.emit("live-error", msg);
                        break;
                    }
                    _ => {}
                }
            }
        })
    };

    while let Some(chunk) = audio.recv().await {
        let bytes: Vec<u8> = chunk
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect();
        if tx.send(Message::Binary(bytes.into())).await.is_err() {
            break;
        }
    }
    let _ = tx
        .send(Message::Text(r#"{"type":"close"}"#.to_string().into()))
        .await;
    // The preview is only cosmetic once recording stops; don't hold up the
    // final pass for long.
    let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
    let _ = tx.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_48k_to_16k_across_chunks() {
        let mut r = LinearResampler::new(48000, 16000);
        let input: Vec<f32> = (0..4800).map(|i| i as f32).collect();
        let mut out = Vec::new();
        for chunk in input.chunks(333) {
            out.extend(r.process(chunk));
        }
        assert!((out.len() as i64 - 1600).abs() <= 1);
        // Every third source sample.
        assert_eq!(out[10], 30.0);
        assert_eq!(out[1000], 3000.0);
    }

    #[test]
    fn same_rate_is_passthrough() {
        let mut r = LinearResampler::new(16000, 16000);
        let a = r.process(&[1.0, 2.0, 3.0]);
        let b = r.process(&[4.0]);
        assert_eq!([a, b].concat(), vec![1.0, 2.0, 3.0]);
    }
}
