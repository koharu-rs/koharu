use std::ffi::CStr;

use koharu_torch_sys::{
    torch_exception_get_what_without_backtrace, AOTITorchError, AOTI_TORCH_SUCCESS,
};

use crate::{Result, TchError};

/// Turns a libtorch C function's error code into the message of the
/// exception it caught on this thread.
pub(crate) fn check(code: AOTITorchError) -> Result<()> {
    if code == AOTI_TORCH_SUCCESS as AOTITorchError {
        return Ok(());
    }
    let what = unsafe { torch_exception_get_what_without_backtrace() };
    Err(TchError::Torch(if what.is_null() {
        format!("libtorch error {code}")
    } else {
        unsafe { CStr::from_ptr(what) }
            .to_string_lossy()
            .into_owned()
    }))
}

/// An operator's `int[]` argument: one integer, or a slice, array, or vector
/// of them.
pub trait IntList {
    fn int_list(&self) -> &[i64];
}

impl IntList for i64 {
    fn int_list(&self) -> &[i64] {
        std::slice::from_ref(self)
    }
}

impl IntList for [i64] {
    fn int_list(&self) -> &[i64] {
        self
    }
}

impl<const N: usize> IntList for [i64; N] {
    fn int_list(&self) -> &[i64] {
        self
    }
}

impl IntList for Vec<i64> {
    fn int_list(&self) -> &[i64] {
        self
    }
}

impl<T: IntList + ?Sized> IntList for &T {
    fn int_list(&self) -> &[i64] {
        (**self).int_list()
    }
}

/// An operator's `int[]?` argument: an `IntList`, or `None`.
pub trait IntListOption {
    fn int_list_option(&self) -> Option<&[i64]>;
}

impl<T: IntList + ?Sized> IntListOption for T {
    fn int_list_option(&self) -> Option<&[i64]> {
        Some(self.int_list())
    }
}

impl<T: IntList> IntListOption for Option<T> {
    fn int_list_option(&self) -> Option<&[i64]> {
        self.as_ref().map(IntList::int_list)
    }
}
