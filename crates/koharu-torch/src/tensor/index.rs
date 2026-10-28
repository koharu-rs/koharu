//! The `i` indexing operation.
//!
//! An integer selects along a dimension (negative counts from the end), a
//! range narrows one, `NewAxis` inserts one, and a list of indices or an
//! index tensor selects along one. Unlike NumPy there's no advanced
//! indexing: each index applies to its own dimension, and the result shares
//! the input's storage.
//!
//! ```ignore
//! let t = tensor.i((.., 1..))?;
//! ```

use std::ops::{
    Bound, Range, RangeBounds, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive,
};

use crate::{Kind, Result, TchError, Tensor};

#[derive(Debug, PartialEq, Eq)]
pub struct NewAxis;

#[derive(Debug)]
pub enum TensorIndexer<'a> {
    Select(i64),
    Narrow(Bound<i64>, Bound<i64>),
    Indices(Vec<i64>),
    IndexSelect(&'a Tensor),
    InsertNewAxis,
}

impl From<NewAxis> for TensorIndexer<'_> {
    fn from(_: NewAxis) -> Self {
        TensorIndexer::InsertNewAxis
    }
}

impl From<i64> for TensorIndexer<'_> {
    fn from(index: i64) -> Self {
        TensorIndexer::Select(index)
    }
}

impl From<&[i64]> for TensorIndexer<'_> {
    fn from(indices: &[i64]) -> Self {
        TensorIndexer::Indices(indices.to_vec())
    }
}

impl From<Vec<i64>> for TensorIndexer<'_> {
    fn from(indices: Vec<i64>) -> Self {
        TensorIndexer::Indices(indices)
    }
}

impl<'a> From<&'a Tensor> for TensorIndexer<'a> {
    fn from(tensor: &'a Tensor) -> Self {
        TensorIndexer::IndexSelect(tensor)
    }
}

macro_rules! impl_from_range {
    ($range_type:ty) => {
        impl From<$range_type> for TensorIndexer<'_> {
            fn from(range: $range_type) -> Self {
                TensorIndexer::Narrow(range.start_bound().cloned(), range.end_bound().cloned())
            }
        }
    };
}

impl_from_range!(Range<i64>);
impl_from_range!(RangeFrom<i64>);
impl_from_range!(RangeFull);
impl_from_range!(RangeInclusive<i64>);
impl_from_range!(RangeTo<i64>);
impl_from_range!(RangeToInclusive<i64>);

pub trait IndexOp<T> {
    fn i(&self, index: T) -> Result<Tensor>;
}

impl<'a, A: Into<TensorIndexer<'a>>> IndexOp<A> for Tensor {
    fn i(&self, index: A) -> Result<Tensor> {
        self.indexer(&[index.into()])
    }
}

macro_rules! impl_index_tuple {
    ($($name:ident $index:tt),+) => {
        impl<'a, $($name: Into<TensorIndexer<'a>>),+> IndexOp<($($name,)+)> for Tensor {
            fn i(&self, index: ($($name,)+)) -> Result<Tensor> {
                self.indexer(&[$(index.$index.into()),+])
            }
        }
    };
}

impl_index_tuple!(A 0);
impl_index_tuple!(A 0, B 1);
impl_index_tuple!(A 0, B 1, C 2);
impl_index_tuple!(A 0, B 1, C 2, D 3);
impl_index_tuple!(A 0, B 1, C 2, D 3, E 4);
impl_index_tuple!(A 0, B 1, C 2, D 3, E 4, F 5);
impl_index_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);

impl Tensor {
    fn indexer(&self, spec: &[TensorIndexer<'_>]) -> Result<Tensor> {
        use TensorIndexer::*;
        use std::ops::Bound::*;

        let new_axes = spec
            .iter()
            .filter(|index| matches!(index, InsertNewAxis))
            .count();
        if spec.len() > self.dim() + new_axes {
            return Err(TchError::Shape(format!(
                "too many indices for a tensor of dimension {}",
                self.dim()
            )));
        }

        // Indices apply from left to right.
        let mut tensor = self.alias()?;
        let mut dim: i64 = 0;
        for index in spec {
            let (next, next_dim) = match index {
                InsertNewAxis => (tensor.unsqueeze(dim)?, dim + 1),
                // `select` drops the dimension, so the next index applies to
                // the same position.
                Select(index) => (tensor.select(dim, *index)?, dim),
                Narrow(start, end) => {
                    let length = || tensor.size()[dim as usize];
                    let range = match (start, end) {
                        (Unbounded, Unbounded) => None,
                        (Included(start), Unbounded) => Some((*start, length() - *start)),
                        (Excluded(start), Unbounded) => Some((*start + 1, length() - *start - 1)),
                        (Unbounded, Included(end)) => Some((0, *end + 1)),
                        (Unbounded, Excluded(end)) => Some((0, *end)),
                        (Included(start), Included(end)) => Some((*start, *end - *start + 1)),
                        (Included(start), Excluded(end)) => Some((*start, *end - *start)),
                        (Excluded(start), Included(end)) => Some((*start + 1, *end - *start)),
                        (Excluded(start), Excluded(end)) => Some((*start + 1, *end - *start - 1)),
                    };
                    match range {
                        Some((start, length)) => {
                            (tensor.narrow(dim, start, length.max(0))?, dim + 1)
                        }
                        None => (tensor, dim + 1),
                    }
                }
                Indices(indices) => {
                    let indices = Tensor::from_slice(indices)?.to_device(tensor.device())?;
                    (tensor.index_select(dim, &indices)?, dim + 1)
                }
                IndexSelect(indices) => {
                    if indices.dim() != 1 {
                        return Err(TchError::Shape(
                            "only a one-dimensional tensor can index".into(),
                        ));
                    }
                    if !matches!(
                        indices.kind(),
                        Kind::Int64 | Kind::Int | Kind::Int16 | Kind::Int8
                    ) {
                        return Err(TchError::Kind(format!(
                            "a {:?} tensor can't index",
                            indices.kind()
                        )));
                    }
                    let indices = indices.to_device(tensor.device())?;
                    (tensor.index_select(dim, &indices)?, dim + 1)
                }
            };
            tensor = next;
            dim = next_dim;
        }
        Ok(tensor)
    }
}
