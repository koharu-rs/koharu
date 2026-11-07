pub(crate) mod device;
pub(crate) mod generator;
pub mod kind;
pub(crate) mod layout;
pub(crate) mod scalar;
pub(crate) mod tensor;
pub(crate) mod utils;

// A method or function for each operator, which build.rs generates from
// koharu-torch-sys's bindings. A dunder operator, such as `__and__`, keeps
// its doubled underscores.
#[allow(clippy::all, non_snake_case)]
mod tensor_generated {
    use std::borrow::Borrow;

    use koharu_torch_sys::*;

    use super::utils::{IntList, IntListOption, check};
    use crate::{Device, Generator, Kind, Layout, MemoryFormat, QScheme, Result, Scalar, Tensor};

    include!(concat!(env!("OUT_DIR"), "/tensor_generated.rs"));
}
