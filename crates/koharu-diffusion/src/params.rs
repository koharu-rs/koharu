use std::{ffi::CString, fmt, os::raw::c_char, path::PathBuf, ptr};

use crate::{
    Audio, CacheMode, Error, GrayImage, HiresUpscaler, LoraApplyMode, Prediction, Result, RgbImage,
    RngType, SampleMethod, Scheduler, VaeFormat, Video, WeightType,
    ffi::{c_int_len, cstring, path_cstring, u32_len},
    image::{optional_raw_gray_image, optional_raw_rgb_image, raw_rgb_images},
    sys,
};

#[derive(Default)]
struct StringPool {
    values: Vec<CString>,
}

impl StringPool {
    fn add(&mut self, value: &str, field: &'static str) -> Result<*const c_char> {
        let value = cstring(value, field)?;
        let pointer = value.as_ptr();
        self.values.push(value);
        Ok(pointer)
    }

    fn add_optional(&mut self, value: Option<&str>, field: &'static str) -> Result<*const c_char> {
        value
            .map(|value| self.add(value, field))
            .transpose()
            .map(|pointer| pointer.unwrap_or(ptr::null()))
    }

    fn add_path(&mut self, path: &std::path::Path, field: &'static str) -> Result<*const c_char> {
        let path = path_cstring(path, field)?;
        let pointer = path.as_ptr();
        self.values.push(path);
        Ok(pointer)
    }

    fn add_optional_path(
        &mut self,
        path: Option<&std::path::Path>,
        field: &'static str,
    ) -> Result<*const c_char> {
        path.map(|path| self.add_path(path, field))
            .transpose()
            .map(|pointer| pointer.unwrap_or(ptr::null()))
    }
}

fn mut_ptr_or_null<T>(values: &mut [T]) -> *mut T {
    if values.is_empty() {
        ptr::null_mut()
    } else {
        values.as_mut_ptr()
    }
}

fn ptr_or_null<T>(values: &[T]) -> *const T {
    if values.is_empty() {
        ptr::null()
    } else {
        values.as_ptr()
    }
}

/// A textual-inversion embedding loaded with a model context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embedding {
    pub name: String,
    pub path: PathBuf,
}

/// Model files and backend settings used to create a [`crate::Context`].
#[derive(Debug, Clone)]
pub struct ContextParams {
    pub model_path: Option<PathBuf>,
    pub clip_l_path: Option<PathBuf>,
    pub clip_g_path: Option<PathBuf>,
    pub clip_vision_path: Option<PathBuf>,
    pub t5xxl_path: Option<PathBuf>,
    pub llm_path: Option<PathBuf>,
    pub llm_vision_path: Option<PathBuf>,
    pub diffusion_model_path: Option<PathBuf>,
    pub high_noise_diffusion_model_path: Option<PathBuf>,
    pub uncond_diffusion_model_path: Option<PathBuf>,
    pub embeddings_connectors_path: Option<PathBuf>,
    pub vae_path: Option<PathBuf>,
    pub audio_vae_path: Option<PathBuf>,
    pub taesd_path: Option<PathBuf>,
    pub control_net_path: Option<PathBuf>,
    pub ip_adapter_path: Option<PathBuf>,
    pub motion_module_path: Option<PathBuf>,
    pub embeddings: Vec<Embedding>,
    pub photo_maker_path: Option<PathBuf>,
    pub pulid_weights_path: Option<PathBuf>,
    pub tensor_type_rules: Option<String>,
    pub n_threads: i32,
    pub weight_type: WeightType,
    pub rng_type: RngType,
    pub sampler_rng_type: RngType,
    pub prediction: Prediction,
    pub lora_apply_mode: LoraApplyMode,
    pub enable_mmap: bool,
    pub flash_attention: bool,
    pub diffusion_flash_attention: bool,
    pub tae_preview_only: bool,
    pub diffusion_conv_direct: bool,
    pub vae_conv_direct: bool,
    pub force_sdxl_vae_conv_scale: bool,
    pub vae_format: VaeFormat,
    pub max_vram: Option<String>,
    pub disable_prefetch: bool,
    pub eager_load: bool,
    pub backend: Option<String>,
    pub params_backend: Option<String>,
    pub split_mode: Option<String>,
    pub auto_fit: bool,
    pub rpc_servers: Option<String>,
    pub model_args: Option<String>,
    pub disable_segmented_compute: bool,
}

impl Default for ContextParams {
    fn default() -> Self {
        Self {
            model_path: None,
            clip_l_path: None,
            clip_g_path: None,
            clip_vision_path: None,
            t5xxl_path: None,
            llm_path: None,
            llm_vision_path: None,
            diffusion_model_path: None,
            high_noise_diffusion_model_path: None,
            uncond_diffusion_model_path: None,
            embeddings_connectors_path: None,
            vae_path: None,
            audio_vae_path: None,
            taesd_path: None,
            control_net_path: None,
            ip_adapter_path: None,
            motion_module_path: None,
            embeddings: Vec::new(),
            photo_maker_path: None,
            pulid_weights_path: None,
            tensor_type_rules: None,
            n_threads: crate::physical_core_count(),
            weight_type: WeightType::Auto,
            rng_type: RngType::Cuda,
            sampler_rng_type: RngType::Auto,
            prediction: Prediction::Auto,
            lora_apply_mode: LoraApplyMode::Auto,
            enable_mmap: false,
            flash_attention: false,
            diffusion_flash_attention: false,
            tae_preview_only: false,
            diffusion_conv_direct: false,
            vae_conv_direct: false,
            force_sdxl_vae_conv_scale: false,
            vae_format: VaeFormat::Auto,
            max_vram: None,
            disable_prefetch: false,
            eager_load: false,
            backend: None,
            params_backend: None,
            split_mode: None,
            auto_fit: false,
            rpc_servers: None,
            model_args: None,
            disable_segmented_compute: false,
        }
    }
}

