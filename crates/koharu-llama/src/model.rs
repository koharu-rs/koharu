//! A safe wrapper around `llama_model`.
use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::path::Path;
use std::str::Utf8Error;

use crate::context::params::LlamaContextParams;
use crate::context::LlamaContext;
use crate::llama_backend::LlamaBackend;
use crate::model::params::LlamaModelParams;
use crate::ptr::Ptr;
use crate::sampling::LlamaSampler;
use crate::token::LlamaToken;
use crate::vocab::LlamaVocab;
use crate::{
    ApplyChatTemplateError, ChatTemplateError, LlamaContextLoadError, LlamaLoraAdapterInitError,
    LlamaModelLoadError, MetaValError, NewLlamaChatMessageError,
};

pub mod params;
// For backwards compat.
pub use crate::vocab::{LlamaTokenTypeFromIntError, VocabType};

/// A safe wrapper around `llama_model`.
#[derive(Debug)]
#[repr(transparent)]
#[allow(clippy::module_name_repetitions)]
pub struct LlamaModel {
    pub(crate) model: Ptr<koharu_llama_sys::llama_model>,
}

// SAFETY: The model is immutable except for methods where it's passed as `&mut`.
//
// FIXME(madsmtm): Apart from `llama_new_context_with_model`, which mutates.
unsafe impl Send for LlamaModel {}
// SAFETY: Same as above.
unsafe impl Sync for LlamaModel {}

/// A safe wrapper around `llama_lora_adapter`.
#[derive(Debug)]
#[repr(transparent)]
#[allow(clippy::module_name_repetitions)]
pub struct LlamaLoraAdapter {
    pub(crate) lora_adapter: Ptr<koharu_llama_sys::llama_adapter_lora>,
}

// SAFETY: The lora is immutable.
unsafe impl Send for LlamaLoraAdapter {}
// SAFETY: Same as above.
unsafe impl Sync for LlamaLoraAdapter {}

/// A performance-friendly wrapper around [`LlamaModel::chat_template`] which is then
/// fed into [`LlamaModel::apply_chat_template`] to convert a list of messages into an LLM
/// prompt. Internally the template is stored as a `CString` to avoid round-trip conversions
/// within the FFI.
#[derive(Eq, PartialEq, Clone, PartialOrd, Ord, Hash)]
pub struct LlamaChatTemplate(CString);

impl LlamaChatTemplate {
    /// Create a new template from a string. This can either be the name of a llama.cpp [chat template](https://github.com/ggerganov/llama.cpp/blob/8a8c4ceb6050bd9392609114ca56ae6d26f5b8f5/src/llama-chat.cpp#L27-L61)
    /// like "chatml" or "llama3" or an actual Jinja template for llama.cpp to interpret.
    pub fn new(template: &str) -> Result<Self, std::ffi::NulError> {
        Ok(Self(CString::new(template)?))
    }

    /// Accesses the template as a c string reference.
    pub fn as_c_str(&self) -> &CStr {
        &self.0
    }

    /// Attempts to convert the `CString` into a Rust str reference.
    pub fn to_str(&self) -> Result<&str, Utf8Error> {
        self.0.to_str()
    }

    /// Convenience method to create an owned String.
    pub fn to_string(&self) -> Result<String, Utf8Error> {
        self.to_str().map(str::to_string)
    }
}

impl std::fmt::Debug for LlamaChatTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A Safe wrapper around `llama_chat_message`
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct LlamaChatMessage {
    role: CString,
    content: CString,
}

impl LlamaChatMessage {
    /// Create a new `LlamaChatMessage`
    ///
    /// # Errors
    /// If either of ``role`` or ``content`` contain null bytes.
    pub fn new(role: String, content: String) -> Result<Self, NewLlamaChatMessageError> {
        Ok(Self {
            role: CString::new(role)?,
            content: CString::new(content)?,
        })
    }
}

/// The RoPE type that's used within the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RopeType {
    /// Normal RoPE.
    Norm,
    /// Neox-style dimension interleave.
    NeoX,
    /// multi-section rotary embedding (MRoPE).
    MRope,
    /// Interleaved MRoPE.
    IMRope,
    /// Vision RoPE.
    Vision,
}

