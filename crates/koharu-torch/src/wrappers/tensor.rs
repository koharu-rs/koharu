use std::ffi::c_void;

use koharu_torch_sys::*;
use libc::c_int;

use super::utils::ptr_to_string;
use super::{device::Device, kind, kind::Kind};
use crate::{Result, TchError};

/// A tensor libtorch holds. Every one is defined: nothing makes an empty
/// handle.
#[must_use]
pub struct Tensor {
    pub(crate) c_tensor: *mut C_tensor,
}

// A tensor may move between threads. Two `Tensor`s never share one libtorch
// tensor, so nothing races on its metadata; a view shares only storage.
unsafe impl Send for Tensor {}

impl Tensor {
    /// Takes ownership of a tensor the shim allocated.
    ///
    /// # Safety
    ///
    /// `c_tensor` points to a live tensor that nothing else frees.
    pub unsafe fn from_ptr(c_tensor: *mut C_tensor) -> Self {
        Self { c_tensor }
    }

    pub fn as_ptr(&self) -> *const C_tensor {
        self.c_tensor
    }

    pub fn dim(&self) -> usize {
        unsafe_torch!(at_dim(self.c_tensor))
    }

    pub fn size(&self) -> Vec<i64> {
        let mut size = vec![0i64; self.dim()];
        unsafe_torch!(at_shape(self.c_tensor, size.as_mut_ptr()));
        size
    }

    pub fn size1(&self) -> Result<i64> {
        match self.size().as_slice() {
            &[s0] => Ok(s0),
            size => Err(TchError::Shape(format!("expected one dim, got {size:?}"))),
        }
    }

