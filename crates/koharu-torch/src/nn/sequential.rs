//! Layers applied in turn.

use super::{Module, ModuleT};
use crate::{Result, Tensor};

#[derive(Debug)]
pub struct Sequential {
    layers: Vec<Box<dyn Module>>,
}

pub fn seq() -> Sequential {
    Sequential { layers: vec![] }
}

impl Sequential {
    #[allow(clippy::should_implement_trait)]
    pub fn add<M: Module + 'static>(mut self, layer: M) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

impl Module for Sequential {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        self.layers
            .iter()
            .try_fold(xs.alias()?, |xs, layer| layer.forward(&xs))
    }
}

#[derive(Debug)]
pub struct SequentialT {
    layers: Vec<Box<dyn ModuleT>>,
}

pub fn seq_t() -> SequentialT {
    SequentialT { layers: vec![] }
}

impl SequentialT {
    #[allow(clippy::should_implement_trait)]
    pub fn add<M: ModuleT + 'static>(mut self, layer: M) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

impl ModuleT for SequentialT {
    fn forward_t(&self, xs: &Tensor, train: bool) -> Result<Tensor> {
        self.layers
            .iter()
            .try_fold(xs.alias()?, |xs, layer| layer.forward_t(&xs, train))
    }
}
