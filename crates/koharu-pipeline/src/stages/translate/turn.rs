//! A page's turn of the conversation.

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use koharu_llm::{
    Context, KOHARU, PromptError,
    rig::{
        completion::{
            AssistantContent, GenerationOptions, Message, Reasoning, Usage,
            message::{DocumentSourceKind, Image, ImageMediaType, UserContent},
        },
        driver::DynModel,
        operation::Completion,
        structured_output::json_in_text,
    },
};
use serde_json::{Map, Value, json};

use super::Conversation;
use crate::stages::Failed;

pub(super) struct Turn<'a> {
    system: String,
    /// The clean JSON payload of the page(s) being translated, recorded in
    /// conversation history without transient neighbor wrappers or images.
    page: String,
    /// Optional wrapped user message (e.g. `<previous-pages>`, `<current-page>`,
    /// `<upcoming-pages>`) sent for this turn only.
    prompt_body: Option<String>,
    keys: Vec<String>,
    texts: &'a [String],
    /// The page, which the model sees with its texts.
    image: Option<&'a str>,
    reasoning: Option<Reasoning>,
}

impl<'a> Turn<'a> {
    pub(super) fn new(
        system: String,
        texts: &'a [String],
        image: Option<&'a str>,
        reasoning: Option<Reasoning>,
    ) -> Self {
        Self::keyed(system, keys(texts.len()), texts, image, reasoning)
    }

    pub(super) fn with_prompt_body(mut self, prompt_body: String) -> Self {
        self.prompt_body = Some(prompt_body);
        self
    }

    pub(super) fn page_json(&self) -> &str {
        &self.page
    }

    pub(super) fn keyed(
        system: String,
        keys: Vec<String>,
        texts: &'a [String],
        image: Option<&'a str>,
        reasoning: Option<Reasoning>,
    ) -> Self {
        Self {
            system,
            page: numbered(&keys, texts.iter().map(String::as_str)),
            prompt_body: None,
            keys,
            texts,
            image,
            reasoning,
        }
    }

    /// The page's translations, recorded in `conversation` once they pass.
    pub(super) async fn take(
        self,
        conversation: &mut Conversation,
        model: &DynModel<Completion>,
    ) -> Result<Vec<String>> {
        let next = Message::user(
            self.prompt_body
                .clone()
                .unwrap_or_else(|| self.page.clone()),
        );
        conversation.context.fit(model, &next).await;
        let mut asked = self.ask(model, &mut conversation.context).await?;
        // A small model can answer a page with its answer to the page
        // before, word for word, as Index-Translate 2B did on one of 16
        // Blue Archive pages; without that answer to copy, it reads the page.
        if self.echoes(&asked.0, conversation.last.as_ref()) {
            tracing::info!("the answer echoed the page before's; asking again without the history");
            asked = self.ask(model, &mut Context::default()).await?;
        }
        let (translations, messages, usage) = asked;
        conversation
            .context
            .record([(remembered(messages, &self.page), usage)]);
        conversation.last = Some((self.texts.to_vec(), translations.clone()));
        Ok(translations)
    }

    /// The page asked after `context`, and again with each remedy for why it
    /// got no answer: its translations, the turn's messages, and their usage.
    async fn ask(
        &self,
        model: &DynModel<Completion>,
        context: &mut Context,
    ) -> Result<(Vec<String>, Vec<Message>, Usage)> {
        let page = self.message(model);
        let mut options = GenerationOptions::default();
        if let Some(reasoning) = self.reasoning {
            options = options.reasoning(reasoning);
        }
        let agent = koharu_llm::answers(model, schema(&self.keys)?)
            .preamble(&self.system)
            .options(options)
            .build();
        let request = |context: &Context| {
            let history = context.messages().iter();
            agent
                .prompt(page.clone())
                .history(history.map(|message| Message::clone(message)))
        };
        let mut answer = request(context).await;
        let remedies = match &answer {
            Ok(_) => &[],
            Err(error) => remedies(error, model, context),
        };
        for remedy in remedies {
            tracing::info!(?remedy, "asking the page again");
            answer = match remedy {
                Remedy::Alone => request(&Context::default()).await,
                Remedy::Halved => {
                    context.halve();
                    request(context).await
                }
                Remedy::Warmer(temperature) => request(context).temperature(*temperature).await,
            };
            if answer.is_ok() {
                break;
            }
        }
        let response = answer.map_err(failure)?;
        let translations = answers(&response.output(), &self.keys).map_err(Failed)?;
        // The first call's covers the history, the page, and an answer, as the
        // conversation keeps them; one asked again also read the first answer.
        let usage = response
            .completion_calls
            .first()
            .map_or_else(Usage::default, |call| call.usage);
        Ok((translations, response.messages, usage))
    }

