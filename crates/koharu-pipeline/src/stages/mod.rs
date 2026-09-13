mod detection;
mod inpainting;
mod ocr;
mod translation;

use std::{collections::BTreeSet, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use koharu_scene::{Edit, EntityId, Generation, Patch, ProducerId, Snapshot};

pub use detection::KoharuLayoutRFDetrSeg2XLConfig;
pub use inpainting::{Flux2KleinConfig, RoremMixedConfig};

use crate::{Bounds, ImageCache, InpaintingMask, PipelineConfig, Stage};

#[derive(Clone)]
pub(crate) struct StageInput {
    scene: koharu_scene::Snapshot,
    page: EntityId,
    entities: Option<Arc<BTreeSet<EntityId>>>,
    region: Option<Bounds>,
    images: Arc<ImageCache>,
    inpainting_mask: Option<InpaintingMask>,
}

impl StageInput {
    pub(crate) fn new(
        scene: Snapshot,
        page: EntityId,
        entities: Option<Arc<BTreeSet<EntityId>>>,
        region: Option<Bounds>,
        images: Arc<ImageCache>,
        inpainting_mask: Option<InpaintingMask>,
    ) -> Self {
        Self {
            scene,
            page,
            entities,
            region,
            images,
            inpainting_mask,
        }
    }

    pub(crate) fn page(&self) -> EntityId {
        self.page
    }

    fn contains_entity(&self, entity: EntityId) -> Result<bool> {
        crate::scope::contains_entity(
            &self.scene,
            self.page,
            self.entities.as_deref(),
            self.region,
            entity,
        )
    }
}

#[async_trait]
trait StageProcessor: Send + Sync {
    fn model(&self) -> &'static str;
    fn skip(&self, _input: &StageInput) -> Result<bool> {
        Ok(false)
    }
    // Overlapping work items are safe and do not occupy the accelerator lane.
    fn concurrent(&self) -> bool {
        false
    }
    fn unload(&self) -> bool;
    async fn load(&self) -> Result<()>;
    async fn process(&self, input: StageInput) -> Result<Patch>;
}

pub(crate) struct Stages {
    detection: detection::Processor,
    ocr: ocr::Processor,
    translation: translation::Processor,
    inpainting: inpainting::Processor,
}

impl Stages {
    pub(crate) fn new(
        config: &PipelineConfig,
        translator: koharu_translator::Translator,
        device: &koharu_ml::Device,
    ) -> Result<Self> {
        Ok(Self {
            detection: detection::Processor::new(config.detection()?, device.clone()),
            ocr: ocr::Processor::new(config.ocr.clone(), device.clone()),
            translation: translation::Processor::new(config.translation.clone(), translator),
            inpainting: inpainting::Processor::new(config.inpainting()?, device.clone())?,
        })
    }

    fn processor(&self, stage: Stage) -> &dyn StageProcessor {
        match stage {
            Stage::Detection => &self.detection,
            Stage::Ocr => &self.ocr,
            Stage::Translation => &self.translation,
            Stage::Inpainting => &self.inpainting,
        }
    }

    pub(crate) fn model(&self, stage: Stage) -> &'static str {
        self.processor(stage).model()
    }

    pub(crate) fn skip(&self, stage: Stage, input: &StageInput) -> Result<bool> {
        self.processor(stage).skip(input)
    }

    pub(crate) fn concurrent(&self, stage: Stage) -> bool {
        self.processor(stage).concurrent()
    }

    pub(crate) async fn load(&self, stage: Stage) -> Result<()> {
        self.processor(stage).load().await
    }

    pub(crate) async fn process(&self, stage: Stage, input: StageInput) -> Result<Patch> {
        self.processor(stage).process(input).await
    }

    pub(crate) fn unload(&self, stage: Stage) -> bool {
        self.processor(stage).unload()
    }
}

