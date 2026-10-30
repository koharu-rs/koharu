//! Safe wrapper around `llama_context`.

use std::cell::Cell;
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::num::NonZeroI32;
use std::slice;

use crate::llama_batch::LlamaBatch;
use crate::model::{LlamaLoraAdapter, LlamaModel};
use crate::ptr::Ptr;
use crate::sampling::LlamaSampler;
use crate::timing::LlamaTimings;
use crate::token::data::LlamaTokenData;
use crate::token::data_array::LlamaTokenDataArray;
use crate::token::LlamaToken;
use crate::{
    DecodeError, EmbeddingsError, EmbeddingsSeqError, EncodeError, LlamaLoraAdapterRemoveError,
    LlamaLoraAdapterSetError,
};

pub mod kv_cache;
pub mod params;
pub mod session;

/// Safe wrapper around `llama_context`.
#[allow(clippy::module_name_repetitions)]
pub struct LlamaContext<'a> {
    pub(crate) context: Ptr<koharu_llama_sys::llama_context>,
    /// Backend samplers kept alive for the context's lifetime.
    _backend_samplers: Vec<(i32, LlamaSampler)>,
    /// The context internally holds a reference to the model, which can be
    /// retrieved with `llama_get_model`.
    model: PhantomData<&'a LlamaModel>,
    /// Some data in the context acts as-if behind a `Cell`, such as
    /// `t_start_us` and `n_eval`.
    data: PhantomData<Cell<()>>,
}

// SAFETY: The context itself isn't inherently bound to any particular thread.
//
// A lot of operations mutate internal state though (`ctx->synchronize()`),
// which means that this cannot be `Sync`, see `synchronizable_ptr` below.
unsafe impl Send for LlamaContext<'_> {}

impl Debug for LlamaContext<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaContext")
            .field("context", &self.context)
            .finish()
    }
}

impl<'model> LlamaContext<'model> {
    pub(crate) fn new(llama_context: Ptr<koharu_llama_sys::llama_context>) -> Self {
        Self {
            context: llama_context,
            _backend_samplers: Vec::new(),
            model: PhantomData,
            data: PhantomData,
        }
    }

    pub(crate) fn with_samplers(
        llama_context: Ptr<koharu_llama_sys::llama_context>,
        backend_samplers: Vec<(i32, LlamaSampler)>,
    ) -> Self {
        Self {
            context: llama_context,
            _backend_samplers: backend_samplers,
            model: PhantomData,
            data: PhantomData,
        }
    }

    // FIXME(madsmtm): Somehow return `LlamaModel<'_>` here?
    fn model_ptr(&self) -> *const koharu_llama_sys::llama_model {
        unsafe { koharu_llama_sys::llama_get_model(self.context.as_ptr()) }
    }

    /// Gets the max number of logical tokens that can be submitted to decode. Must be greater than or equal to [`Self::n_ubatch`].
    #[must_use]
    pub fn n_batch(&self) -> u32 {
        unsafe { koharu_llama_sys::llama_n_batch(self.context.as_ptr()) }
    }

    /// Gets the max number of physical tokens (hardware level) to decode in batch. Must be less than or equal to [`Self::n_batch`].
    #[must_use]
    pub fn n_ubatch(&self) -> u32 {
        unsafe { koharu_llama_sys::llama_n_ubatch(self.context.as_ptr()) }
    }

    /// Gets the size of the context.
    #[must_use]
    pub fn n_ctx(&self) -> u32 {
        unsafe { koharu_llama_sys::llama_n_ctx(self.context.as_ptr()) }
    }

    /// Decodes the batch.
    ///
    /// # Errors
    ///
    /// - `DecodeError` if the decoding failed.
    ///
    /// # Panics
    ///
    /// - the returned [`std::ffi::c_int`] from llama-cpp does not fit into a i32 (this should never happen on most systems)
    pub fn decode(&mut self, batch: &mut LlamaBatch) -> Result<(), DecodeError> {
        let result =
            unsafe { koharu_llama_sys::llama_decode(self.context.as_mut_ptr(), batch.llama_batch) };

        match NonZeroI32::new(result) {
            None => Ok(()),
            Some(error) => Err(DecodeError::from(error)),
        }
    }

