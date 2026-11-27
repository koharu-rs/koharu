//! Translation: one conversation per open project, kept in memory, a turn per
//! page in page order. Changing the model, the target, or the prompt starts a
//! new one. A page is its texts as a JSON object numbered in reading order, and
//! its answer must keep the keys, held to a schema; a page that fails never
//! enters the history, and an answer echoing the page before's is asked again
//! without the history it copied. A text given back unchanged, such as a
//! copyright line or a "!?", is in the target's script already, so it keeps
//! its own lettering.

mod batch;
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
use koharu_io::{COMIC_INFO_KEY, ComicInfo};
use koharu_llm::rig::{completion::Reasoning, driver::DynModel, operation::Completion};
use koharu_scene::{Block, LayerId, Op, PageId, Project};
use petgraph::stable_graph::NodeIndex;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::{Value, json};
use tokio::sync::OwnedMutexGuard;
use tracing::{Instrument as _, info_span};

pub(crate) use self::conversation::{Conversation, Conversations};
use self::turn::Turn;
use super::{Failed, Member, Stopped, Task, action, key, writable};
use crate::{PipelineConfig, Stage, gpu::LoadOutcome};

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
    /// Whether to combine consecutive pages into multi-page batches (when
    /// Vision is off) or include marked neighboring dialogue for continuity
    /// (when Vision is on with general models). Inspired by NeKoharu.
    pub combine_pages: bool,
    /// Optional upper limit on pages per combined request (or neighbor window
    /// when Vision is on). `None` sizes batches automatically from the model's
    /// context and output limits.
    #[serde(deserialize_with = "max_pages")]
    #[specta(type = Option<u32>)]
    pub max_pages_per_request: Option<u32>,
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
            combine_pages: true,
            max_pages_per_request: None,
        }
    }
}

/// The system prompt: Index-Translate's structured-data template in English,
/// as bilibili/Index-Translate's `docs/prompts.md` gives it
/// (93ac63774d1c6939aec05e6f0f191bf90f225365, Apache-2.0), for the JSON the
/// answer is parsed by; then optional multi-page continuity guidance, then the
/// user's instructions as they wrote them, whatever their form.
fn system(config: &TranslatorConfig) -> Result<String> {
    system_with_extra(config, None)
}

fn system_with_extra(config: &TranslatorConfig, extra: Option<&str>) -> Result<String> {
    let mut prompt = formatdoc! {"
        Translate the manga text in each message into {target}. Each message is a page's texts as a JSON object, numbered in reading order: translate only the values, and never alter the structure or the keys. Output only the translated JSON, without any explanation.",
        target = name(&config.target)?,
    };
    if let Some(extra) = extra.map(str::trim).filter(|s| !s.is_empty()) {
        prompt.push_str("\n\n");
        prompt.push_str(extra);
    }
    if let instructions @ Some(_) = Some(config.instructions.trim()).filter(|s| !s.is_empty()) {
        prompt.push_str("\n\n");
        prompt.push_str(instructions.unwrap());
    }
    Ok(prompt)
}

fn target<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LanguageIdentifier, D::Error> {
    let target = LanguageIdentifier::deserialize(deserializer)?;
    name(&target).map_err(D::Error::custom)?;
    Ok(target)
}

fn max_pages<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    let limit = Option::<u32>::deserialize(deserializer)?;
    match limit {
        Some(0) => Err(D::Error::custom(
            "max_pages_per_request must be at least 1, or omitted for Auto",
        )),
        other => Ok(other),
    }
}

/// A page's texts, and the page as a JPEG in base 64 when the model sees it.
struct Page {
    /// Each block and its source's language.
    blocks: Vec<(LayerId, LanguageIdentifier)>,
    texts: Vec<String>,
    image: Option<String>,
}