fn generation(producer: &str, model: &str) -> Result<Generation> {
    let mut generation = Generation::new(ProducerId::new(producer)?);
    generation.model = Some(model.to_owned());
    Ok(generation)
}

fn finish(edit: Edit) -> Result<Patch> {
    edit.finish().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(bytes: &'static [u8]) -> koharu_scene::AssetInput {
        koharu_scene::AssetInput::new(
            bytes,
            "image/png",
            koharu_scene::AssetMetadata {
                width: Some(1),
                height: Some(1),
                attributes: std::collections::BTreeMap::new(),
            },
        )
    }

    #[tokio::test]
    async fn registry_contains_every_stage() {
        let translator = koharu_translator::Translator::from_config(
            koharu_ml::Device::cpu(),
            koharu_config::Config::memory(koharu_translator::ProvidersConfig::default()),
        )
        .unwrap();
        let stages = Stages::new(
            &PipelineConfig::default(),
            translator,
            &koharu_ml::Device::cpu(),
        )
        .unwrap();

        assert_eq!(
            Stage::ALL.map(|stage| stages.model(stage)),
            [
                "koharu-layout-rfdetr-seg-2xl",
                "paddleocr-vl-1.6",
                "local",
                "lama",
            ]
        );
    }

    #[tokio::test]
    async fn translation_and_inpainting_compose_without_weakening_text_guards() {
        let mut session = koharu_scene::Session::memory().await.unwrap();
        let mut setup = session.snapshot().edit();
        let page = setup
            .add_page(
                koharu_scene::PageDraft::new("page", 1.0, 1.0),
                koharu_scene::At::End,
            )
            .unwrap();
        let text = setup.add_text_content(page, koharu_scene::At::End).unwrap();
        setup
            .set(
                text,
                &koharu_scene::SourceText {
                    text: koharu_scene::Authored::user("before".to_owned()),
                    language: None,
                },
            )
            .unwrap();
        setup
            .set_asset(
                page,
                &koharu_scene::AssetRole::new("source").unwrap(),
                asset(b"source"),
            )
            .unwrap();
        session.commit(setup.finish().unwrap()).await.unwrap();
        let base = session.snapshot();

        let mut text_edit = base.edit();
        text_edit.observe::<koharu_scene::SourceText>(text).unwrap();
        text_edit
            .observe::<koharu_scene::Translation>(text)
            .unwrap();
        text_edit
            .set(
                text,
                &koharu_scene::Translation {
                    text: koharu_scene::Authored::user("after".to_owned()),
                    language: None,
                },
            )
            .unwrap();
        let text_patch = text_edit.finish().unwrap();

        let mut image_edit = base.edit();
        image_edit.observe_assets(page).unwrap();
        let cleanup = image_edit
            .add_entity(page, koharu_scene::At::Start)
            .unwrap();
        image_edit
            .set(
                cleanup,
                &koharu_scene::RasterLayer {
                    origin: koharu_scene::Origin::User,
                    name: "Cleanup".to_owned(),
                    kind: koharu_scene::RasterLayerKind::Cleanup,
                },
            )
            .unwrap();
        image_edit
            .set_asset(
                cleanup,
                &koharu_scene::AssetRole::new("source").unwrap(),
                asset(b"clean"),
            )
            .unwrap();
        let image_patch = image_edit.finish().unwrap();

        let image_first = base.preview([&image_patch]).unwrap();
        assert!(text_patch.rebase_on(&image_first).is_ok());
        let text_first = base.preview([&text_patch]).unwrap();
        assert!(image_patch.rebase_on(&text_first).is_ok());

        let changed_source = base
            .patch(|edit| {
                edit.set(
                    text,
                    &koharu_scene::SourceText {
                        text: koharu_scene::Authored::user("changed".to_owned()),
                        language: None,
                    },
                )
            })
            .unwrap();
        let changed_source = base.preview([&changed_source]).unwrap();
        assert!(text_patch.rebase_on(&changed_source).is_err());
    }
}