impl LlamaModel {
    /// Get the model's vocabulary.
    #[must_use]
    pub fn vocab(&self) -> LlamaVocab<'_> {
        let ptr = unsafe { koharu_llama_sys::llama_model_get_vocab(self.model.as_ptr()) };
        LlamaVocab::new(ptr).expect("model must have vocabulary")
    }

    /// get the number of tokens the model was trained on
    ///
    /// # Panics
    ///
    /// If the number of tokens the model was trained on does not fit into an `u32`. This should be impossible on most
    /// platforms due to llama.cpp returning a `c_int` (i32 on most platforms) which is almost certainly positive.
    #[must_use]
    pub fn n_ctx_train(&self) -> u32 {
        let n_ctx_train = unsafe { koharu_llama_sys::llama_n_ctx_train(self.model.as_ptr()) };
        u32::try_from(n_ctx_train).expect("n_ctx_train fits into an u32")
    }

    /// Get all tokens in the model.
    pub fn tokens(&self, decode_special: bool) -> impl Iterator<Item = (LlamaToken, Vec<u8>)> + '_ {
        let vocab = self.vocab();
        vocab.tokens().map(move |llama_token| {
            let bytes = vocab.token_to_piece(llama_token, decode_special, None);
            (llama_token, bytes)
        })
    }

    /// Get the decoder start token.
    #[must_use]
    pub fn decode_start_token(&self) -> LlamaToken {
        let token =
            unsafe { koharu_llama_sys::llama_model_decoder_start_token(self.model.as_ptr()) };
        LlamaToken(token)
    }

    /// The number of tokens the model was trained on.
    ///
    /// This returns a `c_int` for maximum compatibility. Most of the time it can be cast to an i32
    /// without issue.
    #[must_use]
    pub fn n_vocab(&self) -> i32 {
        self.vocab().n_tokens()
    }

    /// This returns a `c_int` for maximum compatibility. Most of the time it can be cast to an i32
    /// without issue.
    #[must_use]
    pub fn n_embd(&self) -> c_int {
        unsafe { koharu_llama_sys::llama_n_embd(self.model.as_ptr()) }
    }

    /// The model's *output* embedding width (`n_embd_out`). This is the width
    /// llama.cpp actually extracts embeddings at — `n_embd` and `n_embd_out`
    /// diverge when `{arch}.embedding_length_out` is present (deepstack models
    /// like qwen3vl). Returns a `c_int` for maximum compatibility.
    #[must_use]
    pub fn n_embd_out(&self) -> c_int {
        unsafe { koharu_llama_sys::llama_model_n_embd_out(self.model.as_ptr()) }
    }

    /// The model's classification output width (`n_cls_out`, default 1) — the
    /// width of a RANK-pooled embeddings read (llama.h:1029).
    #[must_use]
    pub fn n_cls_out(&self) -> u32 {
        unsafe { koharu_llama_sys::llama_model_n_cls_out(self.model.as_ptr()) }
    }

    /// Returns the total size of all the tensors in the model in bytes.
    pub fn size(&self) -> u64 {
        unsafe { koharu_llama_sys::llama_model_size(self.model.as_ptr()) }
    }

    /// Returns the number of parameters in the model.
    pub fn n_params(&self) -> u64 {
        unsafe { koharu_llama_sys::llama_model_n_params(self.model.as_ptr()) }
    }

    /// Returns whether the model is a recurrent network (Mamba, RWKV, etc)
    pub fn is_recurrent(&self) -> bool {
        unsafe { koharu_llama_sys::llama_model_is_recurrent(self.model.as_ptr()) }
    }

    /// Returns whether the model is a hybrid network (Jamba, Granite, Qwen3xx, etc)
    ///
    /// Hybrid models have both attention layers and recurrent/SSM layers.
    /// They require special handling for state checkpointing.
    pub fn is_hybrid(&self) -> bool {
        unsafe { koharu_llama_sys::llama_model_is_hybrid(self.model.as_ptr()) }
    }

    /// Returns the number of layers within the model.
    pub fn n_layer(&self) -> u32 {
        // It's never possible for this to panic because while the API interface is defined as an int32_t,
        // the field it's accessing is a uint32_t.
        u32::try_from(unsafe { koharu_llama_sys::llama_model_n_layer(self.model.as_ptr()) })
            .unwrap()
    }

    /// Returns the number of attention heads within the model.
    pub fn n_head(&self) -> u32 {
        // It's never possible for this to panic because while the API interface is defined as an int32_t,
        // the field it's accessing is a uint32_t.
        u32::try_from(unsafe { koharu_llama_sys::llama_model_n_head(self.model.as_ptr()) }).unwrap()
    }

    /// Returns the number of KV attention heads.
    pub fn n_head_kv(&self) -> u32 {
        // It's never possible for this to panic because while the API interface is defined as an int32_t,
        // the field it's accessing is a uint32_t.
        u32::try_from(unsafe { koharu_llama_sys::llama_model_n_head_kv(self.model.as_ptr()) })
            .unwrap()
    }

    /// Get metadata value as a string by key name
    pub fn meta_val_str(&self, key: &str) -> Result<String, MetaValError> {
        let key_cstring = CString::new(key)?;
        let key_ptr = key_cstring.as_ptr();

        extract_meta_string(
            |buf_ptr, buf_len| unsafe {
                koharu_llama_sys::llama_model_meta_val_str(
                    self.model.as_ptr(),
                    key_ptr,
                    buf_ptr,
                    buf_len,
                )
            },
            256,
        )
    }

    /// Get the number of metadata key/value pairs
    pub fn meta_count(&self) -> i32 {
        unsafe { koharu_llama_sys::llama_model_meta_count(self.model.as_ptr()) }
    }

    /// Get metadata key name by index
    pub fn meta_key_by_index(&self, index: i32) -> Result<String, MetaValError> {
        extract_meta_string(
            |buf_ptr, buf_len| unsafe {
                koharu_llama_sys::llama_model_meta_key_by_index(
                    self.model.as_ptr(),
                    index,
                    buf_ptr,
                    buf_len,
                )
            },
            256,
        )
    }

    /// Get metadata value as a string by index
    pub fn meta_val_str_by_index(&self, index: i32) -> Result<String, MetaValError> {
        extract_meta_string(
            |buf_ptr, buf_len| unsafe {
                koharu_llama_sys::llama_model_meta_val_str_by_index(
                    self.model.as_ptr(),
                    index,
                    buf_ptr,
                    buf_len,
                )
            },
            256,
        )
    }

    /// Returns the rope type of the model.
    pub fn rope_type(&self) -> Option<RopeType> {
        match unsafe { koharu_llama_sys::llama_model_rope_type(self.model.as_ptr()) } {
            koharu_llama_sys::LLAMA_ROPE_TYPE_NONE => None,
            koharu_llama_sys::LLAMA_ROPE_TYPE_NORM => Some(RopeType::Norm),
            koharu_llama_sys::LLAMA_ROPE_TYPE_NEOX => Some(RopeType::NeoX),
            koharu_llama_sys::LLAMA_ROPE_TYPE_MROPE => Some(RopeType::MRope),
            koharu_llama_sys::LLAMA_ROPE_TYPE_IMROPE => Some(RopeType::IMRope),
            koharu_llama_sys::LLAMA_ROPE_TYPE_VISION => Some(RopeType::Vision),
            rope_type => {
                tracing::error!(rope_type = rope_type, "Unexpected rope type from llama.cpp");
                None
            }
        }
    }

    /// Get chat template from model by name. If the name parameter is None, the default chat template will be returned.
    ///
    /// You supply this into [`Self::apply_chat_template`] to get back a string with the appropriate template
    /// substitution applied to convert a list of messages into a prompt the LLM can use to complete
    /// the chat.
    ///
    /// You could also use an external jinja parser, like [minijinja](https://github.com/mitsuhiko/minijinja),
    /// to parse jinja templates not supported by the llama.cpp template engine.
    ///
    /// # Errors
    ///
    /// * If the model has no chat template by that name
    /// * If the chat template is not a valid [`CString`].
    pub fn chat_template(
        &self,
        name: Option<&str>,
    ) -> Result<LlamaChatTemplate, ChatTemplateError> {
        let name_cstr = name.map(CString::new);
        let name_ptr = match name_cstr {
            Some(Ok(name)) => name.as_ptr(),
            _ => std::ptr::null(),
        };
        let result =
            unsafe { koharu_llama_sys::llama_model_chat_template(self.model.as_ptr(), name_ptr) };

        // Convert result to Rust String if not null
        if result.is_null() {
            Err(ChatTemplateError::MissingTemplate)
        } else {
            let chat_template_cstr = unsafe { CStr::from_ptr(result) };
            let chat_template = CString::new(chat_template_cstr.to_bytes())?;
            Ok(LlamaChatTemplate(chat_template))
        }
    }

    /// Loads a model from a file.
    ///
    /// # Errors
    ///
    /// See [`LlamaModelLoadError`] for more information.
    #[tracing::instrument(skip_all, fields(params))]
    pub fn load_from_file(
        _: &LlamaBackend,
        path: impl AsRef<Path>,
        params: &LlamaModelParams,
    ) -> Result<Self, LlamaModelLoadError> {
        let path = path.as_ref();
        debug_assert!(Path::new(path).exists(), "{path:?} does not exist");
        let path = path
            .to_str()
            .ok_or(LlamaModelLoadError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;
        let llama_model =
            unsafe { koharu_llama_sys::llama_load_model_from_file(cstr.as_ptr(), params.params) };

        let model = Ptr::new(llama_model).ok_or(LlamaModelLoadError::NullResult)?;

        tracing::debug!(?path, "Loaded model");
        Ok(LlamaModel { model })
    }

    /// Initializes a lora adapter from a file.
    ///
    /// # Errors
    ///
    /// See [`LlamaLoraAdapterInitError`] for more information.
    pub fn lora_adapter_init(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<LlamaLoraAdapter, LlamaLoraAdapterInitError> {
        let path = path.as_ref();
        debug_assert!(Path::new(path).exists(), "{path:?} does not exist");

        let path = path
            .to_str()
            .ok_or(LlamaLoraAdapterInitError::PathToStrError(
                path.to_path_buf(),
            ))?;

        let cstr = CString::new(path)?;
        let adapter = unsafe {
            koharu_llama_sys::llama_adapter_lora_init(self.model.as_mut_ptr(), cstr.as_ptr())
        };

        let adapter = Ptr::new(adapter).ok_or(LlamaLoraAdapterInitError::NullResult)?;

        tracing::debug!(?path, "Initialized lora adapter");
        Ok(LlamaLoraAdapter {
            lora_adapter: adapter,
        })
    }

    /// Create a new context from this model.
    ///
    /// # Errors
    ///
    /// There is many ways this can fail. See [`LlamaContextLoadError`] for more information.
    // we intentionally do not derive Copy on `LlamaContextParams` to allow llama.cpp to change the type to be non-trivially copyable.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_context<'a>(
        &'a self,
        _: &LlamaBackend,
        params: LlamaContextParams,
    ) -> Result<LlamaContext<'a>, LlamaContextLoadError> {
        let context_params = params.context_params;
        // Constructing multiple contexts from a model in parallel is actually
        // unsound, since `llama_new_context_with_model` mutates the model's
        // `n_ctx_train` in some cases:
        // <https://github.com/ggml-org/llama.cpp/blob/f45576aa86c07d60d346b469651a098a15cc4f81/src/llama-context.cpp#L3751>
        // FIXME(madsmtm): Make this sound!
        let context = unsafe {
            koharu_llama_sys::llama_new_context_with_model(
                self.model.as_mut_ptr_unsound(),
                context_params,
            )
        };
        let context = Ptr::new(context).ok_or(LlamaContextLoadError::NullReturn)?;

        Ok(LlamaContext::new(context))
    }

    /// Create a new context bound to another context via llama.cpp's `ctx_other` field.
    ///
    /// This is required for MTP speculative decoding when the target model's
    /// architecture uses `LLM_ARCH_GEMMA4_ASSISTANT`, which asserts that the draft
    /// context references the target context so KV state can be shared.
    ///
    /// # Errors
    ///
    /// See [`LlamaContextLoadError`].
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_context_with_ctx_other<'a>(
        &'a self,
        _: &LlamaBackend,
        params: LlamaContextParams,
        ctx_other: &LlamaContext<'_>,
    ) -> Result<LlamaContext<'a>, LlamaContextLoadError> {
        let mut context_params = params.context_params;
        // FIXME(madsmtm): Use `.as_ptr()` after:
        // https://github.com/ggml-org/llama.cpp/pull/28316
        context_params.ctx_other = unsafe { ctx_other.context.as_mut_ptr_unsound() };
        // Unsoundness: See `LlamaMode::new_context`.
        let context = unsafe {
            koharu_llama_sys::llama_new_context_with_model(
                self.model.as_mut_ptr_unsound(),
                context_params,
            )
        };
        let context = Ptr::new(context).ok_or(LlamaContextLoadError::NullReturn)?;

        Ok(LlamaContext::new(context))
    }

    /// Creates a new context with backend samplers attached for specific sequences.
    ///
    /// Ownership of the samplers is transferred to the context, ensuring they remain
    /// alive for the context's lifetime. Only samplers that support backend execution
    /// (greedy, dist, temp, top_k, top_p, min_p, logit_bias) will run on the backend.
    ///
    /// # Arguments
    ///
    /// * `params` - Context parameters
    /// * `samplers` - Iterator of `(seq_id, sampler)` pairs where sampler must be a chain
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let sampler = LlamaSampler::chain([
    ///     LlamaSampler::min_p(0.01, 64),
    ///     LlamaSampler::temp(0.1),
    ///     LlamaSampler::dist(42),
    /// ], false);
    ///
    /// let ctx = model.new_context_with_samplers(
    ///     &backend,
    ///     ctx_params,
    ///     [(0, sampler)],
    /// )?;
    /// ```
    #[allow(clippy::needless_pass_by_value)]
    pub fn new_context_with_samplers<'a>(
        &'a self,
        _: &LlamaBackend,
        params: LlamaContextParams,
        samplers: impl IntoIterator<Item = (i32, LlamaSampler)>,
    ) -> Result<LlamaContext<'a>, LlamaContextLoadError> {
        let mut samplers: Vec<_> = samplers.into_iter().collect();
        let mut context_params = params.context_params;

        let mut sampler_configs: Vec<koharu_llama_sys::llama_sampler_seq_config> = samplers
            .iter_mut()
            .map(
                |(seq_id, sampler)| koharu_llama_sys::llama_sampler_seq_config {
                    seq_id: *seq_id,
                    sampler: sampler.sampler.as_mut_ptr(),
                },
            )
            .collect();

        if !sampler_configs.is_empty() {
            context_params.samplers = sampler_configs.as_mut_ptr();
            context_params.n_samplers = sampler_configs.len();
        }

        // Unsoundness: See `LlamaMode::new_context`.
        let context = unsafe {
            koharu_llama_sys::llama_new_context_with_model(
                self.model.as_mut_ptr_unsound(),
                context_params,
            )
        };
        let context = Ptr::new(context).ok_or(LlamaContextLoadError::NullReturn)?;

        Ok(LlamaContext::with_samplers(context, samplers))
    }

    /// Apply the models chat template to some messages.
    /// See <https://github.com/ggerganov/llama.cpp/wiki/Templates-supported-by-llama_chat_apply_template>
    ///
    /// Unlike the llama.cpp `apply_chat_template` which just randomly uses the ChatML template when given
    /// a null pointer for the template, this requires an explicit template to be specified. If you want to
    /// use "chatml", then just do `LlamaChatTemplate::new("chatml")` or any other model name or template
    /// string.
    ///
    /// Use [`Self::chat_template`] to retrieve the template baked into the model (this is the preferred
    /// mechanism as using the wrong chat template can result in really unexpected responses from the LLM).
    ///
    /// You probably want to set `add_ass` to true so that the generated template string ends with a the
    /// opening tag of the assistant. If you fail to leave a hanging chat tag, the model will likely generate
    /// one into the output and the output may also have unexpected output aside from that.
    ///
    /// # Errors
    /// There are many ways this can fail. See [`ApplyChatTemplateError`] for more information.
    #[tracing::instrument(skip_all)]
    pub fn apply_chat_template(
        &self,
        tmpl: &LlamaChatTemplate,
        chat: &[LlamaChatMessage],
        add_ass: bool,
    ) -> Result<String, ApplyChatTemplateError> {
        // Buffer is twice the length of messages per their recommendation
        let message_length = chat.iter().fold(0, |acc, c| {
            acc + c.role.to_bytes().len() + c.content.to_bytes().len()
        });
        let mut buff: Vec<u8> = vec![0; message_length * 2];

        // Build our koharu_llama_sys chat messages
        let chat: Vec<koharu_llama_sys::llama_chat_message> = chat
            .iter()
            .map(|c| koharu_llama_sys::llama_chat_message {
                role: c.role.as_ptr(),
                content: c.content.as_ptr(),
            })
            .collect();

        let tmpl_ptr = tmpl.0.as_ptr();

        let res = unsafe {
            koharu_llama_sys::llama_chat_apply_template(
                tmpl_ptr,
                chat.as_ptr(),
                chat.len(),
                add_ass,
                buff.as_mut_ptr().cast::<c_char>(),
                buff.len().try_into().expect("Buffer size exceeds i32::MAX"),
            )
        };

        if res < 0 {
            return Err(ApplyChatTemplateError::FfiError(res));
        }

        if res > buff.len().try_into().expect("Buffer size exceeds i32::MAX") {
            buff.resize(res.try_into().expect("res is negative"), 0);

            let res = unsafe {
                koharu_llama_sys::llama_chat_apply_template(
                    tmpl_ptr,
                    chat.as_ptr(),
                    chat.len(),
                    add_ass,
                    buff.as_mut_ptr().cast::<c_char>(),
                    buff.len().try_into().expect("Buffer size exceeds i32::MAX"),
                )
            };
            if res < 0 {
                return Err(ApplyChatTemplateError::FfiError(res));
            }
            assert_eq!(Ok(res), buff.len().try_into());
        }
        buff.truncate(res.try_into().expect("res is negative"));
        Ok(String::from_utf8(buff)?)
    }
}

