use std::ffi::c_void;

use koharu_torch_sys::*;

use super::{
    device::Device,
    kind::{self, Kind},
    utils::check,
};
use crate::{Error, Result};

/// A tensor: an owned libtorch AtenTensorHandle.
#[must_use]
pub struct Tensor {
    pub(crate) c_tensor: AtenTensorHandle,
}

// A handle owns a reference to an at::Tensor, which any thread may use.
unsafe impl Send for Tensor {}

impl Tensor {
    /// Takes the handles in an array the shim returned, and frees the array.
    pub(crate) unsafe fn from_array(array: *mut AtenTensorHandle, len: i64) -> Vec<Tensor> {
        let tensors = unsafe { std::slice::from_raw_parts(array, len as usize) }
            .iter()
            .map(|&c_tensor| Tensor { c_tensor })
            .collect();
        unsafe { aten_free(array.cast()) };
        tensors
    }

    /// Returns the shape of the input tensor.
    pub fn size(&self) -> Vec<i64> {
        let mut dim = 0;
        let mut sizes = std::ptr::null_mut();
        check(unsafe { aoti_torch_get_dim(self.c_tensor, &mut dim) })
            .expect("a tensor has a shape");
        check(unsafe { aoti_torch_get_sizes(self.c_tensor, &mut sizes) })
            .expect("a tensor has a shape");
        unsafe { std::slice::from_raw_parts(sizes, dim as usize) }.to_vec()
    }

    /// Returns the tensor sizes for two dimension tensors.
    pub fn size2(&self) -> Result<(i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1] => Ok((s0, s1)),
            size => Err(Error::Shape(format!("expected two dims, got {size:?}"))),
        }
    }

    /// Returns the tensor sizes for three dimension tensors.
    pub fn size3(&self) -> Result<(i64, i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1, s2] => Ok((s0, s1, s2)),
            size => Err(Error::Shape(format!("expected three dims, got {size:?}"))),
        }
    }

    /// Returns the tensor sizes for four dimension tensors.
    pub fn size4(&self) -> Result<(i64, i64, i64, i64)> {
        match self.size().as_slice() {
            &[s0, s1, s2, s3] => Ok((s0, s1, s2, s3)),
            size => Err(Error::Shape(format!("expected four dims, got {size:?}"))),
        }
    }

    /// Returns the stride of the input tensor.
    pub fn stride(&self) -> Vec<i64> {
        let mut dim = 0;
        let mut strides = std::ptr::null_mut();
        check(unsafe { aoti_torch_get_dim(self.c_tensor, &mut dim) })
            .expect("a tensor has a shape");
        check(unsafe { aoti_torch_get_strides(self.c_tensor, &mut strides) })
            .expect("a tensor has strides");
        unsafe { std::slice::from_raw_parts(strides, dim as usize) }.to_vec()
    }

    /// Returns the kind of elements stored in the input tensor. Returns
    /// an error on undefined tensors and unsupported data types.
    pub fn f_kind(&self) -> Result<Kind> {
        let mut dtype = 0;
        check(unsafe { aoti_torch_get_dtype(self.c_tensor, &mut dtype) })?;
        Kind::from_c_int(dtype)
    }

    /// Returns the kind of elements stored in the input tensor. Panics
    /// an error on undefined tensors and unsupported data types.
    pub fn kind(&self) -> Kind {
        self.f_kind().unwrap()
    }

    /// Returns the device on which the input tensor is located.
    pub fn device(&self) -> Device {
        let (mut device_type, mut index) = (0, 0);
        check(unsafe { aoti_torch_get_device_type(self.c_tensor, &mut device_type) })
            .and_then(|()| check(unsafe { aoti_torch_get_device_index(self.c_tensor, &mut index) }))
            .and_then(|()| Device::from_c_parts(device_type, index))
            .unwrap()
    }

    /// Returns the address of the first element of this tensor.
    pub fn data_ptr(&self) -> *mut c_void {
        let mut data = std::ptr::null_mut();
        check(unsafe { aoti_torch_get_data_ptr(self.c_tensor, &mut data) })
            .expect("a tensor has data");
        data
    }

    /// Returns a new tensor that share storage with the input tensor.
    pub fn shallow_clone(&self) -> Tensor {
        let mut c_tensor = std::ptr::null_mut();
        check(unsafe { aoti_torch_new_tensor_handle(self.c_tensor, &mut c_tensor) })
            .expect("a handle can be copied");
        Tensor { c_tensor }
    }

    /// Converts a slice to a tensor.
    pub fn from_slice<T: kind::Element>(data: &[T]) -> Result<Tensor> {
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data))
        };
        Tensor::from_data_size(bytes, &[data.len() as i64], T::KIND)
    }

    /// Converts some byte data to a tensor with some specified kind and shape.
    pub fn from_data_size(data: &[u8], size: &[i64], kind: Kind) -> Result<Tensor> {
        let numel = size.iter().product::<i64>() as usize;
        if data.len() != numel * kind.elt_size_in_bytes() {
            return Err(Error::Shape(format!(
                "{} bytes for {size:?} elements of {kind:?}",
                data.len()
            )));
        }
        // A copy into a new tensor, which libtorch allocates aligned.
        let tensor = Tensor::empty_memory_format(size, (kind, Device::Cpu), None)?;
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), tensor.data_ptr().cast(), data.len())
        };
        Ok(tensor)
    }

    /// Copies `numel` elements from `self` to `dst`.
    pub fn copy_data<T: kind::Element>(&self, dst: &mut [T], numel: usize) -> Result<()> {
        if T::KIND != self.f_kind()? {
            return Err(Error::Kind(format!(
                "incoherent elt kind, {:?} != {:?}",
                self.f_kind(),
                T::KIND
            )));
        }
        if dst.len() < numel {
            return Err(Error::Shape(format!("slice len < {numel}")));
        }
        let host = self
            .to(Device::Cpu)?
            .contiguous(crate::MemoryFormat::Contiguous)?;
        if (host.numel()? as usize) < numel {
            return Err(Error::Shape(format!("tensor numel < {numel}")));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(host.data_ptr().cast::<T>(), dst.as_mut_ptr(), numel)
        };
        Ok(())
    }

    /// Moves a tensor to a specified device.
    pub fn to(&self, device: Device) -> Result<Tensor> {
        self.to_device(device, self.f_kind()?, false, false, None)
    }

    /// Casts a tensor to a specified kind.
    pub fn to_kind(&self, kind: Kind) -> Result<Tensor> {
        self.to_dtype(kind, false, false, None)
    }
}

