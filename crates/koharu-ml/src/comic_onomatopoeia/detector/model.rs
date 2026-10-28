//! Inference-only MTSv3 R-50-FPN segmentation detector.
//!
//! The module names and forward order follow these pinned upstream files:
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/MTSv3/maskrcnn_benchmark/modeling/backbone/resnet.py#L403-L704
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/MTSv3/maskrcnn_benchmark/modeling/backbone/fpn.py#L83-L159
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/MTSv3/maskrcnn_benchmark/modeling/segmentation/segmentation.py#L25-L109

use std::path::Path;

use anyhow::Result;
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module, ModuleT},
};

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    backbone: Backbone,
    proposal: SegmentationHead,
}

impl Model {
    pub(super) fn new(device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = &vs.root() / "module";
        let backbone = Backbone::new(&(&root / "backbone"))?;
        let proposal = SegmentationHead::new(&(&root / "proposal" / "head"))?;
        Ok(Self {
            vs,
            backbone,
            proposal,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub(super) fn forward(&self, pixel_values: &Tensor) -> koharu_torch::Result<Tensor> {
        let features = self
            .backbone
            .forward(&pixel_values.to_kind(self.vs.kind())?)?;
        self.proposal.forward(&features)
    }
}

#[derive(Debug)]
struct Backbone {
    body: ResNet,
    fpn: FeaturePyramidNetwork,
}

impl Backbone {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            body: ResNet::new(&(path / "body"))?,
            fpn: FeaturePyramidNetwork::new(&(path / "fpn"))?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Vec<Tensor>> {
        self.fpn.forward(&self.body.forward(input)?)
    }
}

#[derive(Debug)]
struct ResNet {
    stem: Stem,
    layers: [ResNetLayer; 4],
}

impl ResNet {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            stem: Stem::new(&(path / "stem"))?,
            layers: [
                ResNetLayer::new(&(path / "layer1"), 64, 64, 256, 3, 1)?,
                ResNetLayer::new(&(path / "layer2"), 256, 128, 512, 4, 2)?,
                ResNetLayer::new(&(path / "layer3"), 512, 256, 1024, 6, 2)?,
                ResNetLayer::new(&(path / "layer4"), 1024, 512, 2048, 3, 2)?,
            ],
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<[Tensor; 4]> {
        let mut hidden_states = self.stem.forward(input)?;
        let [c2, c3, c4, c5] = std::array::from_fn(|index| {
            hidden_states = self.layers[index].forward(&hidden_states)?;
            hidden_states.alias()
        });
        Ok([c2?, c3?, c4?, c5?])
    }
}

#[derive(Debug)]
struct Stem {
    conv1: nn::Conv2D,
    bn1: FrozenBatchNorm2d,
}

impl Stem {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            conv1: conv2d(&(path / "conv1"), 3, 64, 7, 2, 3, false)?,
            bn1: FrozenBatchNorm2d::new(&(path / "bn1"), 64)?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        self.bn1
            .forward(&self.conv1.forward(input)?)?
            .relu()?
            .max_pool2d([3, 3], [2, 2], [1, 1], [1, 1], false)
    }
}

#[derive(Debug)]
struct ResNetLayer(Vec<Bottleneck>);

impl ResNetLayer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        input_channels: i64,
        bottleneck_channels: i64,
        output_channels: i64,
        block_count: usize,
        first_stride: i64,
    ) -> Result<Self> {
        Ok(Self(
            (0..block_count)
                .map(|index| {
                    Bottleneck::new(
                        &(path / index),
                        if index == 0 {
                            input_channels
                        } else {
                            output_channels
                        },
                        bottleneck_channels,
                        output_channels,
                        if index == 0 { first_stride } else { 1 },
                    )
                })
                .collect::<Result<_>>()?,
        ))
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        self.0
            .iter()
            .try_fold(input.alias()?, |input, block| block.forward(&input))
    }
}

