//! The weights of a model.

use std::collections::{BTreeSet, HashMap};
use std::ops::Div;
use std::sync::{Mutex, PoisonError};

use safetensors::{Dtype, SafeTensors};
use tch::{Device, Kind, Result, TchError, Tensor};

/// The separator between a weight name's parts.
const SEP: char = '.';

/// A model's weights, on one device and of one kind, chosen when the store
/// is made. Weights never track gradients.
#[derive(Debug)]
pub struct VarStore {
    variables: Mutex<HashMap<String, Tensor>>,
    /// The names a checkpoint may leave out.
    optional: Mutex<BTreeSet<String>>,
    device: Device,
    kind: Kind,
}

/// A name prefix in a store, such as `encoder.layers.0`.
#[derive(Debug, Clone)]
pub struct Path<'a> {
    path: Vec<String>,
    var_store: &'a VarStore,
}

impl VarStore {
    pub fn new(device: Device, kind: Kind) -> VarStore {
        VarStore {
            variables: Mutex::new(HashMap::new()),
            optional: Mutex::new(BTreeSet::new()),
            device,
            kind,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn root(&self) -> Path<'_> {
        Path {
            path: vec![],
            var_store: self,
        }
    }

    /// Fills every weight from a `.safetensors` file, keeping each weight's
    /// own kind and device. A weight the file doesn't have, one whose shape
    /// differs, and one the model doesn't declare are errors, as each means
    /// the port's names don't match the checkpoint's.
    pub fn load<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        let file = path.display().to_string();
        let mut variables = self
            .variables
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let optional = self.optional.lock().unwrap_or_else(PoisonError::into_inner);
        let handle = std::fs::File::open(path)?;
        // The file isn't changed while it's mapped: the runtime's store keeps
        // it read-only once it's pinned and checked.
        let mapping = unsafe { memmap2::Mmap::map(&handle) }?;
        let tensors = SafeTensors::deserialize(&mapping)
            .map_err(|err| TchError::FileFormat(format!("{file}: {err}")))?;
        let mut unused = tensors.names().into_iter().collect::<BTreeSet<_>>();
        let _guard = tch::no_grad_guard();
        for (name, variable) in variables.iter_mut() {
            let Ok(view) = tensors.tensor(name) else {
                if optional.contains(name) {
                    continue;
                }
                return Err(TchError::TensorNameNotFound(name.clone(), file));
            };
            let shape = view.shape().iter().map(|&x| x as i64).collect::<Vec<_>>();
            if shape != variable.size() {
                return Err(TchError::Shape(format!(
                    "`{name}` is {shape:?} in {file} but {:?} in the model",
                    variable.size()
                )));
            }
            let kind = match view.dtype() {
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
                dtype => {
                    return Err(TchError::Convert(format!(
                        "`{name}` in {file} is {dtype:?}, which libtorch doesn't take"
                    )));
                }
            };
            variable.f_copy_(&Tensor::f_from_data_size(view.data(), &shape, kind)?)?;
            unused.remove(name.as_str());
        }
        if !unused.is_empty() {
            let names = unused.into_iter().collect::<Vec<_>>().join(", ");
            return Err(TchError::FileFormat(format!(
                "{file} has weights the model doesn't declare: {names}"
            )));
        }
        Ok(())
    }

    fn variables(&self) -> std::sync::MutexGuard<'_, HashMap<String, Tensor>> {
        self.variables
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl<'a> Path<'a> {
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.path.iter().map(String::as_str)
    }

    /// The prefix with `s` appended.
    pub fn sub<T: ToString>(&self, s: T) -> Path<'a> {
        let s = s.to_string();
        assert!(!s.contains(SEP), "a name part can't contain {SEP}: {s}");
        let mut path = self.path.clone();
        path.push(s);
        Path {
            path,
            var_store: self.var_store,
        }
    }

    pub fn device(&self) -> Device {
        self.var_store.device
    }

    pub fn kind(&self) -> Kind {
        self.var_store.kind
    }

    /// The full name of `name` under this prefix.
    pub fn path(&self, name: &str) -> String {
        assert!(
            !name.contains(SEP),
            "a weight name can't contain {SEP}: {name}"
        );
        if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}{SEP}{name}", self.path.join(&SEP.to_string()))
        }
    }

    /// Declares the weight `name` of shape `dims` in the store's kind. It
    /// holds nothing meaningful until `VarStore::load` fills it.
    pub fn var(&self, name: &str, dims: &[i64]) -> Result<Tensor> {
        self.var_with_kind(name, dims, self.kind())
    }

    /// Declares a weight of its own kind, such as an integer buffer.
    pub fn var_with_kind(&self, name: &str, dims: &[i64], kind: Kind) -> Result<Tensor> {
        self.declare(name, Tensor::f_empty(dims, (kind, self.device()))?)
    }

    /// Declares a buffer a checkpoint may leave out, which then stays zero,
    /// as PyTorch fills in a BatchNorm's missing `num_batches_tracked`.
    pub fn optional_var_with_kind(&self, name: &str, dims: &[i64], kind: Kind) -> Result<()> {
        // The store keeps the buffer; nothing reads it during inference.
        let _ = self.declare(name, Tensor::f_zeros(dims, (kind, self.device()))?)?;
        self.var_store
            .optional
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.path(name));
        Ok(())
    }

    fn declare(&self, name: &str, tensor: Tensor) -> Result<Tensor> {
        let path = self.path(name);
        let mut variables = self.var_store.variables();
        if variables.contains_key(&path) {
            return Err(TchError::Convert(format!("`{path}` is declared twice")));
        }
        let view = tensor.f_alias()?;
        variables.insert(path, tensor);
        Ok(view)
    }
}

impl<'a, T: ToString> Div<T> for &Path<'a> {
    type Output = Path<'a>;

    fn div(self, rhs: T) -> Self::Output {
        self.sub(rhs.to_string())
    }
}

impl<'a, T: ToString> Div<T> for Path<'a> {
    type Output = Path<'a>;

    fn div(self, rhs: T) -> Self::Output {
        self.sub(rhs.to_string())
    }
}