impl Drop for Tensor {
    fn drop(&mut self) {
        unsafe { aoti_torch_delete_tensor_object(self.c_tensor) };
    }
}

impl std::fmt::Debug for Tensor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Tensor[{:?}, {:?}, {:?}]",
            self.size(),
            self.f_kind(),
            self.device()
        )
    }
}

/// libtorch's printing of every element, as C++'s `operator<<` writes it.
impl std::fmt::Display for Tensor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut string = std::ptr::null_mut();
        check(unsafe { aten_tensor_to_string(self.c_tensor, &mut string) })
            .map_err(|_| std::fmt::Error)?;
        let (mut data, mut len) = (std::ptr::null(), 0);
        let read = check(unsafe { torch_string_c_str(string, &mut data) })
            .and_then(|()| check(unsafe { torch_string_length(string, &mut len) }))
            .map(|()| unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) });
        let written = match read {
            Ok(bytes) => f.write_str(&String::from_utf8_lossy(bytes)),
            Err(_) => Err(std::fmt::Error),
        };
        unsafe { torch_delete_string(string) };
        written
    }
}

impl AsRef<Tensor> for Tensor {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Runs a closure in libtorch's inference mode.
pub fn inference_mode<T>(f: impl FnOnce() -> T) -> T {
    let _guard = InferenceMode::enter();
    f()
}

/// c10::InferenceMode on the calling thread until dropped: no gradients, and
/// none of autograd's per-operator bookkeeping. A tensor made in it can't be
/// changed in place outside it.
#[must_use]
pub struct InferenceMode {
    // c10 restores the previous mode when a guard is deleted on the thread that
    // made it, so the raw pointer also keeps the guard on that thread.
    guard: AtenInferenceModeHandle,
}

impl InferenceMode {
    pub fn enter() -> Self {
        let mut guard = std::ptr::null_mut();
        check(unsafe { aten_create_inference_mode(&mut guard) })
            .expect("libtorch enters inference mode");
        Self { guard }
    }
}

impl Drop for InferenceMode {
    fn drop(&mut self) {
        check(unsafe { aten_delete_inference_mode(self.guard) })
            .expect("libtorch leaves inference mode");
    }
}