#[derive(Debug)]
struct Bottleneck {
    conv1: nn::Conv2D,
    bn1: FrozenBatchNorm2d,
    conv2: nn::Conv2D,
    bn2: FrozenBatchNorm2d,
    conv3: nn::Conv2D,
    bn3: FrozenBatchNorm2d,
    downsample: Option<(nn::Conv2D, FrozenBatchNorm2d)>,
}

impl Bottleneck {
    fn new(
        path: &nn::Path<'_>,
        input_channels: i64,
        bottleneck_channels: i64,
        output_channels: i64,
        stride: i64,
    ) -> Result<Self> {
        Ok(Self {
            conv1: conv2d(
                &(path / "conv1"),
                input_channels,
                bottleneck_channels,
                1,
                stride,
                0,
                false,
            )?,
            bn1: FrozenBatchNorm2d::new(&(path / "bn1"), bottleneck_channels)?,
            conv2: conv2d(
                &(path / "conv2"),
                bottleneck_channels,
                bottleneck_channels,
                3,
                1,
                1,
                false,
            )?,
            bn2: FrozenBatchNorm2d::new(&(path / "bn2"), bottleneck_channels)?,
            conv3: conv2d(
                &(path / "conv3"),
                bottleneck_channels,
                output_channels,
                1,
                1,
                0,
                false,
            )?,
            bn3: FrozenBatchNorm2d::new(&(path / "bn3"), output_channels)?,
            downsample: (input_channels != output_channels)
                .then(|| -> Result<_> {
                    Ok((
                        conv2d(
                            &(path / "downsample" / 0),
                            input_channels,
                            output_channels,
                            1,
                            stride,
                            0,
                            false,
                        )?,
                        FrozenBatchNorm2d::new(&(path / "downsample" / 1), output_channels)?,
                    ))
                })
                .transpose()?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = self.bn1.forward(&self.conv1.forward(input)?)?.relu()?;
        let hidden_states = self
            .bn2
            .forward(&self.conv2.forward(&hidden_states)?)?
            .relu()?;
        let hidden_states = self.bn3.forward(&self.conv3.forward(&hidden_states)?)?;
        let residual = match &self.downsample {
            Some((conv, norm)) => norm.forward(&conv.forward(input)?)?,
            None => input.alias()?,
        };
        (hidden_states + residual)?.relu()
    }
}

#[derive(Debug)]
struct FrozenBatchNorm2d {
    weight: Tensor,
    bias: Tensor,
    running_mean: Tensor,
    running_var: Tensor,
}

impl FrozenBatchNorm2d {
    fn new(path: &nn::Path<'_>, channels: i64) -> Result<Self> {
        Ok(Self {
            weight: path.var("weight", &[channels])?,
            bias: path.var("bias", &[channels])?,
            running_mean: path.var("running_mean", &[channels])?,
            running_var: path.var("running_var", &[channels])?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let scale = (&self.weight * self.running_var.rsqrt()?)?;
        let bias = (&self.bias - (&self.running_mean * &scale)?)?;
        (input * scale.view([1, -1, 1, 1])?)? + bias.view([1, -1, 1, 1])?
    }
}

#[derive(Debug)]
struct FeaturePyramidNetwork {
    inner: [nn::Conv2D; 4],
    layer: [nn::Conv2D; 4],
}

impl FeaturePyramidNetwork {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            inner: [
                conv2d(&(path / "fpn_inner1"), 256, 256, 1, 1, 0, true)?,
                conv2d(&(path / "fpn_inner2"), 512, 256, 1, 1, 0, true)?,
                conv2d(&(path / "fpn_inner3"), 1024, 256, 1, 1, 0, true)?,
                conv2d(&(path / "fpn_inner4"), 2048, 256, 1, 1, 0, true)?,
            ],
            layer: [
                conv2d(&(path / "fpn_layer1"), 256, 256, 3, 1, 1, true)?,
                conv2d(&(path / "fpn_layer2"), 256, 256, 3, 1, 1, true)?,
                conv2d(&(path / "fpn_layer3"), 256, 256, 3, 1, 1, true)?,
                conv2d(&(path / "fpn_layer4"), 256, 256, 3, 1, 1, true)?,
            ],
        })
    }

    fn forward(&self, features: &[Tensor; 4]) -> koharu_torch::Result<Vec<Tensor>> {
        let mut last_inner = self.inner[3].forward(&features[3])?;
        let mut results = vec![self.layer[3].forward(&last_inner)?];
        for index in (0..3).rev() {
            let size = last_inner.size();
            let top_down = last_inner.upsample_nearest2d(
                [size[2] * 2, size[3] * 2],
                None::<f64>,
                None::<f64>,
            )?;
            last_inner = (self.inner[index].forward(&features[index])? + top_down)?;
            results.insert(0, self.layer[index].forward(&last_inner)?);
        }
        let last = results[3].max_pool2d([1, 1], [2, 2], [0, 0], [1, 1], false)?;
        results.push(last);
        Ok(results)
    }
}

#[derive(Debug)]
struct SegmentationHead {
    fpn_out5: nn::Conv2D,
    fpn_out4: nn::Conv2D,
    fpn_out3: nn::Conv2D,
    fpn_out2: nn::Conv2D,
    seg_conv: nn::Conv2D,
    seg_bn1: nn::BatchNorm,
    seg_deconv1: nn::ConvTranspose2D,
    seg_bn2: nn::BatchNorm,
    seg_deconv2: nn::ConvTranspose2D,
}

impl SegmentationHead {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            fpn_out5: conv2d(&(path / "fpn_out5" / 0), 256, 64, 3, 1, 1, false)?,
            fpn_out4: conv2d(&(path / "fpn_out4" / 0), 256, 64, 3, 1, 1, false)?,
            fpn_out3: conv2d(&(path / "fpn_out3" / 0), 256, 64, 3, 1, 1, false)?,
            fpn_out2: conv2d(&(path / "fpn_out2"), 256, 64, 3, 1, 1, false)?,
            seg_conv: conv2d(&(path / "seg_out" / 0 / 0), 256, 64, 3, 1, 1, false)?,
            seg_bn1: nn::batch_norm2d(path / "seg_out" / 0 / 1, 64, Default::default())?,
            seg_deconv1: nn::conv_transpose2d(
                path / "seg_out" / 1,
                64,
                64,
                2,
                nn::ConvTransposeConfig {
                    stride: 2,
                    ..Default::default()
                },
            )?,
            seg_bn2: nn::batch_norm2d(path / "seg_out" / 2, 64, Default::default())?,
            seg_deconv2: nn::conv_transpose2d(
                path / "seg_out" / 4,
                64,
                1,
                2,
                nn::ConvTransposeConfig {
                    stride: 2,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, features: &[Tensor]) -> koharu_torch::Result<Tensor> {
        let resize = |input: Tensor, scale: i64| {
            let size = input.size();
            input.upsample_nearest2d([size[2] * scale, size[3] * scale], None::<f64>, None::<f64>)
        };
        let p5 = resize(self.fpn_out5.forward(&features[3])?, 8)?;
        let p4 = resize(self.fpn_out4.forward(&features[2])?, 4)?;
        let p3 = resize(self.fpn_out3.forward(&features[1])?, 2)?;
        let p2 = self.fpn_out2.forward(&features[0])?;
        let fused = Tensor::cat(&[p5, p4, p3, p2], 1)?;
        let hidden_states = self
            .seg_bn1
            .forward_t(&self.seg_conv.forward(&fused)?, false)?
            .relu()?;
        let hidden_states = self
            .seg_bn2
            .forward_t(&self.seg_deconv1.forward(&hidden_states)?, false)?
            .relu()?;
        self.seg_deconv2.forward(&hidden_states)?.sigmoid()
    }
}

fn conv2d(
    path: &nn::Path<'_>,
    input_channels: i64,
    output_channels: i64,
    kernel_size: i64,
    stride: i64,
    padding: i64,
    bias: bool,
) -> koharu_torch::Result<nn::Conv2D> {
    nn::conv2d(
        path,
        input_channels,
        output_channels,
        kernel_size,
        nn::ConvConfig {
            stride,
            padding,
            bias,
            ..Default::default()
        },
    )
}
