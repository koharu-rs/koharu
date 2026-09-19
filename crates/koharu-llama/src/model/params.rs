//! A safe wrapper around `llama_model_params`.

use crate::LlamaCppError;
use crate::model::params::kv_overrides::KvOverrides;
use std::ffi::{CStr, c_char, c_void};
use std::fmt::{Debug, Formatter};
use std::pin::Pin;
use std::ptr::null;

pub mod kv_overrides;

#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::cast_possible_truncation)]
const LLAMA_SPLIT_MODE_NONE: i8 = koharu_llama_sys::LLAMA_SPLIT_MODE_NONE as i8;
#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::cast_possible_truncation)]
const LLAMA_SPLIT_MODE_LAYER: i8 = koharu_llama_sys::LLAMA_SPLIT_MODE_LAYER as i8;
#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::cast_possible_truncation)]
const LLAMA_SPLIT_MODE_ROW: i8 = koharu_llama_sys::LLAMA_SPLIT_MODE_ROW as i8;
#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::cast_possible_truncation)]
const LLAMA_SPLIT_MODE_TENSOR: i8 = koharu_llama_sys::LLAMA_SPLIT_MODE_TENSOR as i8;

/// A rusty wrapper around `llama_split_mode`.
#[repr(i8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LlamaSplitMode {
    /// Single GPU
    None = LLAMA_SPLIT_MODE_NONE,
    /// Split layers and KV across GPUs
    Layer = LLAMA_SPLIT_MODE_LAYER,
    /// Split layers and KV across GPUs, use tensor parallelism if supported
    Row = LLAMA_SPLIT_MODE_ROW,
    /// Experimental tensor parallelism across GPUs
    Tensor = LLAMA_SPLIT_MODE_TENSOR,
}

/// An error that occurs when unknown split mode is encountered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlamaSplitModeParseError(pub i32);

/// Create a `LlamaSplitMode` from a `i32`.
///
/// # Errors
/// Returns `LlamaSplitModeParseError` if the value does not correspond to a valid `LlamaSplitMode`.
impl TryFrom<i32> for LlamaSplitMode {
    type Error = LlamaSplitModeParseError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        let i8_value = value
            .try_into()
            .map_err(|_| LlamaSplitModeParseError(value))?;
        match i8_value {
            LLAMA_SPLIT_MODE_NONE => Ok(Self::None),
            LLAMA_SPLIT_MODE_LAYER => Ok(Self::Layer),
            LLAMA_SPLIT_MODE_ROW => Ok(Self::Row),
            LLAMA_SPLIT_MODE_TENSOR => Ok(Self::Tensor),
            _ => Err(LlamaSplitModeParseError(value)),
        }
    }
}

/// Create a `LlamaSplitMode` from a `u32`.
///
/// # Errors
/// Returns `LlamaSplitModeParseError` if the value does not correspond to a valid `LlamaSplitMode`.
impl TryFrom<u32> for LlamaSplitMode {
    type Error = LlamaSplitModeParseError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        let i8_value = value
            .try_into()
            .map_err(|_| LlamaSplitModeParseError(value.try_into().unwrap_or(i32::MAX)))?;
        match i8_value {
            LLAMA_SPLIT_MODE_NONE => Ok(Self::None),
            LLAMA_SPLIT_MODE_LAYER => Ok(Self::Layer),
            LLAMA_SPLIT_MODE_ROW => Ok(Self::Row),
            LLAMA_SPLIT_MODE_TENSOR => Ok(Self::Tensor),
            _ => Err(LlamaSplitModeParseError(
                value.try_into().unwrap_or(i32::MAX),
            )),
        }
    }
}

/// Create a `i32` from a `LlamaSplitMode`.
impl From<LlamaSplitMode> for i32 {
    fn from(value: LlamaSplitMode) -> Self {
        match value {
            LlamaSplitMode::None => LLAMA_SPLIT_MODE_NONE.into(),
            LlamaSplitMode::Layer => LLAMA_SPLIT_MODE_LAYER.into(),
            LlamaSplitMode::Row => LLAMA_SPLIT_MODE_ROW.into(),
            LlamaSplitMode::Tensor => LLAMA_SPLIT_MODE_TENSOR.into(),
        }
    }
}

