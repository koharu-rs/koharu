use core::fmt;
use std::ptr::NonNull;

/// A wrapper around [`NonNull`] for ensuring C++ `const`-rules are upheld.
#[repr(transparent)]
pub(crate) struct Ptr<T: ?Sized>(NonNull<T>);

impl<T: ?Sized> Ptr<T> {
    #[inline]
    pub(crate) fn new(ptr: *mut T) -> Option<Self> {
        NonNull::new(ptr).map(Self)
    }

    #[inline]
    pub(crate) const fn as_ptr(&self) -> *const T {
        self.0.as_ptr().cast_const()
    }

    #[inline]
    pub(crate) const fn as_mut_ptr(&mut self) -> *mut T {
        self.0.as_ptr()
    }

    /// Temporary helper to allow getting a mutable pointer from an immutable
    /// reference.
    /// FIXME(madsmtm): Get rid of this!
    #[inline]
    pub(crate) const unsafe fn as_mut_ptr_unsound(&self) -> *mut T {
        self.0.as_ptr()
    }
}

impl<T: ?Sized> fmt::Debug for Ptr<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Forward to inner Debug
        fmt::Debug::fmt(&self.0, f)
    }
}