pub(crate) struct NativeContextParams {
    pub(crate) raw: sys::sd_ctx_params_t,
    _strings: StringPool,
    _embeddings: Vec<sys::sd_embedding_t>,
}

impl ContextParams {
    pub(crate) fn to_native(&self) -> Result<NativeContextParams> {
        if self.n_threads <= 0 {
            return Err(Error::InvalidParameter {
                name: "n_threads",
                reason: "must be greater than zero",
            });
        }
        let mut strings = StringPool::default();
        let model_path = strings.add_optional_path(self.model_path.as_deref(), "model_path")?;
        let clip_l_path = strings.add_optional_path(self.clip_l_path.as_deref(), "clip_l_path")?;
        let clip_g_path = strings.add_optional_path(self.clip_g_path.as_deref(), "clip_g_path")?;
        let clip_vision_path =
            strings.add_optional_path(self.clip_vision_path.as_deref(), "clip_vision_path")?;
        let t5xxl_path = strings.add_optional_path(self.t5xxl_path.as_deref(), "t5xxl_path")?;
        let llm_path = strings.add_optional_path(self.llm_path.as_deref(), "llm_path")?;
        let llm_vision_path =
            strings.add_optional_path(self.llm_vision_path.as_deref(), "llm_vision_path")?;
        let diffusion_model_path = strings
            .add_optional_path(self.diffusion_model_path.as_deref(), "diffusion_model_path")?;
        let high_noise_diffusion_model_path = strings.add_optional_path(
            self.high_noise_diffusion_model_path.as_deref(),
            "high_noise_diffusion_model_path",
        )?;
        let uncond_diffusion_model_path = strings.add_optional_path(
            self.uncond_diffusion_model_path.as_deref(),
            "uncond_diffusion_model_path",
        )?;
        let embeddings_connectors_path = strings.add_optional_path(
            self.embeddings_connectors_path.as_deref(),
            "embeddings_connectors_path",
        )?;
        let vae_path = strings.add_optional_path(self.vae_path.as_deref(), "vae_path")?;
        let audio_vae_path =
            strings.add_optional_path(self.audio_vae_path.as_deref(), "audio_vae_path")?;
        let taesd_path = strings.add_optional_path(self.taesd_path.as_deref(), "taesd_path")?;
        let control_net_path =
            strings.add_optional_path(self.control_net_path.as_deref(), "control_net_path")?;
        let ip_adapter_path =
            strings.add_optional_path(self.ip_adapter_path.as_deref(), "ip_adapter_path")?;
        let motion_module_path =
            strings.add_optional_path(self.motion_module_path.as_deref(), "motion_module_path")?;

        let mut embeddings = Vec::with_capacity(self.embeddings.len());
        for embedding in &self.embeddings {
            embeddings.push(sys::sd_embedding_t {
                name: strings.add(&embedding.name, "embedding name")?,
                path: strings.add_path(&embedding.path, "embedding path")?,
            });
        }
        let embedding_count = u32_len(embeddings.len(), "embeddings")?;

        let photo_maker_path =
            strings.add_optional_path(self.photo_maker_path.as_deref(), "photo_maker_path")?;
        let pulid_weights_path =
            strings.add_optional_path(self.pulid_weights_path.as_deref(), "pulid_weights_path")?;
        let tensor_type_rules =
            strings.add_optional(self.tensor_type_rules.as_deref(), "tensor_type_rules")?;
        let max_vram = strings.add_optional(self.max_vram.as_deref(), "max_vram")?;
        let backend = strings.add_optional(self.backend.as_deref(), "backend")?;
        let params_backend =
            strings.add_optional(self.params_backend.as_deref(), "params_backend")?;
        let split_mode = strings.add_optional(self.split_mode.as_deref(), "split_mode")?;
        let rpc_servers = strings.add_optional(self.rpc_servers.as_deref(), "rpc_servers")?;
        let model_args = strings.add_optional(self.model_args.as_deref(), "model_args")?;

        let raw = sys::sd_ctx_params_t {
            model_path,
            clip_l_path,
            clip_g_path,
            clip_vision_path,
            t5xxl_path,
            llm_path,
            llm_vision_path,
            diffusion_model_path,
            high_noise_diffusion_model_path,
            uncond_diffusion_model_path,
            embeddings_connectors_path,
            vae_path,
            audio_vae_path,
            taesd_path,
            control_net_path,
            ip_adapter_path,
            motion_module_path,
            embeddings: ptr_or_null(&embeddings),
            embedding_count,
            photo_maker_path,
            pulid_weights_path,
            tensor_type_rules,
            n_threads: self.n_threads,
            wtype: self.weight_type.as_raw(),
            rng_type: self.rng_type.as_raw(),
            sampler_rng_type: self.sampler_rng_type.as_raw(),
            prediction: self.prediction.as_raw(),
            lora_apply_mode: self.lora_apply_mode.as_raw(),
            enable_mmap: self.enable_mmap,
            flash_attn: self.flash_attention,
            diffusion_flash_attn: self.diffusion_flash_attention,
            tae_preview_only: self.tae_preview_only,
            diffusion_conv_direct: self.diffusion_conv_direct,
            vae_conv_direct: self.vae_conv_direct,
            force_sdxl_vae_conv_scale: self.force_sdxl_vae_conv_scale,
            vae_format: self.vae_format.as_raw(),
            max_vram,
            disable_prefetch: self.disable_prefetch,
            eager_load: self.eager_load,
            backend,
            params_backend,
            split_mode,
            auto_fit: self.auto_fit,
            rpc_servers,
            model_args,
            disable_segmented_compute: self.disable_segmented_compute,
        };
        Ok(NativeContextParams {
            raw,
            _strings: strings,
            _embeddings: embeddings,
        })
    }
}