    /// The page's texts, after the page itself when the model sees it, which
    /// one that reads no images doesn't.
    fn message(&self, model: &DynModel<Completion>) -> Message {
        let mut content = Vec::new();
        match self.image {
            Some(image) if model.spec().is_some_and(|spec| spec.input.image) => {
                content.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(image.to_owned()),
                    media_type: Some(ImageMediaType::JPEG),
                    detail: None,
                    native: None,
                }));
            }
            Some(_) => {
                tracing::warn!("the translation model reads no images; leaving the page's out")
            }
            None => {}
        }
        content.push(UserContent::text(
            self.prompt_body
                .clone()
                .unwrap_or_else(|| self.page.clone()),
        ));
        Message::from(content)
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

/// Why a page got no answer it can take, which is the page's alone: a request
/// that failed otherwise would fail for the next page too.
enum Refusal {
    /// The provider blocked the page, or its answer, for its content.
    Blocked,
    /// The page doesn't fit the model's window.
    TooLong,
    /// The answer was cut off.
    CutOff,
    /// The answer doesn't fit the page.
    Misfit,
}

impl Refusal {
    fn of(error: &PromptError) -> Option<Self> {
        match error {
            PromptError::Report(report) if report.refusal => Some(Self::Blocked),
            PromptError::Report(report) if report.is_context_overflow() => Some(Self::TooLong),
            // rig words an answer that was cut off as a cancelled run.
            PromptError::Cancelled { .. } => Some(Self::CutOff),
            PromptError::OutputSchemaMismatch { .. } => Some(Self::Misfit),
            _ => None,
        }
    }
}

/// How a page that got no answer is asked again.
#[derive(Debug)]
enum Remedy {
    /// Without the history.
    Alone,
    /// With the newer half of the history.
    Halved,
    /// At a temperature above the model's own.
    Warmer(f64),
}

/// How a page `error` left without an answer is asked again, in turn.
fn remedies(
    error: &PromptError,
    model: &DynModel<Completion>,
    context: &Context,
) -> &'static [Remedy] {
    let spec = model.spec();
    match Refusal::of(error) {
        // A local model decodes greedily, and would answer the same way, such
        // as one that kept repeating itself. OpenAI's Whisper falls back so
        // (`decode_with_fallback`, whisper/transcribe.py at 86098128).
        Some(Refusal::CutOff) if spec.is_some_and(|it| it.provider.vendor() == KOHARU) => {
            &[Remedy::Warmer(0.4), Remedy::Warmer(0.8)]
        }
        _ if context.messages().is_empty() => &[],
        // The history may be what's blocked.
        Some(Refusal::Blocked) => &[Remedy::Alone],
        Some(Refusal::TooLong) => &[Remedy::Halved],
        // A model that doesn't say its window cuts an answer off at its end.
        Some(Refusal::CutOff) if spec.is_none_or(|it| it.input_limit().is_none()) => {
            &[Remedy::Halved]
        }
        _ => &[],
    }
}

/// `error`, as its page's alone when the page was refused.
fn failure(error: PromptError) -> anyhow::Error {
    let refused = Refusal::of(&error).is_some();
    let error = match error {
        PromptError::Cancelled { reason, .. } => anyhow!(reason),
        error => error.into(),
    };
    match refused {
        true => Failed(error).into(),
        false => error,
    }
}