async fn load_model(
    task: &Task,
    members: &[&Member],
    config: &PipelineConfig,
    translator: &TranslatorConfig,
    conversation: &mut Conversation,
) -> Result<DynModel<Completion>> {
    match key(Stage::Translate, config) {
        // A local model loads on the GPU thread, which only hands it over:
        // its engine runs on a thread of its own, so the turn waits off the
        // GPU thread, and the other stages go on.
        Some(key) => {
            let (reference, images) = (translator.model.clone(), translator.vision);
            let runs: Vec<_> = members.iter().map(|m| m.run).collect();
            task.load_on_gpu(members, move |models| {
                let model = models.get_tracked(&key, &runs, |_| async {
                    let connected = model::load(&reference, images).await?;
                    Ok(LoadOutcome {
                        model: connected.model,
                        teardown: connected.teardown,
                        full_gpu: connected.full_gpu,
                    })
                })?;
                Ok(DynModel::clone(model))
            })
            .await
        }
        None => conversation.hosted(translator).await,
    }
}

struct PageWork {
    member: Member,
    page_id: PageId,
    page: Page,
}

pub(crate) async fn translate(
    task: &Task,
    config: &PipelineConfig,
    mut conversation: OwnedMutexGuard<Conversation>,
) -> Result<Vec<(NodeIndex, String)>> {
    let translator = config.translator.clone();
    let members = task.members.clone();
    let vision = translator.vision;
    let work: Vec<PageWork> = tokio::task::spawn_blocking(move || -> Result<Vec<PageWork>> {
        let mut out = Vec::new();
        for member in members {
            if member.cancel.is_cancelled() {
                continue;
            }
            let page_id = member.page_id();
            let _span =
                info_span!("read", stage = %Stage::Translate, page = member.index + 1).entered();
            if let Some(page) = page(&member, vision)? {
                out.push(PageWork {
                    member,
                    page_id,
                    page,
                });
            }
        }
        Ok(out)
    })
    .await??;

    if work.is_empty() {
        return Ok(Vec::new());
    }

    let conversation = &mut *conversation;
    conversation.keep(&translator);
    let active_refs: Vec<&Member> = work.iter().map(|item| &item.member).collect();
    let model = load_model(task, &active_refs, config, &translator, conversation).await?;

    // Single page or Vision mode: translate one page at a time (attaching
    // read-only marked `<previous-pages>` / `<upcoming-pages>` context for
    // general models when Vision + combine_pages are enabled).
    if work.len() == 1 || translator.vision || !translator.combine_pages {
        let mut failures = Vec::new();
        for item in work {
            if item.member.cancel.is_cancelled() {
                continue;
            }
            task.started(&[&item.member]);
            match translate_single_page(&item, &translator, conversation, &model).await {
                Ok(()) => {}
                Err(error) if error.is::<Stopped>() => continue,
                Err(error) if error.is::<Failed>() => {
                    failures.push((item.member.job, format!("translate failed: {error:#}")));
                }
                Err(error) => return Err(error),
            }
        }
        return Ok(failures);
    }

    // Multi-page text-only batching: pack consecutive pages dynamically to fit
    // the model's context & output limits (differentiating specialized
    // `TRANSLATION` models from general LLMs), with automatic per-page fallback.
    let budget = batch::combined_budget(
        &model,
        translator.max_pages_per_request,
        translator.reasoning,
    );
    let page_tokens: Vec<u64> = work
        .iter()
        .map(|item| batch::estimate_page_tokens(&item.page.texts))
        .collect();
    let ranges = batch::pack_indices(&page_tokens, budget);

    let mut failures = Vec::new();
    for range in ranges {
        let sub: Vec<&PageWork> = work[range]
            .iter()
            .filter(|item| !item.member.cancel.is_cancelled())
            .collect();
        if sub.is_empty() {
            continue;
        }
        let sub_members: Vec<&Member> = sub.iter().map(|item| &item.member).collect();
        task.started(&sub_members);

        if sub.len() == 1 {
            match translate_single_page(sub[0], &translator, conversation, &model).await {
                Ok(()) => {}
                Err(error) if error.is::<Stopped>() => continue,
                Err(error) if error.is::<Failed>() => {
                    failures.push((sub[0].member.job, format!("translate failed: {error:#}")));
                }
                Err(error) => return Err(error),
            }
            continue;
        }

        match translate_combined_group(&sub, &translator, conversation, &model).await {
            Ok(()) => {}
            Err(error) if error.is::<Stopped>() => return Err(Stopped.into()),
            Err(error) if error.is::<Failed>() => {
                tracing::warn!(
                    pages = sub.len(),
                    "multi-page translation batch failed ({error:#}); falling back to page-by-page"
                );
                for item in sub {
                    if item.member.cancel.is_cancelled() {
                        continue;
                    }
                    match translate_single_page(item, &translator, conversation, &model).await {
                        Ok(()) => {}
                        Err(err) if err.is::<Stopped>() => continue,
                        Err(err) if err.is::<Failed>() => {
                            failures.push((item.member.job, format!("translate failed: {err:#}")));
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
            Err(error) => return Err(error),
        }
    }

    Ok(failures)
}

async fn translate_single_page(
    item: &PageWork,
    translator: &TranslatorConfig,
    conversation: &mut Conversation,
    model: &DynModel<Completion>,
) -> Result<()> {
    let mut turn = Turn::new(
        system(translator)?,
        &item.page.texts,
        item.page.image.as_deref(),
        translator.reasoning,
    );

    if translator.combine_pages
        && translator.vision
        && let Some((token_budget, page_cap)) =
            batch::neighbor_budget(model, translator.max_pages_per_request)
    {
        let include_previous = conversation.context.messages().is_empty();
        let member = item.member.clone();
        let page_id = item.page_id;
        let (previous, upcoming) = tokio::task::spawn_blocking(move || {
            let project = member.session.lock().project();
            collect_neighbor_pages(&project, page_id, include_previous, token_budget, page_cap)
        })
        .await?;
        let (body, has_context) =
            batch::format_neighbor_prompt(&previous, turn.page_json(), &upcoming);
        if has_context {
            let extra = "Only translate the JSON object inside <current-page>. Any <previous-pages> or <upcoming-pages> sections are read-only context from neighboring pages for character-name, tone, and plot continuity; never translate or include them in your JSON output.";
            turn = Turn::new(
                system_with_extra(translator, Some(extra))?,
                &item.page.texts,
                item.page.image.as_deref(),
                translator.reasoning,
            )
            .with_prompt_body(body);
        }
    }

    let answer = turn
        .take(conversation, model)
        .instrument(info_span!("llm", texts = item.page.texts.len()));
    let translations = tokio::select! {
        answer = answer => answer?,
        () = item.member.cancel.cancelled() => return Err(Stopped.into()),
    };
    commit_page_translations(item, translations, translator).await
}

async fn translate_combined_group(
    group: &[&PageWork],
    translator: &TranslatorConfig,
    conversation: &mut Conversation,
    model: &DynModel<Completion>,
) -> Result<()> {
    let total_texts: usize = group.iter().map(|item| item.page.texts.len()).sum();
    let width = total_texts.to_string().len();
    let mut all_texts = Vec::with_capacity(total_texts);
    let mut boundaries = Vec::with_capacity(group.len());
    for item in group {
        let start = all_texts.len() + 1;
        all_texts.extend(item.page.texts.iter().cloned());
        let end = all_texts.len();
        boundaries.push(format!(
            "Page {}: keys {:0width$}–{:0width$}",
            item.member.index + 1,
            start,
            end
        ));
    }

    // Specialized `TRANSLATION` models (Index-Translate) keep their exact
    // bare-JSON prompt without extra prose wrappers; general LLMs get explicit
    // page-boundary guidance for cross-page continuity.
    let prompt_system = if batch::is_translation_only(model) {
        system(translator)?
    } else {
        let note = format!(
            "This message combines consecutive manga pages in reading order ({}). Keep character names, terminology, voices, and plot continuity consistent across all pages, and return every key in one JSON object.",
            boundaries.join("; ")
        );
        system_with_extra(translator, Some(&note))?
    };

    let turn = Turn::new(prompt_system, &all_texts, None, translator.reasoning);
    let answer = turn.take(conversation, model).instrument(info_span!(
        "llm",
        pages = group.len(),
        texts = all_texts.len()
    ));
    let cancels: Vec<_> = group
        .iter()
        .map(|item| item.member.cancel.clone())
        .collect();
    let all_cancelled = async move {
        for token in cancels {
            token.cancelled().await;
        }
    };
    let translations = tokio::select! {
        answer = answer => answer?,
        () = all_cancelled => return Err(Stopped.into()),
    };

    let mut iter = translations.into_iter();
    for item in group {
        let count = item.page.texts.len();
        let page_translations: Vec<String> = iter.by_ref().take(count).collect();
        if item.member.cancel.is_cancelled() {
            continue;
        }
        commit_page_translations(item, page_translations, translator).await?;
    }
    Ok(())
}

async fn commit_page_translations(
    item: &PageWork,
    translations: Vec<String>,
    translator: &TranslatorConfig,
) -> Result<()> {
    let translated = item
        .page
        .blocks
        .iter()
        .cloned()
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
    let (read, page_id, target) = (item.member.clone(), item.page_id, translator.target.clone());
    let translated = tokio::task::spawn_blocking(move || {
        let _span = info_span!("place", stage = %Stage::Translate, page = read.index + 1).entered();
        let project = read.session.lock().project();
        placed(&project, page_id, translated, &target)
    })
    .await??;
    let translator = translator.clone();
    item.member
        .apply(Stage::Translate, move |session| {
            Ok(written(&session.project(), translated, &translator))
        })
        .await
}

type PreviousContext = Vec<(usize, Vec<(String, Option<String>)>)>;
type UpcomingContext = Vec<(usize, Vec<String>)>;

fn collect_neighbor_pages(
    project: &Project,
    page_id: PageId,
    include_previous: bool,
    token_budget: u64,
    page_cap: usize,
) -> (PreviousContext, UpcomingContext) {
    let pages: Vec<PageId> = project.pages().map(|p| p.id).collect();
    let Some(pos) = pages.iter().position(|&id| id == page_id) else {
        return (Vec::new(), Vec::new());
    };

    let mut previous = Vec::new();
    let mut prev_tokens = 0u64;
    if include_previous && pos > 0 {
        let prev_budget = token_budget / 2;
        for idx in (0..pos).rev() {
            if previous.len() >= page_cap {
                break;
            }
            let entries: Vec<(String, Option<String>)> = project
                .blocks(pages[idx])
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(_, block)| {
                    let src = block.source.content.text();
                    if src.trim().is_empty() {
                        return None;
                    }
                    let tr = block
                        .translation
                        .as_ref()
                        .map(|t| t.content.text())
                        .filter(|s| !s.trim().is_empty());
                    Some((src, tr))
                })
                .collect();
            if entries.is_empty() {
                continue;
            }
            let cost: u64 = entries
                .iter()
                .map(|(src, tr)| {
                    batch::estimate_text_tokens(src)
                        + tr.as_deref().map_or(0, batch::estimate_text_tokens)
                })
                .sum();
            if prev_tokens + cost > prev_budget && !previous.is_empty() {
                break;
            }
            prev_tokens = prev_tokens.saturating_add(cost);
            previous.push((idx + 1, entries));
        }
        previous.reverse();
    }

    let mut upcoming = Vec::new();
    let remaining_budget = token_budget.saturating_sub(prev_tokens);
    let mut up_tokens = 0u64;
    for (idx, &pid) in pages.iter().enumerate().skip(pos + 1) {
        if upcoming.len() >= page_cap {
            break;
        }
        let entries: Vec<String> = project
            .blocks(pid)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, block)| block.source.content.text())
            .filter(|s| !s.trim().is_empty())
            .collect();
        if entries.is_empty() {
            continue;
        }
        let cost = batch::estimate_page_tokens(&entries);
        if up_tokens + cost > remaining_budget && !upcoming.is_empty() {
            break;
        }
        up_tokens = up_tokens.saturating_add(cost);
        upcoming.push((idx + 1, entries));
    }

    (previous, upcoming)
}

pub(crate) async fn metadata(
    task: &Task,
    config: &PipelineConfig,
    mut conversation: OwnedMutexGuard<Conversation>,
) -> Result<()> {
    let member = &task.members[0];
    if member.cancel.is_cancelled() {
        return Err(Stopped.into());
    }
    let translator = config.translator.clone();
    let read = member.clone();
    let sent: Vec<(&'static str, String)> = tokio::task::spawn_blocking(move || {
        let _span = info_span!("read", stage = %Stage::Translate, metadata = true).entered();
        let project = read.session.lock().project();
        let Some(value) = project.metadata(COMIC_INFO_KEY) else {
            return Ok(Vec::new());
        };
        let info: ComicInfo = serde_json::from_value(value.clone())
            .context("stored ComicInfo metadata is invalid")?;
        Ok::<_, anyhow::Error>(
            info.translatable_fields()
                .into_iter()
                .map(|(name, text)| (name, text.to_owned()))
                .collect(),
        )
    })
    .await??;
    if sent.is_empty() {
        return Ok(());
    }
    let conversation = &mut *conversation;
    conversation.keep(&translator);
    let translations = {
        let model = load_model(task, &[member], config, &translator, conversation).await?;
        task.started(&[member]);
        let keys: Vec<String> = sent.iter().map(|(name, _)| (*name).to_owned()).collect();
        let originals: Vec<String> = sent.iter().map(|(_, text)| text.clone()).collect();
        let turn = Turn::keyed(
            system(&translator)?,
            keys,
            &originals,
            None,
            translator.reasoning,
        );
        let answer = turn
            .take(conversation, &model)
            .instrument(info_span!("llm", metadata_fields = originals.len()));
        tokio::select! {
            answer = answer => answer?,
            () = member.cancel.cancelled() => return Err(Stopped.into()),
        }
    };
    member
        .apply(Stage::Translate, move |session| {
            let project = session.project();
            Ok(write_metadata(&project, sent, translations)?)
        })
        .await
}

