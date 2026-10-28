//! Shared PP-LCNetV4 backbone used by the PP-OCRv6 detector and recognizer.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/pp_lcnet_v4/modeling_pp_lcnet_v4.py

use crate::nn::{self, Module, ModuleT};
use serde::{Deserialize, Deserializer};
use tch::Tensor;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub(crate) enum Spatial {
    Scalar(i64),
    Pair([i64; 2]),
}

impl Spatial {
    pub(crate) fn pair(self) -> [i64; 2] {
        match self {
            Self::Scalar(value) => [value, value],
            Self::Pair(value) => value,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BlockConfig {
    pub kernel_size: i64,
    pub in_channels: i64,
    pub out_channels: i64,
    pub stride: Spatial,
    pub use_squeeze_excitation: bool,
}

impl<'de> Deserialize<'de> for BlockConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (kernel_size, in_channels, out_channels, stride, use_squeeze_excitation) =
            <(i64, i64, i64, Spatial, bool)>::deserialize(deserializer)?;
        Ok(Self {
            kernel_size,
            in_channels,
            out_channels,
            stride,
            use_squeeze_excitation,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PPLCNetV4Config {
    pub scale: f64,
    pub block_configs: Vec<Vec<BlockConfig>>,
    pub stem_channels: Vec<i64>,
    pub reduction: i64,
    pub hidden_act: String,
    pub out_features: Vec<String>,
    pub out_indices: Vec<usize>,
    pub num_channels: i64,
    pub stem_strides: Vec<Spatial>,
    pub stem_type: String,
    pub use_learnable_affine_block: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PPLCNetV4ConfigFields {
    scale: Option<f64>,
    block_configs: Option<Vec<Vec<BlockConfig>>>,
    stem_channels: Option<Vec<i64>>,
    reduction: Option<i64>,
    hidden_act: Option<String>,
    #[serde(alias = "_out_features")]
    out_features: Option<Vec<String>>,
    #[serde(alias = "_out_indices")]
    out_indices: Option<Vec<usize>>,
    num_channels: Option<i64>,
    stem_strides: Option<Vec<Spatial>>,
    stem_type: Option<String>,
    use_learnable_affine_block: Option<bool>,
}

impl<'de> Deserialize<'de> for PPLCNetV4Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = PPLCNetV4ConfigFields::deserialize(deserializer)?;
        let mut config = Self::default();
        if let Some(value) = fields.scale {
            config.scale = value;
        }
        if let Some(value) = fields.block_configs {
            config.block_configs = value;
        }
        if let Some(value) = fields.stem_channels {
            config.stem_channels = value;
        }
        if let Some(value) = fields.reduction {
            config.reduction = value;
        }
        if let Some(value) = fields.hidden_act {
            config.hidden_act = value;
        }
        if let Some(value) = fields.num_channels {
            config.num_channels = value;
        }
        if let Some(value) = fields.stem_strides {
            config.stem_strides = value;
        }
        if let Some(value) = fields.stem_type {
            config.stem_type = value;
        }
        if let Some(value) = fields.use_learnable_affine_block {
            config.use_learnable_affine_block = value;
        }

        let stage_count = config.block_configs.len();
        match (fields.out_features, fields.out_indices) {
            (Some(features), Some(indices)) => {
                let derived = indices
                    .iter()
                    .map(|&index| stage_name(index, stage_count))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| serde::de::Error::custom("invalid PP-LCNetV4 out_indices"))?;
                if features != derived {
                    return Err(serde::de::Error::custom(
                        "PP-LCNetV4 out_features and out_indices do not select the same stages",
                    ));
                }
                config.out_features = features;
                config.out_indices = indices;
            }
            (Some(features), None) => {
                config.out_indices = features
                    .iter()
                    .map(|feature| stage_index(feature, stage_count))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| serde::de::Error::custom("invalid PP-LCNetV4 out_features"))?;
                config.out_features = features;
            }
            (None, Some(indices)) => {
                config.out_features = indices
                    .iter()
                    .map(|&index| stage_name(index, stage_count))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| serde::de::Error::custom("invalid PP-LCNetV4 out_indices"))?;
                config.out_indices = indices;
            }
            (None, None) => {}
        }
        Ok(config)
    }
}

fn stage_name(index: usize, stage_count: usize) -> Option<String> {
    match index {
        0 => Some("stem".into()),
        index if index <= stage_count => Some(format!("stage{index}")),
        _ => None,
    }
}

fn stage_index(name: &str, stage_count: usize) -> Option<usize> {
    if name == "stem" {
        return Some(0);
    }
    name.strip_prefix("stage")
        .and_then(|index| index.parse().ok())
        .filter(|&index| index > 0 && index <= stage_count)
}

impl Default for PPLCNetV4Config {
    fn default() -> Self {
        let block_configs = vec![
            vec![(3, 96, 96, Spatial::Scalar(1), true)],
            vec![
                (3, 96, 96, Spatial::Scalar(1), false),
                (3, 96, 96, Spatial::Scalar(1), false),
            ],
            vec![
                (3, 96, 192, Spatial::Pair([2, 1]), false),
                (3, 192, 192, Spatial::Scalar(1), true),
                (3, 192, 192, Spatial::Scalar(1), false),
                (3, 192, 192, Spatial::Scalar(1), true),
                (3, 192, 192, Spatial::Scalar(1), false),
                (3, 192, 192, Spatial::Scalar(1), true),
                (3, 192, 192, Spatial::Scalar(1), false),
            ],
            vec![
                (3, 192, 384, Spatial::Pair([2, 1]), false),
                (3, 384, 384, Spatial::Scalar(1), true),
                (3, 384, 384, Spatial::Scalar(1), false),
            ],
        ]
        .into_iter()
        .map(|stage| {
            stage
                .into_iter()
                .map(
                    |(kernel_size, in_channels, out_channels, stride, use_squeeze_excitation)| {
                        BlockConfig {
                            kernel_size,
                            in_channels,
                            out_channels,
                            stride,
                            use_squeeze_excitation,
                        }
                    },
                )
                .collect()
        })
        .collect();
        Self {
            scale: 1.0,
            block_configs,
            stem_channels: vec![3, 48, 96],
            reduction: 4,
            hidden_act: "relu".into(),
            out_features: vec!["stage4".into()],
            out_indices: vec![4],
            num_channels: 3,
            stem_strides: vec![
                Spatial::Scalar(2),
                Spatial::Scalar(1),
                Spatial::Scalar(1),
                Spatial::Scalar(2),
                Spatial::Scalar(1),
            ],
            stem_type: "large".into(),
            use_learnable_affine_block: false,
        }
    }
}

impl PPLCNetV4Config {
    pub(crate) fn stage_out_channels(&self) -> Vec<i64> {
        self.block_configs
            .iter()
            .filter_map(|blocks| blocks.last())
            .map(|block| block.out_channels)
            .collect()
    }
}

fn activate(hidden_states: Tensor, activation: Option<&str>) -> tch::Result<Tensor> {
    match activation {
        None => Ok(hidden_states),
        Some("relu") => hidden_states.f_relu(),
        Some("gelu") => hidden_states.f_gelu("none"),
        Some("silu") => hidden_states.f_silu(),
        Some(name) => panic!("unsupported PP-LCNetV4 activation {name}"),
    }
}

#[derive(Debug)]
struct ConvLayer {
    convolution: nn::Conv2D,
    normalization: nn::BatchNorm,
    activation: Option<String>,
    learnable_affine: Option<LearnableAffineBlock>,
}

#[derive(Debug)]
struct LearnableAffineBlock {
    scale: Tensor,
    bias: Tensor,
}

impl LearnableAffineBlock {
    fn new(path: &nn::Path<'_>) -> tch::Result<Self> {
        Ok(Self {
            scale: path.var("scale", &[1])?,
            bias: path.var("bias", &[1])?,
        })
    }

