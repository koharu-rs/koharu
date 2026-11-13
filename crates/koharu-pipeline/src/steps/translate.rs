//! Translation: one conversation per open project, kept in memory, a turn per
//! page in page order. Changing the model, the target, or the prompt starts a
//! new one. A page is its texts as a JSON object numbered in reading order, and
//! its answer must keep the keys, held to a schema; a page that fails never
//! enters the history, and an answer echoing the page before's is asked again
//! without the history it copied. A text given back unchanged, such as a
//! copyright line or a "!?", is in the target's script already, so it keeps
//! its own lettering.

use std::{borrow::Cow, future::IntoFuture};

use anyhow::{Context as _, Result, bail, ensure};
use icu_experimental::displaynames::{
    DisplayNamesOptions, DisplayNamesPreferences,
    multi::{LanguageDisplayNames, LocaleDisplayNamesFormatter},
};
use icu_locale::LocaleExpander;
use icu_locale_core::{LanguageIdentifier, Locale, langid, subtags::language};
use koharu_llm::{
    Context, SummaryPrompts,
    rig::{
        completion::{
            AssistantContent, Effort, GenerationOptions, Message, Usage,
            message::{DocumentSourceKind, Image, ImageMediaType, UserContent},
        },
        driver::DynModel,
        operation::Completion,
    },
};
use koharu_scene::{Content, LayerId, Op, Project, ProjectId};
use rayon::prelude::*;
use serde_json::{Map, Value, json};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use base64::Engine as _;

use super::{Member, Stages, action, blocks, scan};
use crate::{
    Step, TARGET, TranslatorConfig,
    models::{self, Models},
    placement,
    scheduler::{Command, Link},
};

/// A project's conversation, and what it was written with.
pub(crate) struct Conversation {
    /// The model, the target, and the prompt its history was written with.
    with: Option<(String, LanguageIdentifier, String)>,
    context: Context,
    /// A hosted model, connected for its first page.
    hosted: Option<DynModel<Completion>>,
    /// The last page's texts and their translations.
    last: Option<(Vec<String>, Vec<String>)>,
}

impl Conversation {
    pub(crate) fn new() -> Self {
        Self {
            with: None,
            context: Context::new(prompts()),
            hosted: None,
            last: None,
        }
    }
}

/// A conversation lent to a translation, back to the scheduler when the step
/// drops, however it ended.
pub(crate) struct Lent {
    project: ProjectId,
    conversation: Option<Conversation>,
    link: Link,
}

impl Lent {
    pub(crate) fn new(project: ProjectId, conversation: Conversation, link: Link) -> Self {
        Self {
            project,
            conversation: Some(conversation),
            link,
        }
    }
}

impl Drop for Lent {
    fn drop(&mut self) {
        if let Some(conversation) = self.conversation.take() {
            self.link.send(Command::Returned {
                project: self.project,
                conversation: Box::new(conversation),
            });
        }
    }
}

pub(crate) struct Translate {
    config: TranslatorConfig,
    lent: Lent,
}

impl Translate {
    pub(crate) fn new(config: TranslatorConfig, lent: Lent) -> Self {
        Self { config, lent }
    }
}

/// A page's texts, and the page as a JPEG in base 64 when the model sees it.
pub(crate) struct Page {
    /// Each block and its source's language.
    blocks: Vec<(LayerId, LanguageIdentifier)>,
    texts: Vec<String>,
    image: Option<String>,
    cancel: CancellationToken,
}

/// The longest side of the page a model sees, in pixels: about 1,120 image
/// tokens to Index-Translate, above the 1,024 llama.cpp asks of Qwen-VL
/// models, where the whole scan took 4,050 and read no better.
const SIGHT: u32 = 1280;

/// `scan` at `SIGHT` as a JPEG in base 64. Each pixel averages the scan's
/// under it: a fraction of Lanczos's time, and the page is on the
/// translation's critical path.
fn sight(scan: &image::DynamicImage) -> Result<String> {
    let page = scan.thumbnail(SIGHT, SIGHT).to_rgb8();
    let mut bytes = Vec::new();
    page.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Jpeg,
    )?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

