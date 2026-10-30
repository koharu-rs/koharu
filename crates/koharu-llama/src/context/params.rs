//! A safe wrapper around `llama_context_params`.
mod get_set;

/// A rusty wrapper around `rope_scaling_type`.
#[repr(i8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RopeScalingType {
    /// The scaling type is unspecified
    Unspecified = -1,
    /// No scaling
    None = 0,
    /// Linear scaling
    Linear = 1,
    /// Yarn scaling
    Yarn = 2,
}

/// Create a `RopeScalingType` from a `c_int` - returns `RopeScalingType::ScalingUnspecified` if
/// the value is not recognized.
impl From<i32> for RopeScalingType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Linear,
            2 => Self::Yarn,
            _ => Self::Unspecified,
        }
    }
}

/// Create a `c_int` from a `RopeScalingType`.
impl From<RopeScalingType> for i32 {
    fn from(value: RopeScalingType) -> Self {
        match value {
            RopeScalingType::None => 0,
            RopeScalingType::Linear => 1,
            RopeScalingType::Yarn => 2,
            RopeScalingType::Unspecified => -1,
        }
    }
}

/// A rusty wrapper around `LLAMA_POOLING_TYPE`.
#[repr(i8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LlamaPoolingType {
    /// The pooling type is unspecified
    Unspecified = -1,
    /// No pooling
    None = 0,
    /// Mean pooling
    Mean = 1,
    /// CLS pooling
    Cls = 2,
    /// Last pooling
    Last = 3,
    /// Rank pooling
    Rank = 4,
}

/// Create a `LlamaPoolingType` from a `c_int` - returns `LlamaPoolingType::Unspecified` if
/// the value is not recognized.
impl From<i32> for LlamaPoolingType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Mean,
            2 => Self::Cls,
            3 => Self::Last,
            4 => Self::Rank,
            _ => Self::Unspecified,
        }
    }
}

/// Create a `c_int` from a `LlamaPoolingType`.
impl From<LlamaPoolingType> for i32 {
    fn from(value: LlamaPoolingType) -> Self {
        match value {
            LlamaPoolingType::None => 0,
            LlamaPoolingType::Mean => 1,
            LlamaPoolingType::Cls => 2,
            LlamaPoolingType::Last => 3,
            LlamaPoolingType::Rank => 4,
            LlamaPoolingType::Unspecified => -1,
        }
    }
}

/// A rusty wrapper around `LLAMA_ATTENTION_TYPE`.
#[repr(i8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LlamaAttentionType {
    /// The attention type is unspecified
    Unspecified = -1,
    /// Causal attention
    Causal = 0,
    /// Non-causal attention
    NonCausal = 1,
}

/// Create a `LlamaAttentionType` from a `c_int` - returns `LlamaAttentionType::Unspecified` if
/// the value is not recognized.
impl From<i32> for LlamaAttentionType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Causal,
            1 => Self::NonCausal,
            _ => Self::Unspecified,
        }
    }
}

/// Create a `c_int` from a `LlamaAttentionType`.
impl From<LlamaAttentionType> for i32 {
    fn from(value: LlamaAttentionType) -> Self {
        match value {
            LlamaAttentionType::Causal => 0,
            LlamaAttentionType::NonCausal => 1,
            LlamaAttentionType::Unspecified => -1,
        }
    }
}

/// A rusty wrapper around `llama_context_type`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LlamaContextType {
    /// Default decoder context.
    Default,
    /// Multi-token-prediction draft context.
    Mtp,
    /// Unknown context type from a newer llama.cpp.
    Unknown(koharu_llama_sys::llama_context_type),
}

impl From<koharu_llama_sys::llama_context_type> for LlamaContextType {
    fn from(value: koharu_llama_sys::llama_context_type) -> Self {
        match value {
            x if x == koharu_llama_sys::LLAMA_CONTEXT_TYPE_DEFAULT => Self::Default,
            x if x == koharu_llama_sys::LLAMA_CONTEXT_TYPE_MTP => Self::Mtp,
            x => Self::Unknown(x),
        }
    }
}

impl From<LlamaContextType> for koharu_llama_sys::llama_context_type {
    fn from(value: LlamaContextType) -> Self {
        match value {
            LlamaContextType::Default => koharu_llama_sys::LLAMA_CONTEXT_TYPE_DEFAULT,
            LlamaContextType::Mtp => koharu_llama_sys::LLAMA_CONTEXT_TYPE_MTP,
            LlamaContextType::Unknown(raw) => raw,
        }
    }
}

