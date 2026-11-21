//! sd.cpp's option objects, which a model loads by and a generation runs by.

use std::{ffi::CString, path::Path, ptr::NonNull};

use image::{GrayImage, RgbImage};
use koharu_diffusion_sys as sys;

use crate::{Cache, OptionError, SampleMethod, Scheduler, StatusError, VaeFormat};

/// An option object. sd.cpp copies what it's set to and retains the images,
/// so nothing set has to outlive the call.
pub(crate) struct Options(NonNull<sys::sd_opt_t>);

impl Options {
    /// Options for loading a model, `None` when sd.cpp couldn't make them.
    pub(crate) fn context() -> Option<Self> {
        NonNull::new(unsafe { sys::sd_ctx_opt_create() }).map(Self)
    }

    /// Options for one generation, `None` when sd.cpp couldn't make them.
    pub(crate) fn image() -> Option<Self> {
        NonNull::new(unsafe { sys::sd_img_gen_opt_create() }).map(Self)
    }

    pub(crate) fn as_ptr(&self) -> *const sys::sd_opt_t {
        self.0.as_ptr()
    }

    /// Sets the option sd.cpp names `key`; a value left unset keeps sd.cpp's
    /// default.
    pub(crate) fn set<'a>(
        &self,
        key: &'static str,
        value: impl Into<Value<'a>>,
    ) -> Result<(), OptionError> {
        let text = |text: &str| CString::new(text).map_err(|_| OptionError::Nul(key));
        let (options, name) = (self.0.as_ptr(), text(key)?);
        let status = match value.into() {
            Value::Unset => return Ok(()),
            Value::Bool(value) => unsafe { sys::sd_opt_set_bool(options, name.as_ptr(), value) },
            Value::Int(value) => unsafe { sys::sd_opt_set_int(options, name.as_ptr(), value) },
            Value::Float(value) => unsafe { sys::sd_opt_set_float(options, name.as_ptr(), value) },
            Value::Floats(values) => unsafe {
                sys::sd_opt_set_floats(options, name.as_ptr(), values.as_ptr(), values.len())
            },
            Value::Text(value) => {
                let value = text(value)?;
                unsafe { sys::sd_opt_set_string(options, name.as_ptr(), value.as_ptr()) }
            }
            Value::Path(path) => {
                let path = path
                    .to_str()
                    .ok_or_else(|| OptionError::PathToStr(path.to_owned()))?;
                let path = text(path)?;
                unsafe { sys::sd_opt_set_string(options, name.as_ptr(), path.as_ptr()) }
            }
            Value::Image(image) => {
                let image = Tensor::new(image).ok_or(OptionError::Image)?;
                unsafe { sys::sd_opt_set_tensor(options, name.as_ptr(), image.0.as_ptr()) }
            }
            Value::Images(images) => {
                let images = images
                    .into_iter()
                    .map(Tensor::new)
                    .collect::<Option<Vec<_>>>()
                    .ok_or(OptionError::Image)?;
                let images = images
                    .iter()
                    .map(|image| image.0.as_ptr().cast_const())
                    .collect::<Vec<_>>();
                unsafe {
                    sys::sd_opt_set_tensors(options, name.as_ptr(), images.as_ptr(), images.len())
                }
            }
        };
        match status {
            sys::SD_STATUS_OK => Ok(()),
            status => Err(StatusError::from(status).into()),
        }
    }
}

impl Drop for Options {
    fn drop(&mut self) {
        unsafe { sys::sd_opt_free(self.0.as_ptr()) };
    }
}

/// What an option is set to, in the type sd.cpp takes it as.
pub(crate) enum Value<'a> {
    Unset,
    Bool(bool),
    Int(i64),
    Float(f64),
    Floats(&'a [f32]),
    Text(&'a str),
    Path(&'a Path),
    Image(Image<'a>),
    Images(Vec<Image<'a>>),
}

/// An image's pixels, `channels` to a pixel, row by row.
pub(crate) struct Image<'a> {
    channels: u8,
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a, T: Into<Value<'a>>> From<Option<T>> for Value<'a> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Unset, Into::into)
    }
}

impl From<bool> for Value<'_> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<u32> for Value<'_> {
    fn from(value: u32) -> Self {
        Self::Int(value.into())
    }
}

impl From<i64> for Value<'_> {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f32> for Value<'_> {
    fn from(value: f32) -> Self {
        Self::Float(value.into())
    }
}

/// No values leaves the option unset.
impl<'a> From<&'a [f32]> for Value<'a> {
    fn from(values: &'a [f32]) -> Self {
        match values.is_empty() {
            true => Self::Unset,
            false => Self::Floats(values),
        }
    }
}

impl<'a> From<&'a str> for Value<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(value)
    }
}

impl<'a> From<&'a Path> for Value<'a> {
    fn from(value: &'a Path) -> Self {
        Self::Path(value)
    }
}

impl<'a> From<&'a RgbImage> for Value<'a> {
    fn from(image: &'a RgbImage) -> Self {
        Self::Image(image.into())
    }
}

impl<'a> From<&'a GrayImage> for Value<'a> {
    fn from(image: &'a GrayImage) -> Self {
        Self::Image(Image {
            channels: 1,
            width: image.width(),
            height: image.height(),
            pixels: image.as_raw(),
        })
    }
}

/// No images leaves the option unset.
impl<'a> From<&'a [RgbImage]> for Value<'a> {
    fn from(images: &'a [RgbImage]) -> Self {
        match images.is_empty() {
            true => Self::Unset,
            false => Self::Images(images.iter().map(Into::into).collect()),
        }
    }
}

impl<'a> From<&'a RgbImage> for Image<'a> {
    fn from(image: &'a RgbImage) -> Self {
        Self {
            channels: 3,
            width: image.width(),
            height: image.height(),
            pixels: image.as_raw(),
        }
    }
}

impl From<VaeFormat> for Value<'_> {
    fn from(format: VaeFormat) -> Self {
        Self::Text(match format {
            VaeFormat::Flux2 => "flux2",
        })
    }
}

impl From<Scheduler> for Value<'_> {
    fn from(scheduler: Scheduler) -> Self {
        Self::Text(match scheduler {
            Scheduler::Discrete => "discrete",
        })
    }
}

impl From<Cache> for Value<'_> {
    fn from(cache: Cache) -> Self {
        Self::Text(match cache {
            Cache::EasyCache => "easycache",
            Cache::Spectrum => "spectrum",
        })
    }
}

impl From<SampleMethod> for Value<'_> {
    fn from(method: SampleMethod) -> Self {
        Self::Text(match method {
            SampleMethod::Euler => "euler",
        })
    }
}

/// An image sd.cpp copied in, `[channels, width, height]` from the most
/// contiguous dimension, as an `image` crate buffer lays its pixels out.
struct Tensor(NonNull<sys::sd_tensor_t>);

impl Tensor {
    /// `image` copied in, `None` when sd.cpp couldn't.
    fn new(image: Image) -> Option<Self> {
        let shape = [
            i64::from(image.channels),
            i64::from(image.width),
            i64::from(image.height),
        ];
        NonNull::new(unsafe {
            sys::sd_tensor_create(
                sys::SD_TENSOR_U8 as sys::sd_tensor_type_t,
                shape.len(),
                shape.as_ptr(),
                image.pixels.as_ptr().cast(),
                image.pixels.len(),
            )
        })
        .map(Self)
    }
}

impl Drop for Tensor {
    fn drop(&mut self) {
        unsafe { sys::sd_tensor_free(self.0.as_ptr()) };
    }
}
