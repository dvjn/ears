// Live dictation over the STANDARD OpenAI-compatible /v1/audio/transcriptions
// endpoint, with ears-stt's streaming websocket as a better live preview when
// the provider is an ears service (see stream.rs).
//
// Standard Whisper has no streaming API, so without ears-stt "live" is faked
// client-side:
//   * capture audio continuously,
//   * a simple energy VAD splits it at pauses into utterances,
//   * each finished utterance is transcribed with a normal batch request and
//     appended to the on-screen paragraph (live preview),
//   * the same silence detection drives auto-stop.
//
// On stop, the concatenated *speech* audio (silences trimmed by the VAD) is
// re-transcribed in a single pass for the most accurate final text. Trimming
// silence also avoids Whisper's "thank you" hallucinations on quiet audio, so
// this works even against vanilla OpenAI with no server-side VAD.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use reqwest::multipart;
use rubato::{audioadapter_buffers::direct::InterleavedSlice, Fft, FixedSync, Resampler};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::stream::LinearResampler;

const TARGET_RATE: u32 = 16000;

// VAD tuning (milliseconds).
const FRAME_MS: f32 = 30.0; // analysis frame size
const SPEECH_ONSET_MS: f32 = 150.0; // voiced run needed to enter speech
const UTTERANCE_GAP_MS: f32 = 600.0; // silence run that ends an utterance
const PREROLL_MS: f32 = 200.0; // audio kept before onset so words aren't clipped
                               // During continuous speech (no pause), force a chunk after this long so the
                               // live transcript keeps flowing instead of waiting for a pause.
const MAX_SEGMENT_MS: f32 = 6000.0;

// A microphone that sends no audio (no callbacks, or only zeros) for MIC_WAIT
// after opening is reopened, up to MIC_RETRIES times, before giving up.
const MIC_WAIT: Duration = Duration::from_secs(2);
const MIC_RETRIES: u32 = 3;

#[derive(Clone)]
pub struct ApiConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub language: Option<String>,
    /// Try ears-stt's streaming websocket for the live preview.
    pub stream: bool,
    /// Microphone id (see mics.rs); `None` uses the system default.
    pub input_device: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct CaptionPayload {
    pub text: String,
}

/// Shared HTTP client for all provider requests. A request timeout is essential:
/// without it a hung endpoint would block `shutdown()` forever and wedge the
/// app's state machine in `Transcribing`. Reusing one client also keeps the
/// connection pool/TLS session warm across utterances.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client")
    })
}

