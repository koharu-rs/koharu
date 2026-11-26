//! Inference-only port of torchvision's ResNet-50, YuzuMarker's
//! `ResNet50Regressor`, whose last layer is 2048 → 6,162: the font logits,
//! then the direction and regression outputs.
//!
//! Original implementation:
//! https://github.com/pytorch/vision/blob/8ac84ee75afb1c327902156b5336f56ad63b7e2f/torchvision/models/resnet.py#L108-L284
//! https://github.com/JeffersonQin/YuzuMarker.FontDetection/blob/0a94e165fe2b08d2800b723290eabd120b2d3d58/detector/model.py#L80-L96

use std::path::Path;

use anyhow::Result;
use koharu_torch::{Device, Kind, Tensor};

use crate::nn::{self, Module, ModuleT};

/// Each stage's blocks and planes, `resnet50`'s `[3, 4, 6, 3]`.
const LAYERS: [(usize, i64); 4] = [(3, 64), (4, 128), (6, 256), (3, 512)];
/// `Bottleneck.expansion`.
const EXPANSION: i64 = 4;
/// `config.FONT_COUNT + 12`.
const OUTPUTS: i64 = 6162;

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    conv1: nn::Conv2D,
    bn1: nn::BatchNorm,
    layers: Vec<Vec<Bottleneck>>,
    fc: nn::Linear,
}

impl Model {
    pub(super) fn new(device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();
        let conv1 = nn::conv2d(&root / "conv1", 3, 64, 7, conv(2, 3))?;
        let bn1 = nn::batch_norm2d(&root / "bn1", 64, Default::default())?;
        let mut inplanes = 64;
        let mut layers = Vec::with_capacity(LAYERS.len());
        for (index, (blocks, planes)) in LAYERS.into_iter().enumerate() {
            // `_make_layer`: the first stage keeps its size, each later one
            // halves it in its first block.
            let stride = if index == 0 { 1 } else { 2 };
            let path = &root / format!("layer{}", index + 1);
            let mut layer = Vec::with_capacity(blocks);
            for block in 0..blocks {
                layer.push(Bottleneck::new(
                    &(&path / block),
                    inplanes,
                    planes,
                    if block == 0 { stride } else { 1 },
                )?);
                inplanes = planes * EXPANSION;
            }
            layers.push(layer);
        }
        let fc = nn::linear(&root / "fc", 512 * EXPANSION, OUTPUTS, Default::default())?;
        Ok(Self {
            vs,
            conv1,
            bn1,
            layers,
            fc,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    /// `ResNet._forward_impl`: the 6,162 outputs, before the regressor
    /// applies its sigmoids, which only the regression outputs get.
    pub(super) fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let xs = xs.to_kind(self.vs.kind())?;
        let mut xs = self
            .bn1
            .forward_t(&self.conv1.forward(&xs)?, false)?
            .relu()?
            .max_pool2d([3, 3], [2, 2], [1, 1], [1, 1], false)?;
        for layer in &self.layers {
            for block in layer {
                xs = block.forward(&xs)?;
            }
        }
        let xs = xs.adaptive_avg_pool2d([1, 1])?.flatten_using_ints(1, -1)?;
        self.fc.forward(&xs)
    }
}

/// A bias-free convolution, as torchvision's `conv1x1`, `conv3x3`, and stem
/// are.
fn conv(stride: i64, padding: i64) -> nn::ConvConfig {
    nn::ConvConfig {
        stride,
        padding,
        bias: false,
        ..Default::default()
    }
}

/// torchvision's `Bottleneck`, ResNet v1.5, with the stride on its 3×3
/// convolution.
#[derive(Debug)]
struct Bottleneck {
    conv1: nn::Conv2D,
    bn1: nn::BatchNorm,
    conv2: nn::Conv2D,
    bn2: nn::BatchNorm,
    conv3: nn::Conv2D,
    bn3: nn::BatchNorm,
    downsample: Option<(nn::Conv2D, nn::BatchNorm)>,
}

impl Bottleneck {
    fn new(path: &nn::Path<'_>, inplanes: i64, planes: i64, stride: i64) -> Result<Self> {
        let width = planes;
        let out = planes * EXPANSION;
        let downsample = if stride != 1 || inplanes != out {
            Some((
                nn::conv2d(path / "downsample" / 0, inplanes, out, 1, conv(stride, 0))?,
                nn::batch_norm2d(path / "downsample" / 1, out, Default::default())?,
            ))
        } else {
            None
        };
        Ok(Self {
            conv1: nn::conv2d(path / "conv1", inplanes, width, 1, conv(1, 0))?,
            bn1: nn::batch_norm2d(path / "bn1", width, Default::default())?,
            conv2: nn::conv2d(path / "conv2", width, width, 3, conv(stride, 1))?,
            bn2: nn::batch_norm2d(path / "bn2", width, Default::default())?,
            conv3: nn::conv2d(path / "conv3", width, out, 1, conv(1, 0))?,
            bn3: nn::batch_norm2d(path / "bn3", out, Default::default())?,
            downsample,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let out = self
            .bn1
            .forward_t(&self.conv1.forward(xs)?, false)?
            .relu()?;
        let out = self
            .bn2
            .forward_t(&self.conv2.forward(&out)?, false)?
            .relu()?;
        let out = self.bn3.forward_t(&self.conv3.forward(&out)?, false)?;
        let identity = match &self.downsample {
            Some((conv, bn)) => bn.forward_t(&conv.forward(xs)?, false)?,
            None => xs.alias()?,
        };
        out.add_tensor(&identity, 1)?.relu()
    }
}
