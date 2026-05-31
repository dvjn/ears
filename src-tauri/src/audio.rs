use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{audioadapter_buffers::direct::InterleavedSlice, Fft, FixedSync, Resampler};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

pub struct ActiveRecording {
    pub stop_flag: Arc<AtomicBool>,
    pub samples: Arc<Mutex<Vec<f32>>>,
    pub sample_rate: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub fn start_recording() -> Result<ActiveRecording> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("no input device available"))?;

    let config = device.default_input_config()?;
    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let samples_clone = samples.clone();
    let stop_clone = stop_flag.clone();

    let handle = std::thread::spawn(move || {
        let stream_result = match sample_format {
            cpal::SampleFormat::F32 => build_stream_f32(
                &device,
                &stream_config,
                samples_clone,
                stop_clone.clone(),
                channels,
            ),
            cpal::SampleFormat::I16 => build_stream_i16(
                &device,
                &stream_config,
                samples_clone,
                stop_clone.clone(),
                channels,
            ),
            fmt => {
                eprintln!("unsupported sample format: {fmt:?}");
                return;
            }
        };

        match stream_result {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    eprintln!("failed to start stream: {e}");
                    return;
                }
                while !stop_clone.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                // stream drops here, capturing stops
            }
            Err(e) => eprintln!("failed to build audio stream: {e}"),
        }
    });

    Ok(ActiveRecording {
        stop_flag,
        samples,
        sample_rate,
        thread: Some(handle),
    })
}

fn build_stream_f32(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    stop_flag: Arc<AtomicBool>,
    channels: usize,
) -> Result<cpal::Stream> {
    let stream = device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if stop_flag.load(Ordering::Acquire) {
                return;
            }
            let mut buf = samples.lock().unwrap();
            for chunk in data.chunks(channels) {
                let mono: f32 = chunk.iter().sum::<f32>() / channels as f32;
                buf.push(mono);
            }
        },
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

fn build_stream_i16(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    stop_flag: Arc<AtomicBool>,
    channels: usize,
) -> Result<cpal::Stream> {
    let stream = device.build_input_stream(
        config,
        move |data: &[i16], _: &cpal::InputCallbackInfo| {
            if stop_flag.load(Ordering::Acquire) {
                return;
            }
            let mut buf = samples.lock().unwrap();
            for chunk in data.chunks(channels) {
                let mono: f32 = chunk.iter().map(|s| *s as f32 / 32768.0).sum::<f32>()
                    / channels as f32;
                buf.push(mono);
            }
        },
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    Ok(stream)
}


pub fn stop_and_collect(mut recording: ActiveRecording) -> Vec<f32> {
    recording.stop_flag.store(true, Ordering::Release);
    if let Some(handle) = recording.thread.take() {
        let _ = handle.join();
    }
    recording.samples.lock().unwrap().clone()
}

pub fn resample(samples: Vec<f32>, from_rate: u32) -> Result<Vec<f32>> {
    if from_rate == 16000 {
        return Ok(samples);
    }

    let chunk_size = 1024usize;
    let channels = 1usize;
    let nbr_input_frames = samples.len();

    let mut resampler = Fft::<f32>::new(from_rate as usize, 16000, chunk_size, 2, channels, FixedSync::Input)
        .map_err(|e| anyhow::anyhow!("failed to create resampler: {e}"))?;

    let needed_out = resampler.process_all_needed_output_len(nbr_input_frames);
    let mut outdata = vec![0.0f32; needed_out];

    let input_adapter = InterleavedSlice::new(&samples, channels, nbr_input_frames)
        .map_err(|e| anyhow::anyhow!("input adapter error: {e}"))?;
    let mut output_adapter = InterleavedSlice::new_mut(&mut outdata, channels, needed_out)
        .map_err(|e| anyhow::anyhow!("output adapter error: {e}"))?;

    let (_nbr_in, nbr_out) = resampler
        .process_all_into_buffer(&input_adapter, &mut output_adapter, nbr_input_frames, None)
        .map_err(|e| anyhow::anyhow!("resampling failed: {e}"))?;

    outdata.truncate(nbr_out);
    Ok(outdata)
}