/// Create a `u32` from a `LlamaSplitMode`.
impl From<LlamaSplitMode> for u32 {
    fn from(value: LlamaSplitMode) -> Self {
        match value {
            LlamaSplitMode::None => LLAMA_SPLIT_MODE_NONE as u32,
            LlamaSplitMode::Layer => LLAMA_SPLIT_MODE_LAYER as u32,
            LlamaSplitMode::Row => LLAMA_SPLIT_MODE_ROW as u32,
            LlamaSplitMode::Tensor => LLAMA_SPLIT_MODE_TENSOR as u32,
        }
    }
}

/// The default split mode is `Layer` in llama.cpp.
impl Default for LlamaSplitMode {
    fn default() -> Self {
        LlamaSplitMode::Layer
    }
}

/// Controls how llama.cpp loads model data.
#[repr(i32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LlamaLoadMode {
    /// Select the best supported mode for the active device.
    #[default]
    Auto = koharu_llama_sys::LLAMA_LOAD_MODE_AUTO,
    /// Read model data without memory mapping or locking.
    None = koharu_llama_sys::LLAMA_LOAD_MODE_NONE,
    /// Memory-map model data.
    Mmap = koharu_llama_sys::LLAMA_LOAD_MODE_MMAP,
    /// Keep model data resident in RAM without memory mapping it.
    Mlock = koharu_llama_sys::LLAMA_LOAD_MODE_MLOCK,
    /// Memory-map model data and keep it resident in RAM.
    MmapMlock = koharu_llama_sys::LLAMA_LOAD_MODE_MMAP_MLOCK,
    /// Use direct I/O where supported.
    DirectIo = koharu_llama_sys::LLAMA_LOAD_MODE_DIRECT_IO,
}

/// An error returned when llama.cpp reports an unknown model load mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlamaLoadModeParseError(pub i32);

impl TryFrom<i32> for LlamaLoadMode {
    type Error = LlamaLoadModeParseError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            koharu_llama_sys::LLAMA_LOAD_MODE_AUTO => Ok(Self::Auto),
            koharu_llama_sys::LLAMA_LOAD_MODE_NONE => Ok(Self::None),
            koharu_llama_sys::LLAMA_LOAD_MODE_MMAP => Ok(Self::Mmap),
            koharu_llama_sys::LLAMA_LOAD_MODE_MLOCK => Ok(Self::Mlock),
            koharu_llama_sys::LLAMA_LOAD_MODE_MMAP_MLOCK => Ok(Self::MmapMlock),
            koharu_llama_sys::LLAMA_LOAD_MODE_DIRECT_IO => Ok(Self::DirectIo),
            _ => Err(LlamaLoadModeParseError(value)),
        }
    }
}

impl From<LlamaLoadMode> for i32 {
    fn from(value: LlamaLoadMode) -> Self {
        value as i32
    }
}

/// The maximum number of devices supported.
///
/// The real maximum number of devices is the lesser one of this value and the value returned by
/// `koharu_llama::max_devices()`.
pub const LLAMA_CPP_MAX_DEVICES: usize = 16;

/// A safe wrapper around `llama_model_params`.
#[allow(clippy::module_name_repetitions)]
pub struct LlamaModelParams {
    pub(crate) params: koharu_llama_sys::llama_model_params,
    kv_overrides: Vec<koharu_llama_sys::llama_model_kv_override>,
    buft_overrides: Vec<koharu_llama_sys::llama_model_tensor_buft_override>,
    devices: Pin<Box<[koharu_llama_sys::ggml_backend_dev_t; LLAMA_CPP_MAX_DEVICES]>>,
    progress_callback: Option<Box<dyn FnMut(f32) -> bool>>,
}

impl Debug for LlamaModelParams {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaModelParams")
            .field("n_gpu_layers", &self.params.n_gpu_layers)
            .field("main_gpu", &self.params.main_gpu)
            .field("vocab_only", &self.params.vocab_only)
            .field("load_mode", &self.load_mode())
            .field("split_mode", &self.split_mode())
            .field("devices", &self.devices)
            .field("kv_overrides", &"vec of kv_overrides")
            .finish()
    }
}

