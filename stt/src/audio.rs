// Decode uploaded audio to 16 kHz mono f32. 16 kHz mono 16-bit WAV (what the
// ears app sends) is parsed directly; anything else goes through ffmpeg, which
// whisper.cpp already depends on.

use anyhow::{anyhow, Result};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const SAMPLE_RATE: u32 = 16000;

pub async fn decode(bytes: Vec<u8>) -> Result<Vec<f32>> {
    if let Some(samples) = parse_wav_16k_mono(&bytes) {
        return Ok(samples);
    }
    ffmpeg(bytes).await
}

/// Parse a canonical PCM WAV if it is already 16 kHz mono 16-bit.
fn parse_wav_16k_mono(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut format_ok = false;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = b.get(pos + 8..(pos + 8 + len).min(b.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                let format = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                format_ok = format == 1 && channels == 1 && rate == SAMPLE_RATE && bits == 16;
                if !format_ok {
                    return None;
                }
            }
            b"data" if format_ok => {
                return Some(
                    body.as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| i16::from_le_bytes(*s) as f32 / 32768.0)
                        .collect(),
                );
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

async fn ffmpeg(bytes: Vec<u8>) -> Result<Vec<f32>> {
    let mut child = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
        ])
        .args(["-f", "f32le", "-ac", "1", "-ar", "16000", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow!("failed to run ffmpeg: {e}"))?;

    // Feed stdin while reading stdout so neither pipe fills up and blocks.
    let mut stdin = child.stdin.take().unwrap();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&bytes).await;
    });
    let mut raw = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut raw).await?;
    let _ = writer.await;
    let out = child.wait_with_output().await?;
    if !out.status.success() {
        return Err(anyhow!(
            "could not decode audio: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|s| f32::from_le_bytes(*s))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2 * channels as u32).to_le_bytes());
        b.extend_from_slice(&(2 * channels).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }

    #[test]
    fn parses_16k_mono_wav() {
        let s = parse_wav_16k_mono(&wav(16000, 1, &[0, 16384, -32768])).unwrap();
        assert_eq!(s, vec![0.0, 0.5, -1.0]);
    }

    #[test]
    fn other_wavs_fall_back() {
        assert!(parse_wav_16k_mono(&wav(44100, 1, &[0])).is_none());
        assert!(parse_wav_16k_mono(&wav(16000, 2, &[0, 0])).is_none());
        assert!(parse_wav_16k_mono(b"not audio").is_none());
    }
}
