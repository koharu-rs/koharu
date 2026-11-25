//! Loading a model into the device's free memory, and the llama.cpp context
//! its sequences decode in.

use std::{
    ffi::CString,
    num::NonZeroU32,
    path::Path,
    pin::Pin,
    sync::{Mutex, OnceLock, PoisonError},
};

use anyhow::{Context, Result, ensure};
use koharu_llama::{
    context::{LlamaContext, params::LlamaContextParams},
    llama_backend::LlamaBackend,
    model::{LlamaModel, params::LlamaModelParams},
    mtmd::{MtmdContext, MtmdContextParams},
};
use koharu_llama_sys::GGML_LOG_LEVEL_WARN;

use super::Files;
use crate::local::Size;

/// The free device memory a load leaves, and the least context a sequence is
/// fitted down to: llama.cpp's defaults for `--fit`.
const MARGIN: usize = 1 << 30;
const MIN_CONTEXT: u32 = 4096;

fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);
    }
    koharu_llama::send_logs_to_tracing(koharu_llama::LogOptions::default());
    let backend = LlamaBackend::init().context("couldn't start llama.cpp")?;
    Ok(BACKEND.get_or_init(|| backend))
}

/// The model, loaded to fit, and the `size` it fits.
pub(super) fn model(files: &Files, size: Size) -> Result<(LlamaModel, Size)> {
    let (params, context) = fit(files, size)?;
    let model = LlamaModel::load_from_file(backend()?, &files.model, &params)
        .with_context(|| format!("couldn't load {}", files.model.display()))?;
    Ok((model, Size { context, ..size }))
}

/// Fits the model to the device's free memory, and answers how it should load
/// and the context each sequence gets. Fit leaves alone a context and a layer
/// count the caller sets (common/fit.cpp), so it gets neither: it measures from
/// the model's trained context, reduces that, then moves layers to system
/// memory, and the sequences keep to `size` below what it allows.
fn fit(files: &Files, size: Size) -> Result<(Pin<Box<LlamaModelParams>>, u32)> {
    // Fit swaps llama.cpp's global logger while it runs.
    static FITTING: Mutex<()> = Mutex::new(());
    backend()?;
    let mut params = Box::pin(LlamaModelParams::default().with_devices(&[files.device])?);
    let mut context = context_params(size, None, None);
    // Fit doesn't count the projector.
    let projector = match &files.projector {
        Some(projector) => fs_err::metadata(projector)?.len() as usize,
        None => 0,
    };
    let mut margins = vec![MARGIN + projector; koharu_llama::max_devices()];
    let path = files
        .model
        .to_str()
        .with_context(|| format!("{} has no UTF-8 path", files.model.display()))?;
    let path = CString::new(path)?;
    let fitted = {
        let _fitting = FITTING.lock().unwrap_or_else(PoisonError::into_inner);
        params.as_mut().fit_params(
            &path,
            &mut context,
            &mut margins,
            MIN_CONTEXT.min(size.context),
            GGML_LOG_LEVEL_WARN,
        )
    }
    .with_context(|| format!("{} doesn't fit in free memory", files.model.display()))?;
    let context = fitted
        .n_ctx
        .min(size.sequences.saturating_mul(size.context))
        / size.sequences;
    ensure!(context > 0, "{} got no context", files.model.display());
    Ok((params, context))
}

/// The vision projector at `path`, which must read images.
pub(super) fn projector(model: &LlamaModel, path: &Path) -> Result<MtmdContext> {
    let utf8 = path.to_str().context("the projector's path isn't UTF-8")?;
    let mtmd = MtmdContext::init_from_file(
        utf8,
        model,
        &MtmdContextParams {
            use_gpu: true,
            print_timings: false,
            ..MtmdContextParams::default()
        },
    )
    .with_context(|| format!("couldn't load {}", path.display()))?;
    ensure!(mtmd.support_vision(), "{} reads no images", path.display());
    Ok(mtmd)
}

/// The context every sequence of `size` decodes in.
pub(super) fn context<'m>(
    model: &'m LlamaModel,
    size: Size,
    mtmd: Option<&MtmdContext>,
) -> Result<LlamaContext<'m>> {
    // An image decoded with non-causal attention must fit in one micro-batch.
    let non_causal = mtmd.is_some_and(MtmdContext::decode_use_non_causal);
    let params = context_params(
        size,
        Some(size.sequences * size.context),
        non_causal.then_some(batch_size(size)),
    );
    model
        .new_context(backend()?, params)
        .context("couldn't create the llama.cpp context")
}

/// The batch a context decodes in: every running sequence decodes one token per
/// step.
pub(super) fn batch_size(size: Size) -> usize {
    (size.context as usize)
        .min(2048)
        .max(size.sequences as usize)
}

/// A context for `size`. Separate KV streams, so each sequence holds its own
/// tokens; a whole context of `None` is for fit to choose. A layer that attends
/// to a sliding window keeps that window, as llama.cpp's own tools have it
/// (common/common.h), where its library keeps the whole context by default.
fn context_params(size: Size, context: Option<u32>, n_ubatch: Option<usize>) -> LlamaContextParams {
    let n_batch = batch_size(size);
    LlamaContextParams::default()
        .with_n_ctx(context.and_then(NonZeroU32::new))
        .with_n_seq_max(size.sequences)
        .with_kv_unified(false)
        .with_swa_full(false)
        .with_n_batch(n_batch as u32)
        .with_n_ubatch(n_ubatch.unwrap_or_else(|| n_batch.min(512)) as u32)
}
