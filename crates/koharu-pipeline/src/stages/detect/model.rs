use koharu_ml::koharu_layout_highres::KoharuLayoutHighres;
use koharu_runtime::Device;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

/// The detectors, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
pub enum Detector {
    #[default]
    #[strum(to_string = "Koharu Layout HighRes")]
    KoharuLayoutHighres,
}

pub(super) async fn load(
    model: Detector,
    device: &'static Device,
) -> anyhow::Result<KoharuLayoutHighres> {
    match model {
        Detector::KoharuLayoutHighres => KoharuLayoutHighres::load(device).await,
    }
}