/// A rusty wrapper around `ggml_type` for KV cache types.
#[allow(non_camel_case_types, missing_docs)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KvCacheType {
    /// Represents an unknown or not-yet-mapped `ggml_type` and carries the raw value.
    /// When passed through FFI, the raw value is used as-is (if llama.cpp supports it,
    /// the runtime will operate with that type).
    /// This variant preserves API compatibility when new `ggml_type` values are
    /// introduced in the future.
    Unknown(koharu_llama_sys::ggml_type),
    F32,
    F16,
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q8_0,
    Q8_1,
    Q2_K,
    Q3_K,
    Q4_K,
    Q5_K,
    Q6_K,
    Q8_K,
    IQ2_XXS,
    IQ2_XS,
    IQ3_XXS,
    IQ1_S,
    IQ4_NL,
    IQ3_S,
    IQ2_S,
    IQ4_XS,
    I8,
    I16,
    I32,
    I64,
    F64,
    IQ1_M,
    BF16,
    TQ1_0,
    TQ2_0,
    MXFP4,
}

impl From<KvCacheType> for koharu_llama_sys::ggml_type {
    fn from(value: KvCacheType) -> Self {
        match value {
            KvCacheType::Unknown(raw) => raw,
            KvCacheType::F32 => koharu_llama_sys::GGML_TYPE_F32,
            KvCacheType::F16 => koharu_llama_sys::GGML_TYPE_F16,
            KvCacheType::Q4_0 => koharu_llama_sys::GGML_TYPE_Q4_0,
            KvCacheType::Q4_1 => koharu_llama_sys::GGML_TYPE_Q4_1,
            KvCacheType::Q5_0 => koharu_llama_sys::GGML_TYPE_Q5_0,
            KvCacheType::Q5_1 => koharu_llama_sys::GGML_TYPE_Q5_1,
            KvCacheType::Q8_0 => koharu_llama_sys::GGML_TYPE_Q8_0,
            KvCacheType::Q8_1 => koharu_llama_sys::GGML_TYPE_Q8_1,
            KvCacheType::Q2_K => koharu_llama_sys::GGML_TYPE_Q2_K,
            KvCacheType::Q3_K => koharu_llama_sys::GGML_TYPE_Q3_K,
            KvCacheType::Q4_K => koharu_llama_sys::GGML_TYPE_Q4_K,
            KvCacheType::Q5_K => koharu_llama_sys::GGML_TYPE_Q5_K,
            KvCacheType::Q6_K => koharu_llama_sys::GGML_TYPE_Q6_K,
            KvCacheType::Q8_K => koharu_llama_sys::GGML_TYPE_Q8_K,
            KvCacheType::IQ2_XXS => koharu_llama_sys::GGML_TYPE_IQ2_XXS,
            KvCacheType::IQ2_XS => koharu_llama_sys::GGML_TYPE_IQ2_XS,
            KvCacheType::IQ3_XXS => koharu_llama_sys::GGML_TYPE_IQ3_XXS,
            KvCacheType::IQ1_S => koharu_llama_sys::GGML_TYPE_IQ1_S,
            KvCacheType::IQ4_NL => koharu_llama_sys::GGML_TYPE_IQ4_NL,
            KvCacheType::IQ3_S => koharu_llama_sys::GGML_TYPE_IQ3_S,
            KvCacheType::IQ2_S => koharu_llama_sys::GGML_TYPE_IQ2_S,
            KvCacheType::IQ4_XS => koharu_llama_sys::GGML_TYPE_IQ4_XS,
            KvCacheType::I8 => koharu_llama_sys::GGML_TYPE_I8,
            KvCacheType::I16 => koharu_llama_sys::GGML_TYPE_I16,
            KvCacheType::I32 => koharu_llama_sys::GGML_TYPE_I32,
            KvCacheType::I64 => koharu_llama_sys::GGML_TYPE_I64,
            KvCacheType::F64 => koharu_llama_sys::GGML_TYPE_F64,
            KvCacheType::IQ1_M => koharu_llama_sys::GGML_TYPE_IQ1_M,
            KvCacheType::BF16 => koharu_llama_sys::GGML_TYPE_BF16,
            KvCacheType::TQ1_0 => koharu_llama_sys::GGML_TYPE_TQ1_0,
            KvCacheType::TQ2_0 => koharu_llama_sys::GGML_TYPE_TQ2_0,
            KvCacheType::MXFP4 => koharu_llama_sys::GGML_TYPE_MXFP4,
        }
    }
}

