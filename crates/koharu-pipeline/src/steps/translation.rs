//! Translation: a page's text blocks, grouped by balloon, to an LLM in one
//! request, answered as texts that each cover a run of one balloon's blocks.

use anyhow::{Result, anyhow, bail, ensure};
use futures::future::join_all;
use indoc::formatdoc;
use koharu_llm::{
    Provider,
    rig::completion::{
        CompletionRequest, FinishReason, Message,
        message::{DocumentSourceKind, Image, ImageMediaType, MimeType, UserContent},
    },
};
use koharu_scene::{Edit, Layer, LayerId, LayerView, Op, PageView, Pointer, Read, Ref, Session};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Context, Prepared, Scope};
use crate::{Step, TranslationConfig};

pub(crate) struct Work {
    page: Ref,
    /// The blocks to translate, a group per balloon, in reading order; a
    /// block outside any balloon is a group of its own.
    groups: Vec<Vec<Block>>,
    /// Every block's `/content`, the context each translation also read.
    context: Vec<Read>,
    scan: Option<(Vec<u8>, String, Read)>,
    source_language: String,
    previous: Vec<Previous>,
    top: usize,
    reads: Vec<Read>,
}

struct Block {
    id: LayerId,
    text: String,
    read: Read,
}

/// A translation an earlier run made of a block being translated again.
struct Previous {
    id: LayerId,
    /// The block it translates.
    parent: Option<LayerId>,
    agent: String,
}

