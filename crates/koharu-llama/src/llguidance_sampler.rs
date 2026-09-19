//! Pure Rust llguidance sampler for constrained decoding.
//!
//! Implements a custom `llama_sampler` using the `llguidance` and `toktrie` Rust crates
//! to enforce grammar constraints (JSON schema, regex, Lark, etc.) during token sampling.

use std::ffi::c_void;
use std::sync::Arc;

use llguidance::Matcher;
use toktrie::{ApproximateTokEnv, TokEnv, TokRxInfo, TokTrie};

use crate::model::LlamaModel;
use crate::sampling::LlamaSampler;
use crate::token::LlamaToken;

/// Internal state for the llguidance sampler.
struct LlgContext {
    matcher: Matcher,
}

/// Build a [`toktrie::TokEnv`] from a [`LlamaModel`]'s vocabulary.
///
/// Use this to construct and reuse an `llguidance::ParserFactory` for the model.
/// Building the tokenizer environment walks and detokenizes the entire vocabulary,
/// so callers should do it once per model rather than once per grammar.
///
/// This mirrors the logic in upstream `llguidance.cpp` — for each token:
/// - Try normal detokenize (special=false)
/// - If empty, detokenize with special=true and prefix with 0xFF marker byte
pub fn llguidance_build_tok_env(model: &LlamaModel) -> TokEnv {
    let n_vocab = model.n_vocab().cast_unsigned();
    let tok_eos = {
        let eot = unsafe { koharu_llama_sys::llama_vocab_eot(model.vocab_ptr()) };
        if eot == -1 {
            model.token_eos().0.cast_unsigned()
        } else {
            eot.cast_unsigned()
        }
    };
    let info = TokRxInfo::new(n_vocab, tok_eos);
    let mut eog_tokens = vec![tok_eos];

    let mut words = Vec::with_capacity(n_vocab as usize);
    for i in 0..n_vocab.cast_signed() {
        let token = LlamaToken(i);
        if model.is_eog_token(token) && i.cast_unsigned() != tok_eos {
            eog_tokens.push(i.cast_unsigned());
        }
        let bytes = model
            .token_to_piece_bytes(token, 32, false, None)
            .unwrap_or_default();
        if bytes.is_empty() {
            let special_bytes = model
                .token_to_piece_bytes(token, 32, true, None)
                .unwrap_or_default();
            if special_bytes.is_empty() {
                words.push(vec![]);
            } else {
                let mut marked = Vec::with_capacity(special_bytes.len() + 1);
                marked.push(0xFF);
                marked.extend(special_bytes);
                words.push(marked);
            }
        } else {
            words.push(bytes);
        }
    }

    let trie = TokTrie::from(&info, &words).with_eos_tokens(&eog_tokens);
    Arc::new(ApproximateTokEnv::new(trie))
}

// --- extern "C" vtable callbacks ---

unsafe extern "C" fn llg_name(
    _smpl: *const koharu_llama_sys::llama_sampler,
) -> *const std::os::raw::c_char {
    c"llguidance".as_ptr()
}

unsafe extern "C" fn llg_accept(
    smpl: *mut koharu_llama_sys::llama_sampler,
    token: koharu_llama_sys::llama_token,
) {
    let ctx = unsafe { &mut *(*smpl).ctx.cast::<LlgContext>() };
    if ctx.matcher.is_stopped() {
        return;
    }
    let _ = ctx.matcher.consume_token(token.cast_unsigned());
}

unsafe extern "C" fn llg_apply(
    smpl: *mut koharu_llama_sys::llama_sampler,
    cur_p: *mut koharu_llama_sys::llama_token_data_array,
) {
    let ctx = unsafe { &mut *(*smpl).ctx.cast::<LlgContext>() };
    let cur_p = unsafe { &mut *cur_p };

    let Ok(mask) = ctx.matcher.compute_mask_or_eos() else {
        return;
    };

    let data = unsafe { std::slice::from_raw_parts_mut(cur_p.data, cur_p.size) };
    for item in data.iter_mut() {
        if !mask.is_allowed(item.id.cast_unsigned()) {
            item.logit = f32::NEG_INFINITY;
        }
    }
}

unsafe extern "C" fn llg_reset(smpl: *mut koharu_llama_sys::llama_sampler) {
    let ctx = unsafe { &mut *(*smpl).ctx.cast::<LlgContext>() };
    let _ = ctx.matcher.reset();
}

unsafe extern "C" fn llg_clone(
    smpl: *const koharu_llama_sys::llama_sampler,
) -> *mut koharu_llama_sys::llama_sampler {
    let ctx = unsafe { &*(*smpl).ctx.cast::<LlgContext>() };
    let new_ctx = Box::new(LlgContext {
        matcher: ctx.matcher.deep_clone(),
    });
    unsafe {
        koharu_llama_sys::llama_sampler_init(
            &raw mut LLG_SAMPLER_I,
            Box::into_raw(new_ctx).cast::<c_void>(),
        )
    }
}

unsafe extern "C" fn llg_free(smpl: *mut koharu_llama_sys::llama_sampler) {
    let ctx_ptr = unsafe { (*smpl).ctx.cast::<LlgContext>() };
    if !ctx_ptr.is_null() {
        drop(unsafe { Box::from_raw(ctx_ptr) });
    }
}

static mut LLG_SAMPLER_I: koharu_llama_sys::llama_sampler_i = koharu_llama_sys::llama_sampler_i {
    name: Some(llg_name),
    accept: Some(llg_accept),
    apply: Some(llg_apply),
    reset: Some(llg_reset),
    clone: Some(llg_clone),
    free: Some(llg_free),
    backend_init: None,
    backend_accept: None,
    backend_apply: None,
    backend_set_input: None,
    backend_reset: None,
    copy_state: None,
};

/// Wrap an already-built [`llguidance::Matcher`] in a llama.cpp sampler.
impl From<Matcher> for LlamaSampler {
    fn from(matcher: Matcher) -> Self {
        let ctx = Box::new(LlgContext { matcher });
        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init(
                &raw mut LLG_SAMPLER_I,
                Box::into_raw(ctx).cast::<c_void>(),
            )
        };
        LlamaSampler { sampler }
    }
}
