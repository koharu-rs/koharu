//! Copies out of tensors into Rust values.

use half::{bf16, f16};

use crate::{Device, Result, TchError, Tensor, kind::Element};

impl<T: Element + Copy> TryFrom<&Tensor> for Vec<T> {
    type Error = TchError;

    fn try_from(tensor: &Tensor) -> Result<Self> {
        let numel = tensor.size1()? as usize;
        let mut vec = vec![T::ZERO; numel];
        tensor.to_kind(T::KIND)?.copy_data(&mut vec, numel)?;
        Ok(vec)
    }
}

impl<T: Element + Copy> TryFrom<&Tensor> for Vec<Vec<T>> {
    type Error = TchError;

    fn try_from(tensor: &Tensor) -> Result<Self> {
        let (rows, columns) = tensor.size2()?;
        let (rows, columns) = (rows as usize, columns as usize);
        let mut all = vec![T::ZERO; rows * columns];
        tensor
            .to_kind(T::KIND)?
            .copy_data(&mut all, rows * columns)?;
        Ok(all
            .chunks(columns.max(1))
            .take(rows)
            .map(<[T]>::to_vec)
            .collect())
    }
}

impl<T: Element + Copy> TryFrom<Tensor> for Vec<T> {
    type Error = TchError;

    fn try_from(tensor: Tensor) -> Result<Self> {
        Vec::<T>::try_from(&tensor)
    }
}

impl<T: Element + Copy> TryFrom<Tensor> for Vec<Vec<T>> {
    type Error = TchError;

    fn try_from(tensor: Tensor) -> Result<Self> {
        Vec::<Vec<T>>::try_from(&tensor)
    }
}

macro_rules! from_tensor {
    ($typ:ident) => {
        impl TryFrom<&Tensor> for $typ {
            type Error = TchError;

            fn try_from(tensor: &Tensor) -> Result<Self> {
                let numel = tensor.numel();
                if numel != 1 {
                    return Err(TchError::Convert(format!(
                        "expected exactly one element, got {numel}"
                    )));
                }
                let mut value = [$typ::ZERO; 1];
                tensor
                    .to_device(Device::Cpu)?
                    .to_kind($typ::KIND)?
                    .copy_data(&mut value, numel)?;
                Ok(value[0])
            }
        }

        impl TryFrom<Tensor> for $typ {
            type Error = TchError;

            fn try_from(tensor: Tensor) -> Result<Self> {
                $typ::try_from(&tensor)
            }
        }
    };
}

from_tensor!(f64);
from_tensor!(f32);
from_tensor!(f16);
from_tensor!(bf16);
from_tensor!(i64);
from_tensor!(i32);
from_tensor!(i16);
from_tensor!(i8);
from_tensor!(u8);
from_tensor!(bool);
