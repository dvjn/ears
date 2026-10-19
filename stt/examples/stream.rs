// Stream a 16 kHz mono 16-bit WAV to a running ears-stt in real time and
// print what comes back:
//   cargo run -p ears-stt --example stream -- file.wav [ws://127.0.0.1:13278] [api-key]

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let wav = std::fs::read(&args[1])?;
    let base = args.get(2).map_or("ws://127.0.0.1:13278", String::as_str);
    let mut req = format!("{base}/v1/audio/transcriptions/stream").into_client_request()?;
    if let Some(key) = args.get(3) {
        req.headers_mut()
            .insert("Authorization", format!("Bearer {key}").parse()?);
    }
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    let (mut tx, mut rx) = ws.split();

    // Skip the 44-byte canonical header; send 250 ms chunks at real time.
    let pcm = wav[44..].to_vec();
    let sender = tokio::spawn(async move {
        for chunk in pcm.chunks(8000) {
            tx.send(Message::Binary(chunk.to_vec().into())).await?;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        tx.send(Message::Text(r#"{"type":"close"}"#.into())).await?;
        anyhow::Ok(())
    });
    while let Some(msg) = rx.next().await {
        if let Message::Text(t) = msg? {
            println!("{t}");
            if t.contains(r#""final""#) {
                break;
            }
        }
    }
    sender.await??;
    Ok(())
}
