//! YOLO26s segmentation model from Ultralytics 8.4.43.
//!
//! Authoritative architecture and modules:
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/cfg/models/26/yolo26-seg.yaml
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/nn/modules/block.py
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/nn/modules/head.py
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/utils/tal.py

use std::path::Path;

use anyhow::{Result, ensure};
use koharu_torch::{
    Kind, Tensor,
    nn::{self, Module, ModuleT},
};

use super::config::ComicLayoutYolo26sConfig;

#[derive(Debug)]
pub struct Output {
    pub pred: Tensor,
    pub proto: Tensor,
}

#[derive(Debug)]
pub struct Model {
    vs: nn::VarStore,
    model: Yolo26Seg,
}

impl Model {
    pub fn new(
        config: &ComicLayoutYolo26sConfig,
        device: koharu_torch::Device,
        kind: Kind,
    ) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let model = Yolo26Seg::new(&(&vs.root() / "model"), config)?;
        Ok(Self { vs, model })
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub fn forward(&self, pixel_values: &Tensor) -> Result<Output> {
        self.model.forward(&pixel_values.to_kind(self.vs.kind())?)
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
    ) -> koharu_torch::Result<Self> {
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
    ) -> koharu_torch::Result<Self> {
        Self::load(path, c1, c2, kernel_size, stride, 1, true)
    }

    fn linear(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        kernel_size: i64,
        groups: i64,
    ) -> koharu_torch::Result<Self> {
        Self::load(path, c1, c2, kernel_size, 1, groups, false)
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let xs = self.bn.forward_t(&self.conv.forward(xs)?, false)?;
        if self.activate { xs.silu() } else { Ok(xs) }
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
    ) -> koharu_torch::Result<Self> {
        let hidden = (c2 as f64 * expansion) as i64;
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, hidden, 3, 1)?,
            cv2: Conv::silu(&(path / "cv2"), hidden, c2, 3, 1)?,
            residual: shortcut && c1 == c2,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let ys = self.cv2.forward(&self.cv1.forward(xs)?)?;
        if self.residual { xs + ys } else { Ok(ys) }
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
    fn load(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        n: usize,
        shortcut: bool,
    ) -> koharu_torch::Result<Self> {
        let hidden = c2 / 2;
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, hidden, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), c1, hidden, 1, 1)?,
            cv3: Conv::silu(&(path / "cv3"), 2 * hidden, c2, 1, 1)?,
            m: (0..n)
                .map(|index| Bottleneck::load(&(path / "m" / index), hidden, hidden, shortcut, 1.0))
                .collect::<koharu_torch::Result<_>>()?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let mut left = self.cv1.forward(xs)?;
        for module in &self.m {
            left = module.forward(&left)?;
        }
        self.cv3
            .forward(&Tensor::cat(&[left, self.cv2.forward(xs)?], 1)?)
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
    fn load(path: &nn::Path<'_>, dim: i64, num_heads: i64) -> koharu_torch::Result<Self> {
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

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let size = xs.size();
        let (batch, channels, height, width) = (size[0], size[1], size[2], size[3]);
        let qkv = self
            .qkv
            .forward(xs)?
            .view([
                batch,
                self.num_heads,
                self.key_dim * 2 + self.head_dim,
                height * width,
            ])?
            .split_with_sizes([self.key_dim, self.key_dim, self.head_dim], 2)?;
        let attention =
            (qkv[0].transpose(-2, -1)?.matmul(&qkv[1])? * self.scale)?.softmax(-1, None::<Kind>)?;
        let value = &qkv[2];
        let xs = (value
            .matmul(&attention.transpose(-2, -1)?)?
            .view([batch, channels, height, width])?
            + self
                .pe
                .forward(&value.reshape([batch, channels, height, width])?)?)?;
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
    fn load(path: &nn::Path<'_>, channels: i64) -> koharu_torch::Result<Self> {
        Ok(Self {
            attn: Attention::load(&(path / "attn"), channels, (channels / 64).max(1))?,
            ffn0: Conv::silu(&(path / "ffn" / 0), channels, 2 * channels, 1, 1)?,
            ffn1: Conv::linear(&(path / "ffn" / 1), 2 * channels, channels, 1, 1)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let xs = (xs + self.attn.forward(xs)?)?;
        let ffn = self.ffn1.forward(&self.ffn0.forward(&xs)?)?;
        xs + ffn
    }
}

#[derive(Debug)]
struct AttentionC3k2Block {
    bottleneck: Bottleneck,
    psa: PsaBlock,
}

impl AttentionC3k2Block {
    fn load(path: &nn::Path<'_>, channels: i64) -> koharu_torch::Result<Self> {
        Ok(Self {
            bottleneck: Bottleneck::load(&(path / 0), channels, channels, true, 0.5)?,
            psa: PsaBlock::load(&(path / 1), channels)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        self.psa.forward(&self.bottleneck.forward(xs)?)
    }
}

#[derive(Debug)]
enum C3k2Module {
    Bottleneck(Box<Bottleneck>),
    C3k(Box<C3k>),
    Attention(Box<AttentionC3k2Block>),
}

impl C3k2Module {
    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        match self {
            Self::Bottleneck(module) => module.forward(xs),
            Self::C3k(module) => module.forward(xs),
            Self::Attention(module) => module.forward(xs),
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
        attention: bool,
    ) -> koharu_torch::Result<Self> {
        let hidden = (c2 as f64 * expansion) as i64;
        let m = (0..n)
            .map(|index| {
                let path = path / "m" / index;
                Ok(if attention {
                    C3k2Module::Attention(Box::new(AttentionC3k2Block::load(&path, hidden)?))
                } else if c3k {
                    C3k2Module::C3k(Box::new(C3k::load(&path, hidden, hidden, 2, true)?))
                } else {
                    C3k2Module::Bottleneck(Box::new(Bottleneck::load(
                        &path, hidden, hidden, true, 0.5,
                    )?))
                })
            })
            .collect::<koharu_torch::Result<_>>()?;
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), c1, 2 * hidden, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), (2 + n as i64) * hidden, c2, 1, 1)?,
            m,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let mut ys = self.cv1.forward(xs)?.chunk(2, 1)?;
        for module in &self.m {
            ys.push(module.forward(ys.last().expect("C3k2 chunk"))?);
        }
        self.cv2.forward(&Tensor::cat(&ys, 1)?)
    }
}

#[derive(Debug)]
struct Sppf {
    cv1: Conv,
    cv2: Conv,
}

impl Sppf {
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64) -> koharu_torch::Result<Self> {
        Ok(Self {
            // YOLO26 deliberately disables the first SPPF activation.
            cv1: Conv::linear(&(path / "cv1"), c1, c1 / 2, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), 2 * c1, c2, 1, 1)?,
        })
    }

    fn pool(xs: &Tensor) -> koharu_torch::Result<Tensor> {
        xs.max_pool2d([5, 5], [1, 1], [2, 2], [1, 1], false)
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let x0 = self.cv1.forward(xs)?;
        let x1 = Self::pool(&x0)?;
        let x2 = Self::pool(&x1)?;
        let x3 = Self::pool(&x2)?;
        self.cv2.forward(&Tensor::cat(&[x0, x1, x2, x3], 1)?)? + xs
    }
}