impl LlamaModelParams {
    /// See [`KvOverrides`]
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use koharu_llama::model::params::LlamaModelParams;
    /// let params = Box::pin(LlamaModelParams::default());
    /// let kv_overrides = params.kv_overrides();
    /// let count = kv_overrides.into_iter().count();
    /// assert_eq!(count, 0);
    /// ```
    #[must_use]
    pub fn kv_overrides<'a>(&'a self) -> KvOverrides<'a> {
        KvOverrides::new(self)
    }

    /// Appends a key-value override to the model parameters. It must be pinned as this creates a self-referential struct.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use std::ffi::{CStr, CString};
    /// use std::pin::pin;
    /// # use koharu_llama::model::params::LlamaModelParams;
    /// # use koharu_llama::model::params::kv_overrides::ParamOverrideValue;
    /// let mut params = pin!(LlamaModelParams::default());
    /// let key = CString::new("key").expect("CString::new failed");
    /// params.as_mut().append_kv_override(&key, ParamOverrideValue::Int(50));
    ///
    /// let kv_overrides = params.kv_overrides().into_iter().collect::<Vec<_>>();
    /// assert_eq!(kv_overrides.len(), 1);
    ///
    /// let (k, v) = &kv_overrides[0];
    /// assert_eq!(v, &ParamOverrideValue::Int(50));
    ///
    /// assert_eq!(k.to_bytes(), b"key", "expected key to be 'key', was {:?}", k);
    /// ```
    #[allow(clippy::missing_panics_doc)] // panics are just to enforce internal invariants, not user errors
    pub fn append_kv_override(
        mut self: Pin<&mut Self>,
        key: &CStr,
        value: kv_overrides::ParamOverrideValue,
    ) {
        let kv_override = self
            .kv_overrides
            .get_mut(0)
            .expect("kv_overrides did not have a next allocated");

        assert_eq!(kv_override.key[0], 0, "last kv_override was not empty");

        // There should be some way to do this without iterating over everything.
        for (i, &c) in key.to_bytes_with_nul().iter().enumerate() {
            kv_override.key[i] = c_char::try_from(c).expect("invalid character in key");
        }

        kv_override.tag = value.tag();
        kv_override.__bindgen_anon_1 = value.value();

        // set to null pointer for panic safety (as push may move the vector, invalidating the pointer)
        self.params.kv_overrides = null();

        // push the next one to ensure we maintain the iterator invariant of ending with a 0
        self.kv_overrides
            .push(koharu_llama_sys::llama_model_kv_override {
                key: [0; 128],
                tag: 0,
                __bindgen_anon_1: koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 {
                    val_i64: 0,
                },
            });

        // set the pointer to the (potentially) new vector
        self.params.kv_overrides = self.kv_overrides.as_ptr();

        eprintln!("saved ptr: {:?}", self.params.kv_overrides);
    }
}

impl LlamaModelParams {
    /// Adds buffer type overides to move all mixture-of-experts layers to CPU.
    pub fn add_cpu_moe_override(self: Pin<&mut Self>) {
        self.add_cpu_buft_override(c"\\.ffn_(up|down|gate)_(ch|)exps");
    }

    /// Appends a buffer type override to the model parameters, to move layers matching pattern to CPU.
    /// It must be pinned as this creates a self-referential struct.
    pub fn add_cpu_buft_override(mut self: Pin<&mut Self>, key: &CStr) {
        let buft_override = self
            .buft_overrides
            .get_mut(0)
            .expect("buft_overrides did not have a next allocated");

        assert!(
            buft_override.pattern.is_null(),
            "last buft_override was not empty"
        );

        // There should be some way to do this without iterating over everything.
        for &c in key.to_bytes_with_nul().iter() {
            c_char::try_from(c).expect("invalid character in key");
        }

        buft_override.pattern = key.as_ptr();
        buft_override.buft = unsafe { koharu_llama_sys::ggml_backend_cpu_buffer_type() };

        // set to null pointer for panic safety (as push may move the vector, invalidating the pointer)
        self.params.tensor_buft_overrides = null();

        // push the next one to ensure we maintain the iterator invariant of ending with a 0
        self.buft_overrides
            .push(koharu_llama_sys::llama_model_tensor_buft_override {
                pattern: std::ptr::null(),
                buft: std::ptr::null_mut(),
            });

        // set the pointer to the (potentially) new vector
        self.params.tensor_buft_overrides = self.buft_overrides.as_ptr();
    }

