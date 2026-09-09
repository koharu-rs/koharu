mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::{HGNetV2Config, PPDocLayoutV3Config},
    processor::{
        PPDocLayoutV3Detections, PPDocLayoutV3ImageProcessor, PPDocLayoutV3Region, SizeDict,
    },
};

use self::model::Model;

crate::model_repository!("PaddlePaddle/PP-DocLayoutV3_safetensors" @ "97d101e6db2642e162a1d05392d1b0231c91033e" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
    PROCESSOR = "preprocessor_config.json",
});

#[derive(Debug)]
pub struct PPDocLayoutV3 {
    device: Device,
    model: Model,
    processor: PPDocLayoutV3ImageProcessor,
}

impl PPDocLayoutV3 {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;

        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve PP-DocLayout-V3 config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve PP-DocLayout-V3 weights")?;
        let processor_path = PROCESSOR
            .resolve()
            .await
            .context("failed to resolve PP-DocLayout-V3 image processor")?;

        let mut config = PPDocLayoutV3Config::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let labels = config.labels();
        let processor = PPDocLayoutV3ImageProcessor::from_file(&processor_path)
            .with_context(|| format!("failed to read {}", processor_path.display()))?
            .with_labels(labels);

        // The processor always produces this resolution, so the model can reuse the
        // fixed RT-DETR anchors instead of rebuilding and uploading them per page.
        config.anchor_image_size = Some(vec![processor.size.height, processor.size.width]);

        let mut model = Model::new(config, device);
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;

        Ok(Self {
            device,
            model,
            processor,
        })
    }

    pub fn inference(
        &self,
        image: &DynamicImage,
        threshold: f32,
    ) -> Result<PPDocLayoutV3Detections> {
        koharu_torch::no_grad(|| {
            let pixel_values = self.processor.preprocess(image, self.device)?;
            let outputs = self.model.forward(&pixel_values);
            self.processor.postprocess(&outputs, image, threshold)
        })
    }
}
