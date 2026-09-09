// Defaults are intentionally implemented only for enums whose upstream C API
// defines a meaningful default or sentinel; the shared enum macros also emit
// enums where `Default` would be misleading.
#![allow(clippy::derivable_impls)]

use std::{ffi::CStr, fmt, str::FromStr};

use crate::{Error, Result, ffi::NativeCall, sys};

macro_rules! named_ffi_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident: $raw_ty:ty, $kind:literal, $name_fn:path, $parse_fn:path, $invalid:path;
        {
            $($variant:ident = $raw:path => $text:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(i32)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant = $raw as i32),+
        }

        impl $name {
            #[must_use]
            pub const fn as_raw(self) -> $raw_ty {
                match self {
                    $(Self::$variant => $raw),+
                }
            }

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            /// Returns the name supplied by the loaded native library.
            #[must_use]
            pub fn native_name(self) -> String {
                let _call = NativeCall::enter();
                let pointer = unsafe { $name_fn(self.as_raw()) };
                if pointer.is_null() {
                    return String::new();
                }
                unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned()
            }

            /// Parses a name using the loaded native library's parser.
            pub fn parse_native(value: &str) -> Result<Self> {
                let value_c = crate::ffi::cstring(value, $kind)?;
                let _call = NativeCall::enter();
                let raw = unsafe { $parse_fn(value_c.as_ptr()) };
                if raw == $invalid {
                    return Err(Error::UnknownEnumName {
                        kind: $kind,
                        value: value.to_owned(),
                    });
                }
                Self::try_from(raw)
            }
        }

        impl TryFrom<$raw_ty> for $name {
            type Error = Error;

            fn try_from(value: $raw_ty) -> Result<Self> {
                match value {
                    $($raw => Ok(Self::$variant),)+
                    value => Err(Error::InvalidEnum {
                        kind: $kind,
                        value: i64::from(value),
                    }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                match value {
                    $($text => Ok(Self::$variant),)+
                    value => Err(Error::UnknownEnumName {
                        kind: $kind,
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

macro_rules! ffi_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident: $raw_ty:ty, $kind:literal {
            $($variant:ident = $raw:path => $text:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(i32)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant = $raw as i32),+
        }

        impl $name {
            #[must_use]
            pub const fn as_raw(self) -> $raw_ty {
                match self {
                    $(Self::$variant => $raw),+
                }
            }

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl TryFrom<$raw_ty> for $name {
            type Error = Error;

            fn try_from(value: $raw_ty) -> Result<Self> {
                match value {
                    $($raw => Ok(Self::$variant),)+
                    value => Err(Error::InvalidEnum {
                        kind: $kind,
                        value: i64::from(value),
                    }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                match value {
                    $($text => Ok(Self::$variant),)+
                    value => Err(Error::UnknownEnumName {
                        kind: $kind,
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

named_ffi_enum! {
    /// Tensor type used for model weights and conversion output.
    pub enum WeightType: sys::sd_type_t, "weight type", sys::sd_type_name, sys::str_to_sd_type, sys::SD_TYPE_COUNT;
    {
        F32 = sys::SD_TYPE_F32 => "f32",
        F16 = sys::SD_TYPE_F16 => "f16",
        Q4_0 = sys::SD_TYPE_Q4_0 => "q4_0",
        Q4_1 = sys::SD_TYPE_Q4_1 => "q4_1",
        Q5_0 = sys::SD_TYPE_Q5_0 => "q5_0",
        Q5_1 = sys::SD_TYPE_Q5_1 => "q5_1",
        Q8_0 = sys::SD_TYPE_Q8_0 => "q8_0",
        Q8_1 = sys::SD_TYPE_Q8_1 => "q8_1",
        Q2K = sys::SD_TYPE_Q2_K => "q2_k",
        Q3K = sys::SD_TYPE_Q3_K => "q3_k",
        Q4K = sys::SD_TYPE_Q4_K => "q4_k",
        Q5K = sys::SD_TYPE_Q5_K => "q5_k",
        Q6K = sys::SD_TYPE_Q6_K => "q6_k",
        Q8K = sys::SD_TYPE_Q8_K => "q8_k",
        Iq2Xxs = sys::SD_TYPE_IQ2_XXS => "iq2_xxs",
        Iq2Xs = sys::SD_TYPE_IQ2_XS => "iq2_xs",
        Iq3Xxs = sys::SD_TYPE_IQ3_XXS => "iq3_xxs",
        Iq1S = sys::SD_TYPE_IQ1_S => "iq1_s",
        Iq4Nl = sys::SD_TYPE_IQ4_NL => "iq4_nl",
        Iq3S = sys::SD_TYPE_IQ3_S => "iq3_s",
        Iq2S = sys::SD_TYPE_IQ2_S => "iq2_s",
        Iq4Xs = sys::SD_TYPE_IQ4_XS => "iq4_xs",
        I8 = sys::SD_TYPE_I8 => "i8",
        I16 = sys::SD_TYPE_I16 => "i16",
        I32 = sys::SD_TYPE_I32 => "i32",
        I64 = sys::SD_TYPE_I64 => "i64",
        F64 = sys::SD_TYPE_F64 => "f64",
        Iq1M = sys::SD_TYPE_IQ1_M => "iq1_m",
        Bf16 = sys::SD_TYPE_BF16 => "bf16",
        Tq1_0 = sys::SD_TYPE_TQ1_0 => "tq1_0",
        Tq2_0 = sys::SD_TYPE_TQ2_0 => "tq2_0",
        Mxfp4 = sys::SD_TYPE_MXFP4 => "mxfp4",
        Nvfp4 = sys::SD_TYPE_NVFP4 => "nvfp4",
        Q1_0 = sys::SD_TYPE_Q1_0 => "q1_0",
        Q2_0 = sys::SD_TYPE_Q2_0 => "q2_0",
        F8E4m3 = sys::SD_TYPE_F8_E4M3 => "f8_e4m3",
        F8E5m2 = sys::SD_TYPE_F8_E5M2 => "f8_e5m2",
        Auto = sys::SD_TYPE_COUNT => "auto"
    }
}

impl Default for WeightType {
    fn default() -> Self {
        Self::Auto
    }
}

named_ffi_enum! {
    /// Random-number generator implementation.
    pub enum RngType: sys::rng_type_t, "RNG type", sys::sd_rng_type_name, sys::str_to_rng_type, sys::RNG_TYPE_COUNT;
    {
        Standard = sys::STD_DEFAULT_RNG => "std_default",
        Cuda = sys::CUDA_RNG => "cuda",
        Cpu = sys::CPU_RNG => "cpu",
        Auto = sys::RNG_TYPE_COUNT => "auto"
    }
}

named_ffi_enum! {
    /// Diffusion sampling method.
    pub enum SampleMethod: sys::sample_method_t, "sample method", sys::sd_sample_method_name, sys::str_to_sample_method, sys::SAMPLE_METHOD_COUNT;
    {
        Euler = sys::EULER_SAMPLE_METHOD => "euler",
        EulerA = sys::EULER_A_SAMPLE_METHOD => "euler_a",
        Heun = sys::HEUN_SAMPLE_METHOD => "heun",
        Dpm2 = sys::DPM2_SAMPLE_METHOD => "dpm2",
        DpmPlusPlus2sA = sys::DPMPP2S_A_SAMPLE_METHOD => "dpm++2s_a",
        DpmPlusPlus2m = sys::DPMPP2M_SAMPLE_METHOD => "dpm++2m",
        DpmPlusPlus2mV2 = sys::DPMPP2Mv2_SAMPLE_METHOD => "dpm++2mv2",
        Ipndm = sys::IPNDM_SAMPLE_METHOD => "ipndm",
        IpndmV = sys::IPNDM_V_SAMPLE_METHOD => "ipndm_v",
        Lcm = sys::LCM_SAMPLE_METHOD => "lcm",
        DdimTrailing = sys::DDIM_TRAILING_SAMPLE_METHOD => "ddim_trailing",
        Tcd = sys::TCD_SAMPLE_METHOD => "tcd",
        ResMultistep = sys::RES_MULTISTEP_SAMPLE_METHOD => "res_multistep",
        Res2s = sys::RES_2S_SAMPLE_METHOD => "res_2s",
        ErSde = sys::ER_SDE_SAMPLE_METHOD => "er_sde",
        EulerCfgPlusPlus = sys::EULER_CFG_PP_SAMPLE_METHOD => "euler_cfg_pp",
        EulerACfgPlusPlus = sys::EULER_A_CFG_PP_SAMPLE_METHOD => "euler_a_cfg_pp",
        EulerGe = sys::EULER_GE_SAMPLE_METHOD => "euler_ge",
        DpmPlusPlus2mSde = sys::DPMPP2M_SDE_SAMPLE_METHOD => "dpm++2m_sde",
        DpmPlusPlus2mSdeBt = sys::DPMPP2M_SDE_BT_SAMPLE_METHOD => "dpm++2m_sde_bt",
        Lms = sys::LMS_SAMPLE_METHOD => "lms",
        Auto = sys::SAMPLE_METHOD_COUNT => "auto"
    }
}

impl Default for SampleMethod {
    fn default() -> Self {
        Self::Auto
    }
}

named_ffi_enum! {
    /// Noise schedule used by the sampler.
    pub enum Scheduler: sys::scheduler_t, "scheduler", sys::sd_scheduler_name, sys::str_to_scheduler, sys::SCHEDULER_COUNT;
    {
        Discrete = sys::DISCRETE_SCHEDULER => "discrete",
        Karras = sys::KARRAS_SCHEDULER => "karras",
        Exponential = sys::EXPONENTIAL_SCHEDULER => "exponential",
        Ays = sys::AYS_SCHEDULER => "ays",
        Gits = sys::GITS_SCHEDULER => "gits",
        SgmUniform = sys::SGM_UNIFORM_SCHEDULER => "sgm_uniform",
        Simple = sys::SIMPLE_SCHEDULER => "simple",
        Smoothstep = sys::SMOOTHSTEP_SCHEDULER => "smoothstep",
        KlOptimal = sys::KL_OPTIMAL_SCHEDULER => "kl_optimal",
        Lcm = sys::LCM_SCHEDULER => "lcm",
        BongTangent = sys::BONG_TANGENT_SCHEDULER => "bong_tangent",
        Ltx2 = sys::LTX2_SCHEDULER => "ltx2",
        LogitNormal = sys::LOGIT_NORMAL_SCHEDULER => "logit_normal",
        Flux2 = sys::FLUX2_SCHEDULER => "flux2",
        Flux = sys::FLUX_SCHEDULER => "flux",
        Beta = sys::BETA_SCHEDULER => "beta",
        Auto = sys::SCHEDULER_COUNT => "auto"
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::Auto
    }
}

named_ffi_enum! {
    /// Model prediction parameterization.
    pub enum Prediction: sys::prediction_t, "prediction", sys::sd_prediction_name, sys::str_to_prediction, sys::PREDICTION_COUNT;
    {
        Epsilon = sys::EPS_PRED => "eps",
        V = sys::V_PRED => "v",
        EdmV = sys::EDM_V_PRED => "edm_v",
        Sd3Flow = sys::FLOW_PRED => "sd3_flow",
        FluxFlow = sys::FLUX_FLOW_PRED => "flux_flow",
        SefiFlow = sys::SEFI_FLOW_PRED => "sefi_flow",
        MiniT2iFlow = sys::MINIT2I_FLOW_PRED => "minit2i_flow",
        Auto = sys::PREDICTION_COUNT => "auto"
    }
}

impl Default for Prediction {
    fn default() -> Self {
        Self::Auto
    }
}

named_ffi_enum! {
    /// Preview decoder used while sampling.
    pub enum PreviewMode: sys::preview_t, "preview mode", sys::sd_preview_name, sys::str_to_preview, sys::PREVIEW_COUNT;
    {
        None = sys::PREVIEW_NONE => "none",
        Projection = sys::PREVIEW_PROJ => "proj",
        TinyAutoencoder = sys::PREVIEW_TAE => "tae",
        Vae = sys::PREVIEW_VAE => "vae"
    }
}

impl Default for PreviewMode {
    fn default() -> Self {
        Self::None
    }
}

named_ffi_enum! {
    /// When LoRA weights are applied.
    pub enum LoraApplyMode: sys::lora_apply_mode_t, "LoRA apply mode", sys::sd_lora_apply_mode_name, sys::str_to_lora_apply_mode, sys::LORA_APPLY_MODE_COUNT;
    {
        Auto = sys::LORA_APPLY_AUTO => "auto",
        Immediately = sys::LORA_APPLY_IMMEDIATELY => "immediately",
        AtRuntime = sys::LORA_APPLY_AT_RUNTIME => "at_runtime"
    }
}

impl Default for LoraApplyMode {
    fn default() -> Self {
        Self::Auto
    }
}

named_ffi_enum! {
    /// Upscaling algorithm used for high-resolution refinement.
    pub enum HiresUpscaler: sys::sd_hires_upscaler_t, "high-resolution upscaler", sys::sd_hires_upscaler_name, sys::str_to_sd_hires_upscaler, sys::SD_HIRES_UPSCALER_COUNT;
    {
        None = sys::SD_HIRES_UPSCALER_NONE => "None",
        Latent = sys::SD_HIRES_UPSCALER_LATENT => "Latent",
        LatentNearest = sys::SD_HIRES_UPSCALER_LATENT_NEAREST => "Latent (nearest)",
        LatentNearestExact = sys::SD_HIRES_UPSCALER_LATENT_NEAREST_EXACT => "Latent (nearest-exact)",
        LatentAntialiased = sys::SD_HIRES_UPSCALER_LATENT_ANTIALIASED => "Latent (antialiased)",
        LatentBicubic = sys::SD_HIRES_UPSCALER_LATENT_BICUBIC => "Latent (bicubic)",
        LatentBicubicAntialiased = sys::SD_HIRES_UPSCALER_LATENT_BICUBIC_ANTIALIASED => "Latent (bicubic antialiased)",
        Lanczos = sys::SD_HIRES_UPSCALER_LANCZOS => "Lanczos",
        Nearest = sys::SD_HIRES_UPSCALER_NEAREST => "Nearest",
        Model = sys::SD_HIRES_UPSCALER_MODEL => "Model"
    }
}

ffi_enum! {
    /// VAE tensor naming/layout format.
    pub enum VaeFormat: sys::sd_vae_format_t, "VAE format" {
        Auto = sys::SD_VAE_FORMAT_AUTO => "auto",
        Flux = sys::SD_VAE_FORMAT_FLUX => "flux",
        Sd3 = sys::SD_VAE_FORMAT_SD3 => "sd3",
        Flux2 = sys::SD_VAE_FORMAT_FLUX2 => "flux2",
        Wan = sys::SD_VAE_FORMAT_WAN => "wan"
    }
}

impl Default for VaeFormat {
    fn default() -> Self {
        Self::Auto
    }
}

ffi_enum! {
    /// Diffusion cache implementation.
    pub enum CacheMode: sys::sd_cache_mode_t, "cache mode" {
        Disabled = sys::SD_CACHE_DISABLED => "disabled",
        EasyCache = sys::SD_CACHE_EASYCACHE => "easycache",
        UCache = sys::SD_CACHE_UCACHE => "ucache",
        DbCache = sys::SD_CACHE_DBCACHE => "dbcache",
        TaylorSeer = sys::SD_CACHE_TAYLORSEER => "taylorseer",
        CacheDit = sys::SD_CACHE_CACHE_DIT => "cache_dit",
        Spectrum = sys::SD_CACHE_SPECTRUM => "spectrum"
    }
}

impl Default for CacheMode {
    fn default() -> Self {
        Self::Disabled
    }
}

ffi_enum! {
    /// Severity attached to a native log message.
    pub enum LogLevel: sys::sd_log_level_t, "log level" {
        Debug = sys::SD_LOG_DEBUG => "debug",
        Info = sys::SD_LOG_INFO => "info",
        Warn = sys::SD_LOG_WARN => "warn",
        Error = sys::SD_LOG_ERROR => "error"
    }
}

ffi_enum! {
    /// Cancellation behavior for an active generation.
    pub enum CancelMode: sys::sd_cancel_mode_t, "cancel mode" {
        All = sys::SD_CANCEL_ALL => "all",
        NewLatents = sys::SD_CANCEL_NEW_LATENTS => "new_latents",
        Reset = sys::SD_CANCEL_RESET => "reset"
    }
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, SampleMethod, Scheduler, VaeFormat, WeightType};
    use crate::{Error, sys};

    #[test]
    fn enum_names_round_trip_without_loading_native_library() {
        assert!(matches!(
            "dpm++2m_sde".parse(),
            Ok(SampleMethod::DpmPlusPlus2mSde)
        ));
        assert_eq!(Scheduler::Flux2.to_string(), "flux2");
        assert!(matches!(WeightType::try_from(30), Ok(WeightType::Bf16)));
    }

    #[test]
    fn enum_raw_values_use_the_native_binding_types() {
        let weight_type: sys::sd_type_t = WeightType::Bf16.as_raw();
        assert_eq!(weight_type, sys::SD_TYPE_BF16);
        assert_eq!(VaeFormat::Auto.as_raw(), sys::SD_VAE_FORMAT_AUTO);

        let invalid = sys::sd_log_level_t::MAX;
        assert!(matches!(
            LogLevel::try_from(invalid),
            Err(Error::InvalidEnum {
                kind: "log level",
                value,
            }) if value == i64::from(invalid)
        ));
    }
}
