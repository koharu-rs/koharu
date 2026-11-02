//! Scalar elements.

use koharu_torch_sys::{
    KoharuTorchScalar, KoharuTorchScalar__bindgen_ty_1,
    KoharuTorchScalar__bindgen_ty_1__bindgen_ty_1, KOHARU_TORCH_SCALAR_BOOL,
    KOHARU_TORCH_SCALAR_COMPLEX, KOHARU_TORCH_SCALAR_FLOAT, KOHARU_TORCH_SCALAR_INT,
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
    pub(crate) fn c_scalar(self) -> KoharuTorchScalar {
        let (type_, value) = match self {
            Scalar::Bool(b) => (
                KOHARU_TORCH_SCALAR_BOOL,
                KoharuTorchScalar__bindgen_ty_1 { b },
            ),
            Scalar::Int(i) => (
                KOHARU_TORCH_SCALAR_INT,
                KoharuTorchScalar__bindgen_ty_1 { i },
            ),
            Scalar::Float(f) => (
                KOHARU_TORCH_SCALAR_FLOAT,
                KoharuTorchScalar__bindgen_ty_1 { f },
            ),
            Scalar::Complex { real, imag } => (
                KOHARU_TORCH_SCALAR_COMPLEX,
                KoharuTorchScalar__bindgen_ty_1 {
                    c: KoharuTorchScalar__bindgen_ty_1__bindgen_ty_1 { real, imag },
                },
            ),
        };
        KoharuTorchScalar { type_, value }
    }

    pub(crate) fn from_c_scalar(scalar: KoharuTorchScalar) -> Result<Scalar> {
        // The tag says which field the shim wrote.
        unsafe {
            Ok(match scalar.type_ {
                KOHARU_TORCH_SCALAR_BOOL => Scalar::Bool(scalar.value.b),
                KOHARU_TORCH_SCALAR_INT => Scalar::Int(scalar.value.i),
                KOHARU_TORCH_SCALAR_FLOAT => Scalar::Float(scalar.value.f),
                KOHARU_TORCH_SCALAR_COMPLEX => Scalar::Complex {
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