fn write_metadata(
    project: &Project,
    sent: Vec<(&'static str, String)>,
    translations: Vec<String>,
) -> Result<Vec<Op>> {
    let Some(value) = project.metadata(COMIC_INFO_KEY) else {
        return Ok(Vec::new());
    };
    let mut info: ComicInfo =
        serde_json::from_value(value.clone()).context("stored ComicInfo metadata is invalid")?;
    let mut changed = false;
    for ((field_name, expected_original), translated) in sent.into_iter().zip(translations) {
        changed |= info.set_translated_if_unchanged(field_name, &expected_original, translated);
    }
    if !changed {
        return Ok(Vec::new());
    }
    Ok(vec![Op::SetProjectMetadata {
        key: COMIC_INFO_KEY.to_owned(),
        value: Some(serde_json::to_value(&info)?),
    }])
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

    #[test]
    fn write_metadata_skips_fields_whose_original_changed_in_flight() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let initial = ComicInfo {
            title: Some(koharu_io::LocalizedField::new("Edited In Flight")),
            series: Some(koharu_io::LocalizedField::new("よつばと!")),
            ..Default::default()
        };
        session.apply(Edit {
            label: "meta".to_owned(),
            ops: vec![Op::SetProjectMetadata {
                key: COMIC_INFO_KEY.to_owned(),
                value: Some(serde_json::to_value(&initial)?),
            }],
        })?;

        let ops = write_metadata(
            &session.project(),
            vec![
                ("Title", "Old Title".to_owned()),
                ("Series", "よつばと!".to_owned()),
            ],
            vec!["Stale Title".to_owned(), "Yotsuba&!".to_owned()],
        )?;
        session.apply(Edit {
            label: "translate".to_owned(),
            ops,
        })?;
        let updated: ComicInfo = serde_json::from_value(
            session
                .project()
                .metadata(COMIC_INFO_KEY)
                .expect("comic_info")
                .clone(),
        )?;
        assert_eq!(updated.title.as_ref().unwrap().translated, None);
        assert_eq!(
            updated.series.as_ref().unwrap().translated.as_deref(),
            Some("Yotsuba&!")
        );
        Ok(())
    }

    #[test]
    fn translator_config_defaults_combine_pages_on_and_rejects_zero_max_pages() {
        let cfg = TranslatorConfig::default();
        assert!(cfg.combine_pages);
        assert_eq!(cfg.max_pages_per_request, None);

        assert!(
            serde_json::from_str::<TranslatorConfig>(r#"{"max_pages_per_request": 0}"#).is_err()
        );
        let parsed: TranslatorConfig =
            serde_json::from_str(r#"{"combine_pages": false, "max_pages_per_request": 4}"#)
                .unwrap();
        assert!(!parsed.combine_pages);
        assert_eq!(parsed.max_pages_per_request, Some(4));
    }

    #[tokio::test]
    async fn combined_group_differentiates_general_and_translation_models_and_falls_back_per_page()
    -> Result<()> {
        use rig_core::{
            catalog::ModelSpec,
            providers::registry::ProviderId,
            test_utils::{MockCompletionModel, MockTurn},
        };
        use tokio_util::sync::CancellationToken;

        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("multi.khr"), koharu_scene::Meta::default())?;
        session.apply(Edit {
            label: "pages".to_owned(),
            ops: serde_json::from_value(json!([
                {"op": "add_page", "id": "$p1", "index": 0, "size": {"width": 400, "height": 400}},
                {"op": "add_layer", "page": "$p1", "index": 0, "layer": {"kind": "block", "source": {
                    "content": "おはよう", "language": "ja",
                    "frame": {"box": {"bounds": {"x": 20.0, "y": 20.0, "width": 60.0, "height": 100.0}}},
                }}},
                {"op": "add_page", "id": "$p2", "index": 1, "size": {"width": 400, "height": 400}},
                {"op": "add_layer", "page": "$p2", "index": 0, "layer": {"kind": "block", "source": {
                    "content": "おやすみ", "language": "ja",
                    "frame": {"box": {"bounds": {"x": 20.0, "y": 20.0, "width": 60.0, "height": 100.0}}},
                }}},
            ]))?,
        })?;
        let page_ids: Vec<PageId> = session.project().pages().map(|p| p.id).collect();
        let session = std::sync::Arc::new(parking_lot::Mutex::new(session));

        let build_work = |vision: bool| -> Vec<PageWork> {
            page_ids
                .iter()
                .enumerate()
                .map(|(index, &page_id)| {
                    let member = Member {
                        run: crate::RunId(1),
                        job: NodeIndex::new(index),
                        page: Some(page_id),
                        index,
                        blocks: None,
                        session: std::sync::Arc::clone(&session),
                        cancel: CancellationToken::new(),
                        started: Default::default(),
                    };
                    let page = page(&member, vision).unwrap().unwrap();
                    PageWork {
                        member,
                        page_id,
                        page,
                    }
                })
                .collect()
        };

        // 1. General model combined call gets page-boundary note and writes both pages.
        let general_mock = MockCompletionModel::from_turns([MockTurn::text(
            r#"{"1": "Good morning", "2": "Good night"}"#,
        )])
        .priced_as(
            ModelSpec::new(ProviderId::declare("test")?, "general").with_context_window(64_000),
        );
        let work = build_work(false);
        let refs: Vec<&PageWork> = work.iter().collect();
        let mut conv = Conversation::new();
        translate_combined_group(
            &refs,
            &TranslatorConfig::default(),
            &mut conv,
            &general_mock.clone().erase(),
        )
        .await?;
        let req_dump = format!("{:?}", general_mock.requests()[0].chat_history);
        assert!(
            req_dump.contains("Page 1: keys 1–1; Page 2: keys 2–2"),
            "general model should receive page boundary guidance: {req_dump}"
        );
        let p1_text = session.lock().project().blocks(page_ids[0])?[0]
            .1
            .translation
            .as_ref()
            .unwrap()
            .content
            .text();
        let p2_text = session.lock().project().blocks(page_ids[1])?[0]
            .1
            .translation
            .as_ref()
            .unwrap()
            .content
            .text();
        assert_eq!(p1_text, "Good morning");
        assert_eq!(p2_text, "Good night");

        // 2. Specialized TRANSLATION model (Index-Translate) keeps bare JSON prompt without extra boundary prose.
        let mut tr_spec = ModelSpec::new(ProviderId::declare("test")?, "index-translate")
            .with_context_window(32_768);
        tr_spec.details.kind = Some(koharu_llm::TRANSLATION.to_owned());
        let tr_mock = MockCompletionModel::from_turns([MockTurn::text(
            r#"{"1": "Morning!", "2": "Night!"}"#,
        )])
        .priced_as(tr_spec);
        let work2 = build_work(false);
        let refs2: Vec<&PageWork> = work2.iter().collect();
        translate_combined_group(
            &refs2,
            &TranslatorConfig::default(),
            &mut Conversation::new(),
            &tr_mock.clone().erase(),
        )
        .await?;
        let tr_req = format!("{:?}", tr_mock.requests()[0].chat_history);
        assert!(
            !tr_req.contains("Page 1: keys"),
            "TRANSLATION-kind model must not receive extra boundary prose"
        );

        // 3. Combined batch failure (`Failed`) surfaces after schema retry so caller falls back page-by-page without fatal abort.
        let failing_mock = MockCompletionModel::from_turns([
            MockTurn::text(r#"{"1": "Only one key"}"#),
            MockTurn::text(r#"{"1": "Still missing key 2"}"#),
        ])
        .priced_as(
            ModelSpec::new(ProviderId::declare("test")?, "general").with_context_window(64_000),
        );
        let work_fail = build_work(false);
        let refs_fail: Vec<&PageWork> = work_fail.iter().collect();
        let err = translate_combined_group(
            &refs_fail,
            &TranslatorConfig::default(),
            &mut Conversation::new(),
            &failing_mock.erase(),
        )
        .await
        .expect_err("missing key in combined batch must fail closed");
        assert!(
            err.is::<Failed>(),
            "missing key must be page/batch-level Failed for fallback: {err:#}"
        );

        // 4. Vision mode on general model attaches `<upcoming-pages>` context and strips it from Conversation history.
        let mut vision_cfg = TranslatorConfig::default();
        vision_cfg.vision = true;
        let vision_mock =
            MockCompletionModel::from_turns([MockTurn::text(r#"{"1": "Hey morning"}"#)]).priced_as(
                ModelSpec::new(ProviderId::declare("test")?, "vision-general")
                    .with_context_window(64_000),
            );
        let work_v = build_work(false);
        let mut conv_v = Conversation::new();
        translate_single_page(
            &work_v[0],
            &vision_cfg,
            &mut conv_v,
            &vision_mock.clone().erase(),
        )
        .await?;
        let v_prompt = format!("{:?}", vision_mock.requests()[0].chat_history);
        assert!(
            v_prompt.contains("<current-page>"),
            "missing <current-page>: {v_prompt}"
        );
        assert!(
            v_prompt.contains("<upcoming-pages>"),
            "missing <upcoming-pages>: {v_prompt}"
        );
        assert!(
            v_prompt.contains("おやすみ"),
            "missing upcoming page text: {v_prompt}"
        );
        // Stored conversation history must NOT retain `<upcoming-pages>`!
        let stored_history = format!("{:?}", conv_v.context.messages());
        assert!(
            !stored_history.contains("<upcoming-pages>"),
            "conversation history must strip <upcoming-pages>: {stored_history}"
        );

        // 5. Vision mode on specialized TRANSLATION model skips `<upcoming-pages>` prose wrapper.
        let mut tr_vision_spec = ModelSpec::new(ProviderId::declare("test")?, "index-translate")
            .with_context_window(32_768);
        tr_vision_spec.details.kind = Some(koharu_llm::TRANSLATION.to_owned());
        let tr_vision_mock =
            MockCompletionModel::from_turns([MockTurn::text(r#"{"1": "Mornin"}"#)])
                .priced_as(tr_vision_spec);
        let work_tv = build_work(false);
        translate_single_page(
            &work_tv[0],
            &vision_cfg,
            &mut Conversation::new(),
            &tr_vision_mock.clone().erase(),
        )
        .await?;
        let tv_prompt = format!("{:?}", tr_vision_mock.requests()[0].chat_history);
        assert!(
            !tv_prompt.contains("<upcoming-pages>"),
            "TRANSLATION-kind model with Vision must not get <upcoming-pages> prose wrapper"
        );
        Ok(())
    }
}
