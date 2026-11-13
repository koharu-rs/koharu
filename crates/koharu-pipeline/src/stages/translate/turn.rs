//! A page's turn of the conversation.

use std::future::IntoFuture;

use anyhow::{Context as _, Result, bail, ensure};
use koharu_llm::rig::{
    completion::{
        AssistantContent, Effort, GenerationOptions, Message, Usage,
        message::{DocumentSourceKind, Image, ImageMediaType, UserContent},
    },
    driver::DynModel,
    operation::Completion,
};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::Conversation;

pub(super) struct Turn<'a> {
    system: String,
    page: String,
    keys: Vec<String>,
    texts: &'a [String],
    /// The page, which the model sees with its texts.
    image: Option<&'a str>,
    reasoning: Option<Effort>,
    cancel: &'a CancellationToken,
}

impl<'a> Turn<'a> {
    pub(super) fn new(
        system: String,
        texts: &'a [String],
        image: Option<&'a str>,
        reasoning: Option<Effort>,
        cancel: &'a CancellationToken,
    ) -> Self {
        let keys = keys(texts.len());
        Self {
            system,
            page: numbered(&keys, texts.iter().map(String::as_str)),
            keys,
            texts,
            image,
            reasoning,
            cancel,
        }
    }

    /// The page's translations, recorded in `conversation` once they pass;
    /// `None` when the run stopped first.
    pub(super) async fn take(
        self,
        conversation: &mut Conversation,
        model: &DynModel<Completion>,
    ) -> Result<Option<Vec<String>>> {
        conversation.context.fit(model).await?;
        let history: Vec<Message> = conversation
            .context
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
        if self.echoes(&asked.0, conversation.last.as_ref()) {
            tracing::info!("the answer echoed the page before's; asking again without the history");
            let Some(again) = self.ask(model, Vec::new()).await? else {
                return Ok(None);
            };
            asked = again;
        }
        let (translations, messages, usage) = asked;
        conversation.context.record([(remembered(messages), usage)]);
        conversation.last = Some((self.texts.to_vec(), translations.clone()));
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
            .preamble(&self.system)
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
