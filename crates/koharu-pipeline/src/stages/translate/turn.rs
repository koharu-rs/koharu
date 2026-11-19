//! A page's turn of the conversation.

use anyhow::{Context as _, Result, bail, ensure};
use futures::future::join_all;
use koharu_llm::rig::{
    completion::{
        AssistantContent, Message, Reasoning, Usage,
        message::{DocumentSourceKind, Image, ImageMediaType, UserContent},
    },
    driver::DynModel,
    operation::Completion,
};
use koharu_scene::{GlossaryEntry, GlossaryKind, LayerId};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::{Conversation, rules::Rules};

const PROTECTED_BATCH_LINES: usize = 4;
const INDEPENDENT_HOSTED_WORKERS: usize = 4;

struct Batch<'a> {
    system: String,
    input: String,
    keys: Vec<String>,
    rules: Rules<'a>,
}

pub(super) struct Turn<'a> {
    system: String,
    layers: Vec<LayerId>,
    glossary: &'a [GlossaryEntry],
    texts: &'a [String],
    /// The page, which the model sees with its texts.
    image: Option<&'a str>,
    reasoning: Option<Reasoning>,
    cancel: &'a CancellationToken,
}

impl<'a> Turn<'a> {
    pub(super) fn new(
        system: String,
        texts: &'a [String],
        layers: impl ExactSizeIterator<Item = LayerId>,
        glossary: &'a [GlossaryEntry],
        image: Option<&'a str>,
        reasoning: Option<Reasoning>,
        cancel: &'a CancellationToken,
    ) -> Self {
        Self {
            system,
            layers: layers.collect(),
            glossary,
            texts,
            image,
            reasoning,
            cancel,
        }
    }

    /// The page's translations, recorded in `conversation` when its model uses
    /// history. `None` when the run stopped first.
    pub(super) async fn take(
        self,
        conversation: &mut Conversation,
        model: &DynModel<Completion>,
    ) -> Result<Option<Vec<String>>> {
        let source_keys = keys(self.texts.len());
        let isolated = model.spec().is_some_and(koharu_llm::go_deepseek);
        let protected = self.glossary.iter().any(|entry| {
            entry.kind == GlossaryKind::Preserve
                && self
                    .layers
                    .iter()
                    .zip(self.texts)
                    .any(|(&layer, text)| entry.applies_to(layer) && entry.matches(text))
        });
        let width = if isolated {
            1
        } else if protected {
            PROTECTED_BATCH_LINES
        } else {
            self.texts.len().max(1)
        };
        let batches: Vec<_> = self
            .layers
            .chunks(width)
            .zip(source_keys.chunks(width).zip(self.texts.chunks(width)))
            .map(|(layers, (keys, texts))| {
                let mut system = self.system.clone();
                let (rules, fields) = Rules::prepare(
                    self.glossary,
                    layers.iter().copied().zip(keys.iter().zip(texts)),
                    &mut system,
                );
                let (keys, values): (Vec<_>, Vec<_>) = fields.into_iter().unzip();
                let input = numbered(&keys, values.into_iter());
                Batch {
                    system,
                    input,
                    keys,
                    rules,
                }
            })
            .collect();

        if isolated {
            // Go's DeepSeek can permute the values of a correctly numbered reply.
            // Each request sees one source block, without earlier numbered turns.
            return Ok(self
                .independent(&batches, model, INDEPENDENT_HOSTED_WORKERS)
                .await?
                .map(|(translations, _)| translations));
        }

        let (translations, messages, usage) = if protected {
            // Batches on these pages have their own keys and literal positions.
            // Keep previous pages' numbered answers out of these requests.
            let workers = if model
                .spec()
                .is_some_and(|spec| spec.provider.vendor() != koharu_llm::KOHARU)
            {
                2
            } else {
                1
            };
            let Some((translations, usage)) = self.independent(&batches, model, workers).await?
            else {
                return Ok(None);
            };
            let messages = vec![
                Message::user(numbered(
                    &source_keys,
                    self.texts.iter().map(String::as_str),
                )),
                Message::assistant(numbered(
                    &source_keys,
                    translations.iter().map(String::as_str),
                )),
            ];
            (translations, messages, usage)
        } else {
            let batch = &batches[0];
            conversation.context.fit(model).await?;
            let history = conversation
                .context
                .messages()
                .iter()
                .map(|message| Message::clone(message))
                .collect();
            let Some(mut asked) = self.ask(batch, model, history).await? else {
                return Ok(None);
            };
            // A small model can echo the previous page without reading the new input.
            if self.echoes(&asked.0, conversation.last.as_ref()) {
                tracing::info!(
                    "the answer echoed the page before's; asking again without the history"
                );
                let Some(again) = self.ask(batch, model, Vec::new()).await? else {
                    return Ok(None);
                };
                asked = again;
            }
            let (translations, messages, usage) = asked;
            (translations, remembered(messages), usage)
        };
        conversation.context.record([(messages, usage)]);
        conversation.last = Some((self.texts.to_vec(), translations.clone()));
        Ok(Some(translations))
    }

