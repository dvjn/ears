// Safe wrapper over the parts of the llama.cpp C API ears-llm uses.

use anyhow::{anyhow, bail, Result};
use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::sync::{Mutex, Once};

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unnecessary_transmutes,
    clippy::all
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// Load the installed ggml backends (CUDA, Vulkan, CPU variants) once, and
/// keep llama.cpp's logging to warnings and errors.
fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        ffi::llama_log_set(Some(log), std::ptr::null_mut());
        ffi::ggml_backend_load_all();
        ffi::llama_backend_init();
    });
}

unsafe extern "C" fn log(
    level: ffi::ggml_log_level,
    text: *const c_char,
    _: *mut std::ffi::c_void,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    // CONT lines continue the previous message, so follow its verdict.
    static SHOWING: AtomicBool = AtomicBool::new(false);
    let show = if level == ffi::ggml_log_level_GGML_LOG_LEVEL_CONT {
        SHOWING.load(Ordering::Relaxed)
    } else {
        let show = level >= ffi::ggml_log_level_GGML_LOG_LEVEL_WARN;
        SHOWING.store(show, Ordering::Relaxed);
        show
    };
    if show && !text.is_null() {
        eprint!("{}", CStr::from_ptr(text).to_string_lossy());
    }
}

pub struct Message<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

pub struct Generation {
    pub text: String,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    /// "stop" (end of generation) or "length" (hit max_tokens).
    pub finish_reason: &'static str,
}

pub struct Params {
    pub temperature: f32,
    pub max_tokens: usize,
    pub seed: u32,
}

struct Inner {
    ctx: *mut ffi::llama_context,
}

/// A loaded model plus one context. Requests are serialized: the context holds
/// the KV cache and is cleared before each one.
pub struct Model {
    model: *mut ffi::llama_model,
    vocab: *const ffi::llama_vocab,
    template: Option<CString>,
    inner: Mutex<Inner>,
    n_ctx: usize,
    n_batch: usize,
}

// The raw pointers are only used under the inner mutex (or read-only).
unsafe impl Send for Model {}
unsafe impl Sync for Model {}

impl Model {
    pub fn load(path: &Path, n_ctx: u32, gpu: bool) -> Result<Model> {
        init();
        let cpath = CString::new(path.to_string_lossy().as_bytes())?;
        unsafe {
            let mut mp = ffi::llama_model_default_params();
            mp.n_gpu_layers = if gpu { -1 } else { 0 };
            let model = ffi::llama_model_load_from_file(cpath.as_ptr(), mp);
            if model.is_null() {
                bail!("failed to load model {}", path.display());
            }

            let mut cp = ffi::llama_context_default_params();
            cp.n_ctx = n_ctx;
            // Prompts are short; a small batch keeps the compute buffer (and
            // VRAM) small.
            cp.n_batch = n_ctx.min(512);
            cp.n_ubatch = cp.n_batch;
            cp.n_seq_max = 1;
            cp.no_perf = true;
            let ctx = ffi::llama_init_from_model(model, cp);
            if ctx.is_null() {
                ffi::llama_model_free(model);
                bail!("failed to create a context for {}", path.display());
            }

            let template = ffi::llama_model_chat_template(model, std::ptr::null());
            let template = (!template.is_null()).then(|| CStr::from_ptr(template).to_owned());
            Ok(Model {
                model,
                vocab: ffi::llama_model_get_vocab(model),
                template,
                inner: Mutex::new(Inner { ctx }),
                n_ctx: ffi::llama_n_ctx(ctx) as usize,
                n_batch: cp.n_batch as usize,
            })
        }
    }

    /// Format messages with the model's chat template (llama.cpp's built-in
    /// implementations of the common templates), ending with the start of the
    /// assistant turn.
    pub fn format(&self, messages: &[Message]) -> Result<String> {
        let owned: Vec<(CString, CString)> = messages
            .iter()
            .map(|m| Ok((CString::new(m.role)?, CString::new(m.content)?)))
            .collect::<Result<_>>()?;
        let chat: Vec<ffi::llama_chat_message> = owned
            .iter()
            .map(|(role, content)| ffi::llama_chat_message {
                role: role.as_ptr(),
                content: content.as_ptr(),
            })
            .collect();
        let tmpl = self
            .template
            .as_ref()
            .map_or(std::ptr::null(), |t| t.as_ptr());

        let mut buf = vec![0u8; messages.iter().map(|m| m.content.len()).sum::<usize>() * 2 + 512];
        loop {
            let n = unsafe {
                ffi::llama_chat_apply_template(
                    tmpl,
                    chat.as_ptr(),
                    chat.len(),
                    true,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as i32,
                )
            };
            if n < 0 {
                bail!("this model's chat template is not supported by llama.cpp");
            }
            if n as usize <= buf.len() {
                buf.truncate(n as usize);
                return Ok(String::from_utf8_lossy(&buf).into_owned());
            }
            buf.resize(n as usize, 0);
        }
    }

