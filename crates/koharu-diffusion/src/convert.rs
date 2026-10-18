use std::{
    marker::PhantomData,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    Error, Result, RgbImage, WeightType,
    ffi::{NativeCall, configure_native, optional_cstring, optional_path_cstring, path_cstring},
    image::raw_rgb_image,
    sys,
};

/// Single-file model conversion settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub input_path: PathBuf,
    pub vae_path: Option<PathBuf>,
    pub output_path: PathBuf,
    pub output_type: WeightType,
    pub tensor_type_rules: Option<String>,
    pub convert_tensor_names: bool,
}

/// Multi-component model conversion settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentConversion {
    pub model_path: Option<PathBuf>,
    pub clip_l_path: Option<PathBuf>,
    pub clip_g_path: Option<PathBuf>,
    pub t5xxl_path: Option<PathBuf>,
    pub diffusion_model_path: Option<PathBuf>,
    pub vae_path: Option<PathBuf>,
    pub output_path: PathBuf,
    pub output_type: WeightType,
    pub tensor_type_rules: Option<String>,
    pub convert_tensor_names: bool,
    pub n_threads: i32,
}

/// Converts a model file, optionally merging a separate VAE.
pub fn convert(params: &Conversion) -> Result<()> {
    if params.output_type == WeightType::Auto {
        return Err(Error::InvalidParameter {
            name: "output_type",
            reason: "must be a concrete tensor type",
        });
    }
    let input = path_cstring(&params.input_path, "input_path")?;
    let vae = optional_path_cstring(params.vae_path.as_deref(), "vae_path")?;
    let output = path_cstring(&params.output_path, "output_path")?;
    let rules = optional_cstring(params.tensor_type_rules.as_deref(), "tensor_type_rules")?;
    let _call = NativeCall::enter();
    let succeeded = unsafe {
        sys::convert(
            input.as_ptr(),
            vae.as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr()),
            output.as_ptr(),
            params.output_type.as_raw(),
            rules
                .as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr()),
            params.convert_tensor_names,
        )
    };
    if succeeded {
        Ok(())
    } else {
        Err(Error::ConversionFailed)
    }
}

/// Converts and merges separately stored model components.
pub fn convert_with_components(params: &ComponentConversion) -> Result<()> {
    if params.output_type == WeightType::Auto {
        return Err(Error::InvalidParameter {
            name: "output_type",
            reason: "must be a concrete tensor type",
        });
    }
    if params.n_threads <= 0 {
        return Err(Error::InvalidParameter {
            name: "n_threads",
            reason: "must be greater than zero",
        });
    }
    if params.model_path.is_none()
        && params.clip_l_path.is_none()
        && params.clip_g_path.is_none()
        && params.t5xxl_path.is_none()
        && params.diffusion_model_path.is_none()
        && params.vae_path.is_none()
    {
        return Err(Error::InvalidParameter {
            name: "component paths",
            reason: "at least one input component is required",
        });
    }

    let model = optional_path_cstring(params.model_path.as_deref(), "model_path")?;
    let clip_l = optional_path_cstring(params.clip_l_path.as_deref(), "clip_l_path")?;
    let clip_g = optional_path_cstring(params.clip_g_path.as_deref(), "clip_g_path")?;
    let t5xxl = optional_path_cstring(params.t5xxl_path.as_deref(), "t5xxl_path")?;
    let diffusion = optional_path_cstring(
        params.diffusion_model_path.as_deref(),
        "diffusion_model_path",
    )?;
    let vae = optional_path_cstring(params.vae_path.as_deref(), "vae_path")?;
    let output = path_cstring(&params.output_path, "output_path")?;
    let rules = optional_cstring(params.tensor_type_rules.as_deref(), "tensor_type_rules")?;
    let pointer = |value: &Option<std::ffi::CString>| {
        value
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr())
    };
    let _call = NativeCall::enter();
    let succeeded = unsafe {
        sys::convert_with_components(
            pointer(&model),
            pointer(&clip_l),
            pointer(&clip_g),
            pointer(&t5xxl),
            pointer(&diffusion),
            pointer(&vae),
            output.as_ptr(),
            params.output_type.as_raw(),
            pointer(&rules),
            params.convert_tensor_names,
            params.n_threads,
            std::ptr::null(),
            0,
        )
    };
    if succeeded {
        Ok(())
    } else {
        Err(Error::ConversionFailed)
    }
}

/// Canny edge detector settings used by stable-diffusion.cpp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CannyParams {
    pub high_threshold: f32,
    pub low_threshold: f32,
    pub weak: f32,
    pub strong: f32,
    pub inverse: bool,
}

impl Default for CannyParams {
    fn default() -> Self {
        Self {
            high_threshold: 0.08,
            low_threshold: 0.08,
            weak: 0.8,
            strong: 1.0,
            inverse: false,
        }
    }
}

/// Applies the native Canny preprocessor in place.
pub fn preprocess_canny(image: &mut RgbImage, params: CannyParams) -> Result<()> {
    let image = raw_rgb_image(image)?;
    let _call = NativeCall::enter();
    let succeeded = unsafe {
        sys::preprocess_canny(
            image,
            params.high_threshold,
            params.low_threshold,
            params.weak,
            params.strong,
            params.inverse,
        )
    };
    if succeeded {
        Ok(())
    } else {
        Err(Error::PreprocessFailed)
    }
}

/// Loads an importance matrix into stable-diffusion.cpp.
pub fn load_imatrix(path: impl Into<PathBuf>) -> Result<()> {
    let path = path.into();
    let path = path_cstring(&path, "imatrix_path")?;
    let succeeded = configure_native(|| unsafe { sys::load_imatrix(path.as_ptr()) })?;
    if succeeded {
        Ok(())
    } else {
        Err(Error::ImatrixLoadFailed)
    }
}

static IMATRIX_COLLECTION_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Guard for process-wide importance-matrix collection.
///
/// Dropping the guard disables collection. Collection replaces a custom graph
/// evaluation callback in the native library.
#[derive(Debug)]
pub struct ImatrixCollector {
    active: bool,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

/// Enables process-wide importance-matrix collection.
pub fn begin_imatrix_collection() -> Result<ImatrixCollector> {
    if IMATRIX_COLLECTION_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::ImatrixCollectionAlreadyActive);
    }
    if let Err(error) = configure_native(|| unsafe { sys::enable_imatrix_collection() }) {
        IMATRIX_COLLECTION_ACTIVE.store(false, Ordering::Release);
        return Err(error);
    }
    Ok(ImatrixCollector {
        active: true,
        _not_send_or_sync: PhantomData,
    })
}

impl ImatrixCollector {
    /// Saves the currently collected matrix.
    pub fn save(&self, path: impl Into<PathBuf>) -> Result<()> {
        let path = path.into();
        let path = path_cstring(&path, "imatrix_path")?;
        configure_native(|| unsafe { sys::save_imatrix(path.as_ptr()) })
    }

    /// Explicitly disables collection and reports reentrancy errors.
    pub fn finish(mut self) -> Result<()> {
        self.disable()?;
        Ok(())
    }

    fn disable(&mut self) -> Result<()> {
        if self.active {
            configure_native(|| unsafe { sys::disable_imatrix_collection() })?;
            self.active = false;
            IMATRIX_COLLECTION_ACTIVE.store(false, Ordering::Release);
        }
        Ok(())
    }
}

impl Drop for ImatrixCollector {
    fn drop(&mut self) {
        let _ = self.disable();
    }
}
