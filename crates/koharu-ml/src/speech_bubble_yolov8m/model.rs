//! YOLOv8 segmentation model ported from Candle Transformers.
//!
//! Original implementation:
//! https://github.com/huggingface/candle/blob/b7a98f1d1ab830d729556a92fae440b3debf4980/candle-examples/examples/yolo-v8/model.rs

use std::path::Path;

use anyhow::{Result, bail};
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module, ModuleT},
};

use super::config::YoloV8mSpeechBubbleConfig;

#[derive(Debug)]
pub struct Output {
    pub pred: Tensor,
    pub proto: Tensor,
}

#[derive(Debug)]
pub struct Model {
    vs: nn::VarStore,
    model: YoloV8Seg,
}

impl Model {
    pub fn new(config: &YoloV8mSpeechBubbleConfig, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let model = YoloV8Seg::load(&(&vs.root() / "model"), config)?;
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

#[derive(Clone, Copy, PartialEq, Debug)]
struct Multiples {
    depth: f64,
    width: f64,
    ratio: f64,
}

impl Multiples {
    fn n() -> Self {
        Self {
            depth: 0.33,
            width: 0.25,
            ratio: 2.0,
        }
    }

    fn s() -> Self {
        Self {
            depth: 0.33,
            width: 0.50,
            ratio: 2.0,
        }
    }

    fn m() -> Self {
        Self {
            depth: 0.67,
            width: 0.75,
            ratio: 1.5,
        }
    }

    fn l() -> Self {
        Self {
            depth: 1.00,
            width: 1.00,
            ratio: 1.0,
        }
    }

    fn x() -> Self {
        Self {
            depth: 1.00,
            width: 1.25,
            ratio: 1.0,
        }
    }

    fn from_variant(variant: &str) -> Result<Self> {
        match variant {
            "n" => Ok(Self::n()),
            "s" => Ok(Self::s()),
            "m" => Ok(Self::m()),
            "l" => Ok(Self::l()),
            "x" => Ok(Self::x()),
            _ => bail!("unsupported YOLOv8 variant {variant:?}"),
        }
    }

    fn filters(&self) -> (i64, i64, i64) {
        let f1 = (256.0 * self.width) as i64;
        let f2 = (512.0 * self.width) as i64;
        let f3 = (512.0 * self.width * self.ratio) as i64;
        (f1, f2, f3)
    }
}

#[derive(Debug)]
struct Upsample {
    scale_factor: i64,
}

impl Upsample {
    fn new(scale_factor: i64) -> Self {
        Self { scale_factor }
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let size = xs.size();
        xs.upsample_nearest2d(
            [self.scale_factor * size[2], self.scale_factor * size[3]],
            None::<f64>,
            None::<f64>,
        )
    }
}

#[derive(Debug)]
struct ConvBlock {
    conv: nn::Conv2D,
    bn: nn::BatchNorm,
}

impl ConvBlock {
    fn load(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        kernel_size: i64,
        stride: i64,
        padding: Option<i64>,
    ) -> koharu_torch::Result<Self> {
        let conv = nn::conv2d(
            path / "conv",
            c1,
            c2,
            kernel_size,
            nn::ConvConfig {
                stride,
                padding: padding.unwrap_or(kernel_size / 2),
                bias: false,
                ..Default::default()
            },
        )?;
        let bn = nn::batch_norm2d(
            path / "bn",
            c2,
            nn::BatchNormConfig {
                eps: 1e-3,
                ..Default::default()
            },
        )?;
        Ok(Self { conv, bn })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        self.bn.forward_t(&self.conv.forward(xs)?, false)?.silu()
    }
}

#[derive(Debug)]
struct Bottleneck {
    cv1: ConvBlock,
    cv2: ConvBlock,
    residual: bool,
}

impl Bottleneck {
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64, shortcut: bool) -> koharu_torch::Result<Self> {
        let channel_factor = 1.0;
        let hidden = (c2 as f64 * channel_factor) as i64;
        Ok(Self {
            cv1: ConvBlock::load(&(path / "cv1"), c1, hidden, 3, 1, None)?,
            cv2: ConvBlock::load(&(path / "cv2"), hidden, c2, 3, 1, None)?,
            residual: c1 == c2 && shortcut,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let ys = self.cv2.forward(&self.cv1.forward(xs)?)?;
        if self.residual { xs + ys } else { Ok(ys) }
    }
}

#[derive(Debug)]
struct C2f {
    cv1: ConvBlock,
    cv2: ConvBlock,
    bottleneck: Vec<Bottleneck>,
}

impl C2f {
    fn load(
        path: &nn::Path<'_>,
        c1: i64,
        c2: i64,
        n: usize,
        shortcut: bool,
    ) -> koharu_torch::Result<Self> {
        let hidden = (c2 as f64 * 0.5) as i64;
        let bottleneck = (0..n)
            .map(|idx| Bottleneck::load(&(path / "m" / idx), hidden, hidden, shortcut))
            .collect::<koharu_torch::Result<_>>()?;
        Ok(Self {
            cv1: ConvBlock::load(&(path / "cv1"), c1, 2 * hidden, 1, 1, None)?,
            cv2: ConvBlock::load(&(path / "cv2"), (2 + n as i64) * hidden, c2, 1, 1, None)?,
            bottleneck,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let mut ys = self.cv1.forward(xs)?.chunk(2, 1)?;
        for module in &self.bottleneck {
            ys.push(module.forward(ys.last().expect("c2f chunk"))?);
        }
        self.cv2.forward(&Tensor::cat(&ys, 1)?)
    }
}

#[derive(Debug)]
struct Sppf {
    cv1: ConvBlock,
    cv2: ConvBlock,
    kernel_size: i64,
}

impl Sppf {
    fn load(path: &nn::Path<'_>, c1: i64, c2: i64, kernel_size: i64) -> koharu_torch::Result<Self> {
        let hidden = c1 / 2;
        Ok(Self {
            cv1: ConvBlock::load(&(path / "cv1"), c1, hidden, 1, 1, None)?,
            cv2: ConvBlock::load(&(path / "cv2"), hidden * 4, c2, 1, 1, None)?,
            kernel_size,
        })
    }

    fn pool(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        xs.constant_pad_nd([
            self.kernel_size / 2,
            self.kernel_size / 2,
            self.kernel_size / 2,
            self.kernel_size / 2,
        ])?
        .max_pool2d(
            [self.kernel_size, self.kernel_size],
            [1, 1],
            [0, 0],
            [1, 1],
            false,
        )
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let xs = self.cv1.forward(xs)?;
        let xs2 = self.pool(&xs)?;
        let xs3 = self.pool(&xs2)?;
        let xs4 = self.pool(&xs3)?;
        self.cv2.forward(&Tensor::cat(&[xs, xs2, xs3, xs4], 1)?)
    }
}

#[derive(Debug)]
struct Dfl {
    conv: nn::Conv2D,
    num_classes: i64,
}

impl Dfl {
    fn load(path: &nn::Path<'_>, num_classes: i64) -> Result<Self> {
        Ok(Self {
            conv: nn::conv2d(
                path / "conv",
                num_classes,
                1,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?,
            num_classes,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let size = xs.size();
        let xs = xs
            .view([size[0], 4, self.num_classes, size[2]])?
            .transpose(2, 1)?
            .softmax(1, None::<Kind>)?;
        self.conv.forward(&xs)?.view([size[0], 4, size[2]])
    }
}

#[derive(Debug)]
struct DarkNet {
    b1_0: ConvBlock,
    b1_1: ConvBlock,
    b2_0: C2f,
    b2_1: ConvBlock,
    b2_2: C2f,
    b3_0: ConvBlock,
    b3_1: C2f,
    b4_0: ConvBlock,
    b4_1: C2f,
    b5: Sppf,
}

impl DarkNet {
    fn load(path: &nn::Path<'_>, multiples: Multiples) -> koharu_torch::Result<Self> {
        let (w, r, d) = (multiples.width, multiples.ratio, multiples.depth);
        Ok(Self {
            b1_0: ConvBlock::load(&(path / 0), 3, (64.0 * w) as i64, 3, 2, Some(1))?,
            b1_1: ConvBlock::load(
                &(path / 1),
                (64.0 * w) as i64,
                (128.0 * w) as i64,
                3,
                2,
                Some(1),
            )?,
            b2_0: C2f::load(
                &(path / 2),
                (128.0 * w) as i64,
                (128.0 * w) as i64,
                (3.0 * d).round() as usize,
                true,
            )?,
            b2_1: ConvBlock::load(
                &(path / 3),
                (128.0 * w) as i64,
                (256.0 * w) as i64,
                3,
                2,
                Some(1),
            )?,
            b2_2: C2f::load(
                &(path / 4),
                (256.0 * w) as i64,
                (256.0 * w) as i64,
                (6.0 * d).round() as usize,
                true,
            )?,
            b3_0: ConvBlock::load(
                &(path / 5),
                (256.0 * w) as i64,
                (512.0 * w) as i64,
                3,
                2,
                Some(1),
            )?,
            b3_1: C2f::load(
                &(path / 6),
                (512.0 * w) as i64,
                (512.0 * w) as i64,
                (6.0 * d).round() as usize,
                true,
            )?,
            b4_0: ConvBlock::load(
                &(path / 7),
                (512.0 * w) as i64,
                (512.0 * w * r) as i64,
                3,
                2,
                Some(1),
            )?,
            b4_1: C2f::load(
                &(path / 8),
                (512.0 * w * r) as i64,
                (512.0 * w * r) as i64,
                (3.0 * d).round() as usize,
                true,
            )?,
            b5: Sppf::load(
                &(path / 9),
                (512.0 * w * r) as i64,
                (512.0 * w * r) as i64,
                5,
            )?,
        })
    }

    fn forward(&self, xs: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let x1 = self.b1_1.forward(&self.b1_0.forward(xs)?)?;
        let x2 = self
            .b2_2
            .forward(&self.b2_1.forward(&self.b2_0.forward(&x1)?)?)?;
        let x3 = self.b3_1.forward(&self.b3_0.forward(&x2)?)?;
        let x4 = self.b4_1.forward(&self.b4_0.forward(&x3)?)?;
        let x5 = self.b5.forward(&x4)?;
        Ok((x2, x3, x5))
    }
}

#[derive(Debug)]
struct YoloV8Neck {
    up: Upsample,
    n1: C2f,
    n2: C2f,
    n3: ConvBlock,
    n4: C2f,
    n5: ConvBlock,
    n6: C2f,
}

impl YoloV8Neck {
    fn load(path: &nn::Path<'_>, multiples: Multiples) -> koharu_torch::Result<Self> {
        let (w, r, d) = (multiples.width, multiples.ratio, multiples.depth);
        let n = (3.0 * d).round() as usize;
        Ok(Self {
            up: Upsample::new(2),
            n1: C2f::load(
                &(path / 12),
                (512.0 * w * (1.0 + r)) as i64,
                (512.0 * w) as i64,
                n,
                false,
            )?,
            n2: C2f::load(
                &(path / 15),
                (768.0 * w) as i64,
                (256.0 * w) as i64,
                n,
                false,
            )?,
            n3: ConvBlock::load(
                &(path / 16),
                (256.0 * w) as i64,
                (256.0 * w) as i64,
                3,
                2,
                Some(1),
            )?,
            n4: C2f::load(
                &(path / 18),
                (768.0 * w) as i64,
                (512.0 * w) as i64,
                n,
                false,
            )?,
            n5: ConvBlock::load(
                &(path / 19),
                (512.0 * w) as i64,
                (512.0 * w) as i64,
                3,
                2,
                Some(1),
            )?,
            n6: C2f::load(
                &(path / 21),
                (512.0 * w * (1.0 + r)) as i64,
                (512.0 * w * r) as i64,
                n,
                false,
            )?,
        })
    }

    fn forward(&self, p3: &Tensor, p4: &Tensor, p5: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let x = self
            .n1
            .forward(&Tensor::cat(&[self.up.forward(p5)?, p4.alias()?], 1)?)?;
        let head_1 = self
            .n2
            .forward(&Tensor::cat(&[self.up.forward(&x)?, p3.alias()?], 1)?)?;
        let head_2 = self
            .n4
            .forward(&Tensor::cat(&[self.n3.forward(&head_1)?, x], 1)?)?;
        let head_3 = self
            .n6
            .forward(&Tensor::cat(&[self.n5.forward(&head_2)?, p5.alias()?], 1)?)?;
        Ok((head_1, head_2, head_3))
    }
}

fn make_anchors(
    xs0: &Tensor,
    xs1: &Tensor,
    xs2: &Tensor,
    (s0, s1, s2): (i64, i64, i64),
    grid_cell_offset: f64,
) -> Result<(Tensor, Tensor)> {
    let device = xs0.device();
    let kind = xs0.kind();
    let mut anchor_points = Vec::new();
    let mut stride_tensors = Vec::new();
    for (xs, stride) in [(xs0, s0), (xs1, s1), (xs2, s2)] {
        let size = xs.size();
        let height = size[2];
        let width = size[3];
        let sx = (Tensor::arange(width, (kind, device))? + grid_cell_offset)?
            .view([1, width])?
            .repeat([height, 1])?
            .view([-1])?;
        let sy = (Tensor::arange(height, (kind, device))? + grid_cell_offset)?
            .view([height, 1])?
            .repeat([1, width])?
            .view([-1])?;
        anchor_points.push(Tensor::stack(&[sx, sy], -1)?);
        stride_tensors.push(Tensor::full([height * width], stride, (kind, device))?);
    }
    Ok((
        Tensor::cat(&anchor_points, 0)?,
        Tensor::cat(&stride_tensors, 0)?.unsqueeze(1)?,
    ))
}

fn dist2bbox(distance: &Tensor, anchor_points: &Tensor) -> Result<Tensor> {
    let chunks = distance.chunk(2, 1)?;
    let x1y1 = (anchor_points - &chunks[0])?;
    let x2y2 = (anchor_points + &chunks[1])?;
    let center = ((&x1y1 + &x2y2)? * 0.5)?;
    let dimensions = (x2y2 - x1y1)?;
    Ok(Tensor::cat(&[center, dimensions], 1)?)
}

#[derive(Debug)]
struct HeadBranch {
    block0: ConvBlock,
    block1: ConvBlock,
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
            block0: ConvBlock::load(&(path / 0), in_channels, hidden_channels, 3, 1, None)?,
            block1: ConvBlock::load(&(path / 1), hidden_channels, hidden_channels, 3, 1, None)?,
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
    cv1: ConvBlock,
    upsample: nn::ConvTranspose2D,
    cv2: ConvBlock,
    cv3: ConvBlock,
}

impl Proto {
    fn load(path: &nn::Path<'_>, c1: i64, c_mid: i64, c2: i64) -> Result<Self> {
        Ok(Self {
            cv1: ConvBlock::load(&(path / "cv1"), c1, c_mid, 3, 1, None)?,
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
            cv2: ConvBlock::load(&(path / "cv2"), c_mid, c_mid, 3, 1, None)?,
            cv3: ConvBlock::load(&(path / "cv3"), c_mid, c2, 1, 1, None)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> koharu_torch::Result<Tensor> {
        let xs = self.cv1.forward(xs)?;
        let xs = self.upsample.forward(&xs)?;
        let xs = self.cv2.forward(&xs)?;
        self.cv3.forward(&xs)
    }
}

// Ultralytics' Segment head supplies the prototype and mask-coefficient branches
// missing from Candle Transformers' detection-only YOLOv8 example.
// https://github.com/ultralytics/ultralytics/blob/2b7fac4db483aa89542c361ad4384e9119f0573d/ultralytics/nn/modules/head.py
#[derive(Debug)]
struct SegmentHead {
    dfl: Dfl,
    cv2: [HeadBranch; 3],
    cv3: [HeadBranch; 3],
    proto: Proto,
    cv4: [HeadBranch; 3],
    reg_max: i64,
    no: i64,
    num_masks: i64,
}

impl SegmentHead {
    fn load(
        path: &nn::Path<'_>,
        num_classes: i64,
        num_masks: i64,
        num_prototypes: i64,
        reg_max: i64,
        filters: (i64, i64, i64),
    ) -> Result<Self> {
        let c1 = filters.0.max(num_classes);
        let c2 = (filters.0 / 4).max(reg_max * 4);
        let c4 = (filters.0 / 4).max(num_masks);
        Ok(Self {
            dfl: Dfl::load(&(path / "dfl"), reg_max)?,
            cv2: [
                HeadBranch::load(&(path / "cv2" / 0), filters.0, c2, 4 * reg_max)?,
                HeadBranch::load(&(path / "cv2" / 1), filters.1, c2, 4 * reg_max)?,
                HeadBranch::load(&(path / "cv2" / 2), filters.2, c2, 4 * reg_max)?,
            ],
            cv3: [
                HeadBranch::load(&(path / "cv3" / 0), filters.0, c1, num_classes)?,
                HeadBranch::load(&(path / "cv3" / 1), filters.1, c1, num_classes)?,
                HeadBranch::load(&(path / "cv3" / 2), filters.2, c1, num_classes)?,
            ],
            proto: Proto::load(&(path / "proto"), filters.0, num_prototypes, num_masks)?,
            cv4: [
                HeadBranch::load(&(path / "cv4" / 0), filters.0, c4, num_masks)?,
                HeadBranch::load(&(path / "cv4" / 1), filters.1, c4, num_masks)?,
                HeadBranch::load(&(path / "cv4" / 2), filters.2, c4, num_masks)?,
            ],
            reg_max,
            no: num_classes + reg_max * 4,
            num_masks,
        })
    }

    fn forward_cv2_cv3(&self, xs: &Tensor, index: usize) -> Result<Tensor> {
        Ok(Tensor::cat(
            &[self.cv2[index].forward(xs)?, self.cv3[index].forward(xs)?],
            1,
        )?)
    }

    fn forward_detection(&self, xs0: &Tensor, xs1: &Tensor, xs2: &Tensor) -> Result<Tensor> {
        let xs0 = self.forward_cv2_cv3(xs0, 0)?;
        let xs1 = self.forward_cv2_cv3(xs1, 1)?;
        let xs2 = self.forward_cv2_cv3(xs2, 2)?;
        let (anchors, strides) = make_anchors(&xs0, &xs1, &xs2, (8, 16, 32), 0.5)?;
        let anchors = anchors.transpose(0, 1)?.unsqueeze(0)?;
        let strides = strides.transpose(0, 1)?;

        let reshape = |xs: &Tensor| {
            let size = xs.size();
            xs.view([size[0], self.no, size[2] * size[3]])
        };
        let xs = Tensor::cat(&[reshape(&xs0)?, reshape(&xs1)?, reshape(&xs2)?], 2)?;
        let boxes = xs.slice(1, 0, self.reg_max * 4, 1)?;
        let classes = xs.slice(1, self.reg_max * 4, self.no, 1)?;
        let boxes = (dist2bbox(&self.dfl.forward(&boxes)?, &anchors)? * strides)?;
        Ok(Tensor::cat(&[boxes, classes.sigmoid()?], 1)?)
    }

    fn forward_cv4(&self, xs: &Tensor, index: usize) -> Result<Tensor> {
        let size = xs.size();
        Ok(self.cv4[index]
            .forward(xs)?
            .view([size[0], self.num_masks, size[2] * size[3]])?)
    }

    fn forward(&self, xs0: &Tensor, xs1: &Tensor, xs2: &Tensor) -> Result<Output> {
        let pred = self.forward_detection(xs0, xs1, xs2)?;
        let proto = self.proto.forward(xs0)?;
        let mask_coefficients = Tensor::cat(
            &[
                self.forward_cv4(xs0, 0)?,
                self.forward_cv4(xs1, 1)?,
                self.forward_cv4(xs2, 2)?,
            ],
            2,
        )?;
        Ok(Output {
            pred: Tensor::cat(&[pred, mask_coefficients], 1)?,
            proto,
        })
    }
}

#[derive(Debug)]
struct YoloV8Seg {
    backbone: DarkNet,
    fpn: YoloV8Neck,
    head: SegmentHead,
}

impl YoloV8Seg {
    fn load(path: &nn::Path<'_>, config: &YoloV8mSpeechBubbleConfig) -> Result<Self> {
        if config.model_type != "yolov8-seg" {
            bail!("unsupported model type {:?}", config.model_type);
        }
        let multiples = Multiples::from_variant(&config.variant)?;
        Ok(Self {
            backbone: DarkNet::load(path, multiples)?,
            fpn: YoloV8Neck::load(path, multiples)?,
            head: SegmentHead::load(
                &(path / 22),
                config.num_classes,
                config.num_masks,
                config.num_prototypes,
                config.reg_max,
                multiples.filters(),
            )?,
        })
    }

    fn forward(&self, xs: &Tensor) -> Result<Output> {
        let (p3, p4, p5) = self.backbone.forward(xs)?;
        let (p3, p4, p5) = self.fpn.forward(&p3, &p4, &p5)?;
        self.head.forward(&p3, &p4, &p5)
    }
}