impl fmt::Display for ContextParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "model_path: {:?}", self.model_path)?;
        writeln!(
            formatter,
            "diffusion_model_path: {:?}",
            self.diffusion_model_path
        )?;
        writeln!(
            formatter,
            "high_noise_diffusion_model_path: {:?}",
            self.high_noise_diffusion_model_path
        )?;
        writeln!(formatter, "vae_path: {:?}", self.vae_path)?;
        writeln!(formatter, "n_threads: {}", self.n_threads)?;
        writeln!(formatter, "weight_type: {}", self.weight_type)?;
        writeln!(formatter, "rng_type: {}", self.rng_type)?;
        writeln!(formatter, "sampler_rng_type: {}", self.sampler_rng_type)?;
        writeln!(formatter, "prediction: {}", self.prediction)?;
        writeln!(formatter, "backend: {:?}", self.backend)?;
        writeln!(formatter, "params_backend: {:?}", self.params_backend)?;
        write!(formatter, "auto_fit: {}", self.auto_fit)
    }
}

/// Skip-layer guidance settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SkipLayerGuidance {
    pub layers: Vec<i32>,
    pub layer_start: f32,
    pub layer_end: f32,
    pub scale: f32,
}

impl Default for SkipLayerGuidance {
    fn default() -> Self {
        Self {
            layers: Vec::new(),
            layer_start: 0.01,
            layer_end: 0.2,
            scale: 0.0,
        }
    }
}

/// Text, image, distilled, and skip-layer guidance settings.
#[derive(Debug, Clone, PartialEq)]
pub struct GuidanceParams {
    pub text_cfg: f32,
    pub image_cfg: f32,
    pub distilled_guidance: f32,
    pub skip_layer: SkipLayerGuidance,
}

impl Default for GuidanceParams {
    fn default() -> Self {
        Self {
            text_cfg: 7.0,
            image_cfg: f32::INFINITY,
            distilled_guidance: 3.5,
            skip_layer: SkipLayerGuidance::default(),
        }
    }
}

/// Sampling settings shared by image and video generation.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleParams {
    pub guidance: GuidanceParams,
    pub scheduler: Scheduler,
    pub sample_method: SampleMethod,
    pub sample_steps: i32,
    pub eta: f32,
    pub shifted_timestep: i32,
    pub custom_sigmas: Vec<f32>,
    pub flow_shift: f32,
    pub extra_args: Option<String>,
}

impl Default for SampleParams {
    fn default() -> Self {
        Self {
            guidance: GuidanceParams::default(),
            scheduler: Scheduler::Auto,
            sample_method: SampleMethod::Auto,
            sample_steps: 20,
            eta: f32::INFINITY,
            shifted_timestep: 0,
            custom_sigmas: Vec::new(),
            flow_shift: f32::INFINITY,
            extra_args: None,
        }
    }
}

struct NativeSampleParams {
    raw: sys::sd_sample_params_t,
    _layers: Vec<i32>,
    _sigmas: Vec<f32>,
}

impl SampleParams {
    fn to_native(&self, strings: &mut StringPool) -> Result<NativeSampleParams> {
        let mut layers = self.guidance.skip_layer.layers.clone();
        let mut sigmas = self.custom_sigmas.clone();
        let layer_count = layers.len();
        let custom_sigmas_count = c_int_len(sigmas.len(), "custom sigmas")?;
        let extra_sample_args =
            strings.add_optional(self.extra_args.as_deref(), "extra_sample_args")?;
        Ok(NativeSampleParams {
            raw: sys::sd_sample_params_t {
                guidance: sys::sd_guidance_params_t {
                    txt_cfg: self.guidance.text_cfg,
                    img_cfg: self.guidance.image_cfg,
                    distilled_guidance: self.guidance.distilled_guidance,
                    slg: sys::sd_slg_params_t {
                        layers: mut_ptr_or_null(&mut layers),
                        layer_count,
                        layer_start: self.guidance.skip_layer.layer_start,
                        layer_end: self.guidance.skip_layer.layer_end,
                        scale: self.guidance.skip_layer.scale,
                    },
                },
                scheduler: self.scheduler.as_raw(),
                sample_method: self.sample_method.as_raw(),
                sample_steps: self.sample_steps,
                eta: self.eta,
                shifted_timestep: self.shifted_timestep,
                custom_sigmas: mut_ptr_or_null(&mut sigmas),
                custom_sigmas_count,
                flow_shift: self.flow_shift,
                extra_sample_args,
            },
            _layers: layers,
            _sigmas: sigmas,
        })
    }
}

