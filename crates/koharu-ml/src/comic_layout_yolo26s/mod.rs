//! Comic page layout segmentation with the converted YOLO26s checkpoint.
//!
//! Checkpoint configuration and conversion manifest:
//! https://huggingface.co/mayocream/comic-layout-yolo26s/tree/90f556d6973a8abdefacaace1e7eed4adbcd33a8

mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::ComicLayoutYolo26sConfig,
    processor::{
        ComicLayoutYolo26sImageProcessor, ComicLayoutYolo26sInstance, ComicLayoutYolo26sInstances,
        ComicLayoutYolo26sMask,
    },
};

use self::model::Model;

crate::model_repository!("mayocream/comic-layout-yolo26s" @ "90f556d6973a8abdefacaace1e7eed4adbcd33a8" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
});

#[derive(Debug)]
pub struct ComicLayoutYolo26sSegmenter {
    device: Device,
    model: Model,
    processor: ComicLayoutYolo26sImageProcessor,
}

impl ComicLayoutYolo26sSegmenter {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve comic-layout-yolo26s config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve comic-layout-yolo26s weights")?;
        let config = ComicLayoutYolo26sConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = ComicLayoutYolo26sImageProcessor::new(&config)?;
        let mut model = Model::new(&config, device)?;
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self {
            device,
            model,
            processor,
        })
    }

    pub fn inference(&self, image: &DynamicImage) -> Result<ComicLayoutYolo26sInstances> {
        self.inference_with_threshold(image, 0.25)
    }

    pub fn inference_with_threshold(
        &self,
        image: &DynamicImage,
        confidence_threshold: f32,
    ) -> Result<ComicLayoutYolo26sInstances> {
        koharu_torch::no_grad(|| {
            let (pixel_values, letterbox) = self.processor.preprocess(image, self.device)?;
            let output = self.model.forward(&pixel_values);
            self.processor
                .postprocess(&output, &letterbox, confidence_threshold)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;

    use super::ComicLayoutYolo26sSegmenter;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_matches_ultralytics_structured_output() -> Result<()> {
        crate::init().await?;
        let image = image::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("benches/fixtures/object_detection/1.jpg"),
        )?;
        let model = ComicLayoutYolo26sSegmenter::load(crate::Device::cuda(0)).await?;
        let result = model.inference(&image)?;

        // Ultralytics 8.4.43, `imgsz=1280`, `retina_masks=True`, CUDA.
        let labels = [
            "balloon", "balloon", "balloon", "balloon", "balloon", "balloon", "balloon", "text",
            "text", "text", "text", "frame", "text", "text", "text",
        ];
        let scores = [
            0.990_530_1,
            0.989_932_4,
            0.986_495_8,
            0.984_152_5,
            0.982_764_4,
            0.981_057_9,
            0.978_836_36,
            0.966_664_7,
            0.963_278_9,
            0.960_668,
            0.949_743_33,
            0.942_265_5,
            0.933_951_26,
            0.912_969_8,
            0.818_534_3,
        ];
        let boxes = [
            [567.287_23, 546.299, 691.442_93, 719.966_2],
            [589.763_9, 814.196_5, 708.666_7, 981.020_75],
            [325.602_48, 269.746_8, 416.116_5, 432.190_12],
            [256.010_38, 546.089_36, 362.204_65, 698.065_06],
            [244.485_5, 97.052_055, 361.554_05, 282.517_03],
            [76.357_7, 216.292_48, 178.767_18, 383.584_8],
            [76.496_94, 611.334, 151.132_06, 741.457_6],
            [595.782_04, 560.716_7, 657.130_5, 688.110_6],
            [273.067_6, 124.257_614, 327.039_06, 251.811_07],
            [280.037_26, 560.698_5, 335.014_92, 675.221_2],
            [97.640_81, 628.511_66, 132.523_5, 722.166_7],
            [49.679_337, 554.276_06, 713.380_5, 790.490_6],
            [636.261_4, 829.163_2, 663.573_8, 967.357_24],
            [362.707_28, 282.951_8, 378.997_9, 416.371_8],
            [111.733_15, 231.878_28, 140.875_15, 371.763_92],
        ];
        let mask_areas = [
            18_908, 15_865, 12_221, 13_925, 17_400, 14_152, 8_089, 5_120, 6_257, 5_370, 1_992,
            149_968, 3_124, 1_559, 3_295,
        ];

        assert_eq!(result.instances.len(), 18);
        for (index, instance) in result.instances.iter().take(labels.len()).enumerate() {
            assert_eq!(instance.label, labels[index]);
            assert!((instance.score - scores[index]).abs() < 1e-3);
            for (actual, expected) in instance.bbox.into_iter().zip(boxes[index]) {
                assert!((actual - expected).abs() < 0.3);
            }
            assert!(instance.area.abs_diff(mask_areas[index]) <= 450);
        }
        Ok(())
    }
}
