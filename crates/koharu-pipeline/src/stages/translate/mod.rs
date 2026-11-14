//! Translation: one conversation per open project, kept in memory, a turn per
//! page in page order. Changing the model, the target, or the prompt starts a
//! new one. A page is its texts as a JSON object numbered in reading order, and
//! its answer must keep the keys, held to a schema; a page that fails never
//! enters the history, and an answer echoing the page before's is asked again
//! without the history it copied. A text given back unchanged, such as a
//! copyright line or a "!?", is in the target's script already, so it keeps
//! its own lettering.

mod conversation;
mod model;
mod placement;
mod turn;

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use icu_experimental::displaynames::{
    DisplayNamesOptions, DisplayNamesPreferences,
    multi::{LanguageDisplayNames, LocaleDisplayNamesFormatter},
};
use icu_locale::LocaleExpander;
use icu_locale_core::{LanguageIdentifier, Locale, langid, subtags::language};
use indoc::indoc;
use koharu_llm::rig::{completion::Effort, driver::DynModel};
use koharu_scene::{Content, LayerId, Op, Project};
use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::json;
use tokio::sync::OwnedMutexGuard;
use tracing::{Instrument as _, info_span};

pub(crate) use self::conversation::{Conversation, Conversations};
pub use self::placement::placement;
use self::turn::Turn;
use super::{Member, Stopped, Task, action, key};
use crate::{PipelineConfig, Stage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct TranslatorConfig {
    /// A `vendor/model` reference, as koharu-llm connects one.
    pub model: String,
    /// A language CLDR names in English.
    #[serde(deserialize_with = "target")]
    #[specta(type = String)]
    pub target: LanguageIdentifier,
    /// The system prompt, with `{target}` where the target's name goes.
    #[serde(deserialize_with = "prompt")]
    #[specta(type = String)]
    pub prompt: String,
    /// Whether each page's turn shows the model the page itself beside its
    /// texts, as an image goes with a chat message to OpenAI's and Gemini's
    /// models; only for a model that reads images.
    pub vision: bool,
    /// How much the model reasons before it answers, by rig's portable levels
    /// after OpenAI's `reasoning_effort`; the model's own default when unset.
    #[specta(type = Option<String>)]
    pub reasoning: Option<Effort>,
}

impl Default for TranslatorConfig {
    fn default() -> Self {
        Self {
            // On 16 Blue Archive pages 9B read names and meaning that 2B
            // missed, such as Shale for シャーレ where 2B wrote "tray", at
            // about twice the decoding time.
            model: "koharu/index-translate-9b".to_owned(),
            target: langid!("en"),
            // Index-Translate's instTrans template in English, as
            // bilibili/Index-Translate's `docs/prompts.md` gives it
            // (93ac63774d1c6939aec05e6f0f191bf90f225365, Apache-2.0),
            // measured against its Chinese template on Manga109 pages.
            prompt: indoc! {"
                Please translate the manga text in each message into {target} and strictly follow all constraints. Each message is a page's texts as a JSON object, numbered in reading order.

                [Constraints]
                1. [Hard Constraint] Preserve the JSON format in the source text unchanged

                Output the translation in the same JSON format, keeping the keys unchanged and each value its translation. Output only the JSON without any explanation."}
            .to_owned(),
            vision: false,
            reasoning: None,
        }
    }
}

fn target<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LanguageIdentifier, D::Error> {
    let target = LanguageIdentifier::deserialize(deserializer)?;
    name(&target).map_err(D::Error::custom)?;
    Ok(target)
}

fn prompt<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let prompt = String::deserialize(deserializer)?;
    if !prompt.contains("{target}") {
        return Err(D::Error::custom("the translation prompt has no {target}"));
    }
    Ok(prompt)
}

/// A page's texts, and the page as a JPEG in base 64 when the model sees it.
struct Page {
    /// Each block and its source's language.
    blocks: Vec<(LayerId, LanguageIdentifier)>,
    texts: Vec<String>,
    image: Option<String>,
}

pub(crate) async fn translate(
    task: &Task,
    config: &PipelineConfig,
    mut conversation: OwnedMutexGuard<Conversation>,
) -> Result<()> {
    let member = &task.members[0];
    let translator = config.translator.clone();
    let (read, vision) = (member.clone(), translator.vision);
    let page = tokio::task::spawn_blocking(move || {
        let _span = info_span!("read", stage = %Stage::Translate, page = read.index + 1).entered();
        page(&read, vision)
    })
    .await??;
    let Some(page) = page else {
        return Ok(());
    };
    let conversation = &mut *conversation;
    conversation.keep(&translator);
    let model = match key(Stage::Translate, config) {
        // A local model loads on the GPU thread, which only hands it over:
        // its engine runs on a thread of its own, so the turn waits off the
        // GPU thread, and the other stages' models run meanwhile.
        Some(key) => {
            let (reference, images) = (translator.model.clone(), translator.vision);
            task.inference(&[member], move |models| {
                let model = models.get(&key, |_| model::load(&reference, images))?;
                Ok(DynModel::clone(model))
            })
            .await?
        }
        None => {
            task.started(&[member]);
            conversation.hosted(&translator).await?
        }
    };
    let system = translator
        .prompt
        .replace("{target}", &name(&translator.target)?);
    let turn = Turn::new(
        system,
        &page.texts,
        page.image.as_deref(),
        translator.reasoning,
        &member.cancel,
    );
    let Some(translations) = turn
        .take(conversation, &model)
        .instrument(info_span!("llm", texts = page.texts.len()))
        .await?
    else {
        return Err(Stopped.into());
    };
    let translated: Vec<(LayerId, LanguageIdentifier, String)> = page
        .blocks
        .into_iter()
        .zip(translations)
        .map(|((block, source), text)| (block, source, text))
        .collect();
    member
        .apply(Stage::Translate, move |project| {
            written(project, translated, &translator)
        })
        .await
}

/// The page's texts the task works on, and the page itself when the model
/// sees it; `None` when it has no text.
fn page(member: &Member, vision: bool) -> Result<Option<Page>> {
    // A page with no scan, such as a blank one with typed text, goes
    // without its image.
    let seen = if vision { member.scan()? } else { None };
    let (project, image) = match seen {
        Some((project, scan)) => (project, Some(sight(&scan)?)),
        None => (member.session.lock().project(), None),
    };
    let (blocks, texts): (Vec<_>, Vec<_>) = member
        .blocks(&project)
        .into_iter()
        .map(|(id, block)| {
            let text = block.source.content.text();
            ((id, block.source.language.clone()), text)
        })
        .filter(|(_, text)| !text.trim().is_empty())
        .unzip();
    Ok((!texts.is_empty()).then_some(Page {
        blocks,
        texts,
        image,
    }))
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

/// The ops writing each block's translation, placed, into the project as it
/// is now.
fn written(
    project: &Project,
    translated: Vec<(LayerId, LanguageIdentifier, String)>,
    config: &TranslatorConfig,
) -> Result<Vec<Op>> {
    let target = &config.target;
    // Each block's placement reads the project as it is before any of them,
    // so they're found in parallel: a balloon's room split is a flood over
    // its pixels, run for each of its texts.
    let placed: Vec<_> = translated
        .into_par_iter()
        .filter_map(|(block, source, text)| {
            let Ok(Content::Block(found)) = project.layer(block).map(|layer| &layer.content) else {
                return None;
            };
            // Its clean-up would erase the lettering for the translation to
            // redraw it in another hand.
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
        ops.push(Op::SetText {
            layer: block.into(),
            text,
        });
        ops.push(placed?);
        ops.push(Op::UpdateLayer {
            layer: block.into(),
            patch: json!({"translation": {"language": target}, "cleanup": {"visible": true}}),
        });
        let parameters = json!({"sourceLanguage": source, "targetLanguage": target});
        ops.push(Op::SetAttribute {
            layer: block.into(),
            namespace: "c2pa".to_owned(),
            key: "translation".to_owned(),
            value: Some(action("c2pa.translated", &config.model, Some(parameters))),
        });
    }
    Ok(ops)
}

/// The target's plain English name from CLDR, such as Simplified Chinese.
/// Chinese is named by its script, which its region implies, and another
/// language drops a region its language implies, as ja-JP is Japanese. A
/// language CLDR doesn't name fails, rather than going in as its tag.
fn name(target: &LanguageIdentifier) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_cldr_doesnt_name_fails() {
        let language = LanguageIdentifier::try_from_str("xx").expect("a tag");
        assert!(name(&language).is_err());
    }
}
