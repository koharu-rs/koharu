//! Shapes, views, and conversions on top of the generated ops.

mod convert;
mod index;
mod ops;
mod safetensors;

pub use index::{IndexOp, NewAxis, TensorIndexer};
pub(crate) use safetensors::read as read_safetensors;

use crate::{Device, Kind, Result, Tensor};

pub trait Shape {
    fn to_shape(&self) -> Box<[i64]>;
}

macro_rules! impl_shape {
    ($v:expr) => {
        impl Shape for [i64; $v] {
            fn to_shape(&self) -> Box<[i64]> {
                Box::new(*self)
            }
        }
    };
}

impl_shape!(0);
impl_shape!(1);
impl_shape!(2);
impl_shape!(3);
impl_shape!(4);
impl_shape!(5);
impl_shape!(6);

impl Shape for () {
    fn to_shape(&self) -> Box<[i64]> {
        Box::new([])
    }
}

impl Shape for &[i64] {
    fn to_shape(&self) -> Box<[i64]> {
        (*self).into()
    }
}

impl Shape for i64 {
    fn to_shape(&self) -> Box<[i64]> {
        Box::new([*self])
    }
}

impl Shape for (i64, i64) {
    fn to_shape(&self) -> Box<[i64]> {
        Box::new([self.0, self.1])
    }
}

impl Shape for (i64, i64, i64) {
    fn to_shape(&self) -> Box<[i64]> {
        Box::new([self.0, self.1, self.2])
    }
}

impl Shape for (i64, i64, i64, i64) {
    fn to_shape(&self) -> Box<[i64]> {
        Box::new([self.0, self.1, self.2, self.3])
    }
}

impl Tensor {
    pub fn view<T: Shape>(&self, s: T) -> Result<Tensor> {
        self.view_(&s.to_shape())
    }

    pub fn to_kind(&self, kind: Kind) -> Result<Tensor> {
        self.totype(kind)
    }

    pub fn to_device(&self, device: Device) -> Result<Tensor> {
        self.to(device)
    }

    /// A zero-dimensional CPU tensor holding `v`.
    pub fn scalar<T: crate::kind::Element>(v: T) -> Result<Tensor> {
        Tensor::from_slice(&[v])?.view(())
    }

    /// A two-dimensional CPU tensor holding the rows of `v`.
    pub fn from_slice2<T, U>(v: &[U]) -> Result<Tensor>
    where
        T: crate::kind::Element,
        U: AsRef<[T]>,
    {
        let rows = v
            .iter()
            .map(|row| Tensor::from_slice(row.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        Tensor::stack(&rows, 0)
    }
}