    /// Encodes the batch.
    ///
    /// # Errors
    ///
    /// - `EncodeError` if the decoding failed.
    ///
    /// # Panics
    ///
    /// - the returned [`std::ffi::c_int`] from llama-cpp does not fit into a i32 (this should never happen on most systems)
    pub fn encode(&mut self, batch: &mut LlamaBatch) -> Result<(), EncodeError> {
        let result =
            unsafe { koharu_llama_sys::llama_encode(self.context.as_mut_ptr(), batch.llama_batch) };

        match NonZeroI32::new(result) {
            None => Ok(()),
            Some(error) => Err(EncodeError::from(error)),
        }
    }

    /// Get a mutable pointer to `llama_context` for the cases where the
    /// mutation only happens via `llama_synchronize`.
    ///
    /// # Safety
    ///
    /// Whatever this is passed to must only mutate the state that
    /// `llama_synchronize` mutates, all other state (such as logits etc.)
    /// must remain as-is.
    ///
    /// This is important for allowing logit and embedding methods to return
    /// `&` references, which would otherwise be unsound if these could be
    /// invalidated by a later method call on `LlamaContext`.
    pub(crate) unsafe fn synchronizable_ptr(&self) -> *mut koharu_llama_sys::llama_context {
        // SAFETY: LlamaContext effectively contains `Cell`s in various
        // places, which are updated when `llama_synchronize` is called.
        unsafe { self.context.as_mut_ptr_unsound() }
    }

    /// Explicitly wait until all computations are finished.
    ///
    /// This is automatically done when using methods such as
    /// [`get_logits_ith`][Self::get_logits_ith] to obtain computation results
    /// and is not necessary to call it explicitly in most cases.
    pub fn synchronize(&self) {
        unsafe { koharu_llama_sys::llama_synchronize(self.synchronizable_ptr()) }
    }

    /// Get the embeddings for the `i`th sequence in the current context.
    ///
    /// Returns a slice containing the embeddings for the last decoded batch.
    /// The size is the pooling-derived output width: `n_cls_out` for RANK,
    /// `n_embd_out` otherwise — NOT `n_embd` (llama.h:1029 /
    /// llama-context.cpp's extraction switch).
    ///
    /// # Errors
    ///
    /// - When the current context was constructed without enabling embeddings.
    /// - If the current model had a pooling type of [`koharu_llama_sys::LLAMA_POOLING_TYPE_NONE`]
    /// - If the given sequence index exceeds the max sequence id.
    ///
    /// # Panics
    ///
    /// * `n_embd` does not fit into a usize
    pub fn embeddings_seq_ith(&self, i: i32) -> Result<&[f32], EmbeddingsSeqError> {
        unsafe {
            let embeddings =
                koharu_llama_sys::llama_get_embeddings_seq(self.synchronizable_ptr(), i);

            if embeddings.is_null() {
                Err(EmbeddingsSeqError(()))
            } else {
                Ok(slice::from_raw_parts(embeddings, self.embeddings_out_len()))
            }
        }
    }

    /// Get the embeddings for the `i`th token in the current context.
    ///
    /// # Returns
    ///
    /// A slice containing the embeddings for the last decoded batch of the given token.
    /// The size is the pooling-derived output width: `n_cls_out` for RANK,
    /// `n_embd_out` otherwise — NOT `n_embd` (llama.h:1029 /
    /// llama-context.cpp's extraction switch).
    ///
    /// # Errors
    ///
    /// - When the current context was constructed without enabling embeddings.
    /// - When the given token didn't have logits enabled when it was passed.
    /// - If the given token index exceeds the max token id.
    ///
    /// # Panics
    ///
    /// * `n_embd` does not fit into a usize
    pub fn embeddings_ith(&self, i: i32) -> Result<&[f32], EmbeddingsError> {
        unsafe {
            let embedding =
                koharu_llama_sys::llama_get_embeddings_ith(self.synchronizable_ptr(), i);
            // Technically also possible whenever `i >= batch.n_tokens`, but no good way of checking `n_tokens` here.
            if embedding.is_null() {
                Err(EmbeddingsError(()))
            } else {
                Ok(slice::from_raw_parts(embedding, self.embeddings_out_len()))
            }
        }
    }