    /// Returns the tensor-name patterns of the buffer-type overrides currently set on these
    /// parameters, in order.
    ///
    /// This is the read-only counterpart to [`add_cpu_buft_override`](Self::add_cpu_buft_override)
    /// and [`add_cpu_moe_override`](Self::add_cpu_moe_override). Returns an empty vector when no
    /// overrides are set. The trailing null-terminator entry the override list carries is skipped;
    /// only entries with a non-null pattern are returned.
    #[must_use]
    pub fn tensor_buft_override_patterns(&self) -> Vec<String> {
        self.buft_overrides
            .iter()
            .filter(|o| !o.pattern.is_null())
            .map(|o| {
                // SAFETY: the setter accepts a NUL-terminated `CStr`. Its callers must keep that
                // string alive for at least as long as these params; every in-tree caller passes a
                // static literal.
                unsafe { CStr::from_ptr(o.pattern) }
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

impl LlamaModelParams {
    /// Get the number of layers to offload to the GPU.
    #[must_use]
    pub fn n_gpu_layers(&self) -> i32 {
        self.params.n_gpu_layers
    }

    /// The GPU that is used for scratch and small tensors
    #[must_use]
    pub fn main_gpu(&self) -> i32 {
        self.params.main_gpu
    }

    /// only load the vocabulary, no weights
    #[must_use]
    pub fn vocab_only(&self) -> bool {
        self.params.vocab_only
    }

    /// Gets how llama.cpp loads model data.
    ///
    /// # Errors
    /// Returns [`LlamaLoadModeParseError`] if llama.cpp reports an unknown mode.
    pub fn load_mode(&self) -> Result<LlamaLoadMode, LlamaLoadModeParseError> {
        LlamaLoadMode::try_from(self.params.load_mode)
    }

    /// get the split mode
    ///
    /// # Errors
    /// Returns `LlamaSplitModeParseError` if the unknown split mode is encountered.
    pub fn split_mode(&self) -> Result<LlamaSplitMode, LlamaSplitModeParseError> {
        LlamaSplitMode::try_from(self.params.split_mode)
    }

    /// get the devices
    #[must_use]
    pub fn devices(&self) -> Vec<usize> {
        let mut backend_devices = Vec::new();
        for i in 0..unsafe { koharu_llama_sys::ggml_backend_dev_count() } {
            let dev = unsafe { koharu_llama_sys::ggml_backend_dev_get(i) };
            backend_devices.push(dev);
        }
        let mut devices = Vec::new();
        for &dev in self.devices.iter() {
            if dev.is_null() {
                break;
            }
            if let Some((index, _)) = backend_devices
                .iter()
                .enumerate()
                .find(|&(_i, &d)| d == dev)
            {
                devices.push(index);
            }
        }
        devices
    }

    /// sets the number of gpu layers to offload to the GPU.
    /// ```no_run
    /// # use koharu_llama::model::params::LlamaModelParams;
    /// let params = LlamaModelParams::default();
    /// let params = params.with_n_gpu_layers(1);
    /// assert_eq!(params.n_gpu_layers(), 1);
    /// ```
    #[must_use]
    pub fn with_n_gpu_layers(mut self, n_gpu_layers: u32) -> Self {
        // The only way this conversion can fail is if u32 overflows the i32 - in which case we set
        // to MAX
        let n_gpu_layers = i32::try_from(n_gpu_layers).unwrap_or(i32::MAX);
        self.params.n_gpu_layers = n_gpu_layers;
        self
    }

    /// sets the main GPU
    ///
    /// To enable this option, you must set `split_mode` to `LlamaSplitMode::None` to enable single GPU mode.
    #[must_use]
    pub fn with_main_gpu(mut self, main_gpu: i32) -> Self {
        self.params.main_gpu = main_gpu;
        self
    }

    /// sets `vocab_only`
    #[must_use]
    pub fn with_vocab_only(mut self, vocab_only: bool) -> Self {
        self.params.vocab_only = vocab_only;
        self
    }

    /// Sets how llama.cpp loads model data.
    #[must_use]
    pub fn with_load_mode(mut self, load_mode: LlamaLoadMode) -> Self {
        self.params.load_mode = load_mode.into();
        self
    }

    /// sets `split_mode`
    #[must_use]
    pub fn with_split_mode(mut self, split_mode: LlamaSplitMode) -> Self {
        self.params.split_mode = split_mode.into();
        self
    }

    /// sets `devices`
    ///
    /// The devices are specified as indices that correspond to the ggml backend device indices.
    ///
    /// The maximum number of devices is 16.
    ///
    /// You don't need to specify CPU or ACCEL devices.
    ///
    /// # Errors
    /// Returns `LlamaCppError::BackendDeviceNotFound` if any device index is invalid.
    pub fn with_devices(mut self, devices: &[usize]) -> Result<Self, LlamaCppError> {
        for dev in self.devices.iter_mut() {
            *dev = std::ptr::null_mut();
        }
        // Check device count
        let max_devices = crate::max_devices().min(LLAMA_CPP_MAX_DEVICES);
        if devices.len() > max_devices {
            return Err(LlamaCppError::MaxDevicesExceeded(max_devices));
        }
        for (i, &dev) in devices.iter().enumerate() {
            if dev >= unsafe { koharu_llama_sys::ggml_backend_dev_count() } {
                return Err(LlamaCppError::BackendDeviceNotFound(dev));
            }
            let backend_dev = unsafe { koharu_llama_sys::ggml_backend_dev_get(dev) };
            self.devices[i] = backend_dev;
        }
        if self.devices.is_empty() {
            self.params.devices = std::ptr::null_mut();
        } else {
            self.params.devices = self.devices.as_mut_ptr();
        }
        Ok(self)
    }

    /// Set `no_alloc`
    ///
    /// If this parameter is true, don't allocate memory for the tensor data
    ///
    #[must_use]
    pub fn with_no_alloc(mut self, no_alloc: bool) -> Self {
        self.params.no_alloc = no_alloc;
        if no_alloc {
            self.params.load_mode = koharu_llama_sys::LLAMA_LOAD_MODE_NONE;
        }
        self
    }

    /// Get `no_alloc`
    ///
    /// If this parameter is true, don't allocate memory for the tensor data
    #[must_use]
    pub fn no_alloc(&self) -> bool {
        self.params.no_alloc
    }

    /// Sets a callback invoked during loading with progress in `0.0..=1.0`.
    /// Returning `false` aborts the load (it then fails with `NullResult`).
    #[must_use]
    pub fn with_progress_callback<F: FnMut(f32) -> bool + 'static>(mut self, callback: F) -> Self {
        unsafe extern "C" fn trampoline<F: FnMut(f32) -> bool>(
            progress: f32,
            user_data: *mut c_void,
        ) -> bool {
            let callback = unsafe { &mut *user_data.cast::<F>() };
            callback(progress)
        }

        let mut callback = Box::new(callback);
        self.params.progress_callback_user_data =
            std::ptr::from_mut(&mut *callback).cast::<c_void>();
        self.params.progress_callback = Some(trampoline::<F>);
        self.progress_callback = Some(callback);
        self
    }
}

/// Default parameters for `LlamaModel`. (as defined in llama.cpp by `llama_model_default_params`)
/// ```no_run
/// # use koharu_llama::model::params::LlamaModelParams;
/// use koharu_llama::model::params::{LlamaLoadMode, LlamaSplitMode};
/// let params = LlamaModelParams::default();
/// assert_eq!(params.n_gpu_layers(), -1, "n_gpu_layers should be -1");
/// assert_eq!(params.main_gpu(), 0, "main_gpu should be 0");
/// assert_eq!(params.vocab_only(), false, "vocab_only should be false");
/// assert_eq!(params.load_mode(), Ok(LlamaLoadMode::Auto), "load_mode should be AUTO");
/// assert_eq!(params.split_mode(), Ok(LlamaSplitMode::Layer), "split_mode should be LAYER");
/// assert_eq!(params.devices().len(), 0, "devices should be empty");
/// assert_eq!(params.no_alloc(), false, "no_alloc should be false");
/// ```
impl Default for LlamaModelParams {
    fn default() -> Self {
        let default_params = unsafe { koharu_llama_sys::llama_model_default_params() };
        LlamaModelParams {
            params: default_params,
            // push the next one to ensure we maintain the iterator invariant of ending with a 0
            kv_overrides: vec![koharu_llama_sys::llama_model_kv_override {
                key: [0; 128],
                tag: 0,
                __bindgen_anon_1: koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 {
                    val_i64: 0,
                },
            }],
            buft_overrides: vec![koharu_llama_sys::llama_model_tensor_buft_override {
                pattern: std::ptr::null(),
                buft: std::ptr::null_mut(),
            }],
            devices: Box::pin([std::ptr::null_mut(); 16]),
            progress_callback: None,
        }
    }
}
