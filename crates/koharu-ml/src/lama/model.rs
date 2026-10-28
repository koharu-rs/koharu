//! Inference-only port of LaMa's FFC generator.
//!
//! Original implementation:
//! https://github.com/advimman/lama/blob/786f5936b27fb3dacd2b1ad799e4de968ea697e7/saicinpainting/training/modules/ffc.py#L49-L369

use std::path::Path;

use crate::nn::{self, Module, ModuleT};
use anyhow::Result;
use tch::{Device, Kind, Tensor};

use super::config::FFCResNetGeneratorConfig;

#[derive(Debug)]
pub struct Model {
    vs: nn::VarStore,
    generator: FFCResNetGenerator,
}

impl Model {
    pub fn new(config: &FFCResNetGeneratorConfig, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let generator = FFCResNetGenerator::new(&vs.root(), config)?;
        Ok(Self { vs, generator })
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub fn forward(&self, image: &Tensor, mask: &Tensor) -> tch::Result<Tensor> {
        let image = image.f_to_kind(self.vs.kind())?;
        let mask = mask.f_to_kind(self.vs.kind())?;
        let inverse_mask = mask.f_ones_like()?.f_sub(&mask)?;
        let masked_image = image.f_mul(&inverse_mask)?;
        let input = Tensor::f_cat(&[masked_image, mask.f_alias()?], 1)?;
        let predicted = self.generator.forward(&input)?;
        (predicted.f_mul(&mask)?).f_add(&inverse_mask.f_mul(&image)?)
    }
}

#[derive(Debug)]
struct FFCResNetGenerator {
    initial: FFC_BN_ACT,
    downsample: Vec<FFC_BN_ACT>,
    blocks: Vec<FFCResnetBlock>,
    concat_tuple: ConcatTupleLayer,
    upsample: Vec<(nn::ConvTranspose2D, nn::BatchNorm)>,
    final_conv: nn::Conv2D,
}

impl FFCResNetGenerator {
    fn new(path: &nn::Path<'_>, config: &FFCResNetGeneratorConfig) -> Result<Self> {
        let initial = FFC_BN_ACT::new(
            &(path / "model" / 1),
            config.input_nc,
            config.ngf,
            7,
            1,
            0,
            config.init_conv_kwargs.ratio_gin,
            config.init_conv_kwargs.ratio_gout,
        )?;

        let downsample = (0..config.n_downsampling)
            .map(|idx| {
                let multiplier = 1_i64 << idx;
                let ratio_gout = if idx + 1 == config.n_downsampling {
                    config.resnet_conv_kwargs.ratio_gin
                } else {
                    config.downsample_conv_kwargs.ratio_gout
                };
                FFC_BN_ACT::new(
                    &(path / "model" / (idx + 2)),
                    config.max_features.min(config.ngf * multiplier),
                    config.max_features.min(config.ngf * multiplier * 2),
                    3,
                    2,
                    1,
                    config.downsample_conv_kwargs.ratio_gin,
                    ratio_gout,
                )
            })
            .collect::<Result<_>>()?;
        let bottleneck_channels = config
            .max_features
            .min(config.ngf * (1_i64 << config.n_downsampling));
        let blocks = (0..config.n_blocks)
            .map(|idx| {
                FFCResnetBlock::new(
                    &(path / "model" / (2 + config.n_downsampling + idx)),
                    bottleneck_channels,
                    config.resnet_conv_kwargs.ratio_gin,
                    config.resnet_conv_kwargs.ratio_gout,
                )
            })
            .collect::<Result<_>>()?;

        let first_upsample_index = 3 + config.n_downsampling + config.n_blocks;
        let mut upsample = Vec::new();
        for idx in 0..config.n_downsampling {
            let multiplier = 1_i64 << (config.n_downsampling - idx);
            let in_channels = config.max_features.min(config.ngf * multiplier);
            let out_channels = config.max_features.min(config.ngf * multiplier / 2);
            let module_index = first_upsample_index + idx * 3;
            upsample.push((
                nn::conv_transpose2d(
                    &(path / "model" / module_index),
                    in_channels,
                    out_channels,
                    3,
                    nn::ConvTransposeConfig {
                        stride: 2,
                        padding: 1,
                        output_padding: 1,
                        bias: true,
                        ..Default::default()
                    },
                )?,
                nn::batch_norm2d(
                    &(path / "model" / (module_index + 1)),
                    out_channels,
                    Default::default(),
                )?,
            ));
        }
        let final_conv_index = first_upsample_index + config.n_downsampling * 3 + 1;
        let final_conv = nn::conv2d(
            &(path / "model" / final_conv_index),
            config.ngf,
            config.output_nc,
            7,
            nn::ConvConfig {
                bias: true,
                ..Default::default()
            },
        )?;

        Ok(Self {
            initial,
            downsample,
            blocks,
            concat_tuple: ConcatTupleLayer,
            upsample,
            final_conv,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let input = input.f_reflection_pad2d([3, 3, 3, 3])?;
        let mut pair = self.initial.forward((input, None))?;
        for layer in &self.downsample {
            pair = layer.forward(pair)?;
        }
        for block in &self.blocks {
            pair = block.forward(pair)?;
        }

        let mut x = self.concat_tuple.forward(pair)?;
        for (conv, bn) in &self.upsample {
            x = bn.forward_t(&conv.forward(&x)?, false)?.f_relu()?;
        }
        self.final_conv
            .forward(&x.f_reflection_pad2d([3, 3, 3, 3])?)?
            .f_sigmoid()
    }
}

#[derive(Debug)]
struct FFCResnetBlock {
    conv1: FFC_BN_ACT,
    conv2: FFC_BN_ACT,
}

impl FFCResnetBlock {
    fn new(path: &nn::Path<'_>, channels: i64, ratio_gin: f64, ratio_gout: f64) -> Result<Self> {
        Ok(Self {
            conv1: FFC_BN_ACT::new(
                &(path / "conv1"),
                channels,
                channels,
                3,
                1,
                1,
                ratio_gin,
                ratio_gout,
            )?,
            conv2: FFC_BN_ACT::new(
                &(path / "conv2"),
                channels,
                channels,
                3,
                1,
                1,
                ratio_gin,
                ratio_gout,
            )?,
        })
    }

    fn forward(&self, input: (Tensor, Option<Tensor>)) -> tch::Result<(Tensor, Option<Tensor>)> {
        let id_l = input.0.f_alias()?;
        let id_g = input.1.as_ref().map(Tensor::f_alias).transpose()?;
        let output = self.conv2.forward(self.conv1.forward(input)?)?;
        Ok((
            output.0.f_add(&id_l)?,
            match (output.1, id_g) {
                (Some(output), Some(id)) => Some(output.f_add(&id)?),
                (Some(output), None) => Some(output),
                (None, Some(id)) => Some(id),
                (None, None) => None,
            },
        ))
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug)]
struct FFC_BN_ACT {
    ffc: FFC,
    bn_l: Option<nn::BatchNorm>,
    bn_g: Option<nn::BatchNorm>,
    act_l: bool,
    act_g: bool,
}

impl FFC_BN_ACT {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel: i64,
        stride: i64,
        padding: i64,
        ratio_gin: f64,
        ratio_gout: f64,
    ) -> Result<Self> {
        let global_channels = (out_channels as f64 * ratio_gout) as i64;
        let local_channels = out_channels - global_channels;
        let ffc = FFC::new(
            &(path / "ffc"),
            in_channels,
            out_channels,
            kernel,
            stride,
            padding,
            ratio_gin,
            ratio_gout,
        )?;
        let bn_l = (ratio_gout != 1.0)
            .then(|| nn::batch_norm2d(&(path / "bn_l"), local_channels, Default::default()))
            .transpose()?;
        let bn_g = (ratio_gout != 0.0)
            .then(|| nn::batch_norm2d(&(path / "bn_g"), global_channels, Default::default()))
            .transpose()?;
        Ok(Self {
            ffc,
            bn_l,
            bn_g,
            act_l: ratio_gout != 1.0,
            act_g: ratio_gout != 0.0,
        })
    }

    fn forward(&self, input: (Tensor, Option<Tensor>)) -> tch::Result<(Tensor, Option<Tensor>)> {
        let output = self.ffc.forward(input)?;
        let local = self
            .bn_l
            .as_ref()
            .map(|bn| bn.forward_t(&output.0, false))
            .transpose()?
            .unwrap_or(output.0);
        let local = if self.act_l { local.f_relu()? } else { local };

        let global = match (output.1, self.bn_g.as_ref()) {
            (Some(global), Some(bn)) => {
                let global = bn.forward_t(&global, false)?;
                Some(if self.act_g { global.f_relu()? } else { global })
            }
            (Some(global), None) => Some(global),
            (None, _) => None,
        };

        Ok((local, global))
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
struct FFC {
    convl2l: Option<nn::Conv2D>,
    convl2g: Option<nn::Conv2D>,
    convg2l: Option<nn::Conv2D>,
    convg2g: Option<SpectralTransform>,
}

impl FFC {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel: i64,
        stride: i64,
        padding: i64,
        ratio_gin: f64,
        ratio_gout: f64,
    ) -> Result<Self> {
        let in_cg = (in_channels as f64 * ratio_gin) as i64;
        let in_cl = in_channels - in_cg;
        let out_cg = (out_channels as f64 * ratio_gout) as i64;
        let out_cl = out_channels - out_cg;
        let convolution_config = nn::ConvConfig {
            stride,
            padding,
            bias: false,
            padding_mode: nn::PaddingMode::Reflect,
            ..Default::default()
        };

        let convl2l = (in_cl != 0 && out_cl != 0)
            .then(|| {
                nn::conv2d(
                    &(path / "convl2l"),
                    in_cl,
                    out_cl,
                    kernel,
                    convolution_config,
                )
            })
            .transpose()?;
        let convl2g = (in_cl != 0 && out_cg != 0)
            .then(|| {
                nn::conv2d(
                    &(path / "convl2g"),
                    in_cl,
                    out_cg,
                    kernel,
                    convolution_config,
                )
            })
            .transpose()?;
        let convg2l = (in_cg != 0 && out_cl != 0)
            .then(|| {
                nn::conv2d(
                    &(path / "convg2l"),
                    in_cg,
                    out_cl,
                    kernel,
                    convolution_config,
                )
            })
            .transpose()?;
        let convg2g = (in_cg != 0 && out_cg != 0)
            .then(|| SpectralTransform::new(&(path / "convg2g"), in_cg, out_cg, stride))
            .transpose()?;

        Ok(Self {
            convl2l,
            convl2g,
            convg2l,
            convg2g,
        })
    }

    fn forward(&self, input: (Tensor, Option<Tensor>)) -> tch::Result<(Tensor, Option<Tensor>)> {
        let local_from_local = self
            .convl2l
            .as_ref()
            .map(|conv| conv.forward(&input.0))
            .transpose()?;
        let local_from_global = match (self.convg2l.as_ref(), input.1.as_ref()) {
            (Some(conv), Some(global)) => Some(conv.forward(global)?),
            _ => None,
        };
        let global_from_local = self
            .convl2g
            .as_ref()
            .map(|conv| conv.forward(&input.0))
            .transpose()?;
        let global_from_global = match (self.convg2g.as_ref(), input.1.as_ref()) {
            (Some(conv), Some(global)) => Some(conv.forward(global)?),
            _ => None,
        };

        let local = match (local_from_local, local_from_global) {
            (Some(left), Some(right)) => left.f_add(&right)?,
            (Some(output), None) | (None, Some(output)) => output,
            (None, None) => panic!("FFC local output is empty"),
        };
        let global = match (global_from_local, global_from_global) {
            (Some(left), Some(right)) => Some(left.f_add(&right)?),
            (Some(output), None) | (None, Some(output)) => Some(output),
            (None, None) => None,
        };
        Ok((local, global))
    }
}

#[derive(Debug)]
struct SpectralTransform {
    stride: i64,
    conv1: nn::Conv2D,
    bn1: nn::BatchNorm,
    fu: FourierUnit,
    conv2: nn::Conv2D,
}

impl SpectralTransform {
    fn new(path: &nn::Path<'_>, in_channels: i64, out_channels: i64, stride: i64) -> Result<Self> {
        let hidden_channels = out_channels / 2;
        Ok(Self {
            stride,
            conv1: nn::conv2d(
                &(path / "conv1" / 0),
                in_channels,
                hidden_channels,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?,
            bn1: nn::batch_norm2d(&(path / "conv1" / 1), hidden_channels, Default::default())?,
            fu: FourierUnit::new(&(path / "fu"), hidden_channels, hidden_channels)?,
            conv2: nn::conv2d(
                &(path / "conv2"),
                hidden_channels,
                out_channels,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let input = if self.stride == 2 {
            input.f_avg_pool2d([2, 2], [2, 2], [0, 0], false, true, None)?
        } else {
            input.f_alias()?
        };
        let x = self
            .bn1
            .forward_t(&self.conv1.forward(&input)?, false)?
            .f_relu()?;
        let output = self.fu.forward(&x)?;
        self.conv2.forward(&x.f_add(&output)?)
    }
}

#[derive(Debug)]
struct FourierUnit {
    conv_layer: nn::Conv2D,
    bn: nn::BatchNorm,
}

impl FourierUnit {
    fn new(path: &nn::Path<'_>, in_channels: i64, out_channels: i64) -> Result<Self> {
        Ok(Self {
            conv_layer: nn::conv2d(
                &(path / "conv_layer"),
                in_channels * 2,
                out_channels * 2,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?,
            bn: nn::batch_norm2d(&(path / "bn"), out_channels * 2, Default::default())?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let size = input.size();
        let batch = size[0];
        let height = size[2];
        let width = size[3];
        let model_kind = input.kind();

        // The FFC unit treats real and imaginary FFT components as channels for
        // a learned 1x1 convolution, then reconstructs the complex spectrum.
        // https://github.com/advimman/lama/blob/786f5936b27fb3dacd2b1ad799e4de968ea697e7/saicinpainting/training/modules/ffc.py#L76-L114
        let fft_dims = [-2, -1];
        // torch.fft does not support BF16. Keep only the FFT boundary in FP32;
        // the learned convolution and batch normalization remain in the model dtype.
        let ffted =
            input
                .f_to_kind(Kind::Float)?
                .f_fft_rfftn(None::<&[i64]>, &fft_dims[..], "ortho")?;
        let ffted = Tensor::f_stack(&[ffted.f_real()?, ffted.f_imag()?], -1)?
            .f_permute([0, 1, 4, 2, 3])?
            .f_contiguous()?
            .f_to_kind(model_kind)?;
        let ffted_size = ffted.size();
        let ffted = ffted.f_view([batch, -1, ffted_size[3], ffted_size[4]])?;

        let ffted = self
            .bn
            .forward_t(&self.conv_layer.forward(&ffted)?, false)?
            .f_relu()?;
        let ffted_size = ffted.size();
        let ffted = ffted
            .f_view([batch, -1, 2, ffted_size[2], ffted_size[3]])?
            .f_permute([0, 1, 3, 4, 2])?
            .f_contiguous()?;
        let ffted = ffted.f_to_kind(Kind::Float)?;
        let ffted = Tensor::f_complex(&ffted.f_select(-1, 0)?, &ffted.f_select(-1, 1)?)?;
        let output_size = [height, width];
        ffted
            .f_fft_irfftn(&output_size[..], &fft_dims[..], "ortho")?
            .f_to_kind(model_kind)
    }
}

#[derive(Debug)]
struct ConcatTupleLayer;

impl ConcatTupleLayer {
    fn forward(&self, input: (Tensor, Option<Tensor>)) -> tch::Result<Tensor> {
        match input.1 {
            Some(global) => Tensor::f_cat(&[input.0, global], 1),
            None => Ok(input.0),
        }
    }
}