impl Stages for Translate {
    type Input = Page;
    /// The model and the page, for the turn `finish` takes.
    type Output = (DynModel<Completion>, Page);
    /// Each block, its source's language, and its translation; `None` when
    /// the run stopped first.
    type Ready = Option<Vec<(LayerId, LanguageIdentifier, String)>>;

    const STEP: Step = Step::Translate;

    /// A local model loads on the GPU thread, which only hands it over: its
    /// engine runs on a thread of its own, so the turn waits off the GPU
    /// thread, and the other steps' models run meanwhile.
    fn on_gpu(&self) -> bool {
        models::local(&self.config.model)
    }

    fn read(&self, member: &Member) -> Result<Option<Self::Input>> {
        // A page with no scan, such as a blank one with typed text, goes
        // without its image.
        let seen = if self.config.vision {
            scan(member)?
        } else {
            None
        };
        let (project, image) = match seen {
            Some((project, scan)) => (project, Some(sight(&scan)?)),
            None => (member.session.lock().project(), None),
        };
        let (blocks, texts): (Vec<_>, Vec<_>) =
            blocks(&project, member.page, member.blocks.as_deref())
                .into_iter()
                .map(|(id, block)| {
                    let text = block.source.content.text();
                    ((id, block.source.language.clone()), text)
                })
                .filter(|(_, text)| !text.trim().is_empty())
                .unzip();
        Ok((!texts.is_empty()).then(|| Page {
            blocks,
            texts,
            image,
            cancel: member.cancel.clone(),
        }))
    }

    fn infer(
        &mut self,
        models: &mut Models,
        inputs: Vec<Self::Input>,
    ) -> Result<Vec<Self::Output>> {
        let config = &self.config;
        let conversation = self
            .lent
            .conversation
            .as_mut()
            .expect("a conversation is lent until the step drops");
        let with = (
            config.model.clone(),
            config.target.clone(),
            config.prompt.clone(),
        );
        if conversation.with.as_ref() != Some(&with) {
            *conversation = Conversation::new();
            conversation.with = Some(with);
        }
        let model = if models::local(&config.model) {
            models.translator(&config.model, config.vision)?
        } else if let Some(model) = &conversation.hosted {
            model.clone()
        } else {
            let model = models
                .runtime()
                .block_on(koharu_llm::connect(&config.model, models::LANGUAGE))?;
            conversation.hosted = Some(model.clone());
            model
        };
        Ok(inputs
            .into_iter()
            .map(|page| (model.clone(), page))
            .collect())
    }

    fn finish(&mut self, (model, page): Self::Output) -> Result<Self::Ready> {
        let config = &self.config;
        let conversation = self
            .lent
            .conversation
            .as_mut()
            .expect("a conversation is lent until the step drops");
        let system = config.prompt.replace(TARGET, &name(&config.target)?);
        let keys = keys(page.texts.len());
        let turn = Turn {
            system: &system,
            page: numbered(&keys, page.texts.iter().map(String::as_str)),
            keys,
            texts: &page.texts,
            image: page.image.as_deref(),
            reasoning: config.reasoning,
            last: conversation.last.as_ref(),
            cancel: &page.cancel,
        };
        let _span = tracing::info_span!("llm", texts = page.texts.len()).entered();
        let translations =
            Handle::current().block_on(turn.take(&mut conversation.context, &model))?;
        if let Some(translations) = &translations {
            conversation.last = Some((page.texts.clone(), translations.clone()));
        }
        Ok(translations.map(|translations| {
            page.blocks
                .into_iter()
                .zip(translations)
                .map(|((block, source), text)| (block, source, text))
                .collect()
        }))
    }