impl fmt::Display for SampleParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "(text_cfg: {:.2}, image_cfg: {:.2}, distilled_guidance: {:.2}, scheduler: {}, sample_method: {}, sample_steps: {}, eta: {:.2}, shifted_timestep: {}, flow_shift: {:.2}, extra_args: {:?})",
            self.guidance.text_cfg,
            if self.guidance.image_cfg.is_finite() {
                self.guidance.image_cfg
            } else {
                self.guidance.text_cfg
            },
            self.guidance.distilled_guidance,
            self.scheduler,
            self.sample_method,
            self.sample_steps,
            self.eta,
            self.shifted_timestep,
            self.flow_shift,
            self.extra_args,
        )
    }
}

/// VAE tiling controls.
#[derive(Debug, Clone, PartialEq)]
pub struct TilingParams {
    pub enabled: bool,
    pub temporal_tiling: bool,
    pub tile_size_x: i32,
    pub tile_size_y: i32,
    pub target_overlap: f32,
    pub relative_size_x: f32,
    pub relative_size_y: f32,
    pub extra_args: Option<String>,
}

impl Default for TilingParams {
    fn default() -> Self {
        Self {
            enabled: false,
            temporal_tiling: false,
            tile_size_x: 0,
            tile_size_y: 0,
            target_overlap: 0.5,
            relative_size_x: 0.0,
            relative_size_y: 0.0,
            extra_args: None,
        }
    }
}

impl TilingParams {
    fn to_native(&self, strings: &mut StringPool) -> Result<sys::sd_tiling_params_t> {
        Ok(sys::sd_tiling_params_t {
            enabled: self.enabled,
            temporal_tiling: self.temporal_tiling,
            tile_size_x: self.tile_size_x,
            tile_size_y: self.tile_size_y,
            target_overlap: self.target_overlap,
            rel_size_x: self.relative_size_x,
            rel_size_y: self.relative_size_y,
            extra_tiling_args: strings
                .add_optional(self.extra_args.as_deref(), "extra_tiling_args")?,
        })
    }
}

/// Diffusion feature-cache controls.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheParams {
    pub mode: CacheMode,
    pub reuse_threshold: f32,
    pub start_percent: f32,
    pub end_percent: f32,
    pub error_decay_rate: f32,
    pub use_relative_threshold: bool,
    pub reset_error_on_compute: bool,
    pub fn_compute_blocks: i32,
    pub bn_compute_blocks: i32,
    pub residual_diff_threshold: f32,
    pub max_warmup_steps: i32,
    pub max_cached_steps: i32,
    pub max_continuous_cached_steps: i32,
    pub taylorseer_derivatives: i32,
    pub taylorseer_skip_interval: i32,
    pub scm_mask: Option<String>,
    pub scm_policy_dynamic: bool,
    pub spectrum_w: f32,
    pub spectrum_m: i32,
    pub spectrum_lambda: f32,
    pub spectrum_window_size: i32,
    pub spectrum_flex_window: f32,
    pub spectrum_warmup_steps: i32,
    pub spectrum_stop_percent: f32,
}

impl Default for CacheParams {
    fn default() -> Self {
        Self {
            mode: CacheMode::Disabled,
            reuse_threshold: f32::INFINITY,
            start_percent: 0.15,
            end_percent: 0.95,
            error_decay_rate: 1.0,
            use_relative_threshold: true,
            reset_error_on_compute: true,
            fn_compute_blocks: 8,
            bn_compute_blocks: 0,
            residual_diff_threshold: 0.08,
            max_warmup_steps: 8,
            max_cached_steps: -1,
            max_continuous_cached_steps: -1,
            taylorseer_derivatives: 1,
            taylorseer_skip_interval: 1,
            scm_mask: None,
            scm_policy_dynamic: true,
            spectrum_w: 0.4,
            spectrum_m: 3,
            spectrum_lambda: 1.0,
            spectrum_window_size: 2,
            spectrum_flex_window: 0.5,
            spectrum_warmup_steps: 4,
            spectrum_stop_percent: 0.9,
        }
    }
}

