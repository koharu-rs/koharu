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

use std::collections::HashMap;

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use icu_experimental::displaynames::{
    DisplayNamesOptions, DisplayNamesPreferences,
    multi::{LanguageDisplayNames, LocaleDisplayNamesFormatter},
};
use icu_locale::LocaleExpander;
use icu_locale_core::{LanguageIdentifier, Locale, langid, subtags::language};
use indoc::formatdoc;
use koharu_llm::rig::{completion::Reasoning, driver::DynModel};
use koharu_scene::{Block, LayerId, Op, PageId, Project};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::{Value, json};
use tokio::sync::OwnedMutexGuard;
use tracing::{Instrument as _, info_span};

pub(crate) use self::conversation::{Conversation, Conversations};
use self::turn::Turn;
use super::{Member, Stopped, Task, action, key, writable};
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
    /// The user's own instructions to the model, in any language and form,
    /// such as a name's spelling or the tone; they follow Koharu's own system
    /// prompt as written, and can't replace it.
    pub instructions: String,
    /// Whether each page's turn shows the model the page itself beside its
    /// texts, as an image goes with a chat message to OpenAI's and Gemini's
    /// models; a model that reads no images goes without it.
    pub vision: bool,
    /// How much the model reasons before it answers: off, or one of rig's
    /// portable levels after OpenAI's `reasoning_effort`, written as one word;
    /// the model's own default when unset. rig refuses off for a model that
    /// can't stop reasoning.
    #[serde(with = "koharu_llm::reasoning")]
    #[specta(type = Option<String>)]
    pub reasoning: Option<Reasoning>,
}

impl Default for TranslatorConfig {
    fn default() -> Self {
        Self {
            // On 16 Blue Archive pages 9B read names and meaning that 2B
            // missed, such as Shale for シャーレ where 2B wrote "tray", at
            // about twice the decoding time.
            model: "koharu/index-translate-9b".to_owned(),
            target: langid!("en"),
            instructions: String::new(),
            vision: false,
            reasoning: None,
        }
    }
}

/// The system prompt: Index-Translate's structured-data template in English,
/// as bilibili/Index-Translate's `docs/prompts.md` gives it
/// (93ac63774d1c6939aec05e6f0f191bf90f225365, Apache-2.0), for the JSON the
/// answer is parsed by; then the user's instructions as they wrote them,
/// whatever their form.
fn system(config: &TranslatorConfig) -> Result<String> {
    let template = formatdoc! {"
        Translate the manga text in each message into {target}. Each message is a page's texts as a JSON object, numbered in reading order: translate only the values, and never alter the structure or the keys. Output only the translated JSON, without any explanation.",
        target = name(&config.target)?,
    };
    Ok(match config.instructions.trim() {
        "" => template,
        instructions => formatdoc! {"
            {template}

            {instructions}",
            template = template,
            instructions = instructions,
        },
    })
}

