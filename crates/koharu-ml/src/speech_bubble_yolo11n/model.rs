//! YOLO11n segmentation model from Ultralytics 8.3.227.
//!
//! Authoritative architecture and modules:
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/cfg/models/11/yolo11-seg.yaml
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/nn/modules/block.py
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/nn/modules/head.py
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/utils/tal.py

use std::path::Path;

use crate::nn::{self, Module, ModuleT};
use anyhow::{Result, bail, ensure};
use tch::{Device, Kind, Tensor};

use super::config::Yolo11nSpeechBubbleConfig;

#[derive(Debug)]
pub struct Output {
    pub pred: Tensor,
    pub proto: Tensor,
}

#[derive(Debug)]
pub struct Model {
    vs: nn::VarStore,
    model: Yolo11Seg,
}

impl Model {
    pub fn new(config: &Yolo11nSpeechBubbleConfig, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let model = Yolo11Seg::new(&(&vs.root() / "model"), config)?;
        Ok(Self { vs, model })
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub fn forward(&self, pixel_values: &Tensor) -> Result<Output> {
        self.model.forward(&pixel_values.f_to_kind(self.vs.kind())?)
    }
}

#[derive(Debug)]
struct Conv {
    conv: nn::Conv2D,
    bn: nn::BatchNorm,
    activate: bool,
}

impl Conv {
    fn load(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        kernel_size: i64,
        stride: i64,
        groups: i64,
        activate: bool,
    ) -> tch::Result<Self> {
        let conv = nn::conv2d(
            path / "conv",
            c1,
            c2,
            kernel_size,
            nn::ConvConfig {
                stride,
                padding: kernel_size / 2,
                groups,
                bias: false,
                ..Default::default()
            },
        )?;
        let bn = nn::batch_norm2d(
            path / "bn",
            c2,
            nn::BatchNormConfig {
                eps: 1e-3,
                momentum: 0.03,
                ..Default::default()
            },
        )?;
        Ok(Self { conv, bn, activate })
    }

    fn silu(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        kernel_size: i64,
        stride: i64,
    ) -> tch::Result<Self> {
        Self::load(path, c1, c2, kernel_size, stride, 1, true)
    }

    fn linear(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        kernel_size: i64,
        groups: i64,
    ) -> tch::Result<Self> {
        Self::load(path, c1, c2, kernel_size, 1, groups, false)
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let xs = self.bn.forward_t(&self.conv.forward(xs)?, false)?;
        if self.activate { xs.f_silu() } else { Ok(xs) }
    }
}

#[derive(Debug)]
struct Bottleneck {
    cv1: Conv,
    cv2: Conv,
    residual: bool,
}

impl Bottleneck {
    fn load(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        shortcut: bool,
        expansion: f64,
    ) -> tch::Result<Self> {
        let hidden = (c2 as f64 * expansion) as i64;
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, hidden, 3, 1)?,
            cv2: Conv::silu(&(path / "cv2"), hidden, c2, 3, 1)?,
            residual: shortcut && c1 == c2,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let ys = self.cv2.forward(&self.cv1.forward(xs)?)?;
        if self.residual { xs.f_add(&ys) } else { Ok(ys) }
    }
}

#[derive(Debug)]
struct C3k {
    cv1: Conv,
    cv2: Conv,
    cv3: Conv,
    m: Vec<Bottleneck>,
}

impl C3k {
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64, n: usize, shortcut: bool) -> tch::Result<Self> {
        let hidden = c2 / 2;
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, hidden, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), c1, hidden, 1, 1)?,
            cv3: Conv::silu(&(path / "cv3"), 2 * hidden, c2, 1, 1)?,
            m: (0..n)
                .map(|index| Bottleneck::load(&(path / "m" / index), hidden, hidden, shortcut, 1.0))
                .collect::<tch::Result<_>>()?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let mut left = self.cv1.forward(xs)?;
        for module in &self.m {
            left = module.forward(&left)?;
        }
        self.cv3
            .forward(&Tensor::f_cat(&[left, self.cv2.forward(xs)?], 1)?)
    }
}