pub(super) fn read(
    session: &Session,
    view: &PageView,
    scope: &Scope,
    context: &Context,
) -> Result<Prepared<Work>> {
    let layers = super::walk(&view.layers);
    let blocks: Vec<&LayerView> = super::blocks(&layers)
        .into_iter()
        .filter(|block| match &block.layer {
            Layer::Text(text) => !text.content.text().trim().is_empty(),
            _ => false,
        })
        .collect();
    if blocks.is_empty() {
        return Ok(Prepared::Skip(
            "the page has no text to translate".to_owned(),
        ));
    }
    let balloons = super::balloons(&layers);
    let mut groups: Vec<(Option<usize>, Vec<&LayerView>)> = Vec::new();
    for block in &blocks {
        let balloon = super::balloon_of(block, &balloons);
        match groups
            .iter_mut()
            .find(|(sits_in, _)| sits_in.is_some() && *sits_in == balloon)
        {
            Some((_, group)) => group.push(block),
            None => groups.push((balloon, vec![block])),
        }
    }
    let translations = super::translations(&layers);
    let covering = |block: LayerId| {
        translations
            .iter()
            .filter(move |(_, covers)| covers.contains(&block))
            .map(|(translation, _)| *translation)
    };
    let out_of_date = |translation: &LayerView| {
        translation
            .origin
            .iter()
            .filter(|record| record.action.action == super::TRANSLATED)
            .any(|record| {
                record
                    .writes
                    .iter()
                    .any(|path| translation.stale.contains(path))
            })
    };
    let groups: Vec<Vec<&LayerView>> = groups
        .into_iter()
        .map(|(_, group)| group)
        .filter(|group| match scope {
            Scope::Layers(layers) => group.iter().any(|block| layers.contains(&block.id)),
            Scope::Page | Scope::Area(_) => {
                context.redo
                    || group.iter().any(|block| {
                        let mut translations = covering(block.id).peekable();
                        translations.peek().is_none() || translations.any(out_of_date)
                    })
            }
        })
        .collect();
    if groups.is_empty() {
        return Ok(Prepared::Skip(
            "every text block already has its translation".to_owned(),
        ));
    }

    let content = |block: &LayerView| session.read(block.id, Pointer::from_static("/content"));
    let context_reads = blocks
        .iter()
        .map(|block| Ok(content(block)?.1))
        .collect::<Result<Vec<_>>>()?;
    let mut previous = Vec::new();
    let mut reads = Vec::new();
    let groups = groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .map(|block| {
                    for translation in covering(block.id) {
                        if !previous
                            .iter()
                            .any(|known: &Previous| known.id == translation.id)
                        {
                            reads.push(super::whole(translation));
                            previous.push(Previous {
                                id: translation.id,
                                parent: parent(translation),
                                agent: creator(translation),
                            });
                        }
                    }
                    let (text, read) = content(block)?;
                    Ok(Block {
                        id: block.id,
                        text: serde_json::from_value::<koharu_scene::RichText>(text)?.text(),
                        read,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let languages: Vec<String> = blocks
        .iter()
        .filter_map(|block| match &block.layer {
            Layer::Text(text) => Some(text.language.to_string()),
            _ => None,
        })
        .collect();
    let source_language = match languages.first() {
        Some(first) if languages.iter().all(|language| language == first) => first.clone(),
        _ => "und".to_owned(),
    };
    let scan = match (context.config.translation.vision, super::scan(view)) {
        (true, Some(scan)) => {
            let (file, read) = super::scan_file(session, view.layers[scan].id)?;
            Some((
                file.bytes,
                file.media_type,
                Read {
                    relationship: koharu_scene::Relationship::InputTo,
                    ..read
                },
            ))
        }
        _ => None,
    };
    Ok(Prepared::Work(Work {
        page: view.id.into(),
        groups,
        context: context_reads,
        scan,
        source_language,
        previous,
        top: view.layers.len(),
        reads,
    }))
}

/// The block a translation names as its `parentOf`.
fn parent(translation: &LayerView) -> Option<LayerId> {
    translation
        .origin
        .iter()
        .flat_map(|record| &record.ingredients)
        .find(|read| read.relationship == koharu_scene::Relationship::ParentOf)
        .map(|read| read.layer)
}

/// The software agent that created a layer.
fn creator(layer: &LayerView) -> String {
    layer
        .origin
        .iter()
        .find(|record| record.writes.iter().any(|path| path.is_root()))
        .map(|record| record.action.software_agent.name.clone())
        .unwrap_or_default()
}

/// Sends every page's request at once, so a local model decodes them
/// together, and checks each answer.
pub(super) async fn run(
    model: &koharu_llm::Model,
    works: Vec<Work>,
    context: &Context,
) -> Vec<Result<Edit>> {
    let config = &context.config.translation;
    let answers = join_all(works.iter().map(|work| async move {
        let request = model.structured(request(work, config)?, schema());
        let reply = model.completion.call(request).await?;
        if let Some(error) = reply.error {
            bail!("the model gave no translation: {error}");
        }
        match reply.finish_reason() {
            Some(FinishReason::Length) => bail!("the answer was cut off at the token limit"),
            Some(FinishReason::ContentFilter) => bail!("the provider filtered the answer"),
            _ => {}
        }
        let answer: Answer = serde_json::from_str(reply.text().trim())
            .map_err(|error| anyhow!("the answer isn't the translation's JSON: {error}"))?;
        check(work, &answer, &config.target_language)?;
        Ok(answer)
    }))
    .await;
    works
        .into_iter()
        .zip(answers)
        .map(|(work, answer)| edit(context, work, answer?))
        .collect()
}

#[derive(Deserialize)]
pub(super) struct Answer {
    pub(super) texts: Vec<Text>,
}

#[derive(Deserialize)]
pub(super) struct Text {
    pub(super) first: usize,
    pub(super) last: usize,
    pub(super) text: String,
}

/// The answer's shape. Every property is required and nothing sits beside a
/// `$ref`, so rig's rewrite toward strict mode changes nothing.
fn schema() -> schemars::Schema {
    let text = json!({
        "type": "object",
        "properties": {
            "first": {"type": "integer"},
            "last": {"type": "integer"},
            "text": {"type": "string"},
        },
        "required": ["first", "last", "text"],
        "additionalProperties": false,
    });
    schemars::Schema::try_from(json!({
        "type": "object",
        "properties": {"texts": {"type": "array", "items": text}},
        "required": ["texts"],
        "additionalProperties": false,
    }))
    .expect("a JSON schema")
}

/// The page's blocks numbered from 1 in reading order, by balloon, with the
/// scan before them when vision is on.
fn request(work: &Work, config: &TranslationConfig) -> Result<CompletionRequest> {
    let target = &config.target_language;
    let mut instructions = formatdoc! {"
        You translate the text of one manga page into the language whose BCP 47 tag is {target}.

        The page's text blocks are numbered in reading order and grouped by the speech balloon \
        they sit in; a block outside any balloon is a group of its own. Answer each group with \
        texts that each cover a run of its blocks, from `first` to `last`: blocks that read as \
        one thought become one text. Cover every block of every group once, in order, and never \
        cover blocks of two groups with one text.

        Translate as a professional manga translator would, keeping each speaker's voice, and \
        write every text in {target}.
    "};
    if let Some(extra) = &config.instructions {
        instructions.push('\n');
        instructions.push_str(extra);
    }
    let mut listing = String::new();
    let mut id = 0;
    for (index, group) in work.groups.iter().enumerate() {
        listing.push_str(&format!("Group {}\n", index + 1));
        for block in group {
            id += 1;
            listing.push_str(&format!("{id}: {}\n", block.text.replace('\n', " ")));
        }
        listing.push('\n');
    }
    let mut content = Vec::new();
    if let Some((bytes, media_type, _)) = &work.scan {
        content.push(UserContent::Image(Image {
            data: DocumentSourceKind::Raw(bytes.clone()),
            media_type: ImageMediaType::from_mime_type(media_type),
            detail: None,
            native: None,
        }));
    }
    content.push(UserContent::text(listing));
    let mut request = CompletionRequest::new(Message::User { content }).preamble(instructions);
    let sampling = &config.sampling;
    request.temperature = sampling.temperature;
    request.max_tokens = sampling.max_tokens;
    // llama.cpp's own settings mean nothing to a hosted provider.
    if config.model.provider == Provider::Local {
        request.additional_params = Some(serde_json::to_value(sampling)?);
    }
    Ok(request)
}

/// Each balloon's texts cover its blocks in order, without gaps or overlaps,
/// and each is in the target language.
pub(super) fn check(work: &Work, answer: &Answer, target: &str) -> Result<()> {
    let mut texts = answer.texts.iter();
    let mut next = 1;
    for group in &work.groups {
        let last = next + group.len() - 1;
        while next <= last {
            let Some(text) = texts.next() else {
                bail!("the answer leaves block {next} untranslated");
            };
            ensure!(
                text.first == next && text.first <= text.last && text.last <= last,
                "the answer's text for blocks {}–{} doesn't continue from block {next} within its balloon",
                text.first,
                text.last,
            );
            ensure!(
                in_language(&text.text, target),
                "the answer's text for blocks {}–{} isn't in {target}",
                text.first,
                text.last,
            );
            next = text.last + 1;
        }
    }
    ensure!(
        texts.next().is_none(),
        "the answer has texts past the last block"
    );
    Ok(())
}

/// Whether `text` can be in `target`: outside Chinese, Japanese, and Korean,
/// a text still in their scripts wasn't translated.
fn in_language(text: &str, target: &str) -> bool {
    let primary = target.split(['-', '_']).next().unwrap_or(target);
    matches!(primary, "ja" | "zh" | "ko" | "yue")
        || !text.chars().any(|c| {
            matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}')
        })
}

pub(super) fn edit(context: &Context, work: Work, answer: Answer) -> Result<Edit> {
    let config = &context.config.translation;
    let agent = config.model.model.as_str();
    let blocks: Vec<&Block> = work.groups.iter().flatten().collect();
    let mut ops = Vec::new();
    let mut top = work.top;
    let mut kept = Vec::new();
    for text in answer.texts {
        let covered = &blocks[text.first - 1..text.last];
        let parent = covered[0];
        let ingredients: Vec<Read> = std::iter::once(parent.read.clone().parent_of())
            .chain(
                work.context
                    .iter()
                    .filter(|read| read.layer != parent.id)
                    .cloned(),
            )
            .chain(work.scan.iter().map(|(_, _, read)| read.clone()))
            .collect();
        let covers: Vec<String> = covered.iter().map(|block| block.id.to_string()).collect();
        let properties = json!({
            "language": config.target_language,
            "attributes": {"koharu": {"covers": covers}},
        });
        match work
            .previous
            .iter()
            .find(|previous| previous.parent == Some(parent.id))
        {
            Some(previous) => {
                kept.push(previous.id);
                ops.push(Op::SetText {
                    layer: previous.id.into(),
                    text: text.text,
                    ingredients: ingredients.clone(),
                });
                ops.push(Op::UpdateLayer {
                    layer: previous.id.into(),
                    patch: properties,
                    ingredients,
                });
            }
            None => {
                let mut layer = properties;
                let object = layer.as_object_mut().expect("an object");
                object.insert("kind".to_owned(), json!("text"));
                object.insert("content".to_owned(), Value::String(text.text));
                ops.push(Op::AddLayer {
                    id: None,
                    page: work.page.clone(),
                    parent: None,
                    index: top,
                    layer: serde_json::from_value(layer)?,
                    ingredients,
                });
                top += 1;
            }
        }
    }
    // What this run no longer produces goes, but a model removes only what it
    // created: another model's translation stays, out of date.
    for previous in &work.previous {
        if !kept.contains(&previous.id) && previous.agent == agent {
            ops.push(Op::RemoveLayer {
                layer: previous.id.into(),
            });
        }
    }
    let parameters = Map::from_iter([
        ("sourceLanguage".to_owned(), json!(work.source_language)),
        ("targetLanguage".to_owned(), json!(config.target_language)),
    ]);
    let action = super::action(super::TRANSLATED, agent, parameters);
    Ok(super::edit(
        context.project,
        Step::Translation,
        action,
        work.reads,
        ops,
    ))
}