impl CacheParams {
    fn to_native(&self, strings: &mut StringPool) -> Result<sys::sd_cache_params_t> {
        Ok(sys::sd_cache_params_t {
            mode: self.mode.as_raw(),
            reuse_threshold: self.reuse_threshold,
            start_percent: self.start_percent,
            end_percent: self.end_percent,
            error_decay_rate: self.error_decay_rate,
            use_relative_threshold: self.use_relative_threshold,
            reset_error_on_compute: self.reset_error_on_compute,
            Fn_compute_blocks: self.fn_compute_blocks,
            Bn_compute_blocks: self.bn_compute_blocks,
            residual_diff_threshold: self.residual_diff_threshold,
            max_warmup_steps: self.max_warmup_steps,
            max_cached_steps: self.max_cached_steps,
            max_continuous_cached_steps: self.max_continuous_cached_steps,
            taylorseer_n_derivatives: self.taylorseer_derivatives,
            taylorseer_skip_interval: self.taylorseer_skip_interval,
            scm_mask: strings.add_optional(self.scm_mask.as_deref(), "scm_mask")?,
            scm_policy_dynamic: self.scm_policy_dynamic,
            spectrum_w: self.spectrum_w,
            spectrum_m: self.spectrum_m,
            spectrum_lam: self.spectrum_lambda,
            spectrum_window_size: self.spectrum_window_size,
            spectrum_flex_window: self.spectrum_flex_window,
            spectrum_warmup_steps: self.spectrum_warmup_steps,
            spectrum_stop_percent: self.spectrum_stop_percent,
        })
    }
}

/// High-resolution second-pass settings.
#[derive(Debug, Clone, PartialEq)]
pub struct HiresParams {
    pub enabled: bool,
    pub upscaler: HiresUpscaler,
    pub model_path: Option<PathBuf>,
    pub scale: f32,
    pub target_width: i32,
    pub target_height: i32,
    pub steps: i32,
    pub denoising_strength: f32,
    pub upscale_tile_size: i32,
    pub custom_sigmas: Vec<f32>,
}

impl Default for HiresParams {
    fn default() -> Self {
        Self {
            enabled: false,
            upscaler: HiresUpscaler::Latent,
            model_path: None,
            scale: 2.0,
            target_width: 0,
            target_height: 0,
            steps: 0,
            denoising_strength: 0.7,
            upscale_tile_size: 128,
            custom_sigmas: Vec::new(),
        }
    }
}

struct NativeHiresParams {
    raw: sys::sd_hires_params_t,
    _sigmas: Vec<f32>,
}

impl HiresParams {
    fn to_native(&self, strings: &mut StringPool) -> Result<NativeHiresParams> {
        let mut sigmas = self.custom_sigmas.clone();
        let custom_sigmas_count = c_int_len(sigmas.len(), "high-resolution custom sigmas")?;
        Ok(NativeHiresParams {
            raw: sys::sd_hires_params_t {
                enabled: self.enabled,
                upscaler: self.upscaler.as_raw(),
                model_path: strings
                    .add_optional_path(self.model_path.as_deref(), "hires model_path")?,
                scale: self.scale,
                target_width: self.target_width,
                target_height: self.target_height,
                steps: self.steps,
                denoising_strength: self.denoising_strength,
                upscale_tile_size: self.upscale_tile_size,
                custom_sigmas: mut_ptr_or_null(&mut sigmas),
                custom_sigmas_count,
            },
            _sigmas: sigmas,
        })
    }
}

/// A LoRA adapter applied for one generation request.
#[derive(Debug, Clone, PartialEq)]
pub struct Lora {
    pub path: PathBuf,
    pub multiplier: f32,
    pub high_noise: bool,
}

/// PhotoMaker identity inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoMakerParams {
    pub id_images: Vec<RgbImage>,
    pub id_embedding_path: Option<PathBuf>,
    pub style_strength: f32,
}

impl Default for PhotoMakerParams {
    fn default() -> Self {
        Self {
            id_images: Vec::new(),
            id_embedding_path: None,
            style_strength: 20.0,
        }
    }
}

/// PuLID identity input.
#[derive(Debug, Clone, PartialEq)]
pub struct PulidParams {
    pub id_embedding_path: Option<PathBuf>,
    pub weight: f32,
}

impl Default for PulidParams {
    fn default() -> Self {
        Self {
            id_embedding_path: None,
            weight: 1.0,
        }
    }
}

/// Complete image-generation request.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageGenerationParams {
    pub loras: Vec<Lora>,
    pub prompt: String,
    pub negative_prompt: String,
    pub clip_skip: i32,
    pub init_image: Option<RgbImage>,
    pub reference_images: Vec<RgbImage>,
    pub reference_image_args: Option<String>,
    pub mask_image: Option<GrayImage>,
    pub width: i32,
    pub height: i32,
    pub sample: SampleParams,
    pub strength: f32,
    pub seed: i64,
    pub batch_count: i32,
    pub control_image: Option<RgbImage>,
    pub control_strength: f32,
    pub ip_adapter_image: Option<RgbImage>,
    pub ip_adapter_strength: f32,
    pub photo_maker: PhotoMakerParams,
    pub pulid: PulidParams,
    pub vae_tiling: TilingParams,
    pub cache: CacheParams,
    pub hires: HiresParams,
    pub qwen_image_layers: i32,
    pub circular_x: bool,
    pub circular_y: bool,
}

impl Default for ImageGenerationParams {
    fn default() -> Self {
        Self {
            loras: Vec::new(),
            prompt: String::new(),
            negative_prompt: String::new(),
            clip_skip: -1,
            init_image: None,
            reference_images: Vec::new(),
            reference_image_args: None,
            mask_image: None,
            width: 512,
            height: 512,
            sample: SampleParams::default(),
            strength: 0.75,
            seed: -1,
            batch_count: 1,
            control_image: None,
            control_strength: 0.9,
            ip_adapter_image: None,
            ip_adapter_strength: 1.0,
            photo_maker: PhotoMakerParams::default(),
            pulid: PulidParams::default(),
            vae_tiling: TilingParams::default(),
            cache: CacheParams::default(),
            hires: HiresParams::default(),
            qwen_image_layers: 3,
            circular_x: false,
            circular_y: false,
        }
    }
}