#[derive(Debug)]
enum C3k2Module {
    Bottleneck(Box<Bottleneck>),
    C3k(Box<C3k>),
}

impl C3k2Module {
    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        match self {
            Self::Bottleneck(module) => module.forward(xs),
            Self::C3k(module) => module.forward(xs),
        }
    }
}

#[derive(Debug)]
struct C3k2 {
    cv1: Conv,
    cv2: Conv,
    m: Vec<C3k2Module>,
}

impl C3k2 {
    fn load(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        n: usize,
        c3k: bool,
        expansion: f64,
    ) -> tch::Result<Self> {
        let hidden = (c2 as f64 * expansion) as i64;
        let m = (0..n)
            .map(|index| {
                let path = path / "m" / index;
                Ok(if c3k {
                    C3k2Module::C3k(Box::new(C3k::load(&path, hidden, hidden, 2, true)?))
                } else {
                    C3k2Module::Bottleneck(Box::new(Bottleneck::load(
                        &path, hidden, hidden, true, 0.5,
                    )?))
                })
            })
            .collect::<tch::Result<_>>()?;
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, 2 * hidden, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), (2 + n as i64) * hidden, c2, 1, 1)?,
            m,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let mut ys = self.cv1.forward(xs)?.f_chunk(2, 1)?;
        for module in &self.m {
            ys.push(module.forward(ys.last().expect("C3k2 chunk"))?);
        }
        self.cv2.forward(&Tensor::f_cat(&ys, 1)?)
    }
}

#[derive(Debug)]
struct Sppf {
    cv1: Conv,
    cv2: Conv,
}

impl Sppf {
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64) -> tch::Result<Self> {
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, c1 / 2, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), 2 * c1, c2, 1, 1)?,
        })
    }

    fn pool(xs: &Tensor) -> tch::Result<Tensor> {
        xs.f_max_pool2d([5, 5], [1, 1], [2, 2], [1, 1], false)
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let x0 = self.cv1.forward(xs)?;
        let x1 = Self::pool(&x0)?;
        let x2 = Self::pool(&x1)?;
        let x3 = Self::pool(&x2)?;
        self.cv2.forward(&Tensor::f_cat(&[x0, x1, x2, x3], 1)?)
    }
}

#[derive(Debug)]
struct Attention {
    num_heads: i64,
    head_dim: i64,
    key_dim: i64,
    scale: f64,
    qkv: Conv,
    proj: Conv,
    pe: Conv,
}

