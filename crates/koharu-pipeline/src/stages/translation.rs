use anyhow::Result;
use async_trait::async_trait;
use koharu_scene::{Authored, FitsTo, FlowsIn, Geometry, LanguageTag, Origin, Point, SourceText, Translation};
use koharu_translator::{TranslationRequest, Translator};

use crate::TranslationConfig;

use super::{StageInput, StageProcessor, finish, generation};

const PRODUCER: &str = "dev.koharu.pipeline.translation";

pub(super) struct Processor {
    config: TranslationConfig,
    translator: Translator,
    text_region_scale: f64,
}

impl Processor {
    pub(super) fn new(
        config: TranslationConfig,
        translator: Translator,
        text_region_scale: f32,
    ) -> Self {
        Self {
            config,
            translator,
            text_region_scale: f64::from(text_region_scale),
        }
    }
}

#[async_trait]
impl StageProcessor for Processor {
    fn model(&self) -> &'static str {
        Translator::model(&self.config.model)
    }

    fn unload(&self) -> bool {
        self.translator.unload()
    }

    async fn load(&self) -> Result<()> {
        self.translator.load_model(&self.config.model).await
    }

    async fn process(&self, input: StageInput) -> Result<koharu_scene::Patch> {
        let mut targets = Vec::new();
        if let Some(group) = input.scene.page(input.page)?.text_group()? {
            for layer in group.text_layers()? {
                if !input.contains_entity(layer.id())? {
                    continue;
                }
                let content = layer.content()?;
                let Some(source) = content.source()? else {
                    continue;
                };
                if !source.text.value.trim().is_empty() {
                    targets.push((content.id(), source.text.value, layer.id()));
                }
            }
        }
        let mut request = TranslationRequest::new(
            targets.iter().map(|(_, source, _)| source.clone()),
            self.config.target_language,
        );
        if let Some(instructions) = self.config.instructions.as_deref() {
            request = request.with_instructions(instructions);
        }
        if Translator::supports_vision(&self.config.model, &self.config.generation)
            && let Some(image) = input.images.get(&input.scene, input.page, "source").await?
        {
            request = request.with_image(image);
        }
        let (provider, translated) = self
            .translator
            .translate(&self.config.model, self.config.generation, request)
            .await?;
        let language = LanguageTag::new(self.config.target_language.tag())?;
        let generated = generation(PRODUCER, provider)?;
        let mut edit = input.scene.edit_as(generated.clone());
        for (entity, _, layer) in &targets {
            edit.observe::<SourceText>(*entity)?;
            edit.observe::<Translation>(*entity)?;
            let target = if let Some(relation) = input.scene.relation_from::<FlowsIn>(*layer)? {
                Some(relation.value().target)
            } else if let Some(relation) = input.scene.relation_from::<FitsTo>(*layer)? {
                Some(relation.value().target)
            } else {
                None
            };
            if let Some(target) = target
                && let Some(geometry) = input.scene.component::<Geometry>(target)?
            {
                edit.observe::<Geometry>(target)?;
                edit.observe::<Geometry>(*layer)?;
                edit.set(*layer, &scale_geometry(&geometry, self.text_region_scale))?;
            }
        }
        for ((entity, source, _layer), text) in targets.into_iter().zip(translated) {
            if input
                .scene
                .component::<Translation>(entity)?
                .is_some_and(|value| matches!(value.text.origin, Origin::User))
            {
                continue;
            }
            let text = if source.trim() == "\u{2026}" {
                "\u{2026}".to_owned()
            } else {
                text
            };
            edit.set(
                entity,
                &Translation {
                    text: Authored::generated(text, generated.clone()),
                    language: Some(language.clone()),
                },
            )?;
        }
        finish(edit)
    }
}

fn scale_geometry(geometry: &Geometry, scale: f64) -> Geometry {
    let (min_x, max_x, min_y, max_y) = geometry.points.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY),
        |(min_x, max_x, min_y, max_y), point| {
            (
                min_x.min(point.x),
                max_x.max(point.x),
                min_y.min(point.y),
                max_y.max(point.y),
            )
        },
    );
    let center_x = (min_x + max_x) * 0.5;
    let center_y = (min_y + max_y) * 0.5;
    Geometry {
        origin: geometry.origin.clone(),
        points: geometry
            .points
            .iter()
            .map(|point| Point {
                x: center_x + (point.x - center_x) * scale,
                y: center_y + (point.y - center_y) * scale,
            })
            .collect(),
    }
}
