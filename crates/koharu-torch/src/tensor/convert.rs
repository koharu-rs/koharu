//! Implement conversion traits for tensors
use super::Tensor;
use crate::{Device, Error, kind::Element};
use half::{bf16, f16};
use std::convert::TryFrom;

impl<T: Element + Copy> TryFrom<&Tensor> for Vec<T> {
    type Error = Error;
    fn try_from(tensor: &Tensor) -> Result<Self, Self::Error> {
        let size = tensor.size();
        if size.len() != 1 {
            Err(Error::Convert(format!(
                "Attempting to convert a Tensor with {} dimensions to flat vector",
                size.len()
            )))?;
        }
        let numel = size[0] as usize;
        let mut vec = vec![T::ZERO; numel];
        tensor.to_kind(T::KIND)?.copy_data(&mut vec, numel)?;
        Ok(vec)
    }
}

impl<T: Element + Copy> TryFrom<Tensor> for Vec<T> {
    type Error = Error;
    fn try_from(tensor: Tensor) -> Result<Self, Self::Error> {
        Vec::<T>::try_from(&tensor)
    }
}

/// One element of a tensor that holds exactly one.
fn element<T: Element + Copy>(tensor: &Tensor) -> Result<T, Error> {
    let numel = tensor.numel()?;
    if numel != 1 {
        return Err(Error::Convert(format!(
            "expected exactly one element, got {numel}"
        )));
    }
    let mut vec = [T::ZERO; 1];
    tensor
        .to(Device::Cpu)?
        .to_kind(T::KIND)?
        .copy_data(&mut vec, 1)?;
    Ok(vec[0])
}

macro_rules! from_tensor {
    ($typ:ident) => {
        impl TryFrom<&Tensor> for $typ {
            type Error = Error;

            fn try_from(tensor: &Tensor) -> Result<Self, Self::Error> {
                element(tensor)
            }
        }

        impl TryFrom<Tensor> for $typ {
            type Error = Error;

            fn try_from(tensor: Tensor) -> Result<Self, Self::Error> {
                element(&tensor)
            }
        }
    };
}

from_tensor!(f64);
from_tensor!(f32);
from_tensor!(f16);
from_tensor!(i64);
from_tensor!(i32);
from_tensor!(i16);
from_tensor!(i8);
from_tensor!(u8);
from_tensor!(bool);
from_tensor!(bf16);

impl<T: Element> TryFrom<&Vec<T>> for Tensor {
    type Error = Error;

    fn try_from(value: &Vec<T>) -> Result<Self, Self::Error> {
        Self::from_slice(value.as_slice())
    }
}

impl<T: Element> TryFrom<Vec<T>> for Tensor {
    type Error = Error;

    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}
