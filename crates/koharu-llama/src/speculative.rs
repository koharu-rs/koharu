//! Experimental wrappers for llama.cpp speculative decoding helpers.

use crate::context::LlamaContext;
use crate::llama_batch::LlamaBatch;
use crate::ptr::Ptr;
use crate::status_is_ok;
use crate::token::LlamaToken;

/// Parameters for same-model MTP speculative decoding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MtpSpeculativeParams {
    /// Maximum number of draft tokens to propose.
    pub n_max: i32,
    /// Minimum number of draft tokens required before returning a draft.
    pub n_min: i32,
    /// Minimum draft probability accepted by llama.cpp's MTP drafter.
    pub p_min: f32,
}

impl Default for MtpSpeculativeParams {
    fn default() -> Self {
        Self {
            n_max: 3,
            n_min: 0,
            p_min: 0.0,
        }
    }
}

/// Errors returned by the MTP speculative wrapper.
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum MtpSpeculativeError {
    /// Invalid parameters were provided.
    #[error("invalid MTP speculative parameters")]
    InvalidParams,
    /// llama.cpp returned a null speculative handle.
    #[error("llama.cpp failed to initialize MTP speculative decoding")]
    InitFailed,
    /// llama.cpp rejected a wrapper call.
    #[error("llama.cpp MTP speculative call failed with status {0}")]
    Status(i32),
    /// The draft output exceeded the caller-provided bound.
    #[error("llama.cpp MTP draft exceeded configured maximum")]
    DraftOverflow,
}

/// RAII owner for a same-model MTP speculative context.
///
/// This wrapper currently binds llama.cpp's speculative state to sequence 0.
/// Batches passed to [`Self::process`] must therefore contain only sequence 0.
#[derive(Debug)]
pub struct MtpSpeculative<'model> {
    raw: Ptr<koharu_llama_sys::llama_rs_mtp_speculative>,
    target_context: LlamaContext<'model>,
    draft_context: LlamaContext<'model>,
    n_max: usize,
}

// SAFETY: Similar to `LlamaContext`; not bound to any particular thread, but
// also cannot be used from multiple threads at a time.
unsafe impl Send for MtpSpeculative<'_> {}

impl<'model> MtpSpeculative<'model> {
    /// Create a new MTP speculative helper from a target context and an MTP
    /// draft context.
    ///
    /// # Errors
    ///
    /// Returns an error if parameters are invalid or llama.cpp cannot
    /// initialize the speculative implementation for the loaded model.
    pub fn new(
        mut target_context: LlamaContext<'model>,
        mut draft_context: LlamaContext<'model>,
        params: MtpSpeculativeParams,
    ) -> Result<Self, MtpSpeculativeError> {
        if params.n_max <= 0 || params.n_min < 0 || params.n_min > params.n_max {
            return Err(MtpSpeculativeError::InvalidParams);
        }
        let n_max =
            usize::try_from(params.n_max).map_err(|_| MtpSpeculativeError::InvalidParams)?;

        let raw = unsafe {
            koharu_llama_sys::llama_rs_mtp_speculative_init(
                target_context.context.as_mut_ptr(),
                draft_context.context.as_mut_ptr(),
                params.n_max,
                params.n_min,
                params.p_min,
            )
        };
        let raw = Ptr::new(raw).ok_or(MtpSpeculativeError::InitFailed)?;

        Ok(Self {
            raw,
            target_context,
            draft_context,
            n_max,
        })
    }

    /// Access the target context.
    #[must_use]
    pub fn target_context(&self) -> &LlamaContext<'model> {
        &self.target_context
    }

    /// Access the target context for decode and cache rollback operations.
    pub fn target_context_mut(&mut self) -> &mut LlamaContext<'model> {
        &mut self.target_context
    }

    /// Access the draft context for cache rollback operations.
    pub fn draft_context_mut(&mut self) -> &mut LlamaContext<'model> {
        &mut self.draft_context
    }

    /// Begin a new generation from the given prompt tokens.
    ///
    /// # Errors
    ///
    /// Returns an error if llama.cpp rejects the call.
    pub fn begin(&mut self, prompt_tokens: &[LlamaToken]) -> Result<(), MtpSpeculativeError> {
        let prompt = tokens_to_raw(prompt_tokens);
        let status = unsafe {
            koharu_llama_sys::llama_rs_mtp_speculative_begin(
                self.raw.as_mut_ptr(),
                prompt.as_ptr(),
                prompt.len(),
            )
        };
        status_to_result(status)
    }

    /// Process a batch that was just decoded by the target context.
    ///
    /// The batch must contain token input for sequence 0 only.
    ///
    /// # Errors
    ///
    /// Returns an error if llama.cpp cannot update the MTP draft context.
    pub fn process(&mut self, batch: &LlamaBatch<'_>) -> Result<(), MtpSpeculativeError> {
        let status = unsafe {
            koharu_llama_sys::llama_rs_mtp_speculative_process(
                self.raw.as_mut_ptr(),
                std::ptr::from_ref(&batch.llama_batch),
            )
        };
        status_to_result(status)
    }

    /// Generate draft tokens after `id_last`.
    ///
    /// # Errors
    ///
    /// Returns an error if llama.cpp rejects the draft operation or emits more
    /// draft tokens than requested.
    pub fn draft(
        &mut self,
        n_past: i32,
        id_last: LlamaToken,
        prompt_tokens: &[LlamaToken],
    ) -> Result<Vec<LlamaToken>, MtpSpeculativeError> {
        if n_past < 0 {
            return Err(MtpSpeculativeError::InvalidParams);
        }

        let prompt = tokens_to_raw(prompt_tokens);
        let mut raw_out = vec![0; self.n_max];
        let mut out_len = 0_usize;
        let status = unsafe {
            koharu_llama_sys::llama_rs_mtp_speculative_draft(
                self.raw.as_mut_ptr(),
                n_past,
                id_last.0,
                prompt.as_ptr(),
                prompt.len(),
                raw_out.as_mut_ptr(),
                raw_out.len(),
                &raw mut out_len,
            )
        };
        if status == koharu_llama_sys::LLAMA_RS_STATUS_ALLOCATION_FAILED {
            return Err(MtpSpeculativeError::DraftOverflow);
        }
        status_to_result(status)?;
        raw_out.truncate(out_len);
        Ok(raw_out.into_iter().map(LlamaToken).collect())
    }

    /// Notify llama.cpp how many draft tokens the target context accepted.
    ///
    /// # Errors
    ///
    /// Returns an error if llama.cpp rejects the call.
    pub fn accept(&mut self, n_accepted: u16) -> Result<(), MtpSpeculativeError> {
        let status = unsafe {
            koharu_llama_sys::llama_rs_mtp_speculative_accept(self.raw.as_mut_ptr(), n_accepted)
        };
        status_to_result(status)
    }
}

impl Drop for MtpSpeculative<'_> {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::llama_rs_mtp_speculative_free(self.raw.as_mut_ptr()) }
    }
}

fn tokens_to_raw(tokens: &[LlamaToken]) -> Vec<koharu_llama_sys::llama_token> {
    tokens.iter().map(|token| token.0).collect()
}

fn status_to_result(status: koharu_llama_sys::llama_rs_status) -> Result<(), MtpSpeculativeError> {
    if status_is_ok(status) {
        Ok(())
    } else {
        Err(MtpSpeculativeError::Status(status as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_impl_all!(MtpSpeculative<'static>: Send);
    // This would be unsound, see `impl Send for MtpSpeculative`.
    static_assertions::assert_not_impl_any!(MtpSpeculative<'static>: Sync);
}
