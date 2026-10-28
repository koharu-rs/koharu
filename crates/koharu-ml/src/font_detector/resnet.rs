//! The ResNet-50 YuzuMarker builds on, ported from tch-rs's ResNet.
//!
//! Upstream implementation:
//! https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/src/vision/resnet.rs

use koharu_torch::{
    Result, Tensor,
    nn::{self, Conv2D, ModuleT},
};

fn conv2d(
    p: nn::Path,
    c_in: i64,
    c_out: i64,
    ksize: i64,
    padding: i64,
    stride: i64,
) -> Result<Conv2D> {
    let conv2d_cfg = nn::ConvConfig {
        stride,
        padding,
        bias: false,
        ..Default::default()
    };
    nn::conv2d(p, c_in, c_out, ksize, conv2d_cfg)
}

// Upstream returns `impl ModuleT` here and from `bottleneck_layer`; both are a
// `SequentialT`, which the modules holding them name.
fn downsample(p: nn::Path, c_in: i64, c_out: i64, stride: i64) -> Result<nn::SequentialT> {
    if stride != 1 || c_in != c_out {
        Ok(nn::seq_t()
            .add(conv2d(&p / "0", c_in, c_out, 1, 0, stride)?)
            .add(nn::batch_norm2d(&p / "1", c_out, Default::default())?))
    } else {
        Ok(nn::seq_t())
    }
}

// Upstream's `nn::func_t` closures become the forward passes of
// `BottleneckBlock` and `BottleneckResnet`.
#[derive(Debug)]
struct BottleneckBlock {
    conv1: Conv2D,
    bn1: nn::BatchNorm,
    conv2: Conv2D,
    bn2: nn::BatchNorm,
    conv3: Conv2D,
    bn3: nn::BatchNorm,
    downsample: nn::SequentialT,
}

impl ModuleT for BottleneckBlock {
    fn forward_t(&self, xs: &Tensor, train: bool) -> Result<Tensor> {
        let ys = xs
            .apply(&self.conv1)?
            .apply_t(&self.bn1, train)?
            .relu()?
            .apply(&self.conv2)?
            .apply_t(&self.bn2, train)?
            .relu()?
            .apply(&self.conv3)?
            .apply_t(&self.bn3, train)?;
        (xs.apply_t(&self.downsample, train)? + ys)?.relu()
    }
}

fn bottleneck_block(
    p: nn::Path,
    c_in: i64,
    c_out: i64,
    stride: i64,
    e: i64,
) -> Result<impl ModuleT + use<>> {
    let e_dim = e * c_out;
    let conv1 = conv2d(&p / "conv1", c_in, c_out, 1, 0, 1)?;
    let bn1 = nn::batch_norm2d(&p / "bn1", c_out, Default::default())?;
    let conv2 = conv2d(&p / "conv2", c_out, c_out, 3, 1, stride)?;
    let bn2 = nn::batch_norm2d(&p / "bn2", c_out, Default::default())?;
    let conv3 = conv2d(&p / "conv3", c_out, e_dim, 1, 0, 1)?;
    let bn3 = nn::batch_norm2d(&p / "bn3", e_dim, Default::default())?;
    let downsample = downsample(&p / "downsample", c_in, e_dim, stride)?;
    Ok(BottleneckBlock {
        conv1,
        bn1,
        conv2,
        bn2,
        conv3,
        bn3,
        downsample,
    })
}

fn bottleneck_layer(
    p: nn::Path,
    c_in: i64,
    c_out: i64,
    stride: i64,
    cnt: i64,
) -> Result<nn::SequentialT> {
    let mut layer = nn::seq_t().add(bottleneck_block(&p / "0", c_in, c_out, stride, 4)?);
    for block_index in 1..cnt {
        layer = layer.add(bottleneck_block(
            &p / &block_index.to_string(),
            4 * c_out,
            c_out,
            1,
            4,
        )?)
    }
    Ok(layer)
}

#[derive(Debug)]
struct BottleneckResnet {
    conv1: Conv2D,
    bn1: nn::BatchNorm,
    layer1: nn::SequentialT,
    layer2: nn::SequentialT,
    layer3: nn::SequentialT,
    layer4: nn::SequentialT,
    fc: Option<nn::Linear>,
}

impl ModuleT for BottleneckResnet {
    fn forward_t(&self, xs: &Tensor, train: bool) -> Result<Tensor> {
        xs.apply(&self.conv1)?
            .apply_t(&self.bn1, train)?
            .relu()?
            .max_pool2d([3, 3], [2, 2], [1, 1], [1, 1], false)?
            .apply_t(&self.layer1, train)?
            .apply_t(&self.layer2, train)?
            .apply_t(&self.layer3, train)?
            .apply_t(&self.layer4, train)?
            .adaptive_avg_pool2d([1, 1])?
            // Upstream's `flat_view`, `view((batch, -1))`, is not in koharu-torch;
            // `flatten(1, -1)` is the same reshape, as torchvision writes it.
            .flatten(1, -1)?
            .apply_opt(&self.fc)
    }
}

fn bottleneck_resnet(
    p: &nn::Path,
    nclasses: Option<i64>,
    c1: i64,
    c2: i64,
    c3: i64,
    c4: i64,
) -> Result<impl ModuleT + use<>> {
    let conv1 = conv2d(p / "conv1", 3, 64, 7, 3, 2)?;
    let bn1 = nn::batch_norm2d(p / "bn1", 64, Default::default())?;
    let layer1 = bottleneck_layer(p / "layer1", 64, 64, 1, c1)?;
    let layer2 = bottleneck_layer(p / "layer2", 4 * 64, 128, 2, c2)?;
    let layer3 = bottleneck_layer(p / "layer3", 4 * 128, 256, 2, c3)?;
    let layer4 = bottleneck_layer(p / "layer4", 4 * 256, 512, 2, c4)?;
    let fc = nclasses
        .map(|n| nn::linear(p / "fc", 4 * 512, n, Default::default()))
        .transpose()?;
    Ok(BottleneckResnet {
        conv1,
        bn1,
        layer1,
        layer2,
        layer3,
        layer4,
        fc,
    })
}

pub(super) fn resnet50(p: &nn::Path, num_classes: i64) -> Result<impl ModuleT + use<>> {
    bottleneck_resnet(p, Some(num_classes), 3, 4, 6, 3)
}
