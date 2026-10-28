//! Inference-only port of YuzuMarker's `ResNet50Regressor`.
//!
//! Original implementation:
//! https://github.com/JeffersonQin/YuzuMarker.FontDetection/blob/0a94e165fe2b08d2800b723290eabd120b2d3d58/detector/model.py

use std::path::Path;

use anyhow::Result;
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, ModuleT},
};

use super::{
    processor::{FONT_COUNT, OUTPUT_DIM, REGRESSION_DIM},
    resnet::resnet50,
};

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    model: Box<dyn ModuleT>,
}

impl Model {
    pub fn new(device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        // The published safetensors retain Lightning's `model` wrapper and
        // torch.compile's `_orig_mod` wrapper from the upstream checkpoint.
        let model = resnet50(&(&vs.root() / "model" / "_orig_mod" / "model"), OUTPUT_DIM)?;
        Ok(Self {
            vs,
            model: Box::new(model),
        })
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub fn forward(&self, pixel_values: &Tensor) -> koharu_torch::Result<Tensor> {
        let output = self
            .model
            .forward_t(&pixel_values.to_kind(self.vs.kind())?, false)?;
        // Upstream leaves classification logits untouched and applies sigmoid
        // only to the ten regression values.
        let classification = output.narrow(-1, 0, FONT_COUNT + 2)?;
        let regression = output
            .narrow(-1, FONT_COUNT + 2, REGRESSION_DIM)?
            .sigmoid()?;
        Tensor::cat(&[classification, regression], -1)
    }
}
