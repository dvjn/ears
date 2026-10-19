// Safe wrapper over the parts of the whisper.cpp C API ears-stt uses.

use anyhow::{anyhow, Result};
use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::{Mutex, Once};

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// Centiseconds (whisper.cpp's timestamp unit) to seconds.
fn secs(t: i64) -> f64 {
    t as f64 / 100.0
}

pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub struct Segment {
    pub text: String,
    pub no_speech_prob: f32,
    pub words: Vec<Word>,
}

pub struct Options<'a> {
    pub language: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub temperature: f32,
    pub word_timestamps: bool,
}

/// A loaded model plus one decoding state. Calls are serialized: a state
/// holds the KV cache and can only run one decode at a time.
pub struct Model {
    ctx: *mut ffi::whisper_context,
    state: Mutex<*mut ffi::whisper_state>,
    vad_model: Option<CString>,
    multilingual: bool,
}

// The raw pointers are only used under the state mutex (or read-only).
unsafe impl Send for Model {}
unsafe impl Sync for Model {}

/// Load the installed ggml backends (CUDA, Vulkan, CPU variants) once.
fn load_backends() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { ffi::ggml_backend_load_all() });
}

impl Model {
    /// `size` is the model size without the "whisper-" prefix or a quant
    /// suffix (e.g. "small.en"); it selects the alignment heads used for DTW
    /// word timestamps, which streaming relies on.
    pub fn load(path: &Path, size: &str, vad_model: Option<&Path>, use_gpu: bool) -> Result<Model> {
        load_backends();
        let cpath = CString::new(path.to_string_lossy().as_bytes())?;
        unsafe {
            let mut params = ffi::whisper_context_default_params();
            params.use_gpu = use_gpu;
            if let Some(preset) = alignment_heads(size) {
                // DTW timestamps don't work with flash attention.
                params.dtw_token_timestamps = true;
                params.dtw_aheads_preset = preset;
                params.flash_attn = false;
            } else {
                params.flash_attn = use_gpu;
            }
            let ctx = ffi::whisper_init_from_file_with_params_no_state(cpath.as_ptr(), params);
            if ctx.is_null() {
                return Err(anyhow!("failed to load model {}", path.display()));
            }
            let state = ffi::whisper_init_state(ctx);
            if state.is_null() {
                ffi::whisper_free(ctx);
                return Err(anyhow!("failed to allocate whisper state"));
            }
            Ok(Model {
                ctx,
                state: Mutex::new(state),
                vad_model: vad_model
                    .map(|p| CString::new(p.to_string_lossy().as_bytes()))
                    .transpose()?,
                multilingual: ffi::whisper_is_multilingual(ctx) != 0,
            })
        }
    }

