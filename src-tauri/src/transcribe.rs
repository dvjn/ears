use anyhow::Result;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub fn load_model(path: &Path) -> Result<WhisperContext> {
    let ctx = WhisperContext::new_with_params(
        path.to_str().ok_or_else(|| anyhow::anyhow!("invalid model path"))?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| anyhow::anyhow!("failed to load model: {e}"))?;
    Ok(ctx)
}

pub fn transcribe(
    ctx: &WhisperContext,
    samples: &[f32],
    language: Option<&str>,
) -> Result<String> {
    let mut state = ctx
        .create_state()
        .map_err(|e| anyhow::anyhow!("failed to create state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(language);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_no_context(true);

    state
        .full(params, samples)
        .map_err(|e| anyhow::anyhow!("transcription failed: {e}"))?;

    let num_segments = state.full_n_segments();

    let mut text = String::new();
    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            let seg_text = segment
                .to_str()
                .map_err(|e| anyhow::anyhow!("failed to get segment text: {e}"))?;
            text.push_str(seg_text.trim());
            if i < num_segments - 1 {
                text.push(' ');
            }
        }
    }

    Ok(text.trim().to_string())
}