/// Validate a provider base URL before the stored bearer key is ever attached
/// to a request built from it. The key is a secret, so we refuse to send it to
/// non-http(s) schemes or to cloud/link-local metadata endpoints (a classic
/// SSRF target, e.g. `169.254.169.254`). Other private/LAN ranges are allowed
/// on purpose — this app legitimately supports self-hosted, OpenAI-compatible
/// transcription servers.
pub fn validate_base_url(base_url: &str) -> std::result::Result<(), String> {
    let url = reqwest::Url::parse(base_url).map_err(|_| "invalid base URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("unsupported URL scheme: {}", url.scheme()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| "base URL has no host".to_string())?;
    if host.starts_with("169.254.") {
        return Err("base URL host is not allowed".to_string());
    }
    Ok(())
}

/// Outcome of finishing a live session. Distinguishing these lets the UI report
/// an accurate message instead of collapsing every empty result into
/// "No speech transcribed".
pub enum FinalTranscript {
    /// The best available transcript (final pass, or per-utterance preview).
    Text(String),
    /// Audio was captured but speech was never detected.
    NoSpeech,
    /// Speech was detected but transcription returned no text.
    NoText,
    /// Transcription failed and no preview text was available.
    Error(String),
}

pub struct LiveSession {
    id: u64,
    stop: Arc<AtomicBool>,
    committed: Arc<Mutex<String>>, // live preview (per-utterance concatenation)
    speech_audio: Arc<Mutex<Vec<f32>>>, // concatenated speech at device rate
    sample_rate: u32,
    config: ApiConfig,
    last_activity: Arc<Mutex<Instant>>,
    had_speech: Arc<AtomicBool>,
    mic_failed: Arc<AtomicBool>,
    sink: Arc<Mutex<Option<UnboundedSender<Vec<f32>>>>>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    proc_task: Option<tokio::task::JoinHandle<()>>,
    stream_task: Option<tokio::task::JoinHandle<()>>,
}

impl LiveSession {
    /// True once speech has been detected and no voiced audio has arrived for
    /// `timeout`. Drives silence-based auto-stop.
    pub fn silent_for(&self, timeout: Duration) -> bool {
        self.had_speech.load(Ordering::Acquire)
            && self.last_activity.lock().unwrap().elapsed() >= timeout
    }

    /// Unique per session, so a watcher can tell its session from a newer one.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// True once the microphone stayed silent through every reopen.
    pub fn mic_failed(&self) -> bool {
        self.mic_failed.load(Ordering::Acquire)
    }

    /// Stop capture, finish pending utterances, then re-transcribe the full
    /// speech in one pass and return the most accurate final transcript.
    pub async fn shutdown(mut self) -> FinalTranscript {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.capture_thread.take() {
            let _ = tokio::task::spawn_blocking(move || {
                let _ = t.join();
            })
            .await;
        }
        // Sender dropped -> proc loop drains and ends, which closes the
        // stream's audio channel too.
        self.sink.lock().unwrap().take();
        if let Some(h) = self.proc_task.take() {
            let _ = h.await;
        }
        if let Some(h) = self.stream_task.take() {
            let _ = h.await;
        }

        let speech = self.speech_audio.lock().unwrap().clone();
        let committed = self.committed.lock().unwrap().trim().to_string();
        let had_speech = self.had_speech.load(Ordering::Acquire);

        // Per-utterance previews are our fallback when the final pass can't
        // improve on them.
        let preview = || {
            if committed.is_empty() {
                if had_speech {
                    FinalTranscript::NoText
                } else {
                    FinalTranscript::NoSpeech
                }
            } else {
                FinalTranscript::Text(committed.clone())
            }
        };

        if speech.is_empty() {
            return preview();
        }

        let config = self.config.clone();
        let sr = self.sample_rate;
        let result = tokio::task::spawn_blocking(move || resample_to_16k(speech, sr))
            .await
            .ok()
            .and_then(|r| r.ok());
        let Some(samples) = result else {
            return preview();
        };

        // Bound the final pass more tightly than per-utterance requests: the UI
        // is pinned in `Transcribing` until this returns, so a slow endpoint
        // shouldn't hold it for the full client timeout.
        match transcribe_wav(&samples, &config, Some(Duration::from_secs(15))).await {
            Ok(text) if !text.trim().is_empty() => FinalTranscript::Text(text.trim().to_string()),
            Ok(_) => preview(),
            // Surface the error only if we have nothing better to show.
            Err(e) if committed.is_empty() => FinalTranscript::Error(e.to_string()),
            Err(_) => FinalTranscript::Text(committed),
        }
    }
}

impl Drop for LiveSession {
    /// Graceful teardown goes through `shutdown()`, which consumes `self`. If a
    /// session is instead dropped directly (e.g. app quit while recording), make
    /// sure the capture thread and processing task don't keep running and
    /// holding the input stream.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sink.lock().unwrap().take();
        if let Some(t) = self.proc_task.take() {
            t.abort();
        }
        if let Some(t) = self.stream_task.take() {
            t.abort();
        }
        // The capture thread observes `stop` and exits on its own; we don't join
        // here to avoid blocking in a Drop.
    }
}