    fn write(&self, project: &Project, output: Self::Ready) -> Result<Vec<Op>> {
        let target = &self.config.target;
        let model = &self.config.model;
        // Each block's placement reads the project as it is before any of
        // them, so they're found in parallel: a balloon's room split is a
        // flood over its pixels, run for each of its texts.
        let placed: Vec<_> = output
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .into_par_iter()
            .filter_map(|(block, source, text)| {
                let Ok(Content::Block(found)) = project.layer(block).map(|layer| &layer.content)
                else {
                    return None;
                };
                // Its clean-up would erase the lettering for the translation
                // to redraw it in another hand.
                let unchanged = text.trim() == found.source.content.text().trim();
                let placed = (!unchanged).then(|| placement(project, block, target));
                Some((block, source, text, placed))
            })
            .collect();
        let mut ops = Vec::new();
        for (block, source, text, placed) in placed {
            let Some(placed) = placed else {
                ops.push(Op::UpdateLayer {
                    layer: block.into(),
                    patch: json!({"translation": null, "cleanup": {"visible": false}}),
                });
                continue;
            };
            let placed = placed?;
            ops.push(Op::SetText {
                layer: block.into(),
                text,
            });
            ops.push(placed);
            ops.push(Op::UpdateLayer {
                layer: block.into(),
                patch: json!({"translation": {"language": target}, "cleanup": {"visible": true}}),
            });
            let parameters = json!({"sourceLanguage": source, "targetLanguage": target});
            ops.push(Op::SetAttribute {
                layer: block.into(),
                namespace: "c2pa".to_owned(),
                key: "translation".to_owned(),
                value: Some(action("c2pa.translated", model, Some(parameters))),
            });
        }
        Ok(ops)
    }
}

/// A page's turn of the conversation.
struct Turn<'a> {
    system: &'a str,
    page: String,
    keys: Vec<String>,
    texts: &'a [String],
    /// The page, which the model sees with its texts.
    image: Option<&'a str>,
    reasoning: Option<Effort>,
    /// The last page's texts and their translations.
    last: Option<&'a (Vec<String>, Vec<String>)>,
    cancel: &'a CancellationToken,
}

impl Turn<'_> {
    /// The page's translations, recorded in the history once they pass;
    /// `None` when the run stopped first.
    async fn take(
        self,
        context: &mut Context,
        model: &DynModel<Completion>,
    ) -> Result<Option<Vec<String>>> {
        context.fit(model).await?;
        let history: Vec<Message> = context
            .messages()
            .iter()
            .map(|message| Message::clone(message))
            .collect();
        let Some(mut asked) = self.ask(model, history).await? else {
            return Ok(None);
        };
        // A small model can answer a page with its answer to the page
        // before, word for word, as Index-Translate 2B did on one of 16
        // Blue Archive pages; without that answer to copy, it reads the page.
        if self.echoes(&asked.0) {
            tracing::info!("the answer echoed the page before's; asking again without the history");
            let Some(again) = self.ask(model, Vec::new()).await? else {
                return Ok(None);
            };
            asked = again;
        }
        let (translations, messages, usage) = asked;
        context.record([(remembered(messages), usage)]);
        Ok(Some(translations))
    }

    /// The page asked after `history`: its translations, the turn's
    /// messages, and their usage; `None` when the run stopped first.
    async fn ask(
        &self,
        model: &DynModel<Completion>,
        history: Vec<Message>,
    ) -> Result<Option<(Vec<String>, Vec<Message>, Usage)>> {
        let mut content = Vec::new();
        if let Some(image) = self.image {
            ensure!(
                model.spec().is_some_and(|spec| spec.input.image),
                "the translation model reads no images; turn vision off"
            );
            content.push(UserContent::Image(Image {
                data: DocumentSourceKind::Base64(image.to_owned()),
                media_type: Some(ImageMediaType::JPEG),
                detail: None,
                native: None,
            }));
        }
        content.push(UserContent::text(self.page.clone()));
        let mut options = GenerationOptions::default();
        if let Some(effort) = self.reasoning {
            options = options.reasoning(effort);
        }
        let request = koharu_llm::answers(model, schema(&self.keys)?)
            .preamble(self.system)
            .options(options)
            .build()
            .prompt(Message::from(content))
            .history(history);
        let response = tokio::select! {
            response = request.into_future() => response?,
            () = self.cancel.cancelled() => return Ok(None),
        };
        let translations = answers(&response.output(), &self.keys)?;
        let usage = response
            .completion_calls
            .last()
            .map_or_else(Usage::default, |call| call.usage);
        Ok(Some((translations, response.messages, usage)))
    }

    /// Whether `translations` repeat the last page's, in order, for at least
    /// two texts and half of them, though the texts differ.
    fn echoes(&self, translations: &[String]) -> bool {
        let Some((texts, answers)) = self.last else {
            return false;
        };
        let same = translations
            .iter()
            .zip(answers)
            .filter(|(new, old)| new == old)
            .count();
        texts.as_slice() != self.texts && same >= 2 && 2 * same >= translations.len()
    }
}

