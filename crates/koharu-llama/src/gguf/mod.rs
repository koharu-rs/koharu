//! Safe wrapper around `gguf_context` for reading GGUF file metadata.
//!
//! Provides metadata-only access to GGUF files without loading tensor data.
//! Useful for inspecting model architecture parameters before loading a model.

use std::ffi::{CStr, CString};
use std::path::Path;

use crate::ptr::Ptr;

/// A safe wrapper around `gguf_context`.
///
/// Opens a GGUF file and parses only the metadata header; tensor weights are
/// never loaded into memory (`no_alloc = true`).
#[derive(Debug)]
pub struct GgufContext {
    ctx: Ptr<koharu_llama_sys::gguf_context>,
}

// SAFETY: `gguf_context` is mostly POD. Getters use `&`, setters use `&mut`.
unsafe impl Send for GgufContext {}
// SAFETY: Same as above.
unsafe impl Sync for GgufContext {}

impl GgufContext {
    /// Open a GGUF file and parse its metadata header.
    ///
    /// Returns `None` if the path contains a null byte, the file does not
    /// exist, or the file is not a valid GGUF file.
    pub fn from_file(path: &Path) -> Option<Self> {
        let c_path = CString::new(path.to_str()?).ok()?;
        let params = koharu_llama_sys::gguf_init_params {
            no_alloc: true,
            ctx: std::ptr::null_mut(),
        };
        let ptr = unsafe { koharu_llama_sys::gguf_init_from_file(c_path.as_ptr(), params) };
        Some(Self {
            ctx: Ptr::new(ptr)?,
        })
    }

    /// Total number of key-value pairs in the metadata.
    pub fn n_kv(&self) -> i64 {
        unsafe { koharu_llama_sys::gguf_get_n_kv(self.ctx.as_ptr()) }
    }

    /// Find the index of a key by name. Returns `-1` if not found.
    pub fn find_key(&self, key: &str) -> i64 {
        let Ok(c_key) = CString::new(key) else {
            return -1;
        };
        unsafe { koharu_llama_sys::gguf_find_key(self.ctx.as_ptr(), c_key.as_ptr()) }
    }

    /// Return the key name at the given index, or `None` if out of range.
    pub fn key_at(&self, idx: i64) -> Option<&str> {
        let ptr = unsafe { koharu_llama_sys::gguf_get_key(self.ctx.as_ptr(), idx) };
        if ptr.is_null() {
            return None;
        }
        unsafe { CStr::from_ptr(ptr).to_str().ok() }
    }

    /// Return the value type of the KV pair at `idx`.
    pub fn kv_type(&self, idx: i64) -> koharu_llama_sys::gguf_type {
        unsafe { koharu_llama_sys::gguf_get_kv_type(self.ctx.as_ptr(), idx) }
    }

    /// Read a `uint32` value. Panics (inside llama.cpp) if the stored type is
    /// not `GGUF_TYPE_UINT32` — check `kv_type` first if unsure.
    pub fn val_u32(&self, idx: i64) -> u32 {
        unsafe { koharu_llama_sys::gguf_get_val_u32(self.ctx.as_ptr(), idx) }
    }

    /// Read an `int32` value.
    pub fn val_i32(&self, idx: i64) -> i32 {
        unsafe { koharu_llama_sys::gguf_get_val_i32(self.ctx.as_ptr(), idx) }
    }

    /// Read a `uint64` value.
    pub fn val_u64(&self, idx: i64) -> u64 {
        unsafe { koharu_llama_sys::gguf_get_val_u64(self.ctx.as_ptr(), idx) }
    }

    /// Read a string value. Returns `None` if the pointer is null or not
    /// valid UTF-8.
    pub fn val_str(&self, idx: i64) -> Option<&str> {
        let ptr = unsafe { koharu_llama_sys::gguf_get_val_str(self.ctx.as_ptr(), idx) };
        if ptr.is_null() {
            return None;
        }
        unsafe { CStr::from_ptr(ptr).to_str().ok() }
    }

    /// Total number of tensors described in the file.
    pub fn n_tensors(&self) -> i64 {
        unsafe { koharu_llama_sys::gguf_get_n_tensors(self.ctx.as_ptr()) }
    }
}

impl Drop for GgufContext {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::gguf_free(self.ctx.as_mut_ptr()) }
    }
}