    async fn independent(
        &self,
        batches: &[Batch<'_>],
        model: &DynModel<Completion>,
        workers: usize,
    ) -> Result<Option<(Vec<String>, Usage)>> {
        let mut translations = Vec::with_capacity(self.texts.len());
        let mut usage = Usage::default();
        for wave in batches.chunks(workers) {
            let mut requests = Vec::with_capacity(wave.len());
            for batch in wave {
                requests.push(self.ask(batch, model, Vec::new()));
            }
            for result in join_all(requests).await {
                let Some((lines, _, used)) = result? else {
                    return Ok(None);
                };
                translations.extend(lines);
                usage += used;
            }
        }
        Ok(Some((translations, usage)))
    }

    /// The page asked after `history`: its translations, the turn's
    /// messages, and their usage; `None` when the run stopped first.
    async fn ask(
        &self,
        batch: &Batch<'_>,
        model: &DynModel<Completion>,
        mut history: Vec<Message>,
    ) -> Result<Option<(Vec<String>, Vec<Message>, Usage)>> {
        if batch.keys.is_empty() {
            return Ok(Some((
                batch.rules.restore(&[]),
                Vec::new(),
                Usage::default(),
            )));
        }
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
        content.push(UserContent::text(batch.input.clone()));
        let prompt = Message::from(content);
        let schema = schema(&batch.keys)?;
        let mut system = batch.system.clone();
        let mut retry = false;
        loop {
            let request = koharu_llm::answers(model, schema.clone(), self.reasoning)
                .preamble(&system)
                .build()
                .prompt(prompt.clone())
                .history(std::mem::take(&mut history));
            let response = tokio::select! {
                response = koharu_llm::complete(model, request) => response?,
                () = self.cancel.cancelled() => return Ok(None),
            };
            match answers(&response.output(), &batch.keys) {
                Ok(fragments) => {
                    let usage = response
                        .completion_calls
                        .last()
                        .map_or_else(Usage::default, |call| call.usage);
                    return Ok(Some((
                        batch.rules.restore(&fragments),
                        response.messages,
                        usage,
                    )));
                }
                Err(error) if !retry => {
                    tracing::warn!(%error, "retrying page after invalid translation output");
                    system.push_str(&format!(
                        "\n\nThe previous answer failed validation: {error}. Regenerate the complete JSON from the supplied input. Return a non-empty string for every input key, including each numbered fragment."
                    ));
                    retry = true;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Whether `translations` repeat the `last` page's, in order, for at least
    /// two texts and half of them, though the texts differ.
    fn echoes(&self, translations: &[String], last: Option<&(Vec<String>, Vec<String>)>) -> bool {
        let Some((texts, answers)) = last else {
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
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(text),
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
            [" One ", "Two"]
        );
        assert!(answers(r#"{"1": "One"}"#, &keys).is_err());
        assert!(answers(r#"{"1": "One", "2": " "}"#, &keys).is_err());
        assert!(answers(r#"{"1": "One", "2": "Two", "3": "Three"}"#, &keys).is_err());
        assert!(answers("One, Two", &keys).is_err());
    }
}