pub fn start_live(app: AppHandle, config: ApiConfig) -> Result<LiveSession> {
    let device = crate::mics::open(config.input_device.as_deref())?;
    let dev_config = device.default_input_config()?;
    let stream_config = dev_config.config();
    let sample_rate = stream_config.sample_rate;
    let channels = stream_config.channels as usize;
    // Record f32 when the device offers it at this layout (a pulse source
    // always does), else in the device's own format.
    let f32_ok = device.supported_input_configs()?.any(|r| {
        r.sample_format() == SampleFormat::F32
            && r.channels() == stream_config.channels
            && (r.min_sample_rate()..=r.max_sample_rate()).contains(&sample_rate)
    });
    let sample_format = if f32_ok {
        SampleFormat::F32
    } else {
        dev_config.sample_format()
    };

    let stop = Arc::new(AtomicBool::new(false));
    let mic_failed = Arc::new(AtomicBool::new(false));
    let (tx, rx) = unbounded_channel::<Vec<f32>>();

    // Audio capture thread: pushes mono f32 chunks (at device rate) to the
    // channel, reopening a microphone that sends no audio.
    let stop_capture = stop.clone();
    let mic_failed_capture = mic_failed.clone();
    // Callbacks reach the VAD loop through this slot. Stopping empties it,
    // which ends the loop even if a stream is stuck inside the audio backend
    // and never gets dropped.
    let sink = Arc::new(Mutex::new(Some(tx)));
    let capture_sink = sink.clone();
    let capture_thread = std::thread::spawn(move || {
        let heard = Arc::new(AtomicBool::new(false));
        let send = {
            let heard = heard.clone();
            move |mono: Vec<f32>| {
                if mono.iter().any(|s| *s != 0.0) {
                    heard.store(true, Ordering::Release);
                }
                if let Some(tx) = &*capture_sink.lock().unwrap() {
                    let _ = tx.send(mono);
                }
            }
        };
        // Each attempt owns its stream on its own thread until `close` is
        // set. Opening can block (on PulseAudio, play() waits for the first
        // audio), so the loop below never waits on a device directly.
        let open = |close: Arc<AtomicBool>| {
            let device = device.clone();
            let send = send.clone();
            std::thread::spawn(move || {
                let stream = match sample_format {
                    SampleFormat::F32 => build::<f32>(&device, stream_config, send),
                    SampleFormat::I16 => build::<i16>(&device, stream_config, send),
                    SampleFormat::I32 => build::<i32>(&device, stream_config, send),
                    fmt => Err(anyhow!("unsupported sample format: {fmt}")),
                };
                let stream = match stream.and_then(|s| s.play().map(|_| s).map_err(Into::into)) {
                    Ok(s) => s,
                    Err(e) => return eprintln!("[live] opening the microphone failed: {e}"),
                };
                while !close.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                drop(stream);
            });
        };

        for attempt in 0..=MIC_RETRIES {
            if attempt > 0 {
                eprintln!(
                    "[live] no audio from the microphone, reopening it ({attempt}/{MIC_RETRIES})"
                );
            }
            let close = Arc::new(AtomicBool::new(false));
            open(close.clone());
            let opened = Instant::now();
            while !stop_capture.load(Ordering::Acquire)
                && (heard.load(Ordering::Acquire) || opened.elapsed() < MIC_WAIT)
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            close.store(true, Ordering::Release);
            if stop_capture.load(Ordering::Acquire) {
                return;
            }
        }
        eprintln!("[live] still no audio from the microphone, giving up");
        mic_failed_capture.store(true, Ordering::Release);
    });

    eprintln!("[live] capture started: device_rate={sample_rate} channels={channels}");

    let committed = Arc::new(Mutex::new(String::new()));
    let speech_audio = Arc::new(Mutex::new(Vec::<f32>::new()));
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let had_speech = Arc::new(AtomicBool::new(false));

    // ears-stt live preview: a second copy of the audio goes to the websocket.
    let streaming = Arc::new(AtomicBool::new(false));
    let (stream_tx, stream_task) = if config.stream {
        let (stx, srx) = unbounded_channel::<Vec<f32>>();
        let task = tokio::spawn(crate::stream::run(
            app.clone(),
            config.clone(),
            srx,
            streaming.clone(),
            committed.clone(),
        ));
        (Some(stx), Some(task))
    } else {
        (None, None)
    };

    let proc_task = tokio::spawn(run_vad_loop(
        app,
        config.clone(),
        rx,
        sample_rate,
        Preview {
            committed: committed.clone(),
            stream_tx,
            streaming,
        },
        speech_audio.clone(),
        last_activity.clone(),
        had_speech.clone(),
    ));

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    Ok(LiveSession {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        stop,
        committed,
        speech_audio,
        sample_rate,
        config,
        last_activity,
        had_speech,
        mic_failed,
        sink,
        capture_thread: Some(capture_thread),
        proc_task: Some(proc_task),
        stream_task,
    })
}