/// A turn's messages as the conversation keeps them: the page's texts and
/// the answer. The image and the reasoning served this turn alone; kept, every
/// later turn would send them again, and they'd fill the context.
fn remembered(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .map(|message| match message {
            Message::User { mut content } => {
                content.retain(|part| !matches!(part, UserContent::Image(_)));
                Message::User { content }
            }
            Message::Assistant(mut turn) => {
                turn.content
                    .retain(|part| !matches!(part, AssistantContent::Reasoning(_)));
                Message::Assistant(turn)
            }
            message => message,
        })
        .collect()
}

/// What a general model is asked when the conversation is full. rig's own
/// are written for a coding agent.
fn prompts() -> SummaryPrompts {
    SummaryPrompts {
        system: Cow::Borrowed(
            "You summarize a manga's translation so far, so that its translator can go on with \
             the next pages from the summary alone. Read the conversation and write the summary \
             in the exact format asked for.\n\nDo not translate anything new. Output only the \
             summary.",
        ),
        initial: Cow::Borrowed(
            "The conversation above is a manga's pages and their translations so far. Summarize \
             it for the translator of the next pages.",
        ),
        update: Cow::Borrowed(
            "The conversation above is the NEW part of a translation whose earlier part is \
             summarized in <previous-summary>. Update that summary with it, keeping everything \
             in it that still holds.",
        ),
        format: Cow::Borrowed(
            "Write three sections:\n## Characters\nEach character's name as translated, and how \
             they speak.\n## Terms\nRecurring names and terms, and their translations.\n## \
             Story\nWhat has happened so far, briefly.",
        ),
        ..SummaryPrompts::DEFAULT
    }
}

/// The target's plain English name from CLDR, such as Simplified Chinese.
/// Chinese is named by its script, which its region implies, and another
/// language drops a region its language implies, as ja-JP is Japanese. A
/// language CLDR doesn't name fails, rather than going in as its tag.
pub(crate) fn name(target: &LanguageIdentifier) -> Result<String> {
    let mut target = target.clone();
    let expander = LocaleExpander::new_extended();
    if target.language == language!("zh") {
        expander.maximize(&mut target);
        target.region = None;
    } else {
        expander.minimize(&mut target);
    }
    let english = DisplayNamesPreferences::from(&langid!("en"));
    let options = DisplayNamesOptions::default();
    let languages = LanguageDisplayNames::try_new(english, options)
        .context("couldn't load CLDR's language names")?;
    ensure!(
        languages.of(target.language).is_some(),
        "CLDR has no English name for {target}"
    );
    let locales = LocaleDisplayNamesFormatter::try_new(english, options)
        .context("couldn't load CLDR's locale names")?;
    Ok(locales.of(&Locale::from(target)).into_owned())
}

