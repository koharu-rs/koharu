//! libtorch's tensor layout, memory format, and quantization scheme, which
//! cross the C API as their c10 values.

use crate::{Result, TchError};

/// A tensor layout: c10::Layout.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Layout {
    Strided,
    Sparse,
    SparseCsr,
    Mkldnn,
    SparseCsc,
    SparseBsr,
    SparseBsc,
    Jagged,
}

impl Layout {
    pub(crate) fn c_int(self) -> i32 {
        self as i32
    }
}

/// How a tensor's elements are laid out in memory: c10::MemoryFormat.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum MemoryFormat {
    Contiguous,
    Preserve,
    ChannelsLast,
    ChannelsLast3d,
}

impl MemoryFormat {
    pub(crate) fn c_int(self) -> i32 {
        self as i32
    }
}

/// A quantized tensor's scheme: c10::QScheme.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum QScheme {
    PerTensorAffine,
    PerChannelAffine,
    PerTensorSymmetric,
    PerChannelSymmetric,
    PerChannelAffineFloatQParams,
}

impl QScheme {
    pub(crate) fn from_c_int(v: i32) -> Result<QScheme> {
        Ok(match v {
            0 => QScheme::PerTensorAffine,
            1 => QScheme::PerChannelAffine,
            2 => QScheme::PerTensorSymmetric,
            3 => QScheme::PerChannelSymmetric,
            4 => QScheme::PerChannelAffineFloatQParams,
            _ => {
                return Err(TchError::Convert(format!(
                    "unknown quantization scheme {v}"
                )))
            }
        })
    }
}