fn target<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LanguageIdentifier, D::Error> {
    let target = LanguageIdentifier::deserialize(deserializer)?;
    name(&target).map_err(D::Error::custom)?;
    Ok(target)
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
        // GPU thread, and the other stages go on.
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
    let system = system(&translator)?;
    let turn = Turn::new(
        system,
        &page.texts,
        page.image.as_deref(),
        translator.reasoning,
    );
    let answer = turn
        .take(conversation, &model)
        .instrument(info_span!("llm", texts = page.texts.len()));
    let translations = tokio::select! {
        answer = answer => answer?,
        () = member.cancel.cancelled() => return Err(Stopped.into()),
    };
    let translated = page
        .blocks
        .into_iter()
        .zip(translations)
        .map(|((block, source), text)| Translated {
            block,
            source,
            text,
            placed: None,
        })
        .collect();
    // Placing measures the texts, which waits for a font that isn't in yet,
    // so it's done before the project is held to write them.
    let (read, target) = (member.clone(), translator.target.clone());
    let translated = tokio::task::spawn_blocking(move || {
        let _span = info_span!("place", stage = %Stage::Translate, page = read.index + 1).entered();
        let project = read.session.lock().project();
        placed(&project, read.page, translated, &target)
    })
    .await??;
    member
        .apply(Stage::Translate, move |session| {
            Ok(written(&session.project(), translated, &translator))
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

/// A block's new translation.
struct Translated {
    block: LayerId,
    /// Its source's language.
    source: LanguageIdentifier,
    text: String,
    /// The patch placing it, unless it isn't written.
    placed: Option<Value>,
}

/// Whether `text` is `block`'s source given back: its clean-up would erase
/// the lettering for the translation to redraw it in another hand.
fn unchanged(block: &Block, text: &str) -> bool {
    text.trim() == block.source.content.text().trim()
}

/// `translated` placed on `page`, as it is now.
fn placed(
    project: &Project,
    page: PageId,
    mut translated: Vec<Translated>,
    target: &LanguageIdentifier,
) -> Result<Vec<Translated>> {
    let texts: Vec<(LayerId, String)> = translated
        .iter()
        .filter(|each| {
            writable(project, each.block).is_some_and(|block| !unchanged(block, &each.text))
        })
        .map(|each| (each.block, each.text.clone()))
        .collect();
    if texts.is_empty() {
        return Ok(translated);
    }
    let mut patches: HashMap<LayerId, Value> = placement::place(project, page, &texts, target)?
        .into_iter()
        .collect();
    for each in &mut translated {
        each.placed = patches.remove(&each.block);
    }
    Ok(translated)
}

/// The ops writing each block's translation, as it's placed, into the project
/// as it is now.
fn written(project: &Project, translated: Vec<Translated>, config: &TranslatorConfig) -> Vec<Op> {
    let target = &config.target;
    let mut ops = Vec::new();
    for each in translated {
        let block = each.block;
        let Some(found) = writable(project, block) else {
            continue;
        };
        let Some(placed) = each.placed else {
            if unchanged(found, &each.text) {
                ops.push(Op::UpdateLayer {
                    layer: block.into(),
                    patch: json!({"translation": null, "cleanup": {"visible": false}}),
                });
            }
            continue;
        };
        ops.push(Op::SetText {
            layer: block.into(),
            text: each.text,
        });
        ops.push(Op::UpdateLayer {
            layer: block.into(),
            patch: placed,
        });
        ops.push(Op::UpdateLayer {
            layer: block.into(),
            patch: json!({"translation": {"language": target}, "cleanup": {"visible": true}}),
        });
        let parameters = json!({"sourceLanguage": each.source, "targetLanguage": target});
        ops.push(Op::SetAttribute {
            layer: block.into(),
            namespace: "c2pa".to_owned(),
            key: "translation".to_owned(),
            value: Some(action("c2pa.translated", &config.model, Some(parameters))),
        });
    }
    ops
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
    use indoc::indoc;
    use koharu_scene::{Edit, Session};

    use super::*;

    #[test]
    fn a_balloon_locked_while_translating_leaves_the_others_their_translation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let block = |x: f64| {
            json!({"kind": "block", "source": {
                "content": "猫", "language": "ja",
                "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 40.0, "height": 120.0}}},
            }})
        };
        session.apply(Edit {
            label: "test".to_owned(),
            ops: serde_json::from_value(json!([
                {"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 400}},
                {"op": "add_layer", "id": "$balloon", "page": "$p", "index": 0,
                    "layer": {"kind": "balloon", "outline": "M0 0 L200 0 L200 160 L0 160 Z"}},
                {"op": "add_layer", "page": "$p", "parent": "$balloon", "index": 0,
                    "layer": block(20.0)},
                {"op": "add_layer", "page": "$p", "index": 1, "layer": block(220.0)},
                // The lock comes on after the translator read the page.
                {"op": "update_layer", "layer": "$balloon", "patch": {"locked": true}},
            ]))?,
        })?;
        let project = session.project();
        let page = project.pages().next().context("a page")?.id;
        let translated = project
            .blocks(page)?
            .into_iter()
            .map(|(block, _)| Translated {
                block,
                source: langid!("ja"),
                text: "Cat".to_owned(),
                placed: Some(json!({})),
            })
            .collect();
        let ops = written(&project, translated, &TranslatorConfig::default());
        session.apply(Edit {
            label: "translate".to_owned(),
            ops,
        })?;
        let translations: Vec<Option<String>> = session
            .project()
            .blocks(page)?
            .into_iter()
            .map(|(_, block)| block.translation.as_ref().map(|text| text.content.text()))
            .collect();
        assert_eq!(translations, [None, Some("Cat".to_owned())]);
        Ok(())
    }

    #[test]
    fn a_language_cldr_doesnt_name_fails() {
        let language = LanguageIdentifier::try_from_str("xx").expect("a tag");
        assert!(name(&language).is_err());
    }

    #[test]
    fn instructions_follow_the_whole_template() {
        let template = system(&TranslatorConfig::default()).expect("English names it");
        let config = TranslatorConfig {
            instructions: indoc! {"

                シャーレはSchaleと訳す。

                敬称は残す。

            "}
            .to_owned(),
            ..Default::default()
        };
        assert_eq!(
            system(&config).expect("English names it"),
            formatdoc! {"
                {template}

                シャーレはSchaleと訳す。

                敬称は残す。",
                template = template,
            }
        );
    }
}