/// Generic helper function for extracting string values from the C API
/// This are specifically useful for the the metadata functions, where we pass in a buffer
/// to be populated by a string, not yet knowing if the buffer is large enough.
/// If the buffer was not large enough, we get the correct length back, which can be used to
/// construct a buffer of appropriate size.
fn extract_meta_string<F>(c_function: F, capacity: usize) -> Result<String, MetaValError>
where
    F: Fn(*mut c_char, usize) -> i32,
{
    let mut buffer = vec![0u8; capacity];

    // call the foreign function
    let result = c_function(buffer.as_mut_ptr().cast::<c_char>(), buffer.len());
    if result < 0 {
        return Err(MetaValError::NegativeReturn(result));
    }

    // check if the response fit in our buffer
    let returned_len = result as usize;
    if returned_len >= capacity {
        // buffer wasn't large enough, try again with the correct capacity.
        return extract_meta_string(c_function, returned_len + 1);
    }

    // verify null termination
    debug_assert_eq!(
        buffer.get(returned_len),
        Some(&0),
        "should end with null byte"
    );

    // resize, convert, and return
    buffer.truncate(returned_len);
    Ok(String::from_utf8(buffer)?)
}

impl Drop for LlamaModel {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::llama_free_model(self.model.as_mut_ptr()) }
    }
}
