//! Key-value overrides for a model.

use crate::model::params::LlamaModelParams;
use std::ffi::{CStr, CString};
use std::fmt::Debug;

/// An override value for a model parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamOverrideValue {
    /// A string value
    Bool(bool),
    /// A float value
    Float(f64),
    /// A integer value
    Int(i64),
    /// A string value
    Str([std::os::raw::c_char; 128]),
}

impl ParamOverrideValue {
    pub(crate) fn tag(&self) -> koharu_llama_sys::llama_model_kv_override_type {
        match self {
            ParamOverrideValue::Bool(_) => koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_BOOL,
            ParamOverrideValue::Float(_) => koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_FLOAT,
            ParamOverrideValue::Int(_) => koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_INT,
            ParamOverrideValue::Str(_) => koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_STR,
        }
    }

    pub(crate) fn value(&self) -> koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 {
        match self {
            ParamOverrideValue::Bool(value) => {
                koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 { val_bool: *value }
            }
            ParamOverrideValue::Float(value) => {
                koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 { val_f64: *value }
            }
            ParamOverrideValue::Int(value) => {
                koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 { val_i64: *value }
            }
            ParamOverrideValue::Str(c_string) => {
                koharu_llama_sys::llama_model_kv_override__bindgen_ty_1 { val_str: *c_string }
            }
        }
    }
}

impl From<&koharu_llama_sys::llama_model_kv_override> for ParamOverrideValue {
    fn from(
        koharu_llama_sys::llama_model_kv_override {
            key: _,
            tag,
            __bindgen_anon_1,
        }: &koharu_llama_sys::llama_model_kv_override,
    ) -> Self {
        match *tag {
            koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_INT => {
                ParamOverrideValue::Int(unsafe { __bindgen_anon_1.val_i64 })
            }
            koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_FLOAT => {
                ParamOverrideValue::Float(unsafe { __bindgen_anon_1.val_f64 })
            }
            koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_BOOL => {
                ParamOverrideValue::Bool(unsafe { __bindgen_anon_1.val_bool })
            }
            koharu_llama_sys::LLAMA_KV_OVERRIDE_TYPE_STR => {
                ParamOverrideValue::Str(unsafe { __bindgen_anon_1.val_str })
            }
            _ => unreachable!("Unknown tag of {tag}"),
        }
    }
}

/// A struct implementing [`IntoIterator`] over the key-value overrides for a model.
#[derive(Debug)]
pub struct KvOverrides<'a> {
    model_params: &'a LlamaModelParams,
}

impl KvOverrides<'_> {
    pub(super) fn new<'a>(model_params: &'a LlamaModelParams) -> KvOverrides<'a> {
        KvOverrides { model_params }
    }
}

impl<'a> IntoIterator for KvOverrides<'a> {
    // I'm fairly certain this could be written returning by reference, but I'm not sure how to do it safely. I do not
    // expect this to be a performance bottleneck so the copy should be fine. (let me know if it's not fine!)
    type Item = (CString, ParamOverrideValue);
    type IntoIter = KvOverrideValueIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        KvOverrideValueIterator {
            model_params: self.model_params,
            current: 0,
        }
    }
}

/// An iterator over the key-value overrides for a model.
#[derive(Debug)]
pub struct KvOverrideValueIterator<'a> {
    model_params: &'a LlamaModelParams,
    current: usize,
}

impl Iterator for KvOverrideValueIterator<'_> {
    type Item = (CString, ParamOverrideValue);

    fn next(&mut self) -> Option<Self::Item> {
        let overrides = self.model_params.params.kv_overrides;
        if overrides.is_null() {
            return None;
        }

        // SAFETY: llama.cpp seems to guarantee that the last element contains an empty key or is valid. We've checked
        // the prev one in the last iteration, the next one should be valid or 0 (and thus safe to deref)
        let current = unsafe { *overrides.add(self.current) };

        if current.key[0] == 0 {
            return None;
        }

        let value = ParamOverrideValue::from(&current);

        let key = unsafe { CStr::from_ptr(current.key.as_ptr()).to_owned() };

        self.current += 1;
        Some((key, value))
    }
}