    pub fn size2(&self) -> Result<(i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1] => Ok((s0, s1)),
            size => Err(TchError::Shape(format!("expected two dims, got {size:?}"))),
        }
    }

    pub fn size3(&self) -> Result<(i64, i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1, s2] => Ok((s0, s1, s2)),
            size => Err(TchError::Shape(format!(
                "expected three dims, got {size:?}"
            ))),
        }
    }

    pub fn size4(&self) -> Result<(i64, i64, i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1, s2, s3] => Ok((s0, s1, s2, s3)),
            size => Err(TchError::Shape(format!("expected four dims, got {size:?}"))),
        }
    }

    pub fn size5(&self) -> Result<(i64, i64, i64, i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1, s2, s3, s4] => Ok((s0, s1, s2, s3, s4)),
            size => Err(TchError::Shape(format!("expected five dims, got {size:?}"))),
        }
    }

    pub fn stride(&self) -> Vec<i64> {
        let mut stride = vec![0i64; self.dim()];
        unsafe_torch!(at_stride(self.c_tensor, stride.as_mut_ptr()));
        stride
    }

    /// The kind of the tensor's elements.
    pub fn kind(&self) -> Kind {
        match Kind::from_c_int(unsafe_torch!(at_scalar_type(self.c_tensor))) {
            Ok(kind) => kind,
            Err(error) => panic!("{error}"),
        }
    }

    pub fn device(&self) -> Device {
        Device::from_c_int(unsafe_torch!(at_device(self.c_tensor)))
    }

    pub fn numel(&self) -> usize {
        self.size().iter().product::<i64>() as usize
    }

    pub fn is_contiguous(&self) -> bool {
        unsafe_torch!(at_is_contiguous(self.c_tensor)) != 0
    }

    /// The address of the first element.
    pub fn data_ptr(&self) -> *mut c_void {
        unsafe_torch!(at_data_ptr(self.c_tensor))
    }

    /// The element at `index` as a double; the tensor's element there must
    /// be a single value.
    pub fn double_value(&self, index: &[i64]) -> Result<f64> {
        Ok(unsafe_torch_err!(at_double_value_at_indexes(
            self.c_tensor,
            index.as_ptr(),
            index.len() as i32
        )))
    }

    /// The element at `index` as an integer.
    pub fn int64_value(&self, index: &[i64]) -> Result<i64> {
        Ok(unsafe_torch_err!(at_int64_value_at_indexes(
            self.c_tensor,
            index.as_ptr(),
            index.len() as i32
        )))
    }

    /// Copies `numel` elements into `dst`, from whatever device the tensor
    /// is on.
    pub fn copy_data<T: kind::Element>(&self, dst: &mut [T], numel: usize) -> Result<()> {
        if T::KIND != self.kind() {
            return Err(TchError::Kind(format!(
                "a {:?} tensor can't be copied into {:?} elements",
                self.kind(),
                T::KIND
            )));
        }
        if dst.len() < numel {
            return Err(TchError::Shape(format!("slice len < {numel}")));
        }
        unsafe_torch_err!(at_copy_data(
            self.c_tensor,
            dst.as_mut_ptr().cast(),
            numel,
            T::KIND.elt_size_in_bytes(),
        ));
        Ok(())
    }

    /// A one-dimensional CPU tensor holding a copy of `data`.
    pub fn from_slice<T: kind::Element>(data: &[T]) -> Result<Tensor> {
        let c_tensor = unsafe_torch_err!(at_tensor_of_data(
            data.as_ptr().cast(),
            [data.len() as i64].as_ptr(),
            1,
            T::KIND.elt_size_in_bytes(),
            T::KIND.c_int(),
        ));
        Ok(Tensor { c_tensor })
    }

    /// A CPU tensor of `kind` and `size` holding a copy of `data`.
    pub fn from_data_size(data: &[u8], size: &[i64], kind: Kind) -> Result<Tensor> {
        let c_tensor = unsafe_torch_err!(at_tensor_of_data(
            data.as_ptr().cast(),
            size.as_ptr(),
            size.len(),
            kind.elt_size_in_bytes(),
            kind.c_int(),
        ));
        Ok(Tensor { c_tensor })
    }

    /// A tensor over `data` without copying it; empty `strides` are the
    /// default ones.
    ///
    /// # Safety
    ///
    /// `data` holds initialised elements of `kind` for `size` and `strides`
    /// on `device`, and outlives the tensor and every view of it.
    pub unsafe fn from_blob(
        data: *const u8,
        size: &[i64],
        strides: &[i64],
        kind: Kind,
        device: Device,
    ) -> Result<Tensor> {
        let c_tensor = unsafe_torch_err!(at_tensor_of_blob(
            data.cast(),
            size.as_ptr(),
            size.len(),
            strides.as_ptr(),
            strides.len(),
            kind.c_int(),
            device.c_int()
        ));
        Ok(Tensor { c_tensor })
    }

    /// The sub-tensor at `index` along the first dimension.
    pub fn get(&self, index: i64) -> Result<Tensor> {
        let c_tensor = unsafe_torch_err!(at_get(self.c_tensor, index as c_int));
        Ok(Tensor { c_tensor })
    }

    /// Copies `src` into this tensor, converting its kind and device.
    pub fn copy_(&mut self, src: &Tensor) -> Result<()> {
        unsafe_torch_err!(at_copy_(self.c_tensor, src.c_tensor));
        Ok(())
    }

    /// libtorch's printout of every element, which can be large.
    pub fn to_string(&self, line_width: i64) -> Result<String> {
        let printed = unsafe_torch_err!(ptr_to_string(at_to_string(
            self.c_tensor,
            line_width as c_int
        )));
        printed.ok_or_else(|| TchError::Convert("libtorch printed nothing".into()))
    }
}

impl std::fmt::Debug for Tensor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Tensor[{:?}, {:?}, {:?}]",
            self.size(),
            self.kind(),
            self.device()
        )
    }
}

impl Drop for Tensor {
    fn drop(&mut self) {
        unsafe { at_free(self.c_tensor) }
    }
}

impl AsRef<Tensor> for Tensor {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Runs `f` without tracking gradients, and tracks them again afterwards,
/// even if `f` panics.
pub fn no_grad<T>(f: impl FnOnce() -> T) -> T {
    let _guard = no_grad_guard();
    f()
}

/// Stops tracking gradients until it's dropped. Bind it to a name such as
/// `_guard`, as `_` drops it at once.
pub struct NoGradGuard {
    enabled: bool,
}

pub fn no_grad_guard() -> NoGradGuard {
    NoGradGuard {
        enabled: grad_set_enabled(false),
    }
}

impl Drop for NoGradGuard {
    fn drop(&mut self) {
        grad_set_enabled(self.enabled);
    }
}

fn grad_set_enabled(enabled: bool) -> bool {
    unsafe_torch!(at_grad_set_enabled(i32::from(enabled))) != 0
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Reduction {
    None,
    Mean,
    Sum,
}

impl Reduction {
    // In step with ATen/core/Reduction.h.
    pub fn to_int(self) -> i64 {
        match self {
            Reduction::None => 0,
            Reduction::Mean => 1,
            Reduction::Sum => 2,
        }
    }
}