impl From<koharu_llama_sys::ggml_type> for KvCacheType {
    fn from(value: koharu_llama_sys::ggml_type) -> Self {
        match value {
            x if x == koharu_llama_sys::GGML_TYPE_F32 => KvCacheType::F32,
            x if x == koharu_llama_sys::GGML_TYPE_F16 => KvCacheType::F16,
            x if x == koharu_llama_sys::GGML_TYPE_Q4_0 => KvCacheType::Q4_0,
            x if x == koharu_llama_sys::GGML_TYPE_Q4_1 => KvCacheType::Q4_1,
            x if x == koharu_llama_sys::GGML_TYPE_Q5_0 => KvCacheType::Q5_0,
            x if x == koharu_llama_sys::GGML_TYPE_Q5_1 => KvCacheType::Q5_1,
            x if x == koharu_llama_sys::GGML_TYPE_Q8_0 => KvCacheType::Q8_0,
            x if x == koharu_llama_sys::GGML_TYPE_Q8_1 => KvCacheType::Q8_1,
            x if x == koharu_llama_sys::GGML_TYPE_Q2_K => KvCacheType::Q2_K,
            x if x == koharu_llama_sys::GGML_TYPE_Q3_K => KvCacheType::Q3_K,
            x if x == koharu_llama_sys::GGML_TYPE_Q4_K => KvCacheType::Q4_K,
            x if x == koharu_llama_sys::GGML_TYPE_Q5_K => KvCacheType::Q5_K,
            x if x == koharu_llama_sys::GGML_TYPE_Q6_K => KvCacheType::Q6_K,
            x if x == koharu_llama_sys::GGML_TYPE_Q8_K => KvCacheType::Q8_K,
            x if x == koharu_llama_sys::GGML_TYPE_IQ2_XXS => KvCacheType::IQ2_XXS,
            x if x == koharu_llama_sys::GGML_TYPE_IQ2_XS => KvCacheType::IQ2_XS,
            x if x == koharu_llama_sys::GGML_TYPE_IQ3_XXS => KvCacheType::IQ3_XXS,
            x if x == koharu_llama_sys::GGML_TYPE_IQ1_S => KvCacheType::IQ1_S,
            x if x == koharu_llama_sys::GGML_TYPE_IQ4_NL => KvCacheType::IQ4_NL,
            x if x == koharu_llama_sys::GGML_TYPE_IQ3_S => KvCacheType::IQ3_S,
            x if x == koharu_llama_sys::GGML_TYPE_IQ2_S => KvCacheType::IQ2_S,
            x if x == koharu_llama_sys::GGML_TYPE_IQ4_XS => KvCacheType::IQ4_XS,
            x if x == koharu_llama_sys::GGML_TYPE_I8 => KvCacheType::I8,
            x if x == koharu_llama_sys::GGML_TYPE_I16 => KvCacheType::I16,
            x if x == koharu_llama_sys::GGML_TYPE_I32 => KvCacheType::I32,
            x if x == koharu_llama_sys::GGML_TYPE_I64 => KvCacheType::I64,
            x if x == koharu_llama_sys::GGML_TYPE_F64 => KvCacheType::F64,
            x if x == koharu_llama_sys::GGML_TYPE_IQ1_M => KvCacheType::IQ1_M,
            x if x == koharu_llama_sys::GGML_TYPE_BF16 => KvCacheType::BF16,
            x if x == koharu_llama_sys::GGML_TYPE_TQ1_0 => KvCacheType::TQ1_0,
            x if x == koharu_llama_sys::GGML_TYPE_TQ2_0 => KvCacheType::TQ2_0,
            x if x == koharu_llama_sys::GGML_TYPE_MXFP4 => KvCacheType::MXFP4,
            _ => KvCacheType::Unknown(value),
        }
    }
}

/// A safe wrapper around `llama_context_params`.
///
/// Generally this should be created with [`Default::default()`] and then modified with `with_*` methods.
///
/// # Examples
///
/// ```rust
/// # use std::num::NonZeroU32;
/// # use koharu_llama::context::params::LlamaContextParams;
///
/// let ctx_params = LlamaContextParams::default()
///     .with_n_ctx(NonZeroU32::new(2048));
///
/// assert_eq!(ctx_params.n_ctx(), NonZeroU32::new(2048));
/// ```
#[derive(Debug, Clone)]
#[allow(
    missing_docs,
    clippy::struct_excessive_bools,
    clippy::module_name_repetitions
)]
pub struct LlamaContextParams {
    pub(crate) context_params: koharu_llama_sys::llama_context_params,
}

/// SAFETY: we do not currently allow setting or reading the pointers that cause this to not be automatically send or sync.
unsafe impl Send for LlamaContextParams {}
unsafe impl Sync for LlamaContextParams {}

/// Default parameters for `LlamaContext`. (as defined in llama.cpp by `llama_context_default_params`)
/// ```
/// # use std::num::NonZeroU32;
/// # use koharu_llama::context::params::{LlamaContextParams, RopeScalingType};
/// let params = LlamaContextParams::default();
/// assert_eq!(params.n_ctx(), NonZeroU32::new(512), "n_ctx should be 512");
/// assert_eq!(params.rope_scaling_type(), RopeScalingType::Unspecified);
/// ```
impl Default for LlamaContextParams {
    fn default() -> Self {
        let context_params = unsafe { koharu_llama_sys::llama_context_default_params() };
        Self { context_params }
    }
}
