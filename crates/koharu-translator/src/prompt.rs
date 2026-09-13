use std::fmt::Write;

use anyhow::Context;
use indoc::indoc;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Value, json};

use crate::{Language, TranslationContext, TranslationRequest};

pub(crate) fn prompts(request: &TranslationRequest) -> anyhow::Result<(String, String)> {
    let input = TranslationInput {
        source_language: request.source_language,
        target_language: request.target_language,
        context: &request.context,
        segments: request
            .segments
            .iter()
            .enumerate()
            .map(|(id, text)| TranslationInputSegment { id, text })
            .collect(),
    };
    let user = serde_json::to_string(&input).context("failed to serialize translation input")?;
    Ok((translation_system_prompt(request), user))
}

pub(crate) fn translations(
    provider: &str,
    text: &str,
    source_segments: &[String],
) -> TranslationOutcome {
    let (decoded, error) = match serde_json::from_str::<TranslationOutput>(extract_json(text)) {
        Ok(output) => (output.translations, None),
        Err(error) => (
            salvage_segments(text),
            Some(anyhow::Error::new(error).context(format!(
                "{provider} returned invalid translation JSON for {} segments; response was: {}",
                source_segments.len(),
                snippet(text),
            ))),
        ),
    };

    let mut translations = source_segments.to_vec();
    let mut translated = vec![false; source_segments.len()];

    for segment in decoded {
        if segment.id < translations.len() && !translated[segment.id] {
            translations[segment.id] = segment.text;
            translated[segment.id] = true;
        }
    }
    let missing = translated
        .into_iter()
        .enumerate()
        .filter_map(|(index, translated)| (!translated).then_some(index))
        .collect();

    TranslationOutcome {
        translations,
        missing,
        error,
    }
}

/// One translation per input segment.
///
/// Providers recover as many segments as a model response allows; a segment the
/// model never returned keeps its source text and stays listed in the missing
/// field, so a caller can re-request it or report the failure.
pub(crate) struct TranslationOutcome {
    pub(crate) translations: Vec<String>,
    pub(crate) missing: Vec<usize>,
    /// Present when the response could not be decoded as-is.
    pub(crate) error: Option<anyhow::Error>,
}

impl TranslationOutcome {
    /// Wraps a provider that answers with exactly one translation per segment.
    pub(crate) fn complete(translations: Vec<String>) -> Self {
        Self {
            translations,
            missing: Vec::new(),
            error: None,
        }
    }
}

/// Narrows a response to the JSON object it wraps, because JSON mode still
/// returns markdown fences, a leading sentence, or trailing characters.
fn extract_json(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if end > start => &text[start..=end],
        _ => text,
    }
}

/// Recovers whole brace-delimited objects from a response that is not valid
/// JSON as a whole, such as one with trailing characters or an unterminated
/// tail. Objects that are not segments are ignored.
fn salvage_segments(text: &str) -> Vec<TranslationOutputSegment> {
    object_spans(text)
        .into_iter()
        .filter_map(|(start, end)| serde_json::from_str(&text[start..end]).ok())
        .collect()
}

/// Byte ranges of every balanced object in the text.
fn object_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut open = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in text.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => open.push(index),
            b'}' => {
                if let Some(start) = open.pop() {
                    spans.push((start, index + 1));
                }
            }
            _ => {}
        }
    }
    spans
}

/// Keeps a model response readable in a log line without truncating so hard
/// that the shape of the failure is lost.
///
/// Responses are usually pretty-printed, so line breaks and other control
/// characters are escaped rather than passed through: the snippet is formatted
/// into an error context, and one failure should stay one line.
pub(crate) fn snippet(text: &str) -> String {
    const LIMIT: usize = 2000;
    let trimmed = text.trim();
    let (visible, elided) = match trimmed.char_indices().nth(LIMIT) {
        Some((end, _)) => (&trimmed[..end], true),
        None => (trimmed, false),
    };

    let mut output = String::with_capacity(visible.len());
    for character in visible.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(output, "\\u{{{:04x}}}", control as u32);
            }
            character => output.push(character),
        }
    }
    if elided {
        let _ = write!(output, "… ({} bytes total)", trimmed.len());
    }
    output
}

