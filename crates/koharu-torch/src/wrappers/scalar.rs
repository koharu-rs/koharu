//! Scalar elements.

use crate::Result;

/// A single scalar value.
pub struct Scalar {
    pub(crate) c_scalar: *mut koharu_torch_sys::C_scalar,
}

impl Scalar {
    pub fn int(v: i64) -> Result<Scalar> {
        let c_scalar = unsafe_torch_err!(koharu_torch_sys::ats_int(v));
        Ok(Scalar { c_scalar })
    }

    pub fn float(v: f64) -> Result<Scalar> {
        let c_scalar = unsafe_torch_err!(koharu_torch_sys::ats_float(v));
        Ok(Scalar { c_scalar })
    }

    pub fn to_int(&self) -> Result<i64> {
        Ok(unsafe_torch_err!(koharu_torch_sys::ats_to_int(
            self.c_scalar
        )))
    }

    pub fn to_float(&self) -> Result<f64> {
        Ok(unsafe_torch_err!(koharu_torch_sys::ats_to_float(
            self.c_scalar
        )))
    }
}

impl std::fmt::Debug for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.to_float() {
            Ok(value) => write!(f, "scalar<{value}>"),
            Err(_) => write!(f, "scalar<?>"),
        }
    }
}

impl Drop for Scalar {
    fn drop(&mut self) {
        unsafe { koharu_torch_sys::ats_free(self.c_scalar) }
    }
}