    fn tokenize(&self, text: &str) -> Result<Vec<ffi::llama_token>> {
        let mut tokens = vec![0; text.len() + 8];
        let mut n = unsafe {
            ffi::llama_tokenize(
                self.vocab,
                text.as_ptr() as *const c_char,
                text.len() as i32,
                tokens.as_mut_ptr(),
                tokens.len() as i32,
                true,
                true,
            )
        };
        if n < 0 {
            tokens.resize((-n) as usize, 0);
            n = unsafe {
                ffi::llama_tokenize(
                    self.vocab,
                    text.as_ptr() as *const c_char,
                    text.len() as i32,
                    tokens.as_mut_ptr(),
                    tokens.len() as i32,
                    true,
                    true,
                )
            };
        }
        if n < 0 {
            bail!("tokenization failed");
        }
        tokens.truncate(n as usize);
        Ok(tokens)
    }

    fn piece(&self, token: ffi::llama_token, out: &mut Vec<u8>) {
        let mut buf = [0u8; 256];
        let n = unsafe {
            ffi::llama_token_to_piece(
                self.vocab,
                token,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
                0,
                false,
            )
        };
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
        }
    }

    /// Generate a completion for an already formatted prompt.
    pub fn generate(&self, prompt: &str, params: &Params) -> Result<Generation> {
        let mut tokens = self.tokenize(prompt)?;
        let prompt_tokens = tokens.len();
        if prompt_tokens + 1 >= self.n_ctx {
            bail!(
                "prompt is {prompt_tokens} tokens, more than this model's context of {}",
                self.n_ctx
            );
        }
        let max_tokens = params.max_tokens.min(self.n_ctx - prompt_tokens - 1);

        let inner = self.inner.lock().unwrap();
        unsafe {
            ffi::llama_memory_clear(ffi::llama_get_memory(inner.ctx), true);

            for chunk in tokens.chunks_mut(self.n_batch) {
                let batch = ffi::llama_batch_get_one(chunk.as_mut_ptr(), chunk.len() as i32);
                if ffi::llama_decode(inner.ctx, batch) != 0 {
                    bail!("decoding the prompt failed");
                }
            }

            let sampler = Sampler::new(params);
            let mut out = Vec::new();
            let mut finish_reason = "length";
            let mut completion_tokens = 0;
            while completion_tokens < max_tokens {
                let mut token = ffi::llama_sampler_sample(sampler.0, inner.ctx, -1);
                if ffi::llama_vocab_is_eog(self.vocab, token) {
                    finish_reason = "stop";
                    break;
                }
                completion_tokens += 1;
                self.piece(token, &mut out);
                let batch = ffi::llama_batch_get_one(&mut token, 1);
                if ffi::llama_decode(inner.ctx, batch) != 0 {
                    return Err(anyhow!("decoding failed"));
                }
            }

            Ok(Generation {
                text: String::from_utf8_lossy(&out).into_owned(),
                prompt_tokens,
                completion_tokens,
                finish_reason,
            })
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free(self.inner.get_mut().unwrap().ctx);
            ffi::llama_model_free(self.model);
        }
    }
}

struct Sampler(*mut ffi::llama_sampler);

impl Sampler {
    fn new(params: &Params) -> Self {
        unsafe {
            let chain = ffi::llama_sampler_chain_init(ffi::llama_sampler_chain_default_params());
            if params.temperature <= 0.0 {
                ffi::llama_sampler_chain_add(chain, ffi::llama_sampler_init_greedy());
            } else {
                ffi::llama_sampler_chain_add(
                    chain,
                    ffi::llama_sampler_init_temp(params.temperature),
                );
                ffi::llama_sampler_chain_add(chain, ffi::llama_sampler_init_dist(params.seed));
            }
            Sampler(chain)
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        unsafe { ffi::llama_sampler_free(self.0) }
    }
}