pub(crate) struct NativeImageGenerationParams {
    pub(crate) raw: sys::sd_img_gen_params_t,
    _strings: StringPool,
    _loras: Vec<sys::sd_lora_t>,
    _reference_images: Vec<sys::sd_image_t>,
    _photo_maker_images: Vec<sys::sd_image_t>,
    _sample: NativeSampleParams,
    _hires: NativeHiresParams,
}

fn build_loras(loras: &[Lora], strings: &mut StringPool) -> Result<Vec<sys::sd_lora_t>> {
    let mut native = Vec::with_capacity(loras.len());
    for lora in loras {
        native.push(sys::sd_lora_t {
            is_high_noise: lora.high_noise,
            multiplier: lora.multiplier,
            path: strings.add_path(&lora.path, "LoRA path")?,
        });
    }
    Ok(native)
}

impl ImageGenerationParams {
    pub(crate) fn to_native(&self) -> Result<NativeImageGenerationParams> {
        if self.width <= 0 || self.height <= 0 {
            return Err(Error::InvalidParameter {
                name: "image dimensions",
                reason: "width and height must be greater than zero",
            });
        }
        if self.batch_count <= 0 {
            return Err(Error::InvalidParameter {
                name: "batch_count",
                reason: "must be greater than zero",
            });
        }
        let mut strings = StringPool::default();
        let prompt = strings.add(&self.prompt, "prompt")?;
        let negative_prompt = strings.add(&self.negative_prompt, "negative_prompt")?;
        let loras = build_loras(&self.loras, &mut strings)?;
        let lora_count = u32_len(loras.len(), "LoRAs")?;
        let mut reference_images = raw_rgb_images(&self.reference_images)?;
        let reference_image_count = c_int_len(reference_images.len(), "reference images")?;
        let reference_image_args = strings.add_optional(
            self.reference_image_args.as_deref(),
            "reference image arguments",
        )?;
        let mut photo_maker_images = raw_rgb_images(&self.photo_maker.id_images)?;
        let photo_maker_image_count =
            c_int_len(photo_maker_images.len(), "PhotoMaker identity images")?;
        let sample = self.sample.to_native(&mut strings)?;
        let vae_tiling = self.vae_tiling.to_native(&mut strings)?;
        let cache = self.cache.to_native(&mut strings)?;
        let hires = self.hires.to_native(&mut strings)?;
        let photo_maker_path = strings.add_optional_path(
            self.photo_maker.id_embedding_path.as_deref(),
            "PhotoMaker identity embedding path",
        )?;
        let pulid_path = strings.add_optional_path(
            self.pulid.id_embedding_path.as_deref(),
            "PuLID identity embedding path",
        )?;

        let raw = sys::sd_img_gen_params_t {
            loras: ptr_or_null(&loras),
            lora_count,
            prompt,
            negative_prompt,
            clip_skip: self.clip_skip,
            init_image: optional_raw_rgb_image(self.init_image.as_ref())?,
            ref_images: mut_ptr_or_null(&mut reference_images),
            ref_images_count: reference_image_count,
            ref_image_args: reference_image_args,
            mask_image: optional_raw_gray_image(self.mask_image.as_ref())?,
            width: self.width,
            height: self.height,
            sample_params: sample.raw,
            strength: self.strength,
            seed: self.seed,
            batch_count: self.batch_count,
            control_image: optional_raw_rgb_image(self.control_image.as_ref())?,
            control_strength: self.control_strength,
            ip_adapter_image: optional_raw_rgb_image(self.ip_adapter_image.as_ref())?,
            ip_adapter_strength: self.ip_adapter_strength,
            pm_params: sys::sd_pm_params_t {
                id_images: mut_ptr_or_null(&mut photo_maker_images),
                id_images_count: photo_maker_image_count,
                id_embed_path: photo_maker_path,
                style_strength: self.photo_maker.style_strength,
            },
            pulid_params: sys::sd_pulid_params_t {
                id_embedding_path: pulid_path,
                id_weight: self.pulid.weight,
            },
            vae_tiling_params: vae_tiling,
            cache,
            hires: hires.raw,
            qwen_image_layers: self.qwen_image_layers,
            circular_x: self.circular_x,
            circular_y: self.circular_y,
        };
        Ok(NativeImageGenerationParams {
            raw,
            _strings: strings,
            _loras: loras,
            _reference_images: reference_images,
            _photo_maker_images: photo_maker_images,
            _sample: sample,
            _hires: hires,
        })
    }
}