/// An input stream that hands `send` mono f32 chunks, whatever the device's
/// sample type and channel count.
fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    send: impl Fn(Vec<f32>) + Send + 'static,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &_| {
            let mono = data
                .chunks(channels)
                .map(|frame| {
                    frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                })
                .collect();
            send(mono);
        },
        |err| eprintln!("live audio stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

/// Where live-preview text comes from: the ears-stt websocket when it is
/// connected, otherwise a batch request per utterance.
struct Preview {
    committed: Arc<Mutex<String>>,
    stream_tx: Option<UnboundedSender<Vec<f32>>>,
    streaming: Arc<AtomicBool>,
}

#[allow(clippy::too_many_arguments)]
async fn run_vad_loop(
    app: AppHandle,
    config: ApiConfig,
    mut rx: UnboundedReceiver<Vec<f32>>,
    sample_rate: u32,
    preview: Preview,
    speech_audio: Arc<Mutex<Vec<f32>>>,
    last_activity: Arc<Mutex<Instant>>,
    had_speech: Arc<AtomicBool>,
) {
    let Preview {
        committed,
        stream_tx,
        streaming,
    } = preview;
    let mut resampler = LinearResampler::new(sample_rate, TARGET_RATE);
    let frame_len = ((sample_rate as f32 * FRAME_MS / 1000.0) as usize).max(1);
    let preroll = (sample_rate as f32 * PREROLL_MS / 1000.0) as usize;

    let mut noise = 0.01f32; // adaptive ambient-noise estimate
    let mut voiced_ms = 0.0f32;
    let mut silence_ms = 0.0f32;
    let mut in_speech = false;
    let mut seg: Vec<f32> = Vec::new();
    let mut acc: Vec<f32> = Vec::new();

    while let Some(chunk) = rx.recv().await {
        if let Some(stx) = &stream_tx {
            let _ = stx.send(resampler.process(&chunk));
        }
        acc.extend_from_slice(&chunk);
        while acc.len() >= frame_len {
            let frame: Vec<f32> = acc.drain(..frame_len).collect();
            seg.extend_from_slice(&frame);

            let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
            let threshold = noise * 3.0 + 0.005;

            if rms > threshold {
                voiced_ms += FRAME_MS;
                silence_ms = 0.0;
                *last_activity.lock().unwrap() = Instant::now();
                if !in_speech && voiced_ms >= SPEECH_ONSET_MS {
                    in_speech = true;
                    had_speech.store(true, Ordering::Release);
                    // Show an activity cue while the current utterance is spoken.
                    let _ = app.emit("live-partial", CaptionPayload { text: "…".into() });
                }
            } else {
                silence_ms += FRAME_MS;
                voiced_ms = 0.0;
                if !in_speech {
                    // Adapt the noise floor and keep only a short pre-roll so the
                    // segment buffer doesn't grow during quiet periods.
                    noise = noise * 0.95 + rms * 0.05;
                    if seg.len() > preroll {
                        let excess = seg.len() - preroll;
                        seg.drain(..excess);
                    }
                } else if silence_ms >= UTTERANCE_GAP_MS {
                    in_speech = false;
                    let utt = std::mem::take(&mut seg);
                    finalize_segment(
                        &app,
                        &config,
                        sample_rate,
                        utt,
                        &committed,
                        &speech_audio,
                        &streaming,
                    )
                    .await;
                }
            }

            // Force a chunk during long continuous speech so transcription
            // doesn't stall waiting for a pause.
            if in_speech {
                let seg_ms = seg.len() as f32 / sample_rate as f32 * 1000.0;
                if seg_ms >= MAX_SEGMENT_MS {
                    let utt = std::mem::take(&mut seg);
                    finalize_segment(
                        &app,
                        &config,
                        sample_rate,
                        utt,
                        &committed,
                        &speech_audio,
                        &streaming,
                    )
                    .await;
                    // Still speaking: keep the activity cue visible.
                    let _ = app.emit("live-partial", CaptionPayload { text: "…".into() });
                }
            }
        }
    }

    // Channel closed (stopping): flush a trailing in-progress utterance.
    if in_speech && seg.iter().any(|s| *s != 0.0) {
        finalize_segment(
            &app,
            &config,
            sample_rate,
            seg,
            &committed,
            &speech_audio,
            &streaming,
        )
        .await;
    }
}

/// Append an utterance to the accumulated speech and, unless the streaming
/// preview is running, transcribe it for preview.
async fn finalize_segment(
    app: &AppHandle,
    config: &ApiConfig,
    sample_rate: u32,
    utt: Vec<f32>,
    committed: &Arc<Mutex<String>>,
    speech_audio: &Arc<Mutex<Vec<f32>>>,
    streaming: &AtomicBool,
) {
    speech_audio.lock().unwrap().extend_from_slice(&utt);
    if streaming.load(Ordering::Acquire) {
        return;
    }

    let samples = match tokio::task::spawn_blocking(move || resample_to_16k(utt, sample_rate)).await
    {
        Ok(Ok(s)) => s,
        _ => return,
    };

    match transcribe_wav(&samples, config, None).await {
        Ok(text) if !text.trim().is_empty() => {
            let full = {
                let mut c = committed.lock().unwrap();
                if !c.is_empty() {
                    c.push(' ');
                }
                c.push_str(text.trim());
                c.clone()
            };
            let _ = app.emit("live-commit", CaptionPayload { text: full });
            let _ = app.emit(
                "live-partial",
                CaptionPayload {
                    text: String::new(),
                },
            );
        }
        Ok(_) => {
            let _ = app.emit(
                "live-partial",
                CaptionPayload {
                    text: String::new(),
                },
            );
        }
        Err(e) => {
            let _ = app.emit("live-error", e.to_string());
        }
    }
}

fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len() as u32;
    let data_len = num_samples * 2;
    let file_len = 36 + data_len;

    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_len.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

async fn transcribe_wav(
    samples_16k: &[f32],
    config: &ApiConfig,
    timeout: Option<Duration>,
) -> Result<String> {
    validate_base_url(&config.base_url).map_err(|e| anyhow!(e))?;

    let wav = encode_wav(samples_16k, TARGET_RATE);
    let part = multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")?;
    let mut form = multipart::Form::new()
        .part("file", part)
        .text("model", config.model.clone());
    if let Some(lang) = &config.language {
        form = form.text("language", lang.clone());
    }

    let mut req = http_client()
        .post(format!("{}/audio/transcriptions", config.base_url))
        .bearer_auth(&config.api_key)
        .multipart(form);
    if let Some(t) = timeout {
        req = req.timeout(t);
    }
    let resp = req.send().await?.error_for_status()?;

    let json: serde_json::Value = resp.json().await?;
    Ok(json["text"].as_str().unwrap_or("").trim().to_string())
}

fn resample_to_16k(samples: Vec<f32>, from_rate: u32) -> Result<Vec<f32>> {
    if from_rate == TARGET_RATE || samples.is_empty() {
        return Ok(samples);
    }

    let chunk_size = 1024usize;
    let channels = 1usize;
    let n = samples.len();

    let mut resampler = Fft::<f32>::new(
        from_rate as usize,
        TARGET_RATE as usize,
        chunk_size,
        2,
        channels,
        FixedSync::Input,
    )
    .map_err(|e| anyhow!("failed to create resampler: {e}"))?;

    let needed = resampler.process_all_needed_output_len(n);
    let mut outdata = vec![0.0f32; needed];

    let input = InterleavedSlice::new(&samples, channels, n)
        .map_err(|e| anyhow!("input adapter error: {e}"))?;
    let mut output = InterleavedSlice::new_mut(&mut outdata, channels, needed)
        .map_err(|e| anyhow!("output adapter error: {e}"))?;

    let (_in, nout) = resampler
        .process_all_into_buffer(&input, &mut output, n, None)
        .map_err(|e| anyhow!("resampling failed: {e}"))?;

    outdata.truncate(nout);
    Ok(outdata)
}