impl Attention {
    fn load(path: &nn::Path<'_>, dim: i64, num_heads: i64) -> tch::Result<Self> {
        let head_dim = dim / num_heads;
        let key_dim = head_dim / 2;
        let nh_kd = key_dim * num_heads;
        Ok(Self {
            num_heads,
            head_dim,
            key_dim,
            scale: (key_dim as f64).powf(-0.5),
            qkv: Conv::linear(&(path / "qkv"), dim, dim + 2 * nh_kd, 1, 1)?,
            proj: Conv::linear(&(path / "proj"), dim, dim, 1, 1)?,
            pe: Conv::linear(&(path / "pe"), dim, dim, 3, dim)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let size = xs.size();
        let (batch, channels, height, width) = (size[0], size[1], size[2], size[3]);
        let qkv = self
            .qkv
            .forward(xs)?
            .f_view([
                batch,
                self.num_heads,
                self.key_dim * 2 + self.head_dim,
                height * width,
            ])?
            .f_split_with_sizes([self.key_dim, self.key_dim, self.head_dim], 2)?;
        let attention = (qkv[0].f_transpose(-2, -1)?.f_matmul(&qkv[1])?)
            .f_mul_scalar(self.scale)?
            .f_softmax(-1, None::<Kind>)?;
        let value = &qkv[2];
        let xs = (value
            .f_matmul(&attention.f_transpose(-2, -1)?)?
            .f_view([batch, channels, height, width])?)
        .f_add(
            &self
                .pe
                .forward(&value.f_reshape([batch, channels, height, width])?)?,
        )?;
        self.proj.forward(&xs)
    }
}

#[derive(Debug)]
struct PsaBlock {
    attn: Attention,
    ffn0: Conv,
    ffn1: Conv,
}

impl PsaBlock {
    fn load(path: &nn::Path<'_>, channels: i64) -> tch::Result<Self> {
        Ok(Self {
            attn: Attention::load(&(path / "attn"), channels, channels / 64)?,
            ffn0: Conv::silu(&(path / "ffn" / 0), channels, 2 * channels, 1, 1)?,
            ffn1: Conv::linear(&(path / "ffn" / 1), 2 * channels, channels, 1, 1)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let xs = xs.f_add(&self.attn.forward(xs)?)?;
        let ffn = self.ffn1.forward(&self.ffn0.forward(&xs)?)?;
        xs.f_add(&ffn)
    }
}

#[derive(Debug)]
struct C2Psa {
    cv1: Conv,
    cv2: Conv,
    m: Vec<PsaBlock>,
}

impl C2Psa {
    fn load(path: &nn::Path<'_>, channels: i64, n: usize) -> tch::Result<Self> {
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), channels, channels, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), channels, channels, 1, 1)?,
            m: (0..n)
                .map(|index| PsaBlock::load(&(path / "m" / index), channels / 2))
                .collect::<tch::Result<_>>()?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let chunks = self.cv1.forward(xs)?.f_chunk(2, 1)?;
        let left = chunks[0].f_alias()?;
        let mut right = chunks[1].f_alias()?;
        for module in &self.m {
            right = module.forward(&right)?;
        }
        self.cv2.forward(&Tensor::f_cat(&[left, right], 1)?)
    }
}

#[derive(Debug)]
struct Dfl {
    conv: nn::Conv2D,
    reg_max: i64,
}

impl Dfl {
    fn load(path: &nn::Path<'_>, reg_max: i64) -> Result<Self> {
        Ok(Self {
            conv: nn::conv2d(
                path / "conv",
                reg_max,
                1,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?,
            reg_max,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        let size = xs.size();
        self.conv
            .forward(
                &xs.f_view([size[0], 4, self.reg_max, size[2]])?
                    .f_transpose(2, 1)?
                    .f_softmax(1, None::<Kind>)?,
            )?
            .f_view([size[0], 4, size[2]])
    }
}

#[derive(Debug)]
struct HeadBranch {
    block0: Conv,
    block1: Conv,
    conv: nn::Conv2D,
}

impl HeadBranch {
    fn load(
        path: &nn::Path<'_>,
        in_channels: i64,
        hidden_channels: i64,
        out_channels: i64,
    ) -> Result<Self> {
        Ok(Self {
            block0: Conv::silu(&(path / 0), in_channels, hidden_channels, 3, 1)?,
            block1: Conv::silu(&(path / 1), hidden_channels, hidden_channels, 3, 1)?,
            conv: nn::conv2d(
                path / 2,
                hidden_channels,
                out_channels,
                1,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        self.conv
            .forward(&self.block1.forward(&self.block0.forward(xs)?)?)
    }
}

#[derive(Debug)]
struct DepthwisePointwise {
    depthwise: Conv,
    pointwise: Conv,
}

impl DepthwisePointwise {
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64) -> tch::Result<Self> {
        Ok(Self {
            depthwise: Conv::load(&(path / 0), c1, c1, 3, 1, c1, true)?,
            pointwise: Conv::silu(&(path / 1), c1, c2, 1, 1)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        self.pointwise.forward(&self.depthwise.forward(xs)?)
    }
}

#[derive(Debug)]
struct ClassHeadBranch {
    block0: DepthwisePointwise,
    block1: DepthwisePointwise,
    conv: nn::Conv2D,
}

impl ClassHeadBranch {
    fn load(
        path: &nn::Path<'_>,
        in_channels: i64,
        hidden_channels: i64,
        out_channels: i64,
    ) -> Result<Self> {
        Ok(Self {
            block0: DepthwisePointwise::load(&(path / 0), in_channels, hidden_channels)?,
            block1: DepthwisePointwise::load(&(path / 1), hidden_channels, hidden_channels)?,
            conv: nn::conv2d(
                path / 2,
                hidden_channels,
                out_channels,
                1,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        self.conv
            .forward(&self.block1.forward(&self.block0.forward(xs)?)?)
    }
}

#[derive(Debug)]
struct Proto {
    cv1: Conv,
    upsample: nn::ConvTranspose2D,
    cv2: Conv,
    cv3: Conv,
}

impl Proto {
    fn load(path: &nn::Path<'_>, c1: i64, c_mid: i64, c2: i64) -> Result<Self> {
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, c_mid, 3, 1)?,
            upsample: nn::conv_transpose2d(
                path / "upsample",
                c_mid,
                c_mid,
                2,
                nn::ConvTransposeConfig {
                    stride: 2,
                    ..Default::default()
                },
            )?,
            cv2: Conv::silu(&(path / "cv2"), c_mid, c_mid, 3, 1)?,
            cv3: Conv::silu(&(path / "cv3"), c_mid, c2, 1, 1)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        self.cv3.forward(
            &self
                .cv2
                .forward(&self.upsample.forward(&self.cv1.forward(xs)?)?)?,
        )
    }
}

fn make_anchors(xs: [&Tensor; 3], strides: [i64; 3]) -> Result<(Tensor, Tensor)> {
    let device = xs[0].device();
    let kind = xs[0].kind();
    let mut anchor_points = Vec::with_capacity(3);
    let mut stride_tensors = Vec::with_capacity(3);
    for (xs, stride) in xs.into_iter().zip(strides) {
        let size = xs.size();
        let height = size[2];
        let width = size[3];
        let sx = (Tensor::f_arange(width, (kind, device))?)
            .f_add_scalar(0.5)?
            .f_view([1, width])?
            .f_repeat([height, 1])?;
        let sy = (Tensor::f_arange(height, (kind, device))?)
            .f_add_scalar(0.5)?
            .f_view([height, 1])?
            .f_repeat([1, width])?;
        anchor_points.push(Tensor::f_stack(&[sx, sy], -1)?.f_view([-1, 2])?);
        stride_tensors.push(Tensor::f_full([height * width, 1], stride, (kind, device))?);
    }
    Ok((
        Tensor::f_cat(&anchor_points, 0)?,
        Tensor::f_cat(&stride_tensors, 0)?,
    ))
}

fn dist2bbox(distance: &Tensor, anchor_points: &Tensor) -> Result<Tensor> {
    let chunks = distance.f_chunk(2, 1)?;
    let x1y1 = anchor_points.f_sub(&chunks[0])?;
    let x2y2 = anchor_points.f_add(&chunks[1])?;
    Ok(Tensor::f_cat(
        &[(x1y1.f_add(&x2y2)?).f_div_scalar(2.0)?, x2y2.f_sub(&x1y1)?],
        1,
    )?)
}

#[derive(Debug)]
struct Segment {
    cv2: [HeadBranch; 3],
    cv3: [ClassHeadBranch; 3],
    dfl: Dfl,
    proto: Proto,
    cv4: [HeadBranch; 3],
    num_classes: i64,
    num_masks: i64,
    reg_max: i64,
}

impl Segment {
    fn load(
        path: &nn::Path<'_>,
        num_classes: i64,
        num_masks: i64,
        num_prototypes: i64,
        reg_max: i64,
    ) -> Result<Self> {
        let filters = [64, 128, 256];
        let box_channels = 64.max(reg_max * 4);
        let class_channels = 64.max(num_classes.min(100));
        let mask_channels = 16.max(num_masks);
        Ok(Self {
            cv2: [
                HeadBranch::load(&(path / "cv2" / 0), filters[0], box_channels, 4 * reg_max)?,
                HeadBranch::load(&(path / "cv2" / 1), filters[1], box_channels, 4 * reg_max)?,
                HeadBranch::load(&(path / "cv2" / 2), filters[2], box_channels, 4 * reg_max)?,
            ],
            cv3: [
                ClassHeadBranch::load(
                    &(path / "cv3" / 0),
                    filters[0],
                    class_channels,
                    num_classes,
                )?,
                ClassHeadBranch::load(
                    &(path / "cv3" / 1),
                    filters[1],
                    class_channels,
                    num_classes,
                )?,
                ClassHeadBranch::load(
                    &(path / "cv3" / 2),
                    filters[2],
                    class_channels,
                    num_classes,
                )?,
            ],
            dfl: Dfl::load(&(path / "dfl"), reg_max)?,
            proto: Proto::load(&(path / "proto"), filters[0], num_prototypes, num_masks)?,
            cv4: [
                HeadBranch::load(&(path / "cv4" / 0), filters[0], mask_channels, num_masks)?,
                HeadBranch::load(&(path / "cv4" / 1), filters[1], mask_channels, num_masks)?,
                HeadBranch::load(&(path / "cv4" / 2), filters[2], mask_channels, num_masks)?,
            ],
            num_classes,
            num_masks,
            reg_max,
        })
    }

    fn raw_detection(&self, xs: &Tensor, index: usize) -> Result<Tensor> {
        Ok(Tensor::f_cat(
            &[self.cv2[index].forward(xs)?, self.cv3[index].forward(xs)?],
            1,
        )?)
    }

    fn mask_coefficients(&self, xs: &Tensor, index: usize) -> Result<Tensor> {
        let size = xs.size();
        Ok(self.cv4[index]
            .forward(xs)?
            .f_view([size[0], self.num_masks, size[2] * size[3]])?)
    }

    fn forward(&self, xs0: &Tensor, xs1: &Tensor, xs2: &Tensor) -> Result<Output> {
        let proto = self.proto.forward(xs0)?;
        let mask_coefficients = Tensor::f_cat(
            &[
                self.mask_coefficients(xs0, 0)?,
                self.mask_coefficients(xs1, 1)?,
                self.mask_coefficients(xs2, 2)?,
            ],
            2,
        )?;
        let raw = [
            self.raw_detection(xs0, 0)?,
            self.raw_detection(xs1, 1)?,
            self.raw_detection(xs2, 2)?,
        ];
        let no = self.num_classes + self.reg_max * 4;
        let reshape = |xs: &Tensor| {
            let size = xs.size();
            xs.f_view([size[0], no, size[2] * size[3]])
        };
        let detection = Tensor::f_cat(
            &[reshape(&raw[0])?, reshape(&raw[1])?, reshape(&raw[2])?],
            2,
        )?;
        let (anchors, strides) = make_anchors([&raw[0], &raw[1], &raw[2]], [8, 16, 32])?;
        let anchors = anchors.f_transpose(0, 1)?.f_unsqueeze(0)?;
        let strides = strides.f_transpose(0, 1)?;
        let boxes = detection.f_slice(1, 0, self.reg_max * 4, 1)?;
        let classes = detection.f_slice(1, self.reg_max * 4, no, 1)?;
        let boxes = (dist2bbox(&self.dfl.forward(&boxes)?, &anchors)?).f_mul(&strides)?;
        Ok(Output {
            pred: Tensor::f_cat(&[boxes, classes.f_sigmoid()?, mask_coefficients], 1)?,
            proto,
        })
    }
}

#[derive(Debug)]
struct Yolo11Seg {
    layer0: Conv,
    layer1: Conv,
    layer2: C3k2,
    layer3: Conv,
    layer4: C3k2,
    layer5: Conv,
    layer6: C3k2,
    layer7: Conv,
    layer8: C3k2,
    layer9: Sppf,
    layer10: C2Psa,
    layer13: C3k2,
    layer16: C3k2,
    layer17: Conv,
    layer19: C3k2,
    layer20: Conv,
    layer22: C3k2,
    layer23: Segment,
}

impl Yolo11Seg {
    fn new(path: &nn::Path<'_>, config: &Yolo11nSpeechBubbleConfig) -> Result<Self> {
        ensure!(
            config.model_type == "yolo11-seg",
            "unsupported model type {:?}",
            config.model_type
        );
        ensure!(
            config.variant == "n",
            "unsupported YOLO11 segmentation variant {:?}",
            config.variant
        );
        ensure!(config.num_classes > 0, "num_classes must be positive");
        ensure!(config.num_masks > 0, "num_masks must be positive");
        ensure!(
            config.num_prototypes == 64,
            "YOLO11n-seg expects 64 prototype channels, got {}",
            config.num_prototypes
        );
        if config.reg_max != 16 {
            bail!("YOLO11n-seg expects reg_max 16, got {}", config.reg_max);
        }
        Ok(Self {
            layer0: Conv::silu(&(path / 0), 3, 16, 3, 2)?,
            layer1: Conv::silu(&(path / 1), 16, 32, 3, 2)?,
            layer2: C3k2::load(&(path / 2), 32, 64, 1, false, 0.25)?,
            layer3: Conv::silu(&(path / 3), 64, 64, 3, 2)?,
            layer4: C3k2::load(&(path / 4), 64, 128, 1, false, 0.25)?,
            layer5: Conv::silu(&(path / 5), 128, 128, 3, 2)?,
            layer6: C3k2::load(&(path / 6), 128, 128, 1, true, 0.5)?,
            layer7: Conv::silu(&(path / 7), 128, 256, 3, 2)?,
            layer8: C3k2::load(&(path / 8), 256, 256, 1, true, 0.5)?,
            layer9: Sppf::load(&(path / 9), 256, 256)?,
            layer10: C2Psa::load(&(path / 10), 256, 1)?,
            layer13: C3k2::load(&(path / 13), 384, 128, 1, false, 0.5)?,
            layer16: C3k2::load(&(path / 16), 256, 64, 1, false, 0.5)?,
            layer17: Conv::silu(&(path / 17), 64, 64, 3, 2)?,
            layer19: C3k2::load(&(path / 19), 192, 128, 1, false, 0.5)?,
            layer20: Conv::silu(&(path / 20), 128, 128, 3, 2)?,
            layer22: C3k2::load(&(path / 22), 384, 256, 1, true, 0.5)?,
            layer23: Segment::load(
                &(path / 23),
                config.num_classes,
                config.num_masks,
                config.num_prototypes,
                config.reg_max,
            )?,
        })
    }

    fn upsample(xs: &Tensor) -> Result<Tensor> {
        let size = xs.size();
        Ok(xs.f_upsample_nearest2d([2 * size[2], 2 * size[3]], None::<f64>, None::<f64>)?)
    }

    fn forward(&self, xs: &Tensor) -> Result<Output> {
        let x0 = self.layer0.forward(xs)?;
        let x1 = self.layer1.forward(&x0)?;
        let x2 = self.layer2.forward(&x1)?;
        let x3 = self.layer3.forward(&x2)?;
        let x4 = self.layer4.forward(&x3)?;
        let x5 = self.layer5.forward(&x4)?;
        let x6 = self.layer6.forward(&x5)?;
        let x7 = self.layer7.forward(&x6)?;
        let x8 = self.layer8.forward(&x7)?;
        let x9 = self.layer9.forward(&x8)?;
        let x10 = self.layer10.forward(&x9)?;
        let x13 = self
            .layer13
            .forward(&Tensor::f_cat(&[Self::upsample(&x10)?, x6], 1)?)?;
        let x16 = self
            .layer16
            .forward(&Tensor::f_cat(&[Self::upsample(&x13)?, x4], 1)?)?;
        let x19 = self
            .layer19
            .forward(&Tensor::f_cat(&[self.layer17.forward(&x16)?, x13], 1)?)?;
        let x22 = self
            .layer22
            .forward(&Tensor::f_cat(&[self.layer20.forward(&x19)?, x10], 1)?)?;
        self.layer23.forward(&x16, &x19, &x22)
    }
}
