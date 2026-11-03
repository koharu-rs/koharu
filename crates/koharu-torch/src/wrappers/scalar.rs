//! Scalar elements.

use koharu_torch_sys::{
    AtenScalar, AtenScalar__bindgen_ty_1, AtenScalar__bindgen_ty_1__bindgen_ty_1, ATEN_SCALAR_BOOL,
    ATEN_SCALAR_COMPLEX, ATEN_SCALAR_FLOAT, ATEN_SCALAR_INT,
};

use crate::{Result, TchError};

/// A single scalar value, of the type ATen's type promotion reads: an
/// integer added to an integer tensor keeps it an integer tensor.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Scalar {
    Bool(bool),
    Int(i64),
    Float(f64),
    Complex { real: f64, imag: f64 },
}

impl Scalar {
    pub(crate) fn c_scalar(self) -> AtenScalar {
        let (type_, value) = match self {
            Scalar::Bool(b) => (ATEN_SCALAR_BOOL, AtenScalar__bindgen_ty_1 { b }),
            Scalar::Int(i) => (ATEN_SCALAR_INT, AtenScalar__bindgen_ty_1 { i }),
            Scalar::Float(f) => (ATEN_SCALAR_FLOAT, AtenScalar__bindgen_ty_1 { f }),
            Scalar::Complex { real, imag } => (
                ATEN_SCALAR_COMPLEX,
                AtenScalar__bindgen_ty_1 {
                    c: AtenScalar__bindgen_ty_1__bindgen_ty_1 { real, imag },
                },
            ),
        };
        AtenScalar { type_, value }
    }

    pub(crate) fn from_c_scalar(scalar: AtenScalar) -> Result<Scalar> {
        // The tag says which field the shim wrote.
        unsafe {
            Ok(match scalar.type_ {
                ATEN_SCALAR_BOOL => Scalar::Bool(scalar.value.b),
                ATEN_SCALAR_INT => Scalar::Int(scalar.value.i),
                ATEN_SCALAR_FLOAT => Scalar::Float(scalar.value.f),
                ATEN_SCALAR_COMPLEX => Scalar::Complex {
                    real: scalar.value.c.real,
                    imag: scalar.value.c.imag,
                },
                tag => return Err(TchError::Convert(format!("a scalar of unknown type {tag}"))),
            })
        }
    }
}

impl From<bool> for Scalar {
    fn from(v: bool) -> Scalar {
        Scalar::Bool(v)
    }
}

impl From<i64> for Scalar {
    fn from(v: i64) -> Scalar {
        Scalar::Int(v)
    }
}

impl From<f64> for Scalar {
    fn from(v: f64) -> Scalar {
        Scalar::Float(v)
    }
}