pub(crate) fn output_schema(expected: usize) -> Value {
    json!({
        "type": "object",
        "properties": {
            "translations": {
                "type": "array",
                "minItems": expected,
                "maxItems": expected,
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "minimum": 0,
                            "maximum": expected.saturating_sub(1),
                            "description": "The ID copied from the corresponding input segment."
                        },
                        "text": {
                            "type": "string",
                            "description": "The translation of the input segment with this ID."
                        }
                    },
                    "required": ["id", "text"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["translations"],
        "additionalProperties": false
    })
}

fn translation_system_prompt(request: &TranslationRequest) -> String {
    let source = request
        .source_language
        .map(|language| language.to_string())
        .unwrap_or_else(|| "the detected source language".to_owned());
    let mut prompt = format!(
        indoc! {"
            You are a professional manga translator.

            Translation requirements:
            - Translate every input segment from {source} into natural {target}.
            - Preserve meaning, character voice, emotional tone, relationship nuance, emphasis, and sound effects.
            - Localize idioms and sound effects naturally while keeping wording concise enough for speech bubbles.
            - Use surrounding segments only for disambiguation and continuity; never merge or split segments.
            - Write every translated `text` value only in {target}; do not include source text, notes, explanations, or alternatives.
            - Never preserve or repeat original-language text; translate names, terms, and sound effects using natural {target} conventions.

            Output requirements:
            - Each input segment has a numeric `id`.
            - Return only a JSON object whose `translations` array contains one object with `id` and translated `text` for every input segment.
            - Copy every input ID exactly once; order does not matter.
            - Never merge, split, omit, duplicate, or add segments.
        "},
        source = source,
        target = request.target_language,
    )
    .trim_end()
    .to_owned();

    if !request.context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(indoc! {"
            Context requirements:
            Use the supplied context only to preserve terminology, character voice, and dialogue continuity.
            Do not translate or return the context entries.
        "}.trim_end());
    }

    if request.image.is_some() {
        prompt.push_str("\n\n");
        prompt.push_str(indoc! {"
            Image requirements:
            Use the attached original page image as visual context for speaker identity, tone, layout, and ambiguous OCR.
            Translate only the supplied segments; do not add text seen in the image that is absent from the input segments.
        "}.trim_end());
    }

    if let Some(instructions) = request
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|instructions| !instructions.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(instructions);
    }
    prompt
}

#[derive(Serialize)]
struct TranslationInput<'a> {
    source_language: Option<Language>,
    target_language: Language,
    context: &'a [TranslationContext],
    segments: Vec<TranslationInputSegment<'a>>,
}

#[derive(Serialize)]
struct TranslationInputSegment<'a> {
    id: usize,
    text: &'a str,
}

#[derive(Debug, Deserialize)]
struct TranslationOutput {
    translations: Vec<TranslationOutputSegment>,
}

#[derive(Debug, Deserialize)]
struct TranslationOutputSegment {
    #[serde(deserialize_with = "deserialize_segment_id")]
    id: usize,
    text: String,
}

fn deserialize_segment_id<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SegmentId {
        Number(usize),
        String(String),
    }

    match SegmentId::deserialize(deserializer)? {
        SegmentId::Number(id) => Ok(id),
        SegmentId::String(id) => id.trim().parse().map_err(de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_keeps_a_response_on_one_line() {
        let flattened = snippet("{\n  \"translations\": [\n\t\"hello\"\n  ]\n}");

        assert!(!flattened.contains('\n'), "{flattened}");
        assert!(!flattened.contains('\t'), "{flattened}");
        assert_eq!(flattened, r#"{\n  "translations": [\n\t"hello"\n  ]\n}"#);
    }

    #[test]
    fn snippet_escapes_other_control_characters() {
        assert_eq!(snippet("before\u{7}after"), r"before\u{0007}after");
    }

    #[test]
    fn snippet_reports_the_length_it_elided() {
        let flattened = snippet(&"x".repeat(2_500));

        assert!(flattened.starts_with(&"x".repeat(2_000)));
        assert!(
            flattened.ends_with("\u{2026} (2500 bytes total)"),
            "{flattened}"
        );
    }

    #[test]
    fn parses_plain_json() {
        let source = ["one".to_owned(), "two".to_owned()];
        let response = r#"{"translations":[{"id":0,"text":"hello"},{"id":1,"text":"world"}]}"#;

        let outcome = translations("test", response, &source);
        assert_eq!(outcome.translations, ["hello", "world"]);
        assert!(outcome.missing.is_empty());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn unwraps_fenced_and_padded_responses() {
        let source = ["one".to_owned(), "two".to_owned()];
        for response in [
            "```json\n{\"translations\":[{\"id\":0,\"text\":\"hello\"},{\"id\":1,\"text\":\"world\"}]}\n```",
            r#"Here is the result: {"translations": [{"id": 0, "text": "hello"}, {"id": 1, "text": "world"}]}"#,
            "{\"translations\":[{\"id\":0,\"text\":\"hello\"},{\"id\":1,\"text\":\"world\"}]}\n\nhope that helped",
        ] {
            let outcome = translations("test", response, &source);
            assert_eq!(outcome.translations, ["hello", "world"], "{response}");
            assert!(outcome.missing.is_empty(), "{response}");
            assert!(outcome.error.is_none(), "{response}");
        }
    }

    #[test]
    fn salvages_objects_from_a_malformed_array() {
        let source = ["one".to_owned(), "two".to_owned()];
        let response = r#"{"translations":[{"id":0,"text":"hello"},{"id":1,"text":"world"},]}"#;

        let outcome = translations("test", response, &source);
        assert_eq!(outcome.translations, ["hello", "world"]);
        assert!(outcome.missing.is_empty());
        assert!(outcome.error.is_some());
    }

    #[test]
    fn salvages_the_complete_prefix_of_a_truncated_response() {
        let source = ["one".to_owned(), "two".to_owned(), "three".to_owned()];
        let response = r#"{"translations":[{"id":0,"text":"hello"},{"id":1,"text":"world""#;

        let outcome = translations("test", response, &source);
        assert_eq!(outcome.translations, ["hello", "two", "three"]);
        assert_eq!(outcome.missing, [1, 2]);
        assert!(outcome.error.is_some());
    }

    #[test]
    fn reports_a_response_without_any_recoverable_segment() {
        let source = ["one".to_owned(), "two".to_owned()];
        let response = r#"{translations: [{id: 0, text: 'hello'}, {id: 1, text: 'world'},],}"#;

        let outcome = translations("test", response, &source);
        assert_eq!(outcome.translations, source);
        assert_eq!(outcome.missing, [0, 1]);
        assert!(outcome.error.is_some());
    }

    #[test]
    fn parse_error_preserves_the_root_failure_and_response() {
        let source = ["one".to_owned()];
        let response = "{\n  \"translations\": [{\"id\": 0}]\n}";
        let outcome = translations("test", response, &source);
        let error = format!("{:#}", outcome.error.expect("missing text should fail"));

        assert!(error.contains("missing field `text`"), "{error}");
        assert!(
            error.contains(r#"response was: {\n  "translations": [{"id": 0}]\n}"#),
            "{error}"
        );
        assert_eq!(outcome.translations, ["one"]);
        assert_eq!(outcome.missing, [0]);
    }

    #[test]
    fn restores_input_order_from_ids() {
        let source = ["one".to_owned(), "two".to_owned()];
        let response = r#"{"translations":[{"id":1,"text":"world"},{"id":0,"text":"hello"}]}"#;
        let outcome = translations("test", response, &source);
        assert_eq!(outcome.translations, ["hello", "world"]);
        assert!(outcome.missing.is_empty());
    }

    #[test]
    fn ignores_duplicate_and_out_of_range_ids() {
        let source = ["one".to_owned(), "two".to_owned()];
        let response = concat!(
            r#"{"translations":["#,
            r#"{"id":0,"text":"hello"},"#,
            r#"{"id":0,"text":"duplicate"},"#,
            r#"{"id":9,"text":"extra"}"#,
            "]}"
        );
        let outcome = translations("test", response, &source);
        assert_eq!(outcome.translations, ["hello", "two"]);
        assert_eq!(outcome.missing, [1]);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn complete_outcome_reports_nothing_missing() {
        let outcome = TranslationOutcome::complete(vec!["hello".to_owned()]);
        assert_eq!(outcome.translations, ["hello"]);
        assert!(outcome.missing.is_empty());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn prompt_payload_contains_ordered_context() {
        let request = TranslationRequest::new(["new"], Language::English)
            .with_context([TranslationContext::new("old", "previous")]);
        let (_, user) = prompts(&request).unwrap();
        let input: serde_json::Value = serde_json::from_str(&user).unwrap();
        assert_eq!(input["context"][0]["source"], "old");
        assert_eq!(input["context"][0]["translation"], "previous");
        assert_eq!(input["segments"][0]["id"], 0);
        assert_eq!(input["segments"][0]["text"], "new");
    }

    #[test]
    fn system_prompt_encodes_invariants_and_custom_instructions() {
        let request = TranslationRequest::new(["hello"], Language::Korean)
            .with_source_language(Language::Japanese)
            .with_instructions("Use informal speech.");
        let prompt = translation_system_prompt(&request);
        assert!(prompt.contains("from Japanese into natural Korean"));
        assert!(prompt.contains("Copy every input ID exactly once"));
        assert!(prompt.contains("Use informal speech."));
    }

    #[test]
    fn schema_requires_the_expected_number_of_id_text_pairs() {
        let schema = output_schema(3);
        let translations = &schema["properties"]["translations"];
        assert_eq!(translations["minItems"], 3);
        assert_eq!(translations["maxItems"], 3);
        assert_eq!(translations["items"]["properties"]["id"]["minimum"], 0);
        assert_eq!(translations["items"]["properties"]["id"]["maximum"], 2);
        assert_eq!(translations["items"]["additionalProperties"], false);
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn empty_custom_instructions_are_ignored() {
        let request = TranslationRequest::new(["hello"], Language::English).with_instructions("  ");
        assert!(!translation_system_prompt(&request).contains("Additional instructions"));
    }

    #[test]
    fn context_is_reference_only() {
        let request = TranslationRequest::new(["Where is she?"], Language::Japanese)
            .with_context([TranslationContext::new("I saw Alice.", "アリスを見た。")]);
        let prompt = translation_system_prompt(&request);
        assert!(prompt.contains("dialogue continuity"));
        assert!(prompt.contains("Do not translate or return the context"));
    }

    #[test]
    fn image_context_does_not_expand_the_translation_scope() {
        let request = TranslationRequest::new(["text"], Language::English)
            .with_image(std::sync::Arc::new(image::DynamicImage::new_rgb8(1, 1)));
        let prompt = translation_system_prompt(&request);
        assert!(prompt.contains("attached original page image"));
        assert!(prompt.contains("Translate only the supplied segments"));
    }
}