/// A key for each of `count` texts, numbered from 1 and padded to one width,
/// so they sort in reading order wherever an object's keys are sorted.
fn keys(count: usize) -> Vec<String> {
    let width = count.to_string().len();
    (1..=count)
        .map(|number| format!("{number:0width$}"))
        .collect()
}

/// The page as a JSON object of `keys` to `texts`, a text a line, in order.
fn numbered<'t>(keys: &[String], texts: impl Iterator<Item = &'t str>) -> String {
    let lines = keys
        .iter()
        .zip(texts)
        .map(|(key, text)| format!("  {}: {}", json!(key), json!(text)))
        .collect::<Vec<_>>();
    format!("{{\n{}\n}}", lines.join(",\n"))
}

/// An object with a string for each of `keys`. Every property is required,
/// so rig's rewrite toward strict mode leaves it as it is.
fn schema(keys: &[String]) -> Result<schemars::Schema> {
    let properties: Map<String, Value> = keys
        .iter()
        .map(|key| (key.clone(), json!({"type": "string"})))
        .collect();
    schemars::Schema::try_from(json!({
        "type": "object",
        "properties": properties,
        "required": keys,
        "additionalProperties": false,
    }))
    .context("the answer's schema isn't valid")
}

/// The translation for each of `keys` in `answer`, which must hold exactly
/// those keys, each a text that isn't blank.
fn answers(answer: &str, keys: &[String]) -> Result<Vec<String>> {
    let Value::Object(mut answer) =
        serde_json::from_str(answer).context("the answer isn't JSON")?
    else {
        bail!("the answer isn't a JSON object");
    };
    let translations = keys
        .iter()
        .map(|key| match answer.remove(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
            Some(Value::String(_)) => bail!("the answer leaves text {key} blank"),
            Some(_) => bail!("the answer for text {key} isn't text"),
            None => bail!("the answer leaves text {key} out"),
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(answer.is_empty(), "the answer has texts the page doesn't");
    Ok(translations)
}

#[cfg(test)]
mod tests {
    use koharu_llm::rig::completion::AssistantMessage;

    use super::*;

    fn language(tag: &str) -> LanguageIdentifier {
        LanguageIdentifier::try_from_str(tag).expect("a tag")
    }

    #[test]
    fn a_language_cldr_doesnt_name_fails() {
        assert!(name(&language("xx")).is_err());
    }

    #[test]
    fn the_conversation_keeps_no_image_or_reasoning() {
        let image = UserContent::Image(Image {
            data: DocumentSourceKind::Base64("AAAA".to_owned()),
            media_type: Some(ImageMediaType::JPEG),
            detail: None,
            native: None,
        });
        let answer = AssistantMessage::new(vec![
            AssistantContent::reasoning("Sensei is the reader."),
            AssistantContent::text("{\"01\": \"Hi\"}"),
        ]);
        let kept = remembered(vec![
            Message::from(vec![image, UserContent::text("{\"01\": \"やあ\"}")]),
            Message::Assistant(answer),
        ]);
        assert_eq!(
            kept,
            [
                Message::user("{\"01\": \"やあ\"}"),
                Message::assistant("{\"01\": \"Hi\"}"),
            ]
        );
    }

    #[test]
    fn keys_sort_in_reading_order() {
        let mut sorted = keys(12);
        sorted.sort();
        assert_eq!(sorted, keys(12));
        assert_eq!(keys(12)[0], "01");
    }

    #[test]
    fn an_answer_must_hold_every_text_and_no_other() {
        let keys = keys(2);
        assert_eq!(
            answers(r#"{"1": " One ", "2": "Two"}"#, &keys).expect("an answer"),
            ["One", "Two"]
        );
        assert!(answers(r#"{"1": "One"}"#, &keys).is_err());
        assert!(answers(r#"{"1": "One", "2": " "}"#, &keys).is_err());
        assert!(answers(r#"{"1": "One", "2": "Two", "3": "Three"}"#, &keys).is_err());
        assert!(answers("One, Two", &keys).is_err());
    }
}
