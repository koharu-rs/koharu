//! YuzuMarker CJK font and text-style detection.
//!
//! Upstream implementation:
//! https://github.com/JeffersonQin/YuzuMarker.FontDetection/blob/0a94e165fe2b08d2800b723290eabd120b2d3d58/detector/model.py

mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use processor::{FontPrediction, NamedFontPrediction, TextDirection, TopFont};

use self::{model::Model, processor::Processor};

crate::model_repository!("fffonion/yuzumarker-font-detection" @ "7484242bb840f39e27f10df8ade1f8a9a8fa8f53" {
    WEIGHTS = "yuzumarker-font-detection.safetensors",
    LABELS = "font-labels-ex.json",
});

#[derive(Debug)]
pub struct FontDetector {
    device: Device,
    model: Model,
    processor: Processor,
}

impl FontDetector {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve YuzuMarker font detector weights")?;
        let labels_path = LABELS
            .resolve()
            .await
            .context("failed to resolve YuzuMarker font labels")?;

        let mut model = Model::new(device);
        model
            .load(&weights_path)
            .context("failed to load YuzuMarker font detector weights")?;
        let processor = Processor::from_path(&labels_path)?;

        Ok(Self {
            device,
            model,
            processor,
        })
    }

    pub fn inference(&self, images: &[DynamicImage], top_k: usize) -> Result<Vec<FontPrediction>> {
        if images.is_empty() {
            return Ok(Vec::new());
        }

        koharu_torch::no_grad(|| {
            let (pixel_values, image_widths) = self.processor.preprocess(images, self.device)?;
            let output = self.model.forward(&pixel_values);
            self.processor.postprocess(&output, &image_widths, top_k)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{FontDetector, TextDirection};

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_matches_upstream_structured_output() -> anyhow::Result<()> {
        crate::init().await?;
        let model = FontDetector::load(crate::Device::cpu()).await?;
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/ocr/title.png");
        let predictions = model.inference(&[image::open(input)?], 3)?;
        let prediction = &predictions[0];

        assert_eq!(
            prediction
                .top_fonts
                .iter()
                .map(|font| font.index)
                .collect::<Vec<_>>(),
            [3708, 3, 20]
        );
        for (font, expected) in
            prediction
                .top_fonts
                .iter()
                .zip([0.733_454, 0.256_713, 0.003_229_57])
        {
            assert!((font.score - expected).abs() < 1e-4);
        }
        assert_eq!(prediction.direction, TextDirection::Horizontal);
        assert_eq!(prediction.text_color, [5, 146, 244]);
        assert_eq!(prediction.stroke_color, [21, 153, 240]);
        assert!((prediction.font_size_px - 27.423_946).abs() < 1e-3);
        assert!((prediction.stroke_width_px - 1.283_724).abs() < 1e-3);
        assert!((prediction.line_height - 1.754_506).abs() < 1e-4);
        assert!((prediction.angle_deg - 1.175_119).abs() < 1e-3);
        Ok(())
    }
}