#[derive(Debug)]
struct C2Psa {
    cv1: Conv,
    cv2: Conv,
    m: Vec<PsaBlock>,
}

impl C2Psa {
    fn load(path: &nn::Path<'_>, channels: i64, n: usize) -> koharu_torch::Result<Self> {
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), channels, channels, 1, 1)?,
            cv2: Conv::silu(&(path / "cv2"), channels, channels, 1, 1)?,
            m: (0..n)
                .map(|index| PsaBlock::load(&(path / "m" / index), channels / 2))
                .collect::<koharu_torch::Result<_>>()?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let chunks = self.cv1.forward(xs)?.chunk(2, 1)?;
        let left = chunks[0].alias()?;
        let mut right = chunks[1].alias()?;
        for module in &self.m {
            right = module.forward(&right)?;
        }
        self.cv2.forward(&Tensor::cat(&[left, right], 1)?)
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

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
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
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64) -> koharu_torch::Result<Self> {
        Ok(Self {
            depthwise: Conv::load(&(path / 0), c1, c1, 3, 1, c1, true)?,
            pointwise: Conv::silu(&(path / 1), c1, c2, 1, 1)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
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

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
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
    fn load(path: &nn::Path<'_>, channels: i64, num_masks: i64) -> Result<Self> {
        Ok(Self {
            cv1: Conv::silu(&(path / "cv1"), channels, channels, 3, 1)?,
            upsample: nn::conv_transpose2d(
                path / "upsample",
                channels,
                channels,
                2,
                nn::ConvTransposeConfig {
                    stride: 2,
                    ..Default::default()
                },
            )?,
            cv2: Conv::silu(&(path / "cv2"), channels, channels, 3, 1)?,
            cv3: Conv::silu(&(path / "cv3"), channels, num_masks, 1, 1)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        self.cv3.forward(
            &self
                .cv2
                .forward(&self.upsample.forward(&self.cv1.forward(xs)?)?)?,
        )
    }
}

#[derive(Debug)]
struct SemanticSegmentationHead {
    block0: Conv,
    block1: Conv,
    conv: nn::Conv2D,
}

impl SemanticSegmentationHead {
    fn load(path: &nn::Path<'_>, channels: i64, num_classes: i64) -> Result<Self> {
        Ok(Self {
            block0: Conv::silu(&(path / 0), channels, channels, 3, 1)?,
            block1: Conv::silu(&(path / 1), channels, channels, 3, 1)?,
            conv: nn::conv2d(path / 2, channels, num_classes, 1, Default::default())?,
        })
    }

    fn register(&self) {
        // This training-only branch is retained so the module tree exactly matches the checkpoint.
        let _ = (&self.block0, &self.block1, &self.conv);
    }
}

#[derive(Debug)]
struct Proto26 {
    proto: Proto,
    feat_refine: [Conv; 2],
    feat_fuse: Conv,
    semseg: SemanticSegmentationHead,
}

impl Proto26 {
    fn load(path: &nn::Path<'_>, num_classes: i64, num_masks: i64) -> Result<Self> {
        let semseg = SemanticSegmentationHead::load(&(path / "semseg"), 128, num_classes)?;
        semseg.register();
        Ok(Self {
            proto: Proto::load(path, 128, num_masks)?,
            feat_refine: [
                Conv::silu(&(path / "feat_refine" / 0), 256, 128, 1, 1)?,
                Conv::silu(&(path / "feat_refine" / 1), 512, 128, 1, 1)?,
            ],
            feat_fuse: Conv::silu(&(path / "feat_fuse"), 128, 128, 3, 1)?,
            semseg,
        })
    }

    fn forward(&self, xs: [&Tensor; 3]) -> Result<Tensor> {
        let size = xs[0].size();
        let mut feat = xs[0].alias()?;
        for (index, refine) in self.feat_refine.iter().enumerate() {
            let refined = refine.forward(xs[index + 1])?.upsample_nearest2d(
                [size[2], size[3]],
                None::<f64>,
                None::<f64>,
            )?;
            // Python's `feat = feat + up_feat` must allocate a new tensor here;
            // mutating this shallow clone would also overwrite the P3 detection feature.
            feat = (&feat + refined)?;
        }
        let _ = &self.semseg;
        Ok(self.proto.forward(&self.feat_fuse.forward(&feat)?)?)
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
        let sx = (Tensor::arange(width, (kind, device))? + 0.5)?
            .view([1, width])?
            .repeat([height, 1])?;
        let sy = (Tensor::arange(height, (kind, device))? + 0.5)?
            .view([height, 1])?
            .repeat([1, width])?;
        anchor_points.push(Tensor::stack(&[sx, sy], -1)?.view([-1, 2])?);
        stride_tensors.push(Tensor::full([height * width, 1], stride, (kind, device))?);
    }
    Ok((
        Tensor::cat(&anchor_points, 0)?,
        Tensor::cat(&stride_tensors, 0)?,
    ))
}

fn dist2bbox(distance: &Tensor, anchor_points: &Tensor) -> Result<Tensor> {
    let chunks = distance.chunk(2, 1)?;
    Ok(Tensor::cat(
        &[(anchor_points - &chunks[0])?, (anchor_points + &chunks[1])?],
        1,
    )?)
}

#[derive(Debug)]
struct Segment26 {
    // Ultralytics serializes both training branches. The one-to-many heads are
    // registered for an exact checkpoint tree but are not executed in end-to-end inference.
    cv2: [HeadBranch; 3],
    cv3: [ClassHeadBranch; 3],
    cv4: [HeadBranch; 3],
    one2one_cv2: [HeadBranch; 3],
    one2one_cv3: [ClassHeadBranch; 3],
    one2one_cv4: [HeadBranch; 3],
    proto: Proto26,
    num_classes: i64,
    num_masks: i64,
}

impl Segment26 {
    fn load(path: &nn::Path<'_>, num_classes: i64) -> Result<Self> {
        let filters = [128, 256, 512];
        let box_channels = 32;
        let class_channels = 128;
        let mask_channels = 32;
        let box_heads = |name: &str| -> Result<_> {
            Ok([
                HeadBranch::load(&(path / name / 0), filters[0], box_channels, 4)?,
                HeadBranch::load(&(path / name / 1), filters[1], box_channels, 4)?,
                HeadBranch::load(&(path / name / 2), filters[2], box_channels, 4)?,
            ])
        };
        let class_heads = |name: &str| -> Result<_> {
            Ok([
                ClassHeadBranch::load(&(path / name / 0), filters[0], class_channels, num_classes)?,
                ClassHeadBranch::load(&(path / name / 1), filters[1], class_channels, num_classes)?,
                ClassHeadBranch::load(&(path / name / 2), filters[2], class_channels, num_classes)?,
            ])
        };
        let mask_heads = |name: &str| -> Result<_> {
            Ok([
                HeadBranch::load(&(path / name / 0), filters[0], mask_channels, 32)?,
                HeadBranch::load(&(path / name / 1), filters[1], mask_channels, 32)?,
                HeadBranch::load(&(path / name / 2), filters[2], mask_channels, 32)?,
            ])
        };
        Ok(Self {
            cv2: box_heads("cv2")?,
            cv3: class_heads("cv3")?,
            cv4: mask_heads("cv4")?,
            one2one_cv2: box_heads("one2one_cv2")?,
            one2one_cv3: class_heads("one2one_cv3")?,
            one2one_cv4: mask_heads("one2one_cv4")?,
            proto: Proto26::load(&(path / "proto"), num_classes, 32)?,
            num_classes,
            num_masks: 32,
        })
    }

    fn flatten_head(xs: &Tensor, channels: i64) -> Result<Tensor> {
        let size = xs.size();
        Ok(xs.view([size[0], channels, size[2] * size[3]])?)
    }

    fn postprocess(&self, preds: &Tensor) -> Result<Tensor> {
        let boxes = preds.narrow(-1, 0, 4)?;
        let scores = preds.narrow(-1, 4, self.num_classes)?;
        let coefficients = preds.narrow(-1, 4 + self.num_classes, self.num_masks)?;
        let batch = scores.size()[0];
        let k = 300.min(scores.size()[1]);

        // This is Detect.get_topk_index followed by Segment.postprocess. The
        // two top-k stages are the NMS-free YOLO26 end-to-end prediction path.
        let anchor_index = scores
            .max_dim(-1, false)?
            .0
            .topk(k, 1, true, true)?
            .1
            .unsqueeze(-1)?;
        let shortlisted =
            scores.gather(1, &anchor_index.repeat([1, 1, self.num_classes])?, false)?;
        let (scores, flat_index) = shortlisted.flatten(1, -1)?.topk(k, 1, true, true)?;
        let class_index = flat_index.remainder(self.num_classes)?.unsqueeze(-1)?;
        let selected_anchor = anchor_index.gather(
            1,
            &flat_index
                .floor_divide_scalar(self.num_classes)?
                .view([batch, k, 1])?,
            false,
        )?;
        let boxes = boxes.gather(1, &selected_anchor.repeat([1, 1, 4])?, false)?;
        let coefficients =
            coefficients.gather(1, &selected_anchor.repeat([1, 1, self.num_masks])?, false)?;
        Ok(Tensor::cat(
            &[
                boxes,
                scores.unsqueeze(-1)?,
                class_index.to_kind(Kind::Float)?,
                coefficients,
            ],
            -1,
        )?)
    }

    fn forward(&self, xs0: &Tensor, xs1: &Tensor, xs2: &Tensor) -> Result<Output> {
        let xs = [xs0, xs1, xs2];
        let _ = (&self.cv2, &self.cv3, &self.cv4);

        let boxes = Tensor::cat(
            &(0..3)
                .map(|index| Self::flatten_head(&self.one2one_cv2[index].forward(xs[index])?, 4))
                .collect::<Result<Vec<_>>>()?,
            2,
        )?;
        let scores = Tensor::cat(
            &(0..3)
                .map(|index| {
                    Self::flatten_head(
                        &self.one2one_cv3[index].forward(xs[index])?,
                        self.num_classes,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
            2,
        )?;
        let coefficients = Tensor::cat(
            &(0..3)
                .map(|index| {
                    Self::flatten_head(&self.one2one_cv4[index].forward(xs[index])?, self.num_masks)
                })
                .collect::<Result<Vec<_>>>()?,
            2,
        )?;
        let (anchors, strides) = make_anchors(xs, [8, 16, 32])?;
        let anchors = anchors.transpose(0, 1)?.unsqueeze(0)?;
        let strides = strides.transpose(0, 1)?;
        let boxes = (dist2bbox(&boxes, &anchors)? * strides)?;
        let pred = Tensor::cat(&[boxes, scores.sigmoid()?, coefficients], 1)?.permute([0, 2, 1])?;
        let pred = self.postprocess(&pred)?;
        // Segment26 executes Detect.forward before Proto26.forward.
        let proto = self.proto.forward(xs)?;
        Ok(Output { pred, proto })
    }
}

#[derive(Debug)]
struct Yolo26Seg {
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
    layer23: Segment26,
}

impl Yolo26Seg {
    fn new(path: &nn::Path<'_>, config: &ComicLayoutYolo26sConfig) -> Result<Self> {
        ensure!(
            config.architectures == ["YOLO26s-seg"],
            "unsupported architecture {:?}",
            config.architectures
        );
        ensure!(
            config.library_name == "ultralytics",
            "unsupported library {:?}",
            config.library_name
        );
        ensure!(
            config.task == "instance-segmentation",
            "unsupported task {:?}",
            config.task
        );
        ensure!(config.image_size > 0, "image_size must be positive");
        ensure!(
            config.num_classes == 3,
            "comic-layout-yolo26s expects 3 classes, got {}",
            config.num_classes
        );
        let class_names = config.class_names()?;
        ensure!(
            class_names
                .iter()
                .map(String::as_str)
                .eq(["frame", "text", "balloon"]),
            "comic-layout-yolo26s expects labels [frame, text, balloon], got {class_names:?}"
        );
        Ok(Self {
            layer0: Conv::silu(&(path / 0), 3, 32, 3, 2)?,
            layer1: Conv::silu(&(path / 1), 32, 64, 3, 2)?,
            layer2: C3k2::load(&(path / 2), 64, 128, 1, false, 0.25, false)?,
            layer3: Conv::silu(&(path / 3), 128, 128, 3, 2)?,
            layer4: C3k2::load(&(path / 4), 128, 256, 1, false, 0.25, false)?,
            layer5: Conv::silu(&(path / 5), 256, 256, 3, 2)?,
            layer6: C3k2::load(&(path / 6), 256, 256, 1, true, 0.5, false)?,
            layer7: Conv::silu(&(path / 7), 256, 512, 3, 2)?,
            layer8: C3k2::load(&(path / 8), 512, 512, 1, true, 0.5, false)?,
            layer9: Sppf::load(&(path / 9), 512, 512)?,
            layer10: C2Psa::load(&(path / 10), 512, 1)?,
            layer13: C3k2::load(&(path / 13), 768, 256, 1, true, 0.5, false)?,
            layer16: C3k2::load(&(path / 16), 512, 128, 1, true, 0.5, false)?,
            layer17: Conv::silu(&(path / 17), 128, 128, 3, 2)?,
            layer19: C3k2::load(&(path / 19), 384, 256, 1, true, 0.5, false)?,
            layer20: Conv::silu(&(path / 20), 256, 256, 3, 2)?,
            layer22: C3k2::load(&(path / 22), 768, 512, 1, true, 0.5, true)?,
            layer23: Segment26::load(&(path / 23), config.num_classes)?,
        })
    }

    fn upsample(xs: &Tensor) -> Result<Tensor> {
        let size = xs.size();
        Ok(xs.upsample_nearest2d([2 * size[2], 2 * size[3]], None::<f64>, None::<f64>)?)
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
            .forward(&Tensor::cat(&[Self::upsample(&x10)?, x6], 1)?)?;
        let x16 = self
            .layer16
            .forward(&Tensor::cat(&[Self::upsample(&x13)?, x4], 1)?)?;
        let x19 = self
            .layer19
            .forward(&Tensor::cat(&[self.layer17.forward(&x16)?, x13], 1)?)?;
        let x22 = self
            .layer22
            .forward(&Tensor::cat(&[self.layer20.forward(&x19)?, x10], 1)?)?;
        self.layer23.forward(&x16, &x19, &x22)
    }
}
