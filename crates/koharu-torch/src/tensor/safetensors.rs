//! Reading `.safetensors` files.

use std::{fs::File, path::Path};

use safetensors::{Dtype, SafeTensors, tensor::TensorView};

use crate::{Kind, Result, TchError, Tensor};

impl TryFrom<Dtype> for Kind {
    type Error = TchError;

    fn try_from(dtype: Dtype) -> Result<Self> {
        Ok(match dtype {
            Dtype::BOOL => Kind::Bool,
            Dtype::U8 => Kind::Uint8,
            Dtype::I8 => Kind::Int8,
            Dtype::I16 => Kind::Int16,
            Dtype::I32 => Kind::Int,
            Dtype::I64 => Kind::Int64,
            Dtype::BF16 => Kind::BFloat16,
            Dtype::F16 => Kind::Half,
            Dtype::F32 => Kind::Float,
            Dtype::F64 => Kind::Double,
            dtype => return Err(TchError::Convert(format!("unsupported dtype {dtype:?}"))),
        })
    }
}

impl TryFrom<TensorView<'_>> for Tensor {
    type Error = TchError;

    fn try_from(view: TensorView<'_>) -> Result<Self> {
        let size = view.shape().iter().map(|&x| x as i64).collect::<Vec<_>>();
        Tensor::from_data_size(view.data(), &size, view.dtype().try_into()?)
    }
}

/// Maps a `.safetensors` file and hands its tensors to `f` as CPU tensors,
/// each copied once out of the mapping.
pub(crate) fn read<T>(path: &Path, f: impl FnOnce(&SafeTensors) -> Result<T>) -> Result<T> {
    let error = |err| TchError::SafeTensors {
        path: path.display().to_string(),
        err,
    };
    let file = File::open(path)?;
    // The file isn't changed while it's mapped: the runtime's store keeps it
    // read-only once it's pinned and checked.
    let mapping = unsafe { memmap2::Mmap::map(&file) }?;
    let tensors = SafeTensors::deserialize(&mapping).map_err(error)?;
    f(&tensors)
}
