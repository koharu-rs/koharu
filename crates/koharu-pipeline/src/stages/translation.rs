use anyhow::Result;
use async_trait::async_trait;
use koharu_scene::{Authored, LanguageTag, Origin, SourceText, Translation};
use koharu_translator::{TranslationRequest, Translator};

use crate::TranslationConfig;

use super::{StageInput, StageProcessor, finish, generation};

const PRODUCER: &str = "dev.koharu.pipeline.translation";

pub(super) struct Processor {
    config: TranslationConfig,
    translator: Translator,
}

impl Processor {
    pub(super) fn new(config: TranslationConfig, translator: Translator) -> Self {
        Self { config, translator }
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
                    targets.push((content.id(), source.text.value));
                }
            }
        }
        let mut request = build_translation_request(
            &self.config,
            &input,
            targets.iter().map(|(_, source)| source.clone()),
        );
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
        for (entity, _) in &targets {
            edit.observe::<SourceText>(*entity)?;
            edit.observe::<Translation>(*entity)?;
        }
        for ((entity, source), text) in targets.into_iter().zip(translated) {
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

fn build_translation_request(
    config: &TranslationConfig,
    input: &StageInput,
    segments: impl IntoIterator<Item = impl Into<String>>,
) -> TranslationRequest {
    let mut request = TranslationRequest::new(segments, config.target_language)
        .with_terminology(input.terminology.iter().cloned());
    if let Some(instructions) = config.instructions.as_deref() {
        request = request.with_instructions(instructions);
    }
    request
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use koharu_scene::{At, PageDraft};
    use koharu_translator::{TerminologyEntry, TerminologyKind};

    use super::{StageInput, build_translation_request};
    use crate::{ImageCache, TranslationConfig};

    #[tokio::test]
    async fn repeated_translation_stage_requests_receive_same_terminology_snapshot() {
        let mut session = koharu_scene::Session::memory().await.unwrap();
        let setup = session
            .snapshot()
            .patch(|edit| {
                edit.add_page(PageDraft::new("one", 1.0, 1.0), At::End)?;
                edit.add_page(PageDraft::new("two", 1.0, 1.0), At::End)?;
                Ok(())
            })
            .unwrap();
        session.commit(setup).await.unwrap();
        let snapshot = session.snapshot();
        let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
        let terminology: Arc<[TerminologyEntry]> = Arc::from([TerminologyEntry {
            source: "アリス".to_owned(),
            translation: "Alice".to_owned(),
            kind: TerminologyKind::Person,
        }]);
        let input = |page| {
            StageInput::new(
                snapshot.clone(),
                page,
                None,
                None,
                Arc::new(ImageCache::default()),
                None,
                terminology.clone(),
            )
        };
        let config = TranslationConfig::default();

        let first = build_translation_request(&config, &input(pages[0]), ["first"]);
        let second = build_translation_request(&config, &input(pages[1]), ["second"]);

        assert_eq!(first.terminology.as_slice(), terminology.as_ref());
        assert_eq!(second.terminology.as_slice(), terminology.as_ref());
        assert_eq!(first.terminology, second.terminology);
    }
}