/// A turn's messages as the conversation keeps them: the clean page JSON and
/// the answer. The image, any transient neighbor-page context wrapper, the
/// reasoning, and any retry exchange served this turn alone; kept, every later
/// turn would resend them and bloat or duplicate the context.
fn remembered(mut messages: Vec<Message>, clean_page: &str) -> Vec<Message> {
    let answer = messages.pop();
    let has_user = !messages.is_empty();
    let user = has_user.then(|| Message::user(clean_page.to_owned()));
    user.into_iter()
        .chain(answer.map(|message| match message {
            Message::Assistant(mut turn) => {
                turn.content
                    .retain(|part| !matches!(part, AssistantContent::Reasoning(_)));
                Message::Assistant(turn)
            }
            message => message,
        }))
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
/// those keys, each a text that isn't blank. A model asked in its prompt can
/// write prose or a code fence around it.
fn answers(answer: &str, keys: &[String]) -> Result<Vec<String>> {
    let mut answer: Map<String, Value> =
        json_in_text(answer).context("the answer isn't a JSON object")?;
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
    use indoc::indoc;
    use koharu_llm::rig::completion::{AssistantMessage, FinishReason};
    use rig_core::{
        catalog::ModelSpec,
        providers::registry::ProviderId,
        test_utils::{MockCompletionModel, MockTurn},
    };

    use super::*;

    #[tokio::test]
    async fn a_local_answer_cut_off_is_asked_for_again_sampled() -> Result<()> {
        let mock = MockCompletionModel::from_turns([
            MockTurn::text(r#"{"1": "Aaaaaa"#).with_finish_reason(FinishReason::Length),
            MockTurn::text(r#"{"1": "Argh!"}"#),
        ])
        .priced_as(ModelSpec::new(ProviderId::declare(KOHARU)?, "mock"));
        let texts = ["ぎゃああ".to_owned()];
        let answer = Turn::new("Translate.".to_owned(), &texts, None, None)
            .take(&mut Conversation::new(), &mock.clone().erase())
            .await?;
        assert_eq!(answer, ["Argh!"]);
        let requests = mock.requests();
        let temperatures: Vec<_> = requests.iter().map(|asked| asked.temperature).collect();
        assert_eq!(temperatures, [None, Some(0.4)]);
        Ok(())
    }

    #[test]
    fn the_conversation_keeps_no_image_reasoning_or_refused_answer() {
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
        let kept = remembered(
            vec![
                Message::from(vec![
                    image,
                    UserContent::text(
                        "<upcoming-pages>Page 2:\n- 明日</upcoming-pages>\n\n<current-page>\n{\"01\": \"やあ\"}\n</current-page>",
                    ),
                ]),
                Message::assistant("{}"),
                Message::user("Answer again."),
                Message::Assistant(answer),
            ],
            "{\"01\": \"やあ\"}",
        );
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
        let fenced = indoc! {r#"
            ```json
            {"1": "One", "2": "Two"}
            ```"#
        };
        assert_eq!(answers(fenced, &keys).expect("an answer"), ["One", "Two"]);
    }

    #[tokio::test]
    async fn keyed_turn_validates_comic_info_field_keys_against_mock_llm() -> Result<()> {
        let mock = MockCompletionModel::from_turns([MockTurn::text(
            r#"{"Title": "Hot Summer", "Summary": "Everyday life of Yotsuba."}"#,
        )]);
        let keys = vec!["Title".to_owned(), "Summary".to_owned()];
        let texts = vec!["あつい夏".to_owned(), "よつばの日常".to_owned()];
        let answer = Turn::keyed("Translate.".to_owned(), keys, &texts, None, None)
            .take(&mut Conversation::new(), &mock.erase())
            .await?;
        assert_eq!(answer, ["Hot Summer", "Everyday life of Yotsuba."]);
        Ok(())
    }
}
