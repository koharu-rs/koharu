//! Arithmetic operators. Each returns a `Result`, as the op can fail, so an
//! expression reads `(&a + &b)?`; the assigning operators, which can't
//! return one, aren't implemented.

use std::ops::{Add, Div, Mul, Neg, Sub};

use crate::{Result, Scalar, Tensor};

macro_rules! impl_op {
    ($trait:ident, $func:ident, $op:ident, $scalar_op:ident, $reversed:expr) => {
        impl $trait<Tensor> for Tensor {
            type Output = Result<Tensor>;

            fn $func(self, rhs: Tensor) -> Self::Output {
                self.$op(&rhs)
            }
        }

        impl $trait<&Tensor> for Tensor {
            type Output = Result<Tensor>;

            fn $func(self, rhs: &Tensor) -> Self::Output {
                self.$op(rhs)
            }
        }

        impl $trait<&Tensor> for &Tensor {
            type Output = Result<Tensor>;

            fn $func(self, rhs: &Tensor) -> Self::Output {
                self.$op(rhs)
            }
        }

        impl $trait<Tensor> for &Tensor {
            type Output = Result<Tensor>;

            fn $func(self, rhs: Tensor) -> Self::Output {
                self.$op(&rhs)
            }
        }

        impl<S: Into<Scalar>> $trait<S> for &Tensor {
            type Output = Result<Tensor>;

            fn $func(self, rhs: S) -> Self::Output {
                self.$scalar_op(rhs)
            }
        }

        impl<S: Into<Scalar>> $trait<S> for Tensor {
            type Output = Result<Tensor>;

            fn $func(self, rhs: S) -> Self::Output {
                self.$scalar_op(rhs)
            }
        }

        impl_scalar_lhs!($trait, $func, $scalar_op, $reversed, i64, f64);
    };
}

// A scalar on the left: the op with the operands swapped, then undone, as
// `2 - t` is `-(t - 2)` and `2 / t` is `1 / (t / 2)`. Only `i64` and `f64`,
// so a literal there has one type to take.
macro_rules! impl_scalar_lhs {
    ($trait:ident, $func:ident, $scalar_op:ident, $reversed:expr, $($scalar:ty),+) => {
        $(
            impl $trait<Tensor> for $scalar {
                type Output = Result<Tensor>;

                fn $func(self, rhs: Tensor) -> Self::Output {
                    rhs.$scalar_op(self).and_then($reversed)
                }
            }

            impl $trait<&Tensor> for $scalar {
                type Output = Result<Tensor>;

                fn $func(self, rhs: &Tensor) -> Self::Output {
                    rhs.$scalar_op(self).and_then($reversed)
                }
            }
        )+
    };
}

fn same(t: Tensor) -> Result<Tensor> {
    Ok(t)
}

fn negated(t: Tensor) -> Result<Tensor> {
    t.neg()
}

fn inverted(t: Tensor) -> Result<Tensor> {
    t.reciprocal()
}

impl_op!(Add, add, g_add, g_add_scalar, same);
impl_op!(Sub, sub, g_sub, g_sub_scalar, negated);
impl_op!(Mul, mul, g_mul, g_mul_scalar, same);
impl_op!(Div, div, g_div, g_div_scalar, inverted);

impl Neg for Tensor {
    type Output = Result<Tensor>;

    fn neg(self) -> Result<Tensor> {
        Tensor::neg(&self)
    }
}

impl Neg for &Tensor {
    type Output = Result<Tensor>;

    fn neg(self) -> Result<Tensor> {
        Tensor::neg(self)
    }
}

impl From<i64> for Scalar {
    fn from(v: i64) -> Scalar {
        // An integer scalar allocates a few bytes; failing to is out of
        // memory before any tensor could be.
        Scalar::int(v).expect("allocating a scalar")
    }
}

impl From<f64> for Scalar {
    fn from(v: f64) -> Scalar {
        Scalar::float(v).expect("allocating a scalar")
    }
}

impl From<i32> for Scalar {
    fn from(v: i32) -> Scalar {
        Scalar::from(i64::from(v))
    }
}

impl From<f32> for Scalar {
    fn from(v: f32) -> Scalar {
        Scalar::from(f64::from(v))
    }
}
