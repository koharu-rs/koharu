use koharu_ml::koharu_layout_highres::KoharuLayoutHighres;
use koharu_ml::yuzumarker_font_detection::YuzuMarkerFontDetection;
use koharu_runtime::Device;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

/// The detectors, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[derive(specta::Type)]
pub enum Detector {
    #[default]
    #[strum(to_string = "Koharu Layout HighRes")]
    KoharuLayoutHighres,
}

/// The detector, and the font model that reads the style of the lettering it
/// finds.
pub(super) async fn load(
    model: Detector,
    device: &'static Device,
) -> anyhow::Result<(KoharuLayoutHighres, YuzuMarkerFontDetection)> {
    let layout = match model {
        Detector::KoharuLayoutHighres => KoharuLayoutHighres::load(device).await?,
    };
    Ok((layout, YuzuMarkerFontDetection::load(device).await?))
}