impl fmt::Display for ImageGenerationParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "prompt: {}", self.prompt)?;
        writeln!(formatter, "negative_prompt: {}", self.negative_prompt)?;
        writeln!(formatter, "clip_skip: {}", self.clip_skip)?;
        writeln!(formatter, "width: {}", self.width)?;
        writeln!(formatter, "height: {}", self.height)?;
        writeln!(formatter, "sample: {}", self.sample)?;
        writeln!(formatter, "strength: {:.2}", self.strength)?;
        writeln!(formatter, "seed: {}", self.seed)?;
        writeln!(formatter, "batch_count: {}", self.batch_count)?;
        writeln!(
            formatter,
            "reference_images: {}",
            self.reference_images.len()
        )?;
        writeln!(formatter, "control_strength: {:.2}", self.control_strength)?;
        writeln!(formatter, "vae_tiling: {}", self.vae_tiling.enabled)?;
        writeln!(formatter, "cache: {}", self.cache.mode)?;
        write!(
            formatter,
            "hires: enabled={}, upscaler={}, scale={:.2}, target={}x{}",
            self.hires.enabled,
            self.hires.upscaler,
            self.hires.scale,
            self.hires.target_width,
            self.hires.target_height,
        )
    }
}

/// Complete video-generation request.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoGenerationParams {
    pub loras: Vec<Lora>,
    pub prompt: String,
    pub negative_prompt: String,
    pub clip_skip: i32,
    pub init_image: Option<RgbImage>,
    pub end_image: Option<RgbImage>,
    pub reference_images: Vec<RgbImage>,
    pub reference_videos: Vec<Video>,
    pub reference_audios: Vec<Audio>,
    pub control_frames: Vec<RgbImage>,
    pub width: i32,
    pub height: i32,
    pub sample: SampleParams,
    pub high_noise_sample: SampleParams,
    pub moe_boundary: f32,
    pub strength: f32,
    pub seed: i64,
    pub video_frames: i32,
    pub fps: i32,
    pub vace_strength: f32,
    pub vae_tiling: TilingParams,
    pub cache: CacheParams,
    pub hires: HiresParams,
    pub circular_x: bool,
    pub circular_y: bool,
}

impl Default for VideoGenerationParams {
    fn default() -> Self {
        let high_noise_sample = SampleParams {
            sample_steps: -1,
            ..SampleParams::default()
        };
        Self {
            loras: Vec::new(),
            prompt: String::new(),
            negative_prompt: String::new(),
            clip_skip: 0,
            init_image: None,
            end_image: None,
            reference_images: Vec::new(),
            reference_videos: Vec::new(),
            reference_audios: Vec::new(),
            control_frames: Vec::new(),
            width: 512,
            height: 512,
            sample: SampleParams::default(),
            high_noise_sample,
            moe_boundary: 0.875,
            strength: 0.75,
            seed: -1,
            video_frames: 6,
            fps: 16,
            vace_strength: 1.0,
            vae_tiling: TilingParams::default(),
            cache: CacheParams::default(),
            hires: HiresParams::default(),
            circular_x: false,
            circular_y: false,
        }
    }
}

pub(crate) struct NativeVideoGenerationParams {
    pub(crate) raw: sys::sd_vid_gen_params_t,
    _strings: StringPool,
    _loras: Vec<sys::sd_lora_t>,
    _reference_images: Vec<sys::sd_image_t>,
    _reference_video_frames: Vec<Vec<sys::sd_image_t>>,
    _reference_videos: Vec<sys::sd_ref_video_t>,
    _reference_audios: Vec<sys::sd_audio_t>,
    _control_frames: Vec<sys::sd_image_t>,
    _sample: NativeSampleParams,
    _high_noise_sample: NativeSampleParams,
    _hires: NativeHiresParams,
}