    /// The correct output width for an embeddings read, keyed on the context's
    /// LIVE pooling type rather than the model's `n_embd`.
    ///
    /// RANK reads return `float[n_cls_out]` (default 1) per llama.h:1029; every
    /// other pooling mode extracts at `n_embd_out` per llama-context.cpp's
    /// extraction switch (which diverges from `n_embd` whenever
    /// `{arch}.embedding_length_out` is present).
    fn embeddings_out_len(&self) -> usize {
        let pooling = unsafe { koharu_llama_sys::llama_pooling_type(self.context.as_ptr()) };

        let model = self.model_ptr();
        if pooling == koharu_llama_sys::LLAMA_POOLING_TYPE_RANK {
            let n_cls_out = unsafe { koharu_llama_sys::llama_model_n_cls_out(model) };
            usize::try_from(n_cls_out).expect("n_cls_out does not fit into a usize")
        } else {
            let n_embd_out = unsafe { koharu_llama_sys::llama_model_n_embd_out(model) };
            usize::try_from(n_embd_out).expect("n_embd_out does not fit into a usize")
        }
    }

    /// Get the logits for the last token in the context.
    ///
    /// # Returns
    /// An iterator over unsorted `LlamaTokenData` containing the
    /// logits for the last token in the context.
    ///
    /// # Panics
    ///
    /// - underlying logits data is null
    pub fn candidates(&self) -> impl Iterator<Item = LlamaTokenData> + '_ {
        (0_i32..).zip(self.get_logits()).map(|(i, logit)| {
            let token = LlamaToken::new(i);
            LlamaTokenData::new(token, *logit, 0_f32)
        })
    }

    /// Get the token data array for the last token in the context.
    ///
    /// This is a convience method that implements:
    /// ```ignore
    /// LlamaTokenDataArray::from_iter(ctx.candidates(), false)
    /// ```
    ///
    /// # Panics
    ///
    /// - underlying logits data is null
    #[must_use]
    pub fn token_data_array(&self) -> LlamaTokenDataArray {
        LlamaTokenDataArray::from_iter(self.candidates(), false)
    }

    /// Token logits obtained from the last call to `decode()`.
    /// The logits for which `batch.logits[i] != 0` are stored contiguously
    /// in the order they have appeared in the batch.
    /// Rows: number of tokens for which `batch.logits[i] != 0`
    /// Cols: `n_vocab`
    ///
    /// # Returns
    ///
    /// A slice containing the logits for the last decoded token.
    /// The size corresponds to the `n_vocab` parameter of the context's model.
    ///
    /// # Panics
    ///
    /// - `n_vocab` does not fit into a usize
    /// - token data returned is null
    #[must_use]
    pub fn get_logits(&self) -> &[f32] {
        let data = unsafe { koharu_llama_sys::llama_get_logits(self.synchronizable_ptr()) };
        assert!(!data.is_null(), "logits data for last token is null");

        let model = self.model_ptr();
        let vocab = unsafe { koharu_llama_sys::llama_model_get_vocab(model) };
        let n_vocab = unsafe { koharu_llama_sys::llama_vocab_n_tokens(vocab) };
        let len = usize::try_from(n_vocab).expect("n_vocab does not fit into a usize");

        unsafe { slice::from_raw_parts(data, len) }
    }

    /// Get the logits for the ith token in the context.
    ///
    /// # Panics
    ///
    /// - logit `i` is not initialized.
    pub fn candidates_ith(&self, i: i32) -> impl Iterator<Item = LlamaTokenData> + '_ {
        (0_i32..).zip(self.get_logits_ith(i)).map(|(i, logit)| {
            let token = LlamaToken::new(i);
            LlamaTokenData::new(token, *logit, 0_f32)
        })
    }

    /// Get the token data array for the ith token in the context.
    ///
    /// This is a convience method that implements:
    /// ```ignore
    /// LlamaTokenDataArray::from_iter(ctx.candidates_ith(i), false)
    /// ```
    ///
    /// # Panics
    ///
    /// - logit `i` is not initialized.
    #[must_use]
    pub fn token_data_array_ith(&self, i: i32) -> LlamaTokenDataArray {
        LlamaTokenDataArray::from_iter(self.candidates_ith(i), false)
    }

    /// Get the logits for the ith token in the context.
    ///
    /// # Panics
    ///
    /// - `i` is greater than `n_ctx`
    /// - `n_vocab` does not fit into a usize
    /// - logit `i` is not initialized.
    #[must_use]
    pub fn get_logits_ith(&self, i: i32) -> &[f32] {
        let data = unsafe { koharu_llama_sys::llama_get_logits_ith(self.synchronizable_ptr(), i) };

        if data.is_null() {
            panic!("invalid logit index {i}");
        }

        let model = self.model_ptr();
        let vocab = unsafe { koharu_llama_sys::llama_model_get_vocab(model) };
        let n_vocab = unsafe { koharu_llama_sys::llama_vocab_n_tokens(vocab) };
        let len = usize::try_from(n_vocab).expect("n_vocab does not fit into a usize");

        unsafe { slice::from_raw_parts(data, len) }
    }

    /// Reset the timings for the context.
    pub fn reset_timings(&mut self) {
        unsafe { koharu_llama_sys::llama_perf_context_reset(self.context.as_mut_ptr()) }
    }

    /// Returns the timings for the context.
    pub fn timings(&mut self) -> LlamaTimings {
        let timings = unsafe { koharu_llama_sys::llama_perf_context(self.context.as_ptr()) };
        LlamaTimings { timings }
    }

    /// Sets a lora adapter.
    ///
    /// # Errors
    ///
    /// See [`LlamaLoraAdapterSetError`] for more information.
    pub fn lora_adapter_set(
        &mut self,
        adapter: &mut LlamaLoraAdapter,
        scale: f32,
    ) -> Result<(), LlamaLoraAdapterSetError> {
        let mut adapters = [adapter.lora_adapter.as_mut_ptr()];
        let mut scales = [scale];
        let err_code = unsafe {
            koharu_llama_sys::llama_set_adapters_lora(
                self.context.as_mut_ptr(),
                adapters.as_mut_ptr(),
                1,
                scales.as_mut_ptr(),
            )
        };
        if err_code != 0 {
            return Err(LlamaLoraAdapterSetError::ErrorResult(err_code));
        }

        tracing::debug!("Set lora adapter");
        Ok(())
    }

    /// Remove all lora adapters.
    ///
    /// Note: The upstream API now replaces all adapters at once via
    /// `llama_set_adapters_lora`. This clears all adapters from the context.
    ///
    /// # Errors
    ///
    /// See [`LlamaLoraAdapterRemoveError`] for more information.
    pub fn lora_adapter_remove(
        &mut self,
        _adapter: &mut LlamaLoraAdapter,
    ) -> Result<(), LlamaLoraAdapterRemoveError> {
        let err_code = unsafe {
            koharu_llama_sys::llama_set_adapters_lora(
                self.context.as_mut_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
            )
        };
        if err_code != 0 {
            return Err(LlamaLoraAdapterRemoveError::ErrorResult(err_code));
        }

        tracing::debug!("Remove lora adapter");
        Ok(())
    }

    /// Get the backend-sampled token at the given index.
    ///
    /// This is part of the experimental backend sampling API. Only usable
    /// when the context was created with at least one `llama_sampler_seq_config`.
    ///
    /// Returns `None` if no token was sampled at the given index
    /// (i.e. the C API returned `LLAMA_TOKEN_NULL`).
    ///
    /// # Arguments
    ///
    /// * `i` - The token index, matching the order from the batch.
    #[must_use]
    pub fn sampled_token_ith(&self, i: i32) -> Option<LlamaToken> {
        let token =
            unsafe { koharu_llama_sys::llama_get_sampled_token_ith(self.synchronizable_ptr(), i) };
        // LLAMA_TOKEN_NULL is #define'd as -1 in llama.h (not exposed by bindgen)
        if token == -1 {
            None
        } else {
            Some(LlamaToken(token))
        }
    }

    /// Print a breakdown of per-device memory use to the default logger.
    #[cfg(feature = "common")]
    pub fn print_memory_breakdown(&self) {
        unsafe { koharu_llama_sys::llama_rs_memory_breakdown_print(self.context.as_ptr()) }
    }
}

impl Drop for LlamaContext<'_> {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::llama_free(self.context.as_mut_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_impl_all!(LlamaContext<'static>: Send);
    // This would be unsound, see `impl Send for LlamaContext`.
    static_assertions::assert_not_impl_any!(LlamaContext<'static>: Sync);
}