    fn forward(&self, hidden_states: Tensor) -> tch::Result<Tensor> {
        (self.scale.f_mul(&hidden_states)?).f_add(&self.bias)
    }
}

impl ConvLayer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: [i64; 2],
        stride: [i64; 2],
        groups: i64,
        activation: Option<&str>,
        use_learnable_affine_block: bool,
    ) -> tch::Result<Self> {
        Ok(Self {
            convolution: nn::conv(
                path / "convolution",
                in_channels,
                out_channels,
                kernel_size,
                nn::ConvConfigND {
                    stride,
                    padding: [(kernel_size[0] - 1) / 2, (kernel_size[1] - 1) / 2],
                    groups,
                    bias: false,
                    ..Default::default()
                },
            )?,
            normalization: nn::batch_norm2d(
                path / "normalization",
                out_channels,
                Default::default(),
            )?,
            activation: activation.map(str::to_owned),
            learnable_affine: (activation.is_some() && use_learnable_affine_block)
                .then(|| LearnableAffineBlock::new(&(path / "lab")))
                .transpose()?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = self.convolution.forward(input)?;
        let hidden_states = self.normalization.forward_t(&hidden_states, false)?;
        let hidden_states = activate(hidden_states, self.activation.as_deref())?;
        match &self.learnable_affine {
            Some(learnable_affine) => learnable_affine.forward(hidden_states),
            None => Ok(hidden_states),
        }
    }
}

#[derive(Debug)]
struct SqueezeExcitationModule {
    convolution_0: nn::Conv2D,
    convolution_2: nn::Conv2D,
}

impl SqueezeExcitationModule {
    fn new(path: &nn::Path<'_>, channels: i64, reduction: i64) -> tch::Result<Self> {
        Ok(Self {
            convolution_0: nn::conv2d(
                path / "convolutions" / 0,
                channels,
                channels / reduction,
                1,
                Default::default(),
            )?,
            convolution_2: nn::conv2d(
                path / "convolutions" / 2,
                channels / reduction,
                channels,
                1,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let weights = hidden_states.f_adaptive_avg_pool2d([1, 1])?;
        let weights = self.convolution_0.forward(&weights)?.f_relu()?;
        let weights = self.convolution_2.forward(&weights)?.f_hardsigmoid()?;
        hidden_states.f_mul(&weights)
    }
}

#[derive(Debug)]
enum TokenConv {
    Reparameterized(nn::Conv2D),
    Layer(ConvLayer),
}

impl TokenConv {
    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        match self {
            Self::Reparameterized(convolution) => convolution.forward(hidden_states),
            Self::Layer(layer) => layer.forward(hidden_states),
        }
    }
}

#[derive(Debug)]
struct DepthwiseSeparableConvLayer {
    token_conv: TokenConv,
    token_squeeze_excitation: Option<SqueezeExcitationModule>,
    channel_conv1: ConvLayer,
    channel_conv2: ConvLayer,
    has_residual: bool,
}

impl DepthwiseSeparableConvLayer {
    fn new(
        path: &nn::Path<'_>,
        block: &BlockConfig,
        config: &PPLCNetV4Config,
    ) -> tch::Result<Self> {
        let stride = block.stride.pair();
        let in_channels = block.in_channels;
        let out_channels = block.out_channels;
        let use_rep_dw = stride == [1, 1] && in_channels == out_channels;
        let token_conv = if use_rep_dw {
            TokenConv::Reparameterized(nn::conv(
                path / "token_conv",
                in_channels,
                out_channels,
                [block.kernel_size, block.kernel_size],
                nn::ConvConfigND {
                    stride,
                    padding: [block.kernel_size / 2, block.kernel_size / 2],
                    groups: in_channels,
                    ..Default::default()
                },
            )?)
        } else {
            TokenConv::Layer(ConvLayer::new(
                &(path / "token_conv"),
                in_channels,
                in_channels,
                [block.kernel_size, block.kernel_size],
                stride,
                in_channels,
                None,
                false,
            )?)
        };
        Ok(Self {
            token_conv,
            token_squeeze_excitation: block
                .use_squeeze_excitation
                .then(|| {
                    SqueezeExcitationModule::new(
                        &(path / "token_squeeze_excitation"),
                        in_channels,
                        config.reduction,
                    )
                })
                .transpose()?,
            channel_conv1: ConvLayer::new(
                &(path / "channel_conv1"),
                in_channels,
                in_channels * 2,
                [1, 1],
                [1, 1],
                1,
                None,
                false,
            )?,
            channel_conv2: ConvLayer::new(
                &(path / "channel_conv2"),
                in_channels * 2,
                out_channels,
                [1, 1],
                [1, 1],
                1,
                None,
                false,
            )?,
            has_residual: in_channels == out_channels && stride == [1, 1],
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let mut hidden_states = self.token_conv.forward(hidden_states)?;
        if let Some(squeeze_excitation) = &self.token_squeeze_excitation {
            hidden_states = squeeze_excitation.forward(&hidden_states)?;
        }
        let residual = hidden_states.f_alias()?;
        hidden_states = self.channel_conv1.forward(&hidden_states)?.f_gelu("none")?;
        hidden_states = self.channel_conv2.forward(&hidden_states)?;
        if self.has_residual {
            hidden_states.f_add(&residual)
        } else {
            Ok(hidden_states)
        }
    }
}

#[derive(Debug)]
struct LargeStem {
    stem1: ConvLayer,
    stem2a: ConvLayer,
    stem2b: ConvLayer,
    stem3: ConvLayer,
    stem4: ConvLayer,
}

impl LargeStem {
    fn new(path: &nn::Path<'_>, config: &PPLCNetV4Config) -> tch::Result<Self> {
        let channels = &config.stem_channels;
        let strides = &config.stem_strides;
        Ok(Self {
            stem1: ConvLayer::new(
                &(path / "stem1"),
                channels[0],
                channels[1],
                [3, 3],
                strides[0].pair(),
                1,
                Some(&config.hidden_act),
                config.use_learnable_affine_block,
            )?,
            stem2a: ConvLayer::new(
                &(path / "stem2a"),
                channels[1],
                channels[1] / 2,
                [2, 2],
                strides[1].pair(),
                1,
                Some(&config.hidden_act),
                config.use_learnable_affine_block,
            )?,
            stem2b: ConvLayer::new(
                &(path / "stem2b"),
                channels[1] / 2,
                channels[1],
                [2, 2],
                strides[2].pair(),
                1,
                Some(&config.hidden_act),
                config.use_learnable_affine_block,
            )?,
            stem3: ConvLayer::new(
                &(path / "stem3"),
                channels[1] * 2,
                channels[1],
                [3, 3],
                strides[3].pair(),
                1,
                Some(&config.hidden_act),
                config.use_learnable_affine_block,
            )?,
            stem4: ConvLayer::new(
                &(path / "stem4"),
                channels[1],
                channels[2],
                [1, 1],
                strides[4].pair(),
                1,
                Some(&config.hidden_act),
                config.use_learnable_affine_block,
            )?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        let embedding = self.stem1.forward(pixel_values)?;
        let padded = embedding.f_constant_pad_nd([0, 1, 0, 1])?;
        let stem_2a = self.stem2a.forward(&padded)?;
        let stem_2a = self
            .stem2b
            .forward(&stem_2a.f_constant_pad_nd([0, 1, 0, 1])?)?;
        let pooled = padded.f_max_pool2d([2, 2], [1, 1], [0, 0], [1, 1], true)?;
        let embedding = Tensor::f_cat(&[pooled, stem_2a], 1)?;
        self.stem4.forward(&self.stem3.forward(&embedding)?)
    }
}

#[derive(Debug)]
struct SmallStem {
    conv1: ConvLayer,
    conv2: ConvLayer,
}

impl SmallStem {
    fn new(path: &nn::Path<'_>, config: &PPLCNetV4Config) -> tch::Result<Self> {
        let channels = &config.stem_channels;
        Ok(Self {
            conv1: ConvLayer::new(
                &(path / "conv1"),
                channels[0],
                channels[1],
                [3, 3],
                [2, 2],
                1,
                None,
                false,
            )?,
            conv2: ConvLayer::new(
                &(path / "conv2"),
                channels[1],
                channels[2],
                [3, 3],
                [2, 2],
                1,
                None,
                false,
            )?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        self.conv2
            .forward(&self.conv1.forward(pixel_values)?.f_gelu("none")?)
    }
}

#[derive(Debug)]
enum Stem {
    Large(Box<LargeStem>),
    Small(Box<SmallStem>),
}

impl Stem {
    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        match self {
            Self::Large(stem) => stem.forward(pixel_values),
            Self::Small(stem) => stem.forward(pixel_values),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PPLCNetV4Backbone {
    stem: Stem,
    blocks: Vec<Vec<DepthwiseSeparableConvLayer>>,
    out_features: Vec<String>,
    num_channels: i64,
}

impl PPLCNetV4Backbone {
    pub(crate) fn new(path: &nn::Path<'_>, config: &PPLCNetV4Config) -> tch::Result<Self> {
        let stem_path = path / "encoder" / "convolution";
        let stem = match config.stem_type.as_str() {
            "large" => Stem::Large(Box::new(LargeStem::new(&stem_path, config)?)),
            "small" => Stem::Small(Box::new(SmallStem::new(&stem_path, config)?)),
            stem_type => panic!("unsupported PP-LCNetV4 stem type {stem_type}"),
        };
        let blocks = config
            .block_configs
            .iter()
            .enumerate()
            .map(|(stage_index, blocks)| {
                blocks
                    .iter()
                    .enumerate()
                    .map(|(block_index, block)| {
                        DepthwiseSeparableConvLayer::new(
                            &(path / "encoder" / "blocks" / stage_index / "blocks" / block_index),
                            block,
                            config,
                        )
                    })
                    .collect::<tch::Result<_>>()
            })
            .collect::<tch::Result<_>>()?;
        Ok(Self {
            stem,
            blocks,
            out_features: config.out_features.clone(),
            num_channels: config.num_channels,
        })
    }

    pub(crate) fn forward(&self, pixel_values: &Tensor) -> tch::Result<Vec<Tensor>> {
        assert_eq!(
            pixel_values.size().get(1).copied(),
            Some(self.num_channels),
            "PP-LCNetV4 input channel count does not match the configuration"
        );
        let mut hidden_states = self.stem.forward(pixel_values)?;
        let mut feature_maps = Vec::new();
        if self.out_features.iter().any(|feature| feature == "stem") {
            feature_maps.push(hidden_states.f_alias()?);
        }
        for (stage_index, blocks) in self.blocks.iter().enumerate() {
            for block in blocks {
                hidden_states = block.forward(&hidden_states)?;
            }
            let stage = format!("stage{}", stage_index + 1);
            if self.out_features.iter().any(|feature| feature == &stage) {
                feature_maps.push(hidden_states.f_alias()?);
            }
        }
        Ok(feature_maps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_output_features_from_output_indices() {
        let config: PPLCNetV4Config = serde_json::from_str(r#"{"out_indices":[1,3]}"#).unwrap();
        assert_eq!(config.out_features, ["stage1", "stage3"]);
        assert_eq!(config.out_indices, [1, 3]);
    }

    #[test]
    fn rejects_inconsistent_output_features_and_indices() {
        let config = serde_json::from_str::<PPLCNetV4Config>(
            r#"{"out_features":["stage1"],"out_indices":[2]}"#,
        );
        assert!(config.is_err());
    }
}