impl VideoGenerationParams {
    pub(crate) fn to_native(&self) -> Result<NativeVideoGenerationParams> {
        if self.width <= 0 || self.height <= 0 {
            return Err(Error::InvalidParameter {
                name: "video dimensions",
                reason: "width and height must be greater than zero",
            });
        }
        if self.video_frames <= 0 {
            return Err(Error::InvalidParameter {
                name: "video_frames",
                reason: "must be greater than zero",
            });
        }
        if self.fps <= 0 {
            return Err(Error::InvalidParameter {
                name: "fps",
                reason: "must be greater than zero",
            });
        }
        let mut strings = StringPool::default();
        let prompt = strings.add(&self.prompt, "prompt")?;
        let negative_prompt = strings.add(&self.negative_prompt, "negative_prompt")?;
        let loras = build_loras(&self.loras, &mut strings)?;
        let lora_count = u32_len(loras.len(), "LoRAs")?;
        let mut reference_images = raw_rgb_images(&self.reference_images)?;
        let reference_images_count = c_int_len(reference_images.len(), "reference images")?;
        let mut reference_video_frames = self
            .reference_videos
            .iter()
            .map(|video| raw_rgb_images(&video.frames))
            .collect::<Result<Vec<_>>>()?;
        let mut reference_videos = Vec::with_capacity(self.reference_videos.len());
        for (video, frames) in self
            .reference_videos
            .iter()
            .zip(&mut reference_video_frames)
        {
            if frames.is_empty() || video.fps == 0 {
                return Err(Error::InvalidParameter {
                    name: "reference video",
                    reason: "must contain frames and have a positive frame rate",
                });
            }
            reference_videos.push(sys::sd_ref_video_t {
                frames: mut_ptr_or_null(frames),
                frame_count: c_int_len(frames.len(), "reference video frames")?,
                fps: i32::try_from(video.fps).map_err(|_| Error::InvalidParameter {
                    name: "reference video fps",
                    reason: "must fit in a signed 32-bit integer",
                })?,
                audio: video.audio.as_ref().map_or_else(
                    || sys::sd_audio_t {
                        sample_rate: 0,
                        channels: 0,
                        sample_count: 0,
                        data: ptr::null_mut(),
                    },
                    |audio| sys::sd_audio_t {
                        sample_rate: audio.sample_rate(),
                        channels: audio.channels(),
                        sample_count: audio.sample_count(),
                        data: audio.data().as_ptr().cast_mut(),
                    },
                ),
            });
        }
        let reference_videos_count = c_int_len(reference_videos.len(), "reference videos")?;
        let mut reference_audios = self
            .reference_audios
            .iter()
            .map(|audio| sys::sd_audio_t {
                sample_rate: audio.sample_rate(),
                channels: audio.channels(),
                sample_count: audio.sample_count(),
                data: audio.data().as_ptr().cast_mut(),
            })
            .collect::<Vec<_>>();
        let reference_audios_count = c_int_len(reference_audios.len(), "reference audios")?;
        let mut control_frames = raw_rgb_images(&self.control_frames)?;
        let control_frames_size = c_int_len(control_frames.len(), "control frames")?;
        let sample = self.sample.to_native(&mut strings)?;
        let high_noise_sample = self.high_noise_sample.to_native(&mut strings)?;
        let vae_tiling = self.vae_tiling.to_native(&mut strings)?;
        let cache = self.cache.to_native(&mut strings)?;
        let hires = self.hires.to_native(&mut strings)?;

        let raw = sys::sd_vid_gen_params_t {
            loras: ptr_or_null(&loras),
            lora_count,
            prompt,
            negative_prompt,
            clip_skip: self.clip_skip,
            init_image: optional_raw_rgb_image(self.init_image.as_ref())?,
            end_image: optional_raw_rgb_image(self.end_image.as_ref())?,
            ref_images: mut_ptr_or_null(&mut reference_images),
            ref_images_count: reference_images_count,
            ref_videos: mut_ptr_or_null(&mut reference_videos),
            ref_videos_count: reference_videos_count,
            ref_audios: mut_ptr_or_null(&mut reference_audios),
            ref_audios_count: reference_audios_count,
            control_frames: mut_ptr_or_null(&mut control_frames),
            control_frames_size,
            width: self.width,
            height: self.height,
            sample_params: sample.raw,
            high_noise_sample_params: high_noise_sample.raw,
            moe_boundary: self.moe_boundary,
            strength: self.strength,
            seed: self.seed,
            video_frames: self.video_frames,
            fps: self.fps,
            vace_strength: self.vace_strength,
            vae_tiling_params: vae_tiling,
            cache,
            hires: hires.raw,
            circular_x: self.circular_x,
            circular_y: self.circular_y,
        };
        Ok(NativeVideoGenerationParams {
            raw,
            _strings: strings,
            _loras: loras,
            _reference_images: reference_images,
            _reference_video_frames: reference_video_frames,
            _reference_videos: reference_videos,
            _reference_audios: reference_audios,
            _control_frames: control_frames,
            _sample: sample,
            _high_noise_sample: high_noise_sample,
            _hires: hires,
        })
    }
}

#[cfg(test)]
mod tests {
    use ::image::{GrayImage, RgbImage};

    use super::{ImageGenerationParams, SampleParams, VideoGenerationParams};
    use crate::{SampleMethod, Scheduler};

    #[test]
    fn defaults_match_upstream_initializers() {
        let sample = SampleParams::default();
        assert_eq!(sample.sample_method, SampleMethod::Auto);
        assert_eq!(sample.scheduler, Scheduler::Auto);
        assert_eq!(sample.sample_steps, 20);
        assert!(sample.eta.is_infinite());

        let image = ImageGenerationParams::default();
        assert_eq!(
            (image.width, image.height, image.batch_count),
            (512, 512, 1)
        );

        let video = VideoGenerationParams::default();
        assert_eq!((video.video_frames, video.fps), (6, 16));
        assert_eq!(video.high_noise_sample.sample_steps, -1);
    }

    #[test]
    fn image_parameters_preserve_native_pixel_formats() {
        let init_image = RgbImage::from_raw(1, 1, vec![1, 2, 3]).unwrap();
        let mask_image = GrayImage::from_raw(1, 1, vec![255]).unwrap();
        let params = ImageGenerationParams {
            init_image: Some(init_image),
            mask_image: Some(mask_image),
            ..ImageGenerationParams::default()
        };

        let native = params.to_native().unwrap();
        assert_eq!(native.raw.init_image.channel, 3);
        assert_eq!(native.raw.mask_image.channel, 1);
        assert_eq!(
            native.raw.init_image.data,
            params
                .init_image
                .as_ref()
                .unwrap()
                .as_raw()
                .as_ptr()
                .cast_mut()
        );
        assert_eq!(
            native.raw.mask_image.data,
            params
                .mask_image
                .as_ref()
                .unwrap()
                .as_raw()
                .as_ptr()
                .cast_mut()
        );
    }
}
