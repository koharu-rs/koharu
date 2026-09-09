mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::Yolo11nSpeechBubbleConfig,
    processor::{
        Yolo11nSegImageProcessor, Yolo11nSpeechBubbleInstance, Yolo11nSpeechBubbleInstances,
        Yolo11nSpeechBubbleMask,
    },
};

use self::model::Model;

crate::model_repository!("mayocream/manga109-segmentation-bubble" @ "4c9d7cbfa9905f7003677fa1130b48b93365a16a" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
});

#[derive(Debug)]
pub struct Yolo11nSpeechBubbleSegmenter {
    device: Device,
    config: Yolo11nSpeechBubbleConfig,
    model: Model,
    processor: Yolo11nSegImageProcessor,
}

impl Yolo11nSpeechBubbleSegmenter {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve YOLO11n speech bubble config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve YOLO11n speech bubble weights")?;
        let config = Yolo11nSpeechBubbleConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = Yolo11nSegImageProcessor::new(&config)?;
        let mut model = Model::new(&config, device)?;
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self {
            device,
            config,
            model,
            processor,
        })
    }

    pub fn inference(&self, image: &DynamicImage) -> Result<Yolo11nSpeechBubbleInstances> {
        self.inference_with_thresholds(
            image,
            self.config.default_confidence_threshold,
            self.config.default_nms_threshold,
        )
    }

    pub fn inference_with_thresholds(
        &self,
        image: &DynamicImage,
        confidence_threshold: f32,
        nms_threshold: f32,
    ) -> Result<Yolo11nSpeechBubbleInstances> {
        koharu_torch::no_grad(|| {
            let (pixel_values, letterbox) = self.processor.preprocess(image, self.device)?;
            let output = self.model.forward(&pixel_values);
            self.processor
                .postprocess(&output, &letterbox, confidence_threshold, nms_threshold)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;

    use super::Yolo11nSpeechBubbleSegmenter;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_matches_ultralytics_structured_output() -> Result<()> {
        crate::init().await?;
        let image = image::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("benches/fixtures/object_detection/1.jpg"),
        )?;
        let model = Yolo11nSpeechBubbleSegmenter::load(crate::Device::cpu()).await?;
        let result = model.inference(&image)?;

        // Ultralytics 8.3.227, `imgsz=1600`, `retina_masks=True`, CPU.
        let boxes = [
            [588.65356, 814.42773, 708.22894, 981.5542],
            [566.4771, 546.184, 690.2997, 720.03046],
            [244.01305, 97.16937, 360.83, 282.00912],
            [325.01566, 269.10986, 415.32822, 432.4775],
            [255.47812, 546.23315, 361.34872, 697.92224],
            [75.93337, 611.16705, 150.0059, 742.12427],
            [87.51999, 820.4986, 144.62572, 912.24756],
            [75.97409, 217.11365, 177.52692, 382.19717],
        ];
        let scores = [
            0.97215414, 0.97157073, 0.9692747, 0.96337724, 0.9615384, 0.9601294, 0.95542735,
            0.94835824,
        ];
        let mask_areas = [15_744, 18_968, 17_458, 12_262, 13_953, 8_056, 4_179, 13_869];

        assert_eq!(result.instances.len(), boxes.len());
        for (index, bubble) in result.instances.iter().enumerate() {
            assert_eq!(bubble.label, "balloon");
            assert!((bubble.score - scores[index]).abs() < 3e-4);
            for (actual, expected) in bubble.bbox.into_iter().zip(boxes[index]) {
                assert!((actual - expected).abs() < 0.02);
            }
            assert!(bubble.area.abs_diff(mask_areas[index]) <= 20);
        }
        Ok(())
    }
}