    /// Transcribe 16 kHz mono f32 samples.
    pub fn transcribe(&self, samples: &[f32], opts: &Options) -> Result<Vec<Segment>> {
        // English-only models ignore the language; multilingual ones detect
        // it when none is given.
        let language = match (self.multilingual, opts.language) {
            (false, _) => CString::new("en")?,
            (true, Some(l)) if !l.is_empty() => CString::new(l)?,
            (true, _) => CString::new("auto")?,
        };
        let prompt = opts.prompt.map(CString::new).transpose()?;

        let state = self.state.lock().unwrap();
        unsafe {
            let mut p = ffi::whisper_full_default_params(
                ffi::whisper_sampling_strategy_WHISPER_SAMPLING_GREEDY,
            );
            p.n_threads = std::thread::available_parallelism()
                .map(|n| n.get().min(8) as i32)
                .unwrap_or(4);
            p.language = language.as_ptr();
            p.translate = false;
            p.no_context = true;
            p.print_special = false;
            p.print_progress = false;
            p.print_realtime = false;
            p.print_timestamps = false;
            p.suppress_blank = true;
            p.temperature = opts.temperature;
            p.token_timestamps = opts.word_timestamps;
            if let Some(prompt) = &prompt {
                p.initial_prompt = prompt.as_ptr();
            }
            // Skip non-speech before decoding so quiet audio doesn't produce
            // hallucinated filler ("thank you", "thanks for watching").
            if let Some(vad) = &self.vad_model {
                p.vad = true;
                p.vad_model_path = vad.as_ptr();
                p.vad_params = ffi::whisper_vad_default_params();
                p.vad_params.min_silence_duration_ms = 500;
            }

            let rc = ffi::whisper_full_with_state(
                self.ctx,
                *state,
                p,
                samples.as_ptr(),
                samples.len() as i32,
            );
            if rc != 0 {
                return Err(anyhow!("whisper_full failed ({rc})"));
            }

            let eot = ffi::whisper_token_eot(self.ctx);
            let n = ffi::whisper_full_n_segments_from_state(*state);
            let mut segments = Vec::with_capacity(n as usize);
            for i in 0..n {
                let text = strip_markers(&cstr(ffi::whisper_full_get_segment_text_from_state(
                    *state, i,
                )));
                let no_speech_prob =
                    ffi::whisper_full_get_segment_no_speech_prob_from_state(*state, i);
                let mut words: Vec<Word> = Vec::new();
                if opts.word_timestamps {
                    for t in 0..ffi::whisper_full_n_tokens_from_state(*state, i) {
                        let data = ffi::whisper_full_get_token_data_from_state(*state, i, t);
                        // Prefer the DTW start time when available.
                        let start = if data.t_dtw >= 0 { data.t_dtw } else { data.t0 };
                        if data.id >= eot {
                            continue; // special / timestamp token
                        }
                        let piece = cstr(ffi::whisper_full_get_token_text_from_state(
                            self.ctx, *state, i, t,
                        ));
                        if is_marker(piece.trim()) {
                            continue;
                        }
                        // A leading space starts a new word; anything else
                        // continues the previous one.
                        match words.last_mut() {
                            Some(w) if !piece.starts_with(' ') => {
                                w.text.push_str(&piece);
                                w.end = secs(data.t1).max(w.start);
                            }
                            _ => {
                                if let Some(prev) = words.last_mut() {
                                    prev.end = prev.end.min(secs(start)).max(prev.start);
                                }
                                words.push(Word {
                                    start: secs(start),
                                    end: secs(data.t1).max(secs(start)),
                                    text: piece,
                                })
                            }
                        }
                    }
                }
                segments.push(Segment {
                    text,
                    no_speech_prob,
                    words,
                });
            }
            Ok(segments)
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe {
            ffi::whisper_free_state(*self.state.get_mut().unwrap());
            ffi::whisper_free(self.ctx);
        }
    }
}

fn alignment_heads(size: &str) -> Option<ffi::whisper_alignment_heads_preset> {
    use ffi::*;
    Some(match size {
        "tiny.en" => whisper_alignment_heads_preset_WHISPER_AHEADS_TINY_EN,
        "tiny" => whisper_alignment_heads_preset_WHISPER_AHEADS_TINY,
        "base.en" => whisper_alignment_heads_preset_WHISPER_AHEADS_BASE_EN,
        "base" => whisper_alignment_heads_preset_WHISPER_AHEADS_BASE,
        "small.en" => whisper_alignment_heads_preset_WHISPER_AHEADS_SMALL_EN,
        "small" => whisper_alignment_heads_preset_WHISPER_AHEADS_SMALL,
        "medium.en" => whisper_alignment_heads_preset_WHISPER_AHEADS_MEDIUM_EN,
        "medium" => whisper_alignment_heads_preset_WHISPER_AHEADS_MEDIUM,
        "large-v3" => whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3,
        "large-v3-turbo" => whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3_TURBO,
        _ => return None,
    })
}

/// whisper.cpp emits non-speech annotations like "[BLANK_AUDIO]" or
/// "(music)" as text; a dictation client wants none of them.
fn is_marker(s: &str) -> bool {
    (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('(') && s.ends_with(')'))
}

fn strip_markers(text: &str) -> String {
    if is_marker(text.trim()) {
        String::new()
    } else {
        text.to_string()
    }
}

unsafe fn cstr(p: *const std::os::raw::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}
