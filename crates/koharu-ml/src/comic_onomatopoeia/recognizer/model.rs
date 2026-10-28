//! Inference-only port of COO's reported-best TRBA + rotation + SAR + HardROI-half + 2D model.
//!
//! The network and execution order follow these pinned upstream files:
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/model.py
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/modules/transformation.py
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/modules/feature_extraction.py
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/modules/sequence_modeling.py
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/modules/prediction.py

use std::path::Path;

use anyhow::{Context, Result};
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module, ModuleT, RNN},
};

use super::config::Config;

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    transformation: TpsSpatialTransformerNetwork,
    feature_extraction: ResNet,
    sequence_modeling: [BidirectionalLstm; 2],
    prediction: Attention,
    two_dimensional: bool,
}

impl Model {
    pub(super) fn new(config: &Config, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = &vs.root() / "module";
        let transformation = TpsSpatialTransformerNetwork::new(
            &(&root / "Transformation"),
            config.num_fiducial,
            config.image_height,
            config.image_width,
            config.input_channels,
            device,
            vs.kind(),
        )?;
        let feature_extraction = ResNet::new(
            &(&root / "FeatureExtraction" / "ConvNet"),
            config.input_channels,
            config.output_channels,
        )?;
        let sequence_modeling = [
            BidirectionalLstm::new(
                &(&root / "SequenceModeling" / 0),
                config.output_channels,
                config.hidden_size,
                config.hidden_size,
            )?,
            BidirectionalLstm::new(
                &(&root / "SequenceModeling" / 1),
                config.hidden_size,
                config.hidden_size,
                config.hidden_size,
            )?,
        ];
        let prediction = Attention::new(
            &(&root / "Prediction"),
            config.hidden_size,
            config.hidden_size,
            config.num_classes,
            config.batch_max_length,
        )?;
        Ok(Self {
            vs,
            transformation,
            feature_extraction,
            sequence_modeling,
            prediction,
            two_dimensional: config.two_dimensional,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub(super) fn forward(&self, image: &Tensor) -> koharu_torch::Result<Tensor> {
        let image = self
            .transformation
            .forward(&image.to_kind(self.vs.kind())?)?;
        let visual_feature = self.feature_extraction.forward(&image)?;
        let visual_feature = if self.two_dimensional {
            let size = visual_feature.size();
            visual_feature
                .view([size[0], size[1], -1])?
                .permute([0, 2, 1])?
        } else {
            let visual_feature = visual_feature.permute([0, 3, 1, 2])?;
            let channels = visual_feature.size()[2];
            visual_feature
                .adaptive_avg_pool2d([channels, 1])?
                .squeeze_dim(3)?
        };
        let contextual_feature = self
            .sequence_modeling
            .iter()
            .try_fold(visual_feature, |input, layer| layer.forward(&input))?;
        self.prediction.forward(&contextual_feature)
    }
}

#[derive(Debug)]
struct TpsSpatialTransformerNetwork {
    localization_network: LocalizationNetwork,
    grid_generator: GridGenerator,
    image_height: i64,
    image_width: i64,
}

impl TpsSpatialTransformerNetwork {
    fn new(
        path: &nn::Path<'_>,
        num_fiducial: i64,
        image_height: i64,
        image_width: i64,
        input_channels: i64,
        device: Device,
        kind: Kind,
    ) -> Result<Self> {
        Ok(Self {
            localization_network: LocalizationNetwork::new(
                &(path / "LocalizationNetwork"),
                num_fiducial,
                input_channels,
            )?,
            grid_generator: GridGenerator::new(
                num_fiducial,
                image_height,
                image_width,
                device,
                kind,
            )?,
            image_height,
            image_width,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let control_points = self.localization_network.forward(input)?;
        let grid = self.grid_generator.forward(&control_points)?.view([
            input.size()[0],
            self.image_height,
            self.image_width,
            2,
        ])?;
        // interpolation=bilinear, padding=border, align_corners=True.
        input.grid_sampler_2d(&grid, 0, 1, true)
    }
}

#[derive(Debug)]
struct LocalizationNetwork {
    conv1: nn::Conv2D,
    bn1: nn::BatchNorm,
    conv2: nn::Conv2D,
    bn2: nn::BatchNorm,
    conv3: nn::Conv2D,
    bn3: nn::BatchNorm,
    conv4: nn::Conv2D,
    bn4: nn::BatchNorm,
    localization_fc1: nn::Linear,
    localization_fc2: nn::Linear,
    num_fiducial: i64,
}

impl LocalizationNetwork {
    fn new(path: &nn::Path<'_>, num_fiducial: i64, input_channels: i64) -> Result<Self> {
        let conv = path / "conv";
        Ok(Self {
            conv1: conv2d(&(&conv / 0), input_channels, 64, 3, 1, 1, false)?,
            bn1: batch_norm(&(&conv / 1), 64)?,
            conv2: conv2d(&(&conv / 4), 64, 128, 3, 1, 1, false)?,
            bn2: batch_norm(&(&conv / 5), 128)?,
            conv3: conv2d(&(&conv / 8), 128, 256, 3, 1, 1, false)?,
            bn3: batch_norm(&(&conv / 9), 256)?,
            conv4: conv2d(&(&conv / 12), 256, 512, 3, 1, 1, false)?,
            bn4: batch_norm(&(&conv / 13), 512)?,
            localization_fc1: nn::linear(
                path / "localization_fc1" / 0,
                512,
                256,
                Default::default(),
            )?,
            localization_fc2: nn::linear(
                path / "localization_fc2",
                256,
                num_fiducial * 2,
                Default::default(),
            )?,
            num_fiducial,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let mut hidden_states = self
            .bn1
            .forward_t(&self.conv1.forward(input)?, false)?
            .relu()?;
        hidden_states = hidden_states.max_pool2d([2, 2], [2, 2], [0, 0], [1, 1], false)?;
        hidden_states = self
            .bn2
            .forward_t(&self.conv2.forward(&hidden_states)?, false)?
            .relu()?;
        hidden_states = hidden_states.max_pool2d([2, 2], [2, 2], [0, 0], [1, 1], false)?;
        hidden_states = self
            .bn3
            .forward_t(&self.conv3.forward(&hidden_states)?, false)?
            .relu()?;
        hidden_states = hidden_states.max_pool2d([2, 2], [2, 2], [0, 0], [1, 1], false)?;
        hidden_states = self
            .bn4
            .forward_t(&self.conv4.forward(&hidden_states)?, false)?
            .relu()?
            .adaptive_avg_pool2d([1, 1])?;
        let hidden_states = self
            .localization_fc1
            .forward(&hidden_states.flatten(1, -1)?)?
            .relu()?;
        self.localization_fc2
            .forward(&hidden_states)?
            .view([input.size()[0], self.num_fiducial, 2])
    }
}

#[derive(Debug)]
struct GridGenerator {
    inverse_delta_c: Tensor,
    p_hat: Tensor,
}

impl GridGenerator {
    fn new(
        num_fiducial: i64,
        image_height: i64,
        image_width: i64,
        device: Device,
        kind: Kind,
    ) -> Result<Self> {
        let half = num_fiducial / 2;
        let mut control_points = Vec::with_capacity((num_fiducial * 2) as usize);
        for y in [-1.0_f64, 1.0] {
            for index in 0..half {
                control_points.push(-1.0 + 2.0 * index as f64 / (half - 1) as f64);
                control_points.push(y);
            }
        }

        let delta_size = num_fiducial + 3;
        let mut delta_c = vec![0.0_f64; (delta_size * delta_size) as usize];
        let set = |values: &mut [f64], row: i64, column: i64, value: f64| {
            values[(row * delta_size + column) as usize] = value;
        };
        for i in 0..num_fiducial {
            set(&mut delta_c, i, 0, 1.0);
            set(&mut delta_c, i, 1, control_points[(i * 2) as usize]);
            set(&mut delta_c, i, 2, control_points[(i * 2 + 1) as usize]);
            for j in 0..num_fiducial {
                let dx = control_points[(i * 2) as usize] - control_points[(j * 2) as usize];
                let dy =
                    control_points[(i * 2 + 1) as usize] - control_points[(j * 2 + 1) as usize];
                let radius = (dx * dx + dy * dy).sqrt();
                let value = if i == j {
                    0.0
                } else {
                    radius * radius * radius.ln()
                };
                set(&mut delta_c, i, j + 3, value);
            }
        }
        for i in 0..num_fiducial {
            set(
                &mut delta_c,
                num_fiducial,
                i + 3,
                control_points[(i * 2) as usize],
            );
            set(
                &mut delta_c,
                num_fiducial + 1,
                i + 3,
                control_points[(i * 2 + 1) as usize],
            );
            set(&mut delta_c, num_fiducial + 2, i + 3, 1.0);
        }
        // Upstream inverts with NumPy, in float64 on the host; libtorch has no
        // LAPACK there in Koharu's build.
        let size = delta_size as usize;
        let inverse_delta_c = nalgebra::DMatrix::from_row_slice(size, size, &delta_c)
            .try_inverse()
            .context("the TPS fiducial matrix is singular")?;
        let inverse_delta_c = Tensor::from_slice(inverse_delta_c.transpose().as_slice())?
            .view([delta_size, delta_size])?
            .to_kind(kind)?
            .to_device(device)?;

        let mut p_hat = Vec::with_capacity((image_height * image_width * delta_size) as usize);
        for y in 0..image_height {
            let py = (-image_height + 2 * y + 1) as f64 / image_height as f64;
            for x in 0..image_width {
                let px = (-image_width + 2 * x + 1) as f64 / image_width as f64;
                p_hat.extend([1.0, px, py]);
                for index in 0..num_fiducial {
                    let dx = px - control_points[(index * 2) as usize];
                    let dy = py - control_points[(index * 2 + 1) as usize];
                    let radius = (dx * dx + dy * dy).sqrt();
                    p_hat.push(radius * radius * (radius + 1.0e-6).ln());
                }
            }
        }
        let p_hat = Tensor::from_slice(&p_hat)?
            .view([image_height * image_width, delta_size])?
            .to_kind(kind)?
            .to_device(device)?;
        Ok(Self {
            inverse_delta_c,
            p_hat,
        })
    }

    fn forward(&self, control_points: &Tensor) -> koharu_torch::Result<Tensor> {
        let batch_size = control_points.size()[0];
        let zeros = Tensor::zeros(
            [batch_size, 3, 2],
            (control_points.kind(), control_points.device()),
        )?;
        let control_points = Tensor::cat(&[control_points.alias()?, zeros], 1)?;
        let transform = self
            .inverse_delta_c
            .unsqueeze(0)?
            .repeat([batch_size, 1, 1])?
            .bmm(&control_points)?;
        self.p_hat
            .unsqueeze(0)?
            .repeat([batch_size, 1, 1])?
            .bmm(&transform)
    }
}

#[derive(Debug)]
struct ResNet {
    conv0_1: nn::Conv2D,
    bn0_1: nn::BatchNorm,
    conv0_2: nn::Conv2D,
    bn0_2: nn::BatchNorm,
    layer1: ResNetLayer,
    conv1: nn::Conv2D,
    bn1: nn::BatchNorm,
    layer2: ResNetLayer,
    conv2: nn::Conv2D,
    bn2: nn::BatchNorm,
    layer3: ResNetLayer,
    conv3: nn::Conv2D,
    bn3: nn::BatchNorm,
    layer4: ResNetLayer,
    conv4_1: nn::Conv2D,
    bn4_1: nn::BatchNorm,
    conv4_2: nn::Conv2D,
    bn4_2: nn::BatchNorm,
}

impl ResNet {
    fn new(path: &nn::Path<'_>, input_channels: i64, output_channels: i64) -> Result<Self> {
        Ok(Self {
            conv0_1: conv2d(
                &(path / "conv0_1"),
                input_channels,
                output_channels / 16,
                3,
                1,
                1,
                false,
            )?,
            bn0_1: batch_norm(&(path / "bn0_1"), output_channels / 16)?,
            conv0_2: conv2d(
                &(path / "conv0_2"),
                output_channels / 16,
                output_channels / 8,
                3,
                1,
                1,
                false,
            )?,
            bn0_2: batch_norm(&(path / "bn0_2"), output_channels / 8)?,
            layer1: ResNetLayer::new(
                &(path / "layer1"),
                output_channels / 8,
                output_channels / 4,
                1,
            )?,
            conv1: conv2d(
                &(path / "conv1"),
                output_channels / 4,
                output_channels / 4,
                3,
                1,
                1,
                false,
            )?,
            bn1: batch_norm(&(path / "bn1"), output_channels / 4)?,
            layer2: ResNetLayer::new(
                &(path / "layer2"),
                output_channels / 4,
                output_channels / 2,
                2,
            )?,
            conv2: conv2d(
                &(path / "conv2"),
                output_channels / 2,
                output_channels / 2,
                3,
                1,
                1,
                false,
            )?,
            bn2: batch_norm(&(path / "bn2"), output_channels / 2)?,
            layer3: ResNetLayer::new(&(path / "layer3"), output_channels / 2, output_channels, 5)?,
            conv3: conv2d(
                &(path / "conv3"),
                output_channels,
                output_channels,
                3,
                1,
                1,
                false,
            )?,
            bn3: batch_norm(&(path / "bn3"), output_channels)?,
            layer4: ResNetLayer::new(&(path / "layer4"), output_channels, output_channels, 3)?,
            conv4_1: conv2d_nd(
                &(path / "conv4_1"),
                output_channels,
                output_channels,
                [2, 2],
                [2, 1],
                [0, 1],
                false,
            )?,
            bn4_1: batch_norm(&(path / "bn4_1"), output_channels)?,
            conv4_2: conv2d(
                &(path / "conv4_2"),
                output_channels,
                output_channels,
                2,
                1,
                0,
                false,
            )?,
            bn4_2: batch_norm(&(path / "bn4_2"), output_channels)?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let mut hidden_states = self
            .bn0_1
            .forward_t(&self.conv0_1.forward(input)?, false)?
            .relu()?;
        hidden_states = self
            .bn0_2
            .forward_t(&self.conv0_2.forward(&hidden_states)?, false)?
            .relu()?;
        hidden_states = hidden_states.max_pool2d([2, 2], [2, 2], [0, 0], [1, 1], false)?;
        hidden_states = self.layer1.forward(&hidden_states)?;
        hidden_states = self
            .bn1
            .forward_t(&self.conv1.forward(&hidden_states)?, false)?
            .relu()?;
        hidden_states = hidden_states.max_pool2d([2, 2], [2, 2], [0, 0], [1, 1], false)?;
        hidden_states = self.layer2.forward(&hidden_states)?;
        hidden_states = self
            .bn2
            .forward_t(&self.conv2.forward(&hidden_states)?, false)?
            .relu()?;
        hidden_states = hidden_states.max_pool2d([2, 2], [2, 1], [0, 1], [1, 1], false)?;
        hidden_states = self.layer3.forward(&hidden_states)?;
        hidden_states = self
            .bn3
            .forward_t(&self.conv3.forward(&hidden_states)?, false)?
            .relu()?;
        hidden_states = self.layer4.forward(&hidden_states)?;
        hidden_states = self
            .bn4_1
            .forward_t(&self.conv4_1.forward(&hidden_states)?, false)?
            .relu()?;
        self.bn4_2
            .forward_t(&self.conv4_2.forward(&hidden_states)?, false)?
            .relu()
    }
}

#[derive(Debug)]
struct ResNetLayer(Vec<BasicBlock>);

impl ResNetLayer {
    fn new(
        path: &nn::Path<'_>,
        input_channels: i64,
        output_channels: i64,
        blocks: usize,
    ) -> Result<Self> {
        Ok(Self(
            (0..blocks)
                .map(|index| {
                    BasicBlock::new(
                        &(path / index),
                        if index == 0 {
                            input_channels
                        } else {
                            output_channels
                        },
                        output_channels,
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
struct BasicBlock {
    conv1: nn::Conv2D,
    bn1: nn::BatchNorm,
    conv2: nn::Conv2D,
    bn2: nn::BatchNorm,
    downsample: Option<(nn::Conv2D, nn::BatchNorm)>,
}

impl BasicBlock {
    fn new(path: &nn::Path<'_>, input_channels: i64, output_channels: i64) -> Result<Self> {
        Ok(Self {
            conv1: conv2d(
                &(path / "conv1"),
                input_channels,
                output_channels,
                3,
                1,
                1,
                false,
            )?,
            bn1: batch_norm(&(path / "bn1"), output_channels)?,
            conv2: conv2d(
                &(path / "conv2"),
                output_channels,
                output_channels,
                3,
                1,
                1,
                false,
            )?,
            bn2: batch_norm(&(path / "bn2"), output_channels)?,
            downsample: (input_channels != output_channels)
                .then(|| -> Result<_> {
                    Ok((
                        conv2d(
                            &(path / "downsample" / 0),
                            input_channels,
                            output_channels,
                            1,
                            1,
                            0,
                            false,
                        )?,
                        batch_norm(&(path / "downsample" / 1), output_channels)?,
                    ))
                })
                .transpose()?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = self
            .bn1
            .forward_t(&self.conv1.forward(input)?, false)?
            .relu()?;
        let hidden_states = self
            .bn2
            .forward_t(&self.conv2.forward(&hidden_states)?, false)?;
        let residual = match &self.downsample {
            Some((conv, norm)) => norm.forward_t(&conv.forward(input)?, false)?,
            None => input.alias()?,
        };
        (hidden_states + residual)?.relu()
    }
}

#[derive(Debug)]
struct BidirectionalLstm {
    rnn: nn::LSTM,
    linear: nn::Linear,
}

impl BidirectionalLstm {
    fn new(
        path: &nn::Path<'_>,
        input_size: i64,
        hidden_size: i64,
        output_size: i64,
    ) -> Result<Self> {
        Ok(Self {
            rnn: nn::lstm(
                path / "rnn",
                input_size,
                hidden_size,
                nn::RNNConfig {
                    bidirectional: true,
                    train: false,
                    ..Default::default()
                },
            )?,
            linear: nn::linear(
                path / "linear",
                hidden_size * 2,
                output_size,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let (recurrent, _) = self.rnn.seq(input)?;
        self.linear.forward(&recurrent)
    }
}

#[derive(Debug)]
struct Attention {
    attention_cell: AttentionCell,
    generator: nn::Linear,
    char_embeddings: nn::Embedding,
    hidden_size: i64,
    num_steps: i64,
}

impl Attention {
    fn new(
        path: &nn::Path<'_>,
        input_size: i64,
        hidden_size: i64,
        num_classes: i64,
        batch_max_length: i64,
    ) -> Result<Self> {
        Ok(Self {
            attention_cell: AttentionCell::new(
                &(path / "attention_cell"),
                input_size,
                hidden_size,
                256,
            )?,
            generator: nn::linear(
                path / "generator",
                hidden_size,
                num_classes,
                Default::default(),
            )?,
            char_embeddings: nn::embedding(
                path / "char_embeddings",
                num_classes,
                256,
                Default::default(),
            )?,
            hidden_size,
            num_steps: batch_max_length + 1,
        })
    }

    fn forward(&self, batch_h: &Tensor) -> koharu_torch::Result<Tensor> {
        let batch_size = batch_h.size()[0];
        let mut hidden = Tensor::zeros(
            [batch_size, self.hidden_size],
            (batch_h.kind(), batch_h.device()),
        )?;
        let mut cell = hidden.alias()?;
        let mut targets = Tensor::full([batch_size], 2, (Kind::Int64, batch_h.device()))?;
        let mut probabilities = Vec::with_capacity(self.num_steps as usize);
        for _ in 0..self.num_steps {
            let embeddings = self.char_embeddings.forward(&targets)?;
            (hidden, cell) = self
                .attention_cell
                .forward(&hidden, &cell, batch_h, &embeddings)?;
            let step = self.generator.forward(&hidden)?;
            targets = step.argmax(-1, false)?;
            probabilities.push(step);
        }
        Tensor::stack(&probabilities, 1)
    }
}

#[derive(Debug)]
struct AttentionCell {
    i2h: nn::Linear,
    h2h: nn::Linear,
    score: nn::Linear,
    rnn_weight_ih: Tensor,
    rnn_weight_hh: Tensor,
    rnn_bias_ih: Tensor,
    rnn_bias_hh: Tensor,
}

impl AttentionCell {
    fn new(
        path: &nn::Path<'_>,
        input_size: i64,
        hidden_size: i64,
        num_embeddings: i64,
    ) -> Result<Self> {
        let rnn = path / "rnn";
        Ok(Self {
            i2h: nn::linear(
                path / "i2h",
                input_size,
                hidden_size,
                nn::LinearConfig { bias: false },
            )?,
            h2h: nn::linear(path / "h2h", hidden_size, hidden_size, Default::default())?,
            score: nn::linear(
                path / "score",
                hidden_size,
                1,
                nn::LinearConfig { bias: false },
            )?,
            rnn_weight_ih: rnn.var("weight_ih", &[hidden_size * 4, input_size + num_embeddings])?,
            rnn_weight_hh: rnn.var("weight_hh", &[hidden_size * 4, hidden_size])?,
            rnn_bias_ih: rnn.var("bias_ih", &[hidden_size * 4])?,
            rnn_bias_hh: rnn.var("bias_hh", &[hidden_size * 4])?,
        })
    }

    fn forward(
        &self,
        previous_hidden: &Tensor,
        previous_cell: &Tensor,
        batch_h: &Tensor,
        char_embeddings: &Tensor,
    ) -> koharu_torch::Result<(Tensor, Tensor)> {
        let batch_h_projection = self.i2h.forward(batch_h)?;
        let previous_hidden_projection = self.h2h.forward(previous_hidden)?.unsqueeze(1)?;
        let energy = self
            .score
            .forward(&(batch_h_projection + previous_hidden_projection)?.tanh()?)?;
        let alpha = energy.softmax(1, Kind::Float)?.to_kind(batch_h.kind())?;
        let context = alpha.permute([0, 2, 1])?.bmm(batch_h)?.squeeze_dim(1)?;
        let input = Tensor::cat(&[context, char_embeddings.alias()?], 1)?;
        input.lstm_cell(
            &[previous_hidden, previous_cell],
            &self.rnn_weight_ih,
            &self.rnn_weight_hh,
            Some(&self.rnn_bias_ih),
            Some(&self.rnn_bias_hh),
        )
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

fn conv2d_nd(
    path: &nn::Path<'_>,
    input_channels: i64,
    output_channels: i64,
    kernel_size: [i64; 2],
    stride: [i64; 2],
    padding: [i64; 2],
    bias: bool,
) -> koharu_torch::Result<nn::Conv2D> {
    nn::conv(
        path,
        input_channels,
        output_channels,
        kernel_size,
        nn::ConvConfigND {
            stride,
            padding,
            bias,
            ..Default::default()
        },
    )
}

fn batch_norm(path: &nn::Path<'_>, channels: i64) -> koharu_torch::Result<nn::BatchNorm> {
    nn::batch_norm2d(path, channels, Default::default())
}
