// Bulk translation review worker — specialized, minimal-payload path for mass revision.
//
// V3 architecture:
// - One model request per batch on the happy path (no tool loop)
// - Structured output under text.format with local validation and application
// - Minimal textual payload (no geometry, images, provider config, etc.)
// - Structured memory (local merge, bounded, not a full rewrite per batch)
// - Conservative local quality hints (advisory only; never edits text directly)
// - Lowest supported reasoning effort
// - Text-based batch planner using measured page sizes

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use icu_properties::{CodePointMapData, props::Script as IcuScript};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    CodexModel, Control, Host, Invocation, Reasoning, Tool, ToolCall,
    agent::{Event, RunId},
    codex::{Request, ReviewClient, backoff, is_context_limit, message, retry_wait, should_retry},
};

/// Upper bound (in characters) for a single batch's textual payload. Small pages
/// batch together well beyond V2's fixed 5-page cap; big pages stay isolated.
pub(crate) const BULK_INPUT_BUDGET_CHARS: usize = 20_000;

/// Pages read at a time. This is the job's retention ceiling: one group of page
/// payloads is alive at once, never the whole project, so a 200-page review does
/// not trade context pressure for a large host-side serialization.
const MAX_PAGES_PER_READ: usize = 24;

/// Runaway guard for a single tool call from the model.
const MAX_JOB_PAGES: usize = 500;

/// The instruction is quoted back into every batch prompt, so it is capped.
const MAX_INSTRUCTION_CHARS: usize = 4_000;

/// Failed pages reported per job; the rest is only a count.
const MAX_REPORTED_FAILURES: usize = 32;

/// A script must carry at least one in this many translated characters before it
/// counts as a script the batch is written in.
const MIN_TARGET_SCRIPT_SHARE: usize = 3;

/// Per-volume overhead heuristic used when planning batches. Covers the key names
/// and quoting of one element, including the evidence keys that a page can record
/// for it (`origin`, `role`, `region`, `score`).
const ESTIMATED_JSON_OVERHEAD_PER_ELEMENT: usize = 96;

/// Wire values the host uses to describe where an element came from.
const MACHINE_ORIGIN: &str = "gen";
const FREE_TEXT_ROLE: &str = "free";
const ONOMATOPOEIA_REGION: &str = "onomatopoeia";

/// A non-text region the detector was at least this sure about corroborates that
/// the text in it is lettering rather than story.
const SFX_DETECTOR_SCORE: f32 = 0.85;

/// Strong signals from different places in the scene required before a translation
/// may be erased. One is never enough, and the shape of the text does not count.
const MIN_CLEARANCE_SIGNALS: usize = 2;

/// Flags reported per job; the rest is only a count.
const MAX_REPORTED_FLAGS: usize = 64;

/// Structured translation memory is bounded to keep the per-batch prompt tiny.
pub(crate) const MAX_MEMORY_BYTES: usize = 4 * 1024;

const MAX_MEMORY_UPDATES: usize = 64;
const MAX_CHANGE_CHARS: usize = 8192;

/// Minimal page data for bulk text review — source and current translation only.
#[derive(Serialize)]
pub(crate) struct ReviewPage {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) elements: Vec<ReviewElement>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ReviewElement {
    /// Element entity ID
    pub(crate) id: String,
    /// Position in the page's element list, the same order `inspect_pages` reports
    pub(crate) order: usize,
    /// Source text
    pub(crate) source: String,
    /// Language the page records for this source text. Kept per element: one
    /// page can mix scripts, and the reviewer must not assume otherwise. Short
    /// on the wire because the payload budget is the whole point.
    #[serde(skip_serializing_if = "Option::is_none", rename = "lang")]
    pub(crate) source_language: Option<String>,
    /// Current translation (may be absent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) translation: Option<String>,
    /// `gen` when the pipeline wrote the current translation, `user` when a person did
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) origin: Option<String>,
    /// `dialogue` when the scene puts this text in a balloon, `free` when it does not
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<String>,
    /// The label of the region this source was recognized in, when that region is
    /// not an ordinary text region — `onomatopoeia` being the interesting case
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) region: Option<String>,
    /// The detector's own score for that region
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) score: Option<f32>,
    /// Conservative local quality hints (advisory signals only — never applied
    /// as edits by this module), serialized only when non-empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) quality_hints: Vec<&'static str>,
}

/// Structured response from the bulk reviewer.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ReviewResponse {
    /// Changes to apply — only elements that need updating.
    #[serde(default)]
    pub(crate) changes: Vec<TranslationChange>,
    /// Text the reviewer judged should not have been translated at all.
    #[serde(default)]
    pub(crate) flags: Vec<TranslationFlag>,
    /// Memory updates (incremental: add/replace, not full rewrite).
    #[serde(default)]
    pub(crate) memory_updates: Vec<MemoryUpdate>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TranslationChange {
    /// Element entity ID to update
    pub(crate) element: String,
    /// New translation text, or `null` asking for the translation to be removed
    pub(crate) translation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TranslationFlag {
    pub(crate) element: String,
    pub(crate) reason: String,
    pub(crate) kind: SfxKind,
}

/// What a reviewer believes a piece of lettered text is. `meaningful_sfx` is where
/// an `onomatopoeia` label is evidence rather than a verdict: a sign, a letter, or
/// a shouted real word carries content and belongs translated.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SfxKind {
    Vocalization,
    DecorativeSfx,
    MeaningfulSfx,
    Ambiguous,
}

impl SfxKind {
    /// The kinds that say this text is not story text at all. `meaningful_sfx` and
    /// `ambiguous` deliberately do not: content stays translated, and doubt gets
    /// reported rather than acted on.
    fn clears(self) -> bool {
        matches!(self, Self::Vocalization | Self::DecorativeSfx)
    }
}

/// The same four kinds, for the response schema.
const SFX_KIND_NAMES: [&str; 4] = [
    "vocalization",
    "decorative_sfx",
    "meaningful_sfx",
    "ambiguous",
];

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MemoryUpdate {
    pub(crate) key: String,
    pub(crate) value: String,
}

/// Compact translation memory, keyed in sorted order so the prompt prefix it
/// produces is identical for the same content across batches and retries.
#[derive(Clone, Debug, Default)]
pub(crate) struct TranslationMemory {
    entries: BTreeMap<String, String>,
}

impl TranslationMemory {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Apply incremental updates (add/replace) and re-clamp to the size bound.
    pub(crate) fn apply_updates(mut self, updates: Vec<MemoryUpdate>) -> Self {
        for update in updates {
            if update.key.is_empty() {
                continue;
            }
            self.entries.insert(update.key, update.value);
        }
        self
    }

    /// Serialize for the prompt (compact `key: value` lines).
    pub(crate) fn serialize(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        self.entries
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Enforce a size limit over whole entries so multibyte UTF-8 never splits.
    pub(crate) fn bounded(mut self, max_bytes: usize) -> Self {
        let mut size = 0_usize;
        self.entries.retain(|key, value| {
            let entry_size = key.len() + value.len() + 2;
            if size + entry_size > max_bytes {
                return false;
            }
            size += entry_size;
            true
        });
        self
    }
}

/// Scripts a text carries, ignoring punctuation and other shared code points.
fn scripts(text: &str) -> Vec<IcuScript> {
    let map = CodePointMapData::<IcuScript>::new();
    let mut found = Vec::new();
    for character in text.chars() {
        let script = map.get(character);
        if matches!(script, IcuScript::Common | IcuScript::Inherited) || found.contains(&script) {
            continue;
        }
        found.push(script);
    }
    found
}

/// Counts the script-carrying characters of a text, to estimate which scripts a
/// whole batch is written in.
fn count_scripts(text: &str, counts: &mut BTreeMap<IcuScript, usize>) {
    let map = CodePointMapData::<IcuScript>::new();
    for character in text.chars() {
        let script = map.get(character);
        if matches!(script, IcuScript::Common | IcuScript::Inherited) {
            continue;
        }
        *counts.entry(script).or_default() += 1;
    }
}

fn carries_any_script(text: &str, scripts: &[IcuScript]) -> bool {
    let map = CodePointMapData::<IcuScript>::new();
    text.chars()
        .any(|character| scripts.contains(&map.get(character)))
}

/// Estimate the scripts a batch is written in from its own translations. A
/// script must carry a share of the batch's characters, so one element still
/// holding source text cannot declare that text to be the target's script. A
/// translation that merely repeats its source is not evidence at all.
pub(crate) fn batch_target_scripts(batch: &[(&str, &str)]) -> Vec<IcuScript> {
    let mut counts: BTreeMap<IcuScript, usize> = BTreeMap::new();
    for (source, translation) in batch {
        if translation.trim() != source.trim() {
            count_scripts(translation, &mut counts);
        }
    }
    let scripted: usize = counts.values().sum();
    counts
        .into_iter()
        .filter(|(_, count)| count.saturating_mul(MIN_TARGET_SCRIPT_SHARE) >= scripted)
        .map(|(script, _)| script)
        .collect()
}

/// Scripts that would be untranslated residue in a text of this source: the
/// source's own scripts minus the ones the batch is translated into.
pub(crate) fn residue_scripts(source: &str, target_scripts: &[IcuScript]) -> Vec<IcuScript> {
    scripts(source)
        .into_iter()
        .filter(|script| !target_scripts.contains(script))
        .collect()
}

/// Single-letter words that are ordinary in the target language. Unknown
/// targets get no list, which turns the hint off rather than let it flag real
/// words of a language this module does not model.
fn single_letter_words(target: &str) -> Option<&'static [&'static str]> {
    target
        .to_lowercase()
        .contains("portuguese")
        .then_some(&["a", "e", "i", "o", "u", "á", "à", "é", "í", "ó", "ú", "ê"])
}

/// Trailing isolated ASCII letter after sentence-ending punctuation, e.g. the
/// stray "M" in "Me dá aquilo! M". Conservative: single-letter words of the
/// target, initials with a period, and attached cases ("Sim!Muito") are skipped.
fn has_possible_orphan_character(text: &str, target: &str) -> bool {
    let Some(single_letter_words) = single_letter_words(target) else {
        return false;
    };
    if text.chars().count() < 6 {
        return false;
    }
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    let Some((&last, prefix)) = tokens.split_last() else {
        return false;
    };
    if last.chars().count() != 1 {
        return false;
    }
    let last_char = last.chars().next().unwrap();
    if !last_char.is_ascii_alphabetic() {
        return false;
    }
    let lower = last.to_lowercase();
    if single_letter_words.contains(&lower.as_str()) {
        return false;
    }
    let Some(&second_to_last) = prefix.last() else {
        return false;
    };
    second_to_last
        .chars()
        .next_back()
        .is_some_and(|c| matches!(c, '!' | '?' | '.' | '"' | '…' | '。' | '！' | '？'))
}

/// Back-to-back repeated fragments: a duplicated second half ("Vamos ver vamos
/// ver.") or a long word repeated consecutively ("supercalifragilístico
/// supercalifragilístico"). Short emphatic repeats are left alone.
fn has_repeated_fragment(text: &str) -> bool {
    let words = text
        .split_whitespace()
        .map(|word| {
            let lower = word.to_lowercase();
            lower
                .trim_end_matches(|c: char| c.is_ascii_punctuation())
                .to_owned()
        })
        .collect::<Vec<_>>();
    if words.len() >= 4 {
        let half = words.len() / 2;
        if words[..half] == words[half..half * 2] {
            return true;
        }
    }
    words
        .windows(2)
        .any(|pair| pair[0] == pair[1] && pair[0].chars().count() >= 12)
}

/// Doubled or reversed punctuation ("!!", "??", ",,", ".,", "。。"). Ellipses
/// ("..."), "!?" and "?!" are legitimate interjections and are not flagged;
/// ".!" and ".?" are only flagged when the dot is not part of an ellipsis.
fn has_suspicious_punctuation(text: &str) -> bool {
    const BAD_PAIRS: &[&str] = &[
        "!!", "??", ",,", ";;", "::", ".,", ",.", "。。", "，，", "！！", "？？",
    ];
    if BAD_PAIRS.iter().any(|pair| text.contains(pair)) {
        return true;
    }
    let bytes = text.as_bytes();
    bytes.windows(2).enumerate().any(|(index, pair)| {
        matches!((pair[0], pair[1]), (b'.', b'!') | (b'.', b'?') if index == 0 || bytes[index - 1] != b'.')
            || matches!((pair[0], pair[1]), (b'!', b'.') | (b'?', b'.'))
    })
}

/// Conservative local quality hints for one reviewed element, guiding the
/// reviewer toward likely OCR noise or untranslated text. Advisory only — this
/// module never edits text through them. `residue` is the element's source
/// script that the batch's own translations do not use, so a translation still
/// carrying it is untranslated text rather than a legitimate word.
pub(crate) fn quality_hints(
    residue: &[IcuScript],
    translation: Option<&str>,
    target: &str,
) -> Vec<&'static str> {
    let Some(text) = translation.map(str::trim) else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    if carries_any_script(text, residue) {
        hints.push("untranslated_source_script");
    }
    if has_possible_orphan_character(text, target) {
        hints.push("possible_orphan_character");
    }
    if has_repeated_fragment(text) {
        hints.push("repeated_fragment");
    }
    if has_suspicious_punctuation(text) {
        hints.push("suspicious_punctuation");
    }
    hints
}

/// Why a local heuristic thinks an element may be lettering — a sound effect or a
/// vocalization — rather than story text.
///
/// The strong signals are read from three different places in the scene: the class
/// the region was detected as, whether a balloon holds the text, and the detector's
/// own score for that region. None of them is derived from another, so two of them
/// agreeing is real corroboration. The weak ones describe only the shape of the
/// source text, and one-word dialogue ("NO!", "STOP!", "HELP!") has that shape too,
/// so they can never justify destroying a translation on their own.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum SfxSignal {
    DetectorScore,
    NotDialogue,
    PunctuationOnly,
    RegionOnomatopoeia,
    RepeatedChar,
    SingleToken,
}

impl SfxSignal {
    fn strong(self) -> bool {
        matches!(
            self,
            Self::DetectorScore | Self::NotDialogue | Self::RegionOnomatopoeia
        )
    }
}

/// The evidence one element carries, reduced to named signals plus who wrote the
/// translation currently in the project.
#[derive(Clone, Debug, Default)]
pub(crate) struct SfxEvidence {
    /// The translation now in the project came from the pipeline, so erasing it
    /// undoes a machine decision rather than a person's work.
    machine: bool,
    signals: Vec<SfxSignal>,
}

impl SfxEvidence {
    pub(crate) fn of(element: &ReviewElement) -> Self {
        let mut signals = Vec::new();
        if element.region.as_deref() == Some(ONOMATOPOEIA_REGION) {
            signals.push(SfxSignal::RegionOnomatopoeia);
        }
        if element.role.as_deref() == Some(FREE_TEXT_ROLE) {
            signals.push(SfxSignal::NotDialogue);
        }
        if element.region.is_some()
            && element
                .score
                .is_some_and(|score| score >= SFX_DETECTOR_SCORE)
        {
            signals.push(SfxSignal::DetectorScore);
        }
        if !element.source.chars().any(char::is_alphanumeric) {
            signals.push(SfxSignal::PunctuationOnly);
        }
        if !element.source.is_empty() && !element.source.chars().any(char::is_whitespace) {
            signals.push(SfxSignal::SingleToken);
        }
        if has_repeated_char(&element.source) {
            signals.push(SfxSignal::RepeatedChar);
        }
        signals.sort_unstable();
        signals.dedup();
        Self {
            machine: element.origin.as_deref() == Some(MACHINE_ORIGIN),
            signals,
        }
    }

    pub(crate) fn has(&self, signal: SfxSignal) -> bool {
        self.signals.contains(&signal)
    }

    pub(crate) fn strong(&self) -> impl Iterator<Item = SfxSignal> + '_ {
        self.signals
            .iter()
            .copied()
            .filter(|signal| SfxSignal::strong(*signal))
    }

    pub(crate) fn strong_count(&self) -> usize {
        self.strong().count()
    }

    /// Worth a closer look, never worth acting on: either the scene says so, or a
    /// machine translation of a single token that carries no word in it at all.
    pub(crate) fn suspicious(&self) -> bool {
        self.strong().next().is_some()
            || (self.machine
                && self.has(SfxSignal::SingleToken)
                && (self.has(SfxSignal::PunctuationOnly) || self.has(SfxSignal::RepeatedChar)))
    }

    /// The gate an erasure has to pass: the pipeline wrote the text now in the
    /// project, and at least two strong signals corroborate each other.
    ///
    /// Two rules were meant to live here — an onomatopoeia region plus one more
    /// strong signal, or any two independent strong signals — and they collapse into
    /// this one because every strong signal except `not_dialogue` already requires
    /// the region to be classified as something other than text. What is left out on
    /// purpose is the shape of the source: a one-word line in a balloon ("NO!",
    /// "STOP!", "HELP!", "RUN!") is dialogue however short it is, so it can reach at
    /// most one strong signal and therefore only ever earns a flag.
    pub(crate) fn may_clear(&self) -> bool {
        self.machine && self.strong_count() >= MIN_CLEARANCE_SIGNALS
    }
}

/// The same character written twice or more in a row: `!!`, `アアア`, `ゴゴゴ`.
fn has_repeated_char(text: &str) -> bool {
    let characters = text
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .collect::<Vec<_>>();
    characters.windows(2).any(|pair| pair[0] == pair[1])
}

/// The reviewer's system prompt. The target language is a project fact the host
/// supplies; no source language is stated here because it is an element fact.
pub(crate) fn review_instructions(target: &str) -> String {
    format!(
        r#"You are a specialized bulk translation reviewer for manga pages, revising translations into {target}.
You receive minimal textual data: page labels, element IDs, source text, that source's language tag when the page records one, current translations, and optional local quality hints on an element.
An element can also carry scene evidence about where its text came from: "origin" (`gen` = the pipeline wrote the current translation, `user` = a person did), "role" (`dialogue` = a balloon holds the text, `free` = nothing does), and "region" with "score" (the detector read the text out of an area classified as something other than ordinary text, with that confidence). Use it to judge what kind of text you are looking at, never as permission to translate less: story text stays translated whatever its origin, and a short exclamation in a balloon is still dialogue.
Every element carries its own source language and a batch can mix them. Review each element against the language its own source is in; when an element has no tag, detect that language from the text. Never infer an element's source language from another element.
Review every provided translation in this order:
1. SOURCE FIDELITY — does the translation carry the source meaning? Check for lost or added information, wrong subject/object, inverted negation, wrong verb tense, and swapped pronouns or relationships. Accuracy failures are the most serious errors to fix.
2. OCR/TEXT NOISE — translation pipelines leave OCR artifacts. Look for isolated single letters after sentence punctuation, duplicated or cut fragments, misplaced punctuation (doubled marks, commas next to periods), and source-script characters left untranslated in the {target} text. Do not preserve a character just because it exists. Do NOT remove real initials ("K.", "J."), acronyms, units, or onomatopoeia.
3. NAMES/TERMINOLOGY — keep names and terms consistent. The provided translation memory is your canonical glossary: reuse existing keys and spellings and prefer them over new variants. If two spellings of the same name appear in one batch, normalize them to one. Add durable decisions (names, pronouns, relationships, tone) to "memory_updates" only when they are genuinely useful for later batches.
4. NATURAL {target} — only after steps 1–3, fix flow, register, and readability. Do not restyle translations that are already accurate.
Local "hints" are conservative advisory flags for possible OCR noise or untranslated source script. Verify each one against the source before acting; some hints are false positives and identical text is not necessarily an error.
Output ONLY a structured JSON response — no conversational text, no explanations outside the response object.
In "changes" include ONLY elements that need updating. If a translation is already correct, do NOT echo it.
Text that should never have been translated — a vocalization, a laugh, lettering drawn as art — goes in "flags" as {{element, reason, kind}}, with kind one of "vocalization", "decorative_sfx", "meaningful_sfx", "ambiguous". A sound effect carrying real content (a sign, a letter, a shouted word) is "meaningful_sfx" and stays translated: an onomatopoeia label is evidence, not a verdict.
To clear such text, set its "translation" to null in "changes" and flag the same element. The decision is made locally, from the scene evidence beside the text; anything not corroborated is reported to the user as a flag and left untouched. Never null a `user` translation, never null story text, and never null on shortness alone — "NO!" and "STOP!" are dialogue.
In "memory_updates" provide incremental additions or replacements for terminology, names, pronouns, relationships, tone decisions — not a full memory dump."#
    )
}

/// JSON schema for structured output (Responses API `text.format`).
pub(crate) fn review_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "changes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "element": { "type": "string" },
                        "translation": { "type": ["string", "null"] }
                    },
                    "required": ["element", "translation"],
                    "additionalProperties": false
                }
            },
            "flags": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "element": { "type": "string" },
                        "reason": { "type": "string" },
                        "kind": { "type": "string", "enum": SFX_KIND_NAMES }
                    },
                    "required": ["element", "reason", "kind"],
                    "additionalProperties": false
                }
            },
            "memory_updates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "value": { "type": "string" }
                    },
                    "required": ["key", "value"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["changes", "flags", "memory_updates"],
        "additionalProperties": false
    })
}

/// Estimate payload size for text-based batch planning.
pub(crate) fn estimate_text_size(summary: &PageTextSummary) -> usize {
    summary.source_chars
        + summary.translation_chars
        + summary.element_count * ESTIMATED_JSON_OVERHEAD_PER_ELEMENT
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PageTextSummary {
    pub(crate) element_count: usize,
    pub(crate) source_chars: usize,
    pub(crate) translation_chars: usize,
}

/// A page read for review: measured here for planning and kept as the payload the
/// reviewer is sent, so one read serves both and no page is fetched twice.
#[derive(Clone, Debug)]
pub(crate) struct MeasuredPage {
    pub(crate) id: String,
    pub(crate) summary: PageTextSummary,
    pub(crate) payload: Value,
}

/// Measure the pages of one `bulk_review_pages` payload, in the order read.
pub(crate) fn measure_pages(data: &Value) -> Result<Vec<MeasuredPage>> {
    let pages = data
        .get("pages")
        .and_then(Value::as_array)
        .context("bulk review data is missing its pages array")?;
    pages
        .iter()
        .map(|page| {
            let id = page
                .get("id")
                .and_then(Value::as_str)
                .context("bulk review page is missing its id")?
                .to_owned();
            let mut summary = PageTextSummary::default();
            for element in page
                .get("elements")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                summary.element_count = summary.element_count.saturating_add(1);
                summary.source_chars = summary.source_chars.saturating_add(
                    element
                        .get("source")
                        .and_then(Value::as_str)
                        .map_or(0, |text| text.chars().count()),
                );
                summary.translation_chars = summary.translation_chars.saturating_add(
                    element
                        .get("translation")
                        .and_then(Value::as_str)
                        .map_or(0, |text| text.chars().count()),
                );
            }
            Ok(MeasuredPage {
                id,
                summary,
                payload: page.clone(),
            })
        })
        .collect()
}

/// Text-based batch planner — groups measured pages to stay under the input
/// budget while preserving order.
pub(crate) fn plan_text_batches(pages: Vec<MeasuredPage>) -> Vec<Vec<MeasuredPage>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_size = 0_usize;

    for page in pages {
        let page_size = estimate_text_size(&page.summary);

        if !current_batch.is_empty()
            && current_size.saturating_add(page_size) > BULK_INPUT_BUDGET_CHARS
        {
            batches.push(std::mem::take(&mut current_batch));
            current_size = 0;
        }

        current_size = current_size.saturating_add(page_size);
        current_batch.push(page);

        // Very large single pages are isolated.
        if current_batch.len() == 1 && current_size > BULK_INPUT_BUDGET_CHARS {
            batches.push(std::mem::take(&mut current_batch));
            current_size = 0;
        }
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    batches
}

/// What local code knows about the elements one batch sent to the reviewer: which
/// pages it held, the translation each element carried, and the evidence behind it.
#[derive(Clone, Debug, Default)]
pub(crate) struct BatchContext {
    pages: HashSet<String>,
    page_of: HashMap<String, String>,
    current: HashMap<String, String>,
    evidence: HashMap<String, SfxEvidence>,
}

impl BatchContext {
    fn record(&mut self, page_id: &str, element: &ReviewElement) {
        self.pages.insert(page_id.to_owned());
        self.page_of.insert(element.id.clone(), page_id.to_owned());
        if let Some(text) = element.translation.as_deref() {
            self.current.insert(element.id.clone(), text.to_owned());
        }
        self.evidence
            .insert(element.id.clone(), SfxEvidence::of(element));
    }

    fn page(&self, element: &str) -> Result<&String> {
        let page = self
            .page_of
            .get(element)
            .with_context(|| format!("unknown element ID: {element}"))?;
        if !self.pages.contains(page) {
            bail!("element {element} belongs to page {page}, which is not in the target batch");
        }
        Ok(page)
    }

    /// How many elements the scene alone marks as possible lettering.
    pub(crate) fn suspicious(&self) -> usize {
        self.evidence
            .values()
            .filter(|evidence| evidence.suspicious())
            .count()
    }
}

/// One reviewer response turned into what local evidence authorizes.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReviewDecision {
    /// Writes to apply: text to set, or `None` to remove the translation.
    pub(crate) updates: Vec<(String, Option<String>)>,
    /// Translations this code erased, carrying the argument the reviewer gave.
    pub(crate) cleared: Vec<TranslationFlag>,
    /// Text that probably should not have been translated and that this code left
    /// alone: the reviewer's own flags, plus every erasure it refused.
    pub(crate) flags: Vec<TranslationFlag>,
}

impl ReviewDecision {
    /// Records a refused erasure. A flag the reviewer already raised for the element
    /// is rewritten in place, so the report says once per element what was asked and
    /// why it was not done, in payload order.
    fn hold_back(&mut self, element: String, kind: SfxKind, reason: String) {
        let flag = TranslationFlag {
            element,
            reason,
            kind,
        };
        match self
            .flags
            .iter()
            .position(|existing| existing.element == flag.element)
        {
            Some(index) => self.flags[index] = flag,
            None => self.flags.push(flag),
        }
    }
}

/// Validate one structured response, then gate every erasure on local evidence. A
/// malformed change fails the whole response, so nothing partial is ever applied. A
/// `null` that fails the gate is downgraded to a flag: the reviewer's proposal never
/// becomes a write on its own, and never disappears silently either.
pub(crate) fn resolve_review_response(
    response: &ReviewResponse,
    context: &BatchContext,
) -> Result<ReviewDecision> {
    if response.changes.len() > context.page_of.len() {
        bail!("more changes than available elements");
    }
    let mut flagged = HashSet::new();
    for flag in &response.flags {
        if flag.element.trim().is_empty() {
            bail!("flag element IDs cannot be empty");
        }
        context.page(&flag.element)?;
        if flag.reason.trim().is_empty() {
            bail!("flag for element {} needs a reason", flag.element);
        }
        if !flagged.insert(flag.element.clone()) {
            bail!("duplicate flag for element {}", flag.element);
        }
    }
    let mut decision = ReviewDecision {
        flags: response.flags.clone(),
        ..ReviewDecision::default()
    };

    let mut seen = HashSet::new();
    for change in &response.changes {
        if change.element.trim().is_empty() {
            bail!("change element IDs cannot be empty");
        }
        context.page(&change.element)?;
        if !seen.insert(change.element.clone()) {
            bail!("duplicate change for element {}", change.element);
        }
        match change.translation.as_deref() {
            None => {
                // Nothing in the project to erase is not a finding worth reporting: an
                // element without a translation is already what the reviewer wanted.
                if context.current.contains_key(&change.element) {
                    decision.refuse_or_apply(response, context, change);
                }
            }
            Some(text) => {
                let translation_chars = text.chars().count();
                if translation_chars == 0 || text.trim().is_empty() {
                    bail!("translation for element {} cannot be empty", change.element);
                }
                if translation_chars > MAX_CHANGE_CHARS {
                    bail!(
                        "translation for element {} exceeds {} characters",
                        change.element,
                        MAX_CHANGE_CHARS
                    );
                }
                // A change that repeats the current text is not a revision: applying
                // it would mint an undo step and promote pipeline output to user
                // authorship. Removing a translation is a revision, not an echo.
                if context
                    .current
                    .get(&change.element)
                    .is_some_and(|current| current.trim() == text.trim())
                {
                    continue;
                }
                decision
                    .updates
                    .push((change.element.clone(), Some(text.to_owned())));
            }
        }
    }
    if response.memory_updates.len() > MAX_MEMORY_UPDATES {
        bail!("too many memory updates");
    }
    for update in &response.memory_updates {
        if update.key.trim().is_empty() || update.value.trim().is_empty() {
            bail!("memory updates require non-empty key and value");
        }
    }
    Ok(decision)
}

impl ReviewDecision {
    /// One erasure request, decided by the reviewer's argument and the scene's
    /// evidence together. Either half missing keeps the text exactly where it is.
    fn refuse_or_apply(
        &mut self,
        response: &ReviewResponse,
        context: &BatchContext,
        change: &TranslationChange,
    ) {
        let argument = response
            .flags
            .iter()
            .find(|flag| flag.element == change.element);
        let mut why: Vec<&'static str> = Vec::new();
        match argument {
            None => why.push("asked to remove a translation without flagging the element"),
            Some(flag) if !flag.kind.clears() => {
                why.push("the reviewer did not call it a vocalization or decorative lettering")
            }
            Some(_) => {}
        }
        match context.evidence.get(&change.element) {
            None => why.push("the page records no evidence about this element"),
            Some(evidence) if !evidence.machine => {
                why.push("the translation in the project is not machine-written")
            }
            Some(evidence) if !evidence.may_clear() => {
                why.push("the scene corroborates it with fewer than two strong signals")
            }
            Some(_) => {}
        }
        if why.is_empty() {
            if let Some(flag) = argument {
                self.cleared.push(flag.clone());
                // The flag was the argument for the erasure, so it belongs with what
                // was done, not with what this code left alone.
                self.flags
                    .retain(|existing| existing.element != change.element);
            }
            self.updates.push((change.element.clone(), None));
            return;
        }
        let kind = argument.map_or(SfxKind::Ambiguous, |flag| flag.kind);
        let reason = match argument {
            Some(flag) => format!("{} — not removed: {}", flag.reason, why.join("; ")),
            None => format!("not removed: {}", why.join("; ")),
        };
        self.hold_back(change.element.clone(), kind, reason);
    }
}

/// Keeps a job's report bounded: every entry is counted, only the first few are
/// carried, so a 500-page job cannot turn its own findings into context pressure.
fn collect_report(entries: Vec<Value>, total: &mut usize, reported: &mut Vec<Value>) {
    *total = total.saturating_add(entries.len());
    let room = MAX_REPORTED_FLAGS.saturating_sub(reported.len());
    reported.extend(entries.into_iter().take(room));
}

/// Bulk reviews always use the lowest reasoning effort the model supports so
/// the repeated per-batch inferences stay cheap and fast.
pub(crate) fn lowest_reasoning(model: &CodexModel) -> Reasoning {
    model
        .reasoning
        .iter()
        .copied()
        .min_by_key(|reasoning| reasoning_rank(*reasoning))
        .unwrap_or(Reasoning::Low)
}

fn reasoning_rank(reasoning: Reasoning) -> usize {
    match reasoning {
        Reasoning::Low => 0,
        Reasoning::Medium => 1,
        Reasoning::High => 2,
        Reasoning::Xhigh => 3,
        Reasoning::Max => 4,
        Reasoning::Ultra => 5,
    }
}

/// Tool the model calls to start a review job, and the synthetic tool name the
/// run reports per batch. This module owns both, so no other file repeats them.
pub(crate) const REVIEW_JOB_TOOL: &str = "review_translations_job";
const REVIEW_BATCH_TOOL: &str = "review_page_batch";

/// The host tool this reviewer applies its result with. It reuses the model's own
/// write path rather than a second one, so the name must match what the host
/// advertises.
const APPLY_TOOL: &str = "set_translations";

/// Arguments for a review job.
#[derive(Deserialize)]
pub(crate) struct PageJob {
    pub(crate) pages: Vec<String>,
    pub(crate) instruction: String,
}

pub(crate) fn review_job_tool() -> Tool {
    Tool::new(
        REVIEW_JOB_TOOL,
        "Review and revise the translations of an ordered set of pages autonomously in small sequential batches. Use for a translation review spanning more than five pages; the user does not need to request each batch. The result also lists text flagged as lettering that should not have been translated, and any translation it cleared: report both to the user.",
        json!({
            "type": "object",
            "properties": {
                "pages": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "instruction": { "type": "string" }
            },
            "required": ["pages", "instruction"],
            "additionalProperties": false
        }),
    )
}

/// Which run a job belongs to, and therefore which prompt-cache prefix it may
/// reuse. Grouped because a job needs all three together at every step.
pub(crate) struct JobContext {
    pub(crate) run: RunId,
    pub(crate) conversation: Uuid,
    pub(crate) call_id: String,
}

impl JobContext {
    /// Stable for the whole job: the instructions and schema prefix are identical
    /// across batches, only the pages differ.
    fn cache_key(&self) -> String {
        format!("review:{}:{}", self.conversation, self.call_id)
    }
}

/// What every batch of one job shares, so the per-batch request stays small.
struct ReviewJob<'a> {
    instruction: &'a str,
    target: &'a str,
    session: &'a str,
}

/// Deterministic, mockable reviewer orchestrator. Each successful batch is exactly
/// one model request followed by local validation and a single bulk apply.
pub(crate) struct BulkReviewer<'a> {
    client: &'a dyn ReviewClient,
    host: &'a dyn Host,
}

impl<'a> BulkReviewer<'a> {
    /// Run `call` if it is this reviewer's tool, otherwise `None` so the caller
    /// routes it to the host. Keeping the decision here means the tool loop never
    /// matches on a tool name it does not own.
    pub(crate) async fn execute<F>(
        client: &'a dyn ReviewClient,
        host: &'a dyn Host,
        context: JobContext,
        call: &ToolCall,
        model: &CodexModel,
        control: &Control,
        publish: &mut F,
    ) -> Option<Result<Invocation>>
    where
        F: FnMut(Event),
    {
        if call.name != REVIEW_JOB_TOOL {
            return None;
        }
        Some(
            Self::new(client, host)
                .run_job(context, &call.arguments, model, control, publish)
                .await,
        )
    }

    pub(crate) fn new(client: &'a dyn ReviewClient, host: &'a dyn Host) -> Self {
        Self { client, host }
    }

    pub(crate) async fn run_job<F>(
        &self,
        context: JobContext,
        arguments: &str,
        model: &CodexModel,
        control: &Control,
        publish: &mut F,
    ) -> Result<Invocation>
    where
        F: FnMut(Event),
    {
        let run = context.run;
        control.ensure_running()?;
        let job: PageJob = serde_json::from_str(arguments)
            .context(format!("invalid arguments for {REVIEW_JOB_TOOL}"))?;
        anyhow::ensure!(
            !job.pages.is_empty(),
            "page job requires at least one page ID"
        );
        anyhow::ensure!(
            job.instruction.chars().count() <= MAX_INSTRUCTION_CHARS,
            "the job instruction is longer than {MAX_INSTRUCTION_CHARS} characters"
        );
        let mut seen = HashSet::new();
        let pages = job
            .pages
            .into_iter()
            .filter(|page| seen.insert(page.clone()))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            pages.len() <= MAX_JOB_PAGES,
            "a review job covers at most {MAX_JOB_PAGES} pages"
        );
        let total = pages.len();
        // The target language is a project fact the host owns; the reviewer is not
        // allowed to guess it, and never guesses the source.
        let target = self
            .host
            .review_target_language()
            .context("failed to read the project's target language")?;

        let mut completed = 0_usize;
        let mut failed = Vec::new();
        let mut failed_pages = 0_usize;
        let mut flags = Vec::new();
        let mut flag_total = 0_usize;
        let mut cleared = Vec::new();
        let mut cleared_total = 0_usize;
        let mut memory = TranslationMemory::new();
        let mut changed = false;
        let mut metrics = JobMetrics::default();
        let mut sequence = 0_usize;
        let session = context.cache_key();
        let review = ReviewJob {
            instruction: &job.instruction,
            target: &target,
            session: &session,
        };

        for group in pages.chunks(MAX_PAGES_PER_READ) {
            control.ensure_running()?;
            // One read serves measurement and payload: a page is read once per job,
            // and only this group is ever held. Pages in different groups are disjoint,
            // so a later group is not affected by what an earlier batch wrote.
            let data = self
                .host
                .bulk_review_pages(group)
                .await
                .context("failed to read project pages for review")?;
            let measured = measure_pages(&data)?;
            anyhow::ensure!(
                measured.len() == group.len(),
                "reading {} pages for review returned {}",
                group.len(),
                measured.len()
            );
            let mut queue = plan_text_batches(measured)
                .into_iter()
                .collect::<VecDeque<_>>();

            while let Some(batch) = queue.pop_front() {
                control.ensure_running()?;
                sequence += 1;
                let batch_call_id = format!("{}-batch-{sequence}", context.call_id);
                let batch_pages = batch
                    .iter()
                    .map(|page| page.id.as_str())
                    .collect::<Vec<_>>();
                publish(Event::ToolStarted {
                    run,
                    call_id: batch_call_id.clone(),
                    name: REVIEW_BATCH_TOOL.to_owned(),
                });
                match self
                    .run_review_batch(&review, &batch, &memory, model, control, sequence)
                    .await
                {
                    Ok(result) => {
                        changed |= result.changed;
                        memory = result.memory;
                        completed += batch.len();
                        metrics.add_batch(&result.metrics);
                        publish(Event::ToolFinished {
                            run,
                            call_id: batch_call_id,
                            name: REVIEW_BATCH_TOOL.to_owned(),
                            changed: result.changed,
                            output: json!({
                                "pages": batch_pages,
                                "summary": result.metrics.log_summary(),
                                "completed": completed,
                                "total": total,
                            })
                            .to_string(),
                        });
                        collect_report(result.flags, &mut flag_total, &mut flags);
                        collect_report(result.cleared, &mut cleared_total, &mut cleared);
                    }
                    Err(error) if error.context_limit && batch.len() > 1 => {
                        // Split a too-large batch in half and reprocess both halves
                        // from the payload already read. A single page is terminal.
                        let middle = batch.len().div_ceil(2);
                        let left = batch[..middle].to_vec();
                        let right = batch[middle..].to_vec();
                        queue.push_front(right);
                        queue.push_front(left);
                        publish(Event::ToolFinished {
                            run,
                            call_id: batch_call_id,
                            name: REVIEW_BATCH_TOOL.to_owned(),
                            changed: false,
                            output: json!({ "pages": batch_pages, "status": "split" }).to_string(),
                        });
                    }
                    Err(error) => {
                        failed_pages += batch.len();
                        let room = MAX_REPORTED_FAILURES.saturating_sub(failed.len());
                        failed.extend(
                            batch.iter().take(room).map(
                                |page| json!({ "page": page.id, "error": format!("{error:#}") }),
                            ),
                        );
                        publish(Event::ToolFinished {
                            run,
                            call_id: batch_call_id,
                            name: REVIEW_BATCH_TOOL.to_owned(),
                            changed: false,
                            output: json!({
                                "pages": batch_pages,
                                "status": "failed",
                                "completed": completed,
                                "total": total,
                            })
                            .to_string(),
                        });
                        if control.is_cancelled() {
                            return Err(error.source);
                        }
                    }
                }
            }
        }

        if completed == 0 {
            // A job that reviewed nothing is not a success the model should report,
            // however tidy its per-batch failure list looks.
            let reason = failed
                .first()
                .and_then(|failure| failure.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("no batch was attempted");
            bail!("every batch of the review job failed: {reason}");
        }

        metrics.log();
        let findings = |total: usize, reported: Vec<Value>| {
            json!({
                "total": total,
                "reported": reported,
            })
        };
        // Only the outcome stays in conversation history. The memory dump, the
        // metrics and the findings themselves are for this turn; carrying them into
        // every later request is the context pressure this worker exists to avoid,
        // while the failed page IDs are what the model needs in order to not redo
        // the whole range.
        let history = json!({
            "total": total,
            "completed": completed,
            "failed_pages": failed_pages,
            "failed": failed,
            "flagged": flag_total,
            "cleared": cleared_total,
            "omitted_from_history": ["memory", "metrics", "flags", "cleared"],
        });
        let value = json!({
            "total": total,
            "completed": completed,
            "failed_pages": failed_pages,
            "failed": failed.clone(),
            "flags": findings(flag_total, flags),
            "cleared": findings(cleared_total, cleared),
            "memory": memory.serialize(),
            "metrics": json!({
                "batches": metrics.total_batches,
                "changes": metrics.total_changes,
                "hints": metrics.total_hints,
                "lettering_candidates": metrics.total_lettering_candidates,
                "model_requests": metrics.total_model_requests,
                "elapsed_ms": metrics.total_elapsed_ms,
            }),
        });
        let invocation = if changed {
            Invocation::changed(value)?
        } else {
            Invocation::read(value)?
        };
        Ok(invocation.with_history(history))
    }

    /// One model request, local validation, local bulk apply.
    async fn run_review_batch(
        &self,
        job: &ReviewJob<'_>,
        batch: &[MeasuredPage],
        memory: &TranslationMemory,
        model: &CodexModel,
        control: &Control,
        batch_index: usize,
    ) -> std::result::Result<ReviewBatchResult, ReviewBatchError> {
        let instruction = job.instruction;
        let target = job.target;
        let session = job.session;
        let started = Instant::now();
        control.ensure_running().map_err(ReviewBatchError::from)?;

        let mut context = BatchContext::default();
        let mut review_pages = Vec::with_capacity(batch.len());
        let mut element_count = 0_usize;
        let mut source_chars = 0_usize;
        let mut translation_chars = 0_usize;

        for measured in batch {
            let page = &measured.payload;
            let page_id = measured.id.clone();
            let label = page
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut elements = Vec::new();
            for (order, element) in page
                .get("elements")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let id = element
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let source = element
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let source_language = element
                    .get("lang")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let translation = element
                    .get("translation")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let origin = element
                    .get("origin")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let role = element
                    .get("role")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let region = element
                    .get("region")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let score = element
                    .get("score")
                    .and_then(Value::as_f64)
                    .map(|score| score as f32);
                source_chars = source_chars.saturating_add(source.chars().count());
                translation_chars = translation_chars.saturating_add(
                    translation
                        .as_deref()
                        .map_or(0, |text| text.chars().count()),
                );
                element_count = element_count.saturating_add(1);
                let element = ReviewElement {
                    id,
                    order,
                    source,
                    source_language,
                    translation,
                    origin,
                    role,
                    region,
                    score,
                    quality_hints: Vec::new(),
                };
                context.record(&page_id, &element);
                elements.push(element);
            }
            review_pages.push(ReviewPage {
                id: page_id,
                label,
                elements,
            });
        }

        // Hints compare each element's source script against the scripts this batch
        // is written in, which is only known once every translation has been read.
        let translated = review_pages
            .iter()
            .flat_map(|page| page.elements.iter())
            .filter_map(|element| {
                element
                    .translation
                    .as_deref()
                    .map(|text| (element.source.as_str(), text))
            })
            .collect::<Vec<_>>();
        let target_scripts = batch_target_scripts(&translated);
        let mut hints_count = 0_usize;
        for element in review_pages
            .iter_mut()
            .flat_map(|page| page.elements.iter_mut())
        {
            let residue = residue_scripts(&element.source, &target_scripts);
            let hints = quality_hints(&residue, element.translation.as_deref(), target);
            hints_count = hints_count.saturating_add(hints.len());
            element.quality_hints = hints;
        }

        // Elements the scene itself points at as possible lettering. Reported so a
        // slow or expensive job can be read against what it actually looked at.
        let sfx_candidates = context.suspicious();

        // The pages themselves are the payload: the model must see every element's
        // source and current translation, not just the page IDs.
        let pages_json = serde_json::to_string(&review_pages)
            .map_err(|error| ReviewBatchError::from(anyhow!(error)))?;
        let payload_bytes = pages_json.len();
        let memory_text = memory.serialize();
        let prompt = format!(
            "Objective: {instruction}\nTranslation memory (incremental updates only, do not repeat):\n{}\nReview the pages below and return ONLY the structured JSON response.\n\n{pages_json}",
            if memory_text.is_empty() {
                "(none)"
            } else {
                &memory_text
            },
        );
        let input = vec![message("user", prompt)];
        let request = Request::new(
            model.id.clone(),
            review_instructions(target),
            input,
            Vec::new(),
            lowest_reasoning(model),
            session.to_owned(),
        )
        .with_json_schema(review_response_schema(), "review_response");

        let mut attempt = 0_usize;
        let turn = loop {
            control.ensure_running().map_err(ReviewBatchError::from)?;
            match self.client.respond_once(&request, control).await {
                Ok(turn) => break turn,
                Err(error) if should_retry(attempt, &error, false) => {
                    retry_wait(backoff(attempt), control)
                        .await
                        .map_err(ReviewBatchError::from)?;
                    attempt = attempt.saturating_add(1);
                }
                Err(source) => return Err(ReviewBatchError::from(source)),
            }
        };
        let model_requests = attempt.saturating_add(1);

        let response_bytes = turn.text.len();
        let response_json: ReviewResponse = serde_json::from_str(&turn.text)
            .with_context(|| format!("invalid structured review response (batch {batch_index})"))
            .map_err(ReviewBatchError::from)?;
        let decision =
            resolve_review_response(&response_json, &context).map_err(ReviewBatchError::from)?;
        let cleared = decision.cleared.len();
        let flags = decision.flags.len();

        let updates = decision
            .updates
            .iter()
            .map(|(element, text)| json!({ "element": element, "text": text }))
            .collect::<Vec<_>>();
        // Both lists are reported with the page they belong to, so the conversation
        // can point at the text instead of at an opaque entity ID.
        let reported = |entries: &[TranslationFlag]| {
            entries
                .iter()
                .map(|flag| {
                    json!({
                        "page": context.page_of.get(&flag.element),
                        "element": flag.element,
                        "reason": flag.reason,
                        "kind": flag.kind,
                    })
                })
                .collect::<Vec<_>>()
        };
        let reported_flags = reported(&decision.flags);
        let reported_cleared = reported(&decision.cleared);
        let applied = updates.len();
        let mut changed = false;
        if !updates.is_empty() {
            let invocation = self
                .host
                .invoke(
                    ToolCall {
                        call_id: format!("bulk-apply-{batch_index}"),
                        name: APPLY_TOOL.to_owned(),
                        arguments: serde_json::to_string(&json!({ "updates": updates }))
                            .unwrap_or_default(),
                    },
                    control,
                )
                .await
                .map_err(ReviewBatchError::from)?;
            changed = invocation.changed;
        }

        let memory = memory
            .clone()
            .apply_updates(response_json.memory_updates)
            .bounded(MAX_MEMORY_BYTES);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let metrics = BatchMetrics {
            batch_index,
            page_count: batch.len(),
            element_count,
            hints_count,
            lettering_candidates: sfx_candidates,
            source_chars,
            translation_chars,
            payload_bytes,
            model_requests,
            response_bytes,
            changes_applied: applied,
            flags,
            cleared,
            elapsed_ms,
            input_tokens: turn.usage.map(|usage| usage.input_tokens),
            cached_input_tokens: turn.usage.map(|usage| usage.cached_input_tokens),
            output_tokens: turn.usage.map(|usage| usage.output_tokens),
            reasoning_tokens: turn.usage.map(|usage| usage.reasoning_tokens),
        };
        metrics.log();

        Ok(ReviewBatchResult {
            changed,
            memory,
            metrics,
            flags: reported_flags,
            cleared: reported_cleared,
        })
    }
}

struct ReviewBatchResult {
    changed: bool,
    memory: TranslationMemory,
    metrics: BatchMetrics,
    /// Text the reviewer says should not have been translated, none of it touched.
    flags: Vec<Value>,
    /// Translations this code erased, with the argument each was erased on.
    cleared: Vec<Value>,
}

#[derive(Debug)]
struct ReviewBatchError {
    source: anyhow::Error,
    context_limit: bool,
}

impl From<anyhow::Error> for ReviewBatchError {
    fn from(source: anyhow::Error) -> Self {
        let context_limit = is_context_limit(&source);
        Self {
            source,
            context_limit,
        }
    }
}

impl std::fmt::Display for ReviewBatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.source)
    }
}

impl std::error::Error for ReviewBatchError {}

/// Metrics for a single bulk review batch.
#[derive(Clone, Debug, Default)]
pub(crate) struct BatchMetrics {
    pub(crate) batch_index: usize,
    pub(crate) page_count: usize,
    pub(crate) element_count: usize,
    pub(crate) hints_count: usize,
    pub(crate) lettering_candidates: usize,
    pub(crate) source_chars: usize,
    pub(crate) translation_chars: usize,
    pub(crate) payload_bytes: usize,
    pub(crate) model_requests: usize,
    pub(crate) response_bytes: usize,
    pub(crate) changes_applied: usize,
    pub(crate) flags: usize,
    pub(crate) cleared: usize,
    pub(crate) elapsed_ms: u64,
    // Opt-in usage data only when the backend reports it.
    pub(crate) input_tokens: Option<usize>,
    pub(crate) cached_input_tokens: Option<usize>,
    pub(crate) output_tokens: Option<usize>,
    pub(crate) reasoning_tokens: Option<usize>,
}

impl BatchMetrics {
    pub(crate) fn log(&self) {
        tracing::debug!(
            target: "koharu_agent::bulk_review",
            batch = self.batch_index,
            pages = self.page_count,
            elements = self.element_count,
            hints = self.hints_count,
            lettering_candidates = self.lettering_candidates,
            source_chars = self.source_chars,
            translation_chars = self.translation_chars,
            payload_bytes = self.payload_bytes,
            model_requests = self.model_requests,
            response_bytes = self.response_bytes,
            changes = self.changes_applied,
            flags = self.flags,
            cleared = self.cleared,
            elapsed_ms = self.elapsed_ms,
            input_tokens = self.input_tokens,
            cached_input_tokens = self.cached_input_tokens,
            output_tokens = self.output_tokens,
            reasoning_tokens = self.reasoning_tokens,
            "bulk review batch completed"
        );
    }

    pub(crate) fn log_summary(&self) -> String {
        format!(
            "batch {}: {} pages, {} elements, {} changes ({} cleared), {} hints, \
             {} flagged, {} lettering candidate(s), {} request(s), {} ms",
            self.batch_index,
            self.page_count,
            self.element_count,
            self.changes_applied,
            self.cleared,
            self.hints_count,
            self.flags,
            self.lettering_candidates,
            self.model_requests,
            self.elapsed_ms
        )
    }
}

/// Aggregate metrics across all batches in a job.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobMetrics {
    pub(crate) total_batches: usize,
    pub(crate) total_pages: usize,
    pub(crate) total_elements: usize,
    pub(crate) total_hints: usize,
    pub(crate) total_lettering_candidates: usize,
    pub(crate) total_changes: usize,
    pub(crate) total_flags: usize,
    pub(crate) total_cleared: usize,
    pub(crate) total_model_requests: usize,
    pub(crate) total_elapsed_ms: u64,
    pub(crate) total_input_tokens: usize,
    pub(crate) total_cached_input_tokens: usize,
    pub(crate) total_output_tokens: usize,
    pub(crate) total_reasoning_tokens: usize,
}

impl JobMetrics {
    pub(crate) fn add_batch(&mut self, batch: &BatchMetrics) {
        self.total_batches = self.total_batches.saturating_add(1);
        self.total_pages = self.total_pages.saturating_add(batch.page_count);
        self.total_elements = self.total_elements.saturating_add(batch.element_count);
        self.total_hints = self.total_hints.saturating_add(batch.hints_count);
        self.total_lettering_candidates = self
            .total_lettering_candidates
            .saturating_add(batch.lettering_candidates);
        self.total_changes = self.total_changes.saturating_add(batch.changes_applied);
        self.total_flags = self.total_flags.saturating_add(batch.flags);
        self.total_cleared = self.total_cleared.saturating_add(batch.cleared);
        self.total_model_requests = self
            .total_model_requests
            .saturating_add(batch.model_requests);
        self.total_elapsed_ms = self.total_elapsed_ms.saturating_add(batch.elapsed_ms);
        if let Some(tokens) = batch.input_tokens {
            self.total_input_tokens = self.total_input_tokens.saturating_add(tokens);
        }
        if let Some(tokens) = batch.cached_input_tokens {
            self.total_cached_input_tokens = self.total_cached_input_tokens.saturating_add(tokens);
        }
        if let Some(tokens) = batch.output_tokens {
            self.total_output_tokens = self.total_output_tokens.saturating_add(tokens);
        }
        if let Some(tokens) = batch.reasoning_tokens {
            self.total_reasoning_tokens = self.total_reasoning_tokens.saturating_add(tokens);
        }
    }

    pub(crate) fn log(&self) {
        tracing::info!(
            target: "koharu_agent::bulk_review",
            batches = self.total_batches,
            pages = self.total_pages,
            elements = self.total_elements,
            hints = self.total_hints,
            lettering_candidates = self.total_lettering_candidates,
            changes = self.total_changes,
            flags = self.total_flags,
            cleared = self.total_cleared,
            model_requests = self.total_model_requests,
            elapsed_ms = self.total_elapsed_ms,
            input_tokens = self.total_input_tokens,
            cached_input_tokens = self.total_cached_input_tokens,
            output_tokens = self.total_output_tokens,
            reasoning_tokens = self.total_reasoning_tokens,
            "bulk review job completed"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use reqwest::StatusCode;
    use serde_json::json;

    use crate::{
        Invocation,
        codex::{Turn, message, provider_failure, response_error},
    };

    use super::*;

    fn empty_turn() -> Turn {
        Turn {
            output: Vec::new(),
            calls: Vec::new(),
            text: r#"{"changes":[],"memory_updates":[]}"#.to_owned(),
            usage: None,
            payload_bytes: 0,
            first_delta_ms: 0,
        }
    }

    fn review_text(changes: &[(&str, &str)]) -> String {
        review_response(changes, &[])
    }

    fn review_response(changes: &[(&str, &str)], memory_updates: &[(&str, &str)]) -> String {
        serde_json::json!({
            "changes": changes
                .iter()
                .map(|(element, translation)| json!({ "element": element, "translation": translation }))
                .collect::<Vec<_>>(),
            "memory_updates": memory_updates
                .iter()
                .map(|(key, value)| json!({ "key": key, "value": value }))
                .collect::<Vec<_>>(),
        })
        .to_string()
    }

    struct MockClient {
        calls: AtomicUsize,
        script: Mutex<VecDeque<std::result::Result<Turn, anyhow::Error>>>,
        prompts: Mutex<Vec<String>>,
        instructions: Mutex<Vec<String>>,
        cache_keys: Mutex<Vec<String>>,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                script: Mutex::new(VecDeque::new()),
                prompts: Mutex::new(Vec::new()),
                instructions: Mutex::new(Vec::new()),
                cache_keys: Mutex::new(Vec::new()),
            }
        }

        fn push(&self, result: std::result::Result<Turn, anyhow::Error>) {
            self.script.lock().unwrap().push_back(result);
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }

        fn instructions(&self) -> Vec<String> {
            self.instructions.lock().unwrap().clone()
        }

        fn cache_keys(&self) -> Vec<String> {
            self.cache_keys.lock().unwrap().clone()
        }

        fn respond_ok(&self, text: impl Into<String>) -> std::result::Result<Turn, anyhow::Error> {
            let mut turn = empty_turn();
            turn.text = text.into();
            Ok(turn)
        }
    }

    #[async_trait]
    impl ReviewClient for MockClient {
        async fn respond_once(
            &self,
            request: &Request,
            _control: &Control,
        ) -> std::result::Result<Turn, anyhow::Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let prompt = request
                .input
                .first()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            self.prompts.lock().unwrap().push(prompt);
            self.instructions
                .lock()
                .unwrap()
                .push(request.instructions.clone());
            self.cache_keys
                .lock()
                .unwrap()
                .push(request.prompt_cache_key.clone());
            let mut script = self.script.lock().unwrap();
            if let Some(result) = script.pop_front() {
                return result;
            }
            Ok(empty_turn())
        }
    }

    struct MockHost {
        pages: HashMap<String, Vec<Value>>,
        applied: Mutex<Vec<(String, Option<String>)>>,
        reads: Mutex<Vec<Vec<String>>>,
        target: String,
    }

    impl Default for MockHost {
        fn default() -> Self {
            Self {
                pages: HashMap::new(),
                applied: Mutex::new(Vec::new()),
                reads: Mutex::new(Vec::new()),
                target: "Portuguese".to_owned(),
            }
        }
    }

    impl MockHost {
        fn with_target(mut self, target: &str) -> Self {
            self.target = target.to_owned();
            self
        }

        fn add_element(
            &mut self,
            page_id: &str,
            element_id: &str,
            source: &str,
            source_language: Option<&str>,
            translation: Option<&str>,
        ) {
            self.pages
                .entry(page_id.to_owned())
                .or_default()
                .push(json!({
                    "id": element_id,
                    "source": source,
                    "lang": source_language,
                    "translation": translation,
                }));
        }

        /// Like [`Self::add_element`], with the scene evidence keys the host would
        /// record beside the text.
        fn add_element_with_evidence(
            &mut self,
            page_id: &str,
            element_id: &str,
            source: &str,
            translation: Option<&str>,
            evidence: Value,
        ) {
            let mut element = json!({
                "id": element_id,
                "source": source,
                "translation": translation,
            });
            if let (Some(object), Some(extra)) = (element.as_object_mut(), evidence.as_object()) {
                object.extend(extra.clone());
            }
            self.pages
                .entry(page_id.to_owned())
                .or_default()
                .push(element);
        }

        fn add_page(&mut self, id: &str, source_chars: usize) {
            self.add_element(
                id,
                &format!("{id}e1"),
                &"x".repeat(source_chars),
                None,
                None,
            );
        }

        fn applied(&self) -> Vec<(String, Option<String>)> {
            self.applied.lock().unwrap().clone()
        }

        fn reads(&self) -> Vec<Vec<String>> {
            self.reads.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Host for MockHost {
        async fn context(&self) -> Result<Value> {
            Ok(json!({ "project": {}, "pages": [] }))
        }

        fn tools(&self) -> Vec<Tool> {
            Vec::new()
        }

        fn review_target_language(&self) -> Result<String> {
            Ok(self.target.clone())
        }

        async fn invoke(&self, call: ToolCall, _control: &Control) -> Result<Invocation> {
            if call.name != APPLY_TOOL {
                bail!("unexpected tool call {}", call.name);
            }
            let value: Value = serde_json::from_str(&call.arguments)?;
            let updates = value
                .get("updates")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for update in updates {
                let element = update
                    .get("element")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let text = update
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                self.applied.lock().unwrap().push((element, text));
            }
            Invocation::changed(json!({ "ok": true }))
        }

        async fn bulk_review_pages(&self, pages: &[String]) -> Result<Value> {
            self.reads.lock().unwrap().push(pages.to_vec());
            let mut result = Vec::new();
            for id in pages {
                let elements = self.pages.get(id).context("unknown page")?.clone();
                let page = match id.find('p') {
                    Some(_) => id.trim_start_matches('p').to_owned(),
                    None => id.clone(),
                };
                result.push(json!({
                    "id": id,
                    "label": format!("Page {page}"),
                    "elements": elements,
                }));
            }
            Ok(json!({ "pages": result }))
        }
    }

    fn model() -> CodexModel {
        CodexModel {
            id: "codex".to_owned(),
            name: "Codex".to_owned(),
            reasoning: vec![Reasoning::Low, Reasoning::Medium, Reasoning::High],
        }
    }

    fn job_arguments(pages: &[&str]) -> String {
        json!({
            "pages": pages,
            "instruction": "review these pages",
        })
        .to_string()
    }

    fn measured(id: &str, summary: PageTextSummary) -> MeasuredPage {
        MeasuredPage {
            id: id.to_owned(),
            summary,
            payload: json!({ "id": id }),
        }
    }

    fn batch_ids(batches: &[Vec<MeasuredPage>]) -> Vec<Vec<&str>> {
        batches
            .iter()
            .map(|batch| batch.iter().map(|page| page.id.as_str()).collect())
            .collect()
    }

    fn job_context() -> JobContext {
        JobContext {
            run: RunId::default(),
            conversation: Uuid::nil(),
            call_id: "job-1".to_owned(),
        }
    }

    async fn run_job(
        client: &MockClient,
        host: &MockHost,
        pages: &[&str],
    ) -> std::result::Result<Invocation, anyhow::Error> {
        let control = Control::default();
        let reviewer = BulkReviewer::new(client, host);
        reviewer
            .run_job(
                job_context(),
                &job_arguments(pages),
                &model(),
                &control,
                &mut |_event| {},
            )
            .await
    }

    #[test]
    fn translation_memory_applies_updates_and_replaces() {
        let memory = TranslationMemory::new()
            .apply_updates(vec![
                MemoryUpdate {
                    key: "Kimura".to_owned(),
                    value: "keep Kimura".to_owned(),
                },
                MemoryUpdate {
                    key: "RH".to_owned(),
                    value: "人事部".to_owned(),
                },
            ])
            .apply_updates(vec![MemoryUpdate {
                key: "RH".to_owned(),
                value: "shachou".to_owned(),
            }]);
        let serialized = memory.serialize();
        assert!(serialized.contains("Kimura: keep Kimura"));
        assert!(serialized.contains("RH: shachou"));
        assert!(!serialized.contains("人事部"));
    }

    #[test]
    fn translation_memory_enforces_size_limit() {
        let memory = TranslationMemory::new().apply_updates(vec![
            MemoryUpdate {
                key: "a".to_owned(),
                value: "x".repeat(1000),
            },
            MemoryUpdate {
                key: "b".to_owned(),
                value: "y".repeat(1000),
            },
            MemoryUpdate {
                key: "c".to_owned(),
                value: "z".repeat(1000),
            },
        ]);
        let bounded = memory.bounded(2000);
        assert!(bounded.serialize().len() <= 2000);
        assert!(!bounded.entries.is_empty());
    }

    #[test]
    fn translation_memory_is_utf8_safe() {
        let memory = TranslationMemory::new().apply_updates(vec![MemoryUpdate {
            key: "cadena".to_owned(),
            value: "ßé雨🐱".repeat(500),
        }]);
        let bounded = memory.bounded(300);
        assert!(bounded.serialize().len() <= 300);
        assert!(String::from_utf8(bounded.serialize().into_bytes()).is_ok());
    }

    #[test]
    fn text_batch_planner_groups_pages_under_budget() {
        let small = PageTextSummary {
            element_count: 10,
            source_chars: 500,
            translation_chars: 600,
        };
        let big = PageTextSummary {
            element_count: 50,
            source_chars: 19_000,
            translation_chars: 19_000,
        };
        let batches = plan_text_batches(vec![
            measured("p1", small.clone()),
            measured("p2", small),
            measured("p3", big),
        ]);
        // p1+p2 batch together (~3.5k), p3 alone (~41k, exceeds the 20k budget).
        assert_eq!(batch_ids(&batches), vec![vec!["p1", "p2"], vec!["p3"]]);
    }

    #[tokio::test]
    async fn duplicate_page_ids_are_reviewed_once() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        host.add_page("p2", 30);
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[])));

        let invocation = run_job(&client, &host, &["p1", "p1", "p2", "p1"])
            .await
            .unwrap();
        assert_eq!(invocation.value["total"].as_u64(), Some(2));
        assert_eq!(host.reads(), vec![vec!["p1".to_owned(), "p2".to_owned()]]);
    }

    #[tokio::test]
    async fn pages_are_read_in_bounded_groups() {
        let count = MAX_PAGES_PER_READ + 5;
        let mut host = MockHost::default();
        let pages = (0..count)
            .map(|index| format!("p{index}"))
            .collect::<Vec<_>>();
        for page in &pages {
            host.add_page(page, 10);
        }
        let client = MockClient::new();
        for _ in 0..2 {
            client.push(client.respond_ok(review_text(&[])));
        }

        let refs = pages.iter().map(String::as_str).collect::<Vec<_>>();
        let invocation = run_job(&client, &host, &refs).await.unwrap();
        assert_eq!(invocation.value["total"].as_u64(), Some(count as u64));
        // No read is larger than the retention ceiling, and the whole job is covered.
        let expected = [
            pages[..MAX_PAGES_PER_READ].to_vec(),
            pages[MAX_PAGES_PER_READ..].to_vec(),
        ];
        assert_eq!(host.reads(), expected.to_vec());
    }

    #[test]
    fn measure_pages_reports_real_text_sizes_in_read_order() {
        let data = json!({
            "pages": [
                {
                    "id": "p1",
                    "elements": [
                        { "id": "e1", "source": "你好", "translation": "hello" },
                        { "id": "e2", "source": "世界", "translation": "bye" },
                    ],
                },
                { "id": "p2", "elements": [] },
            ]
        });
        let measured = measure_pages(&data).unwrap();
        assert_eq!(
            measured
                .iter()
                .map(|page| page.id.as_str())
                .collect::<Vec<_>>(),
            vec!["p1", "p2"]
        );
        assert_eq!(measured[0].summary.element_count, 2);
        assert_eq!(measured[0].summary.source_chars, 4);
        assert_eq!(measured[0].summary.translation_chars, 8);
        assert_eq!(measured[1].summary.element_count, 0);
    }

    /// An element as the reviewer's payload carries it, evidence keys included.
    fn element(
        id: &str,
        source: &str,
        translation: Option<&str>,
        evidence: Value,
    ) -> ReviewElement {
        ReviewElement {
            id: id.to_owned(),
            source: source.to_owned(),
            translation: translation.map(ToOwned::to_owned),
            origin: evidence
                .get("origin")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            role: evidence
                .get("role")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            region: evidence
                .get("region")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            score: evidence
                .get("score")
                .and_then(Value::as_f64)
                .map(|score| score as f32),
            ..ReviewElement::default()
        }
    }

    /// A batch of elements, all of them on page `p1`.
    fn batch(elements: &[ReviewElement]) -> BatchContext {
        let mut context = BatchContext::default();
        for element in elements {
            context.record("p1", element);
        }
        context
    }

    fn revise(element: &str, text: &str) -> TranslationChange {
        TranslationChange {
            element: element.to_owned(),
            translation: Some(text.to_owned()),
        }
    }

    fn erase(element: &str) -> TranslationChange {
        TranslationChange {
            element: element.to_owned(),
            translation: None,
        }
    }

    fn flag(element: &str, kind: SfxKind) -> TranslationFlag {
        TranslationFlag {
            element: element.to_owned(),
            reason: "lettering, not story text".to_owned(),
            kind,
        }
    }

    fn response(changes: Vec<TranslationChange>, flags: Vec<TranslationFlag>) -> ReviewResponse {
        ReviewResponse {
            changes,
            flags,
            ..ReviewResponse::default()
        }
    }

    /// Machine text outside any balloon, in a region the detector called lettering
    /// and was sure about: the evidence an erasure needs.
    fn lettering_evidence() -> Value {
        json!({
            "origin": "gen",
            "role": "free",
            "region": "onomatopoeia",
            "score": 0.91,
        })
    }

    #[test]
    fn resolves_a_wellformed_response() {
        let elements = [
            element("e1", "Hello", Some("Olá"), json!({})),
            element("e2", "World", Some("Mundo"), json!({})),
        ];
        let decision = resolve_review_response(
            &response(vec![revise("e1", "revised"), revise("e2", "fixed")], vec![]),
            &batch(&elements),
        )
        .unwrap();
        assert_eq!(decision.updates.len(), 2);
        assert!(decision.cleared.is_empty());
        assert!(decision.flags.is_empty());
    }

    #[test]
    fn rejects_element_outside_batch() {
        // A page this batch never sent is not something the reviewer may answer about.
        let context = BatchContext {
            pages: ["p1".to_owned()].into_iter().collect(),
            page_of: HashMap::from([("e_other".to_owned(), "p99".to_owned())]),
            ..BatchContext::default()
        };
        assert!(
            resolve_review_response(&response(vec![revise("e_other", "bad")], vec![]), &context,)
                .is_err()
        );
    }

    #[test]
    fn rejects_unknown_or_duplicate_or_empty_entries() {
        let elements = [element("e1", "Oi", Some("Olá"), json!({}))];
        let context = batch(&elements);
        assert!(
            resolve_review_response(&response(vec![revise("ghost", "x")], vec![]), &context)
                .is_err()
        );
        assert!(
            resolve_review_response(
                &response(vec![revise("e1", "first"), revise("e1", "second")], vec![]),
                &context
            )
            .is_err()
        );
        assert!(
            resolve_review_response(&response(vec![revise("e1", "   ")], vec![]), &context)
                .is_err()
        );
        assert!(
            resolve_review_response(
                &response(
                    vec![],
                    vec![TranslationFlag {
                        element: "e1".to_owned(),
                        reason: "  ".to_owned(),
                        kind: SfxKind::Vocalization,
                    }],
                ),
                &context
            )
            .is_err(),
            "a flag without a reason is not a finding"
        );
        assert!(
            resolve_review_response(
                &response(
                    vec![],
                    vec![
                        flag("e1", SfxKind::Vocalization),
                        flag("e1", SfxKind::Ambiguous)
                    ],
                ),
                &context
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_excessive_changes_and_bad_memory() {
        let elements = [element("e1", "Oi", Some("Olá"), json!({}))];
        let context = batch(&elements);
        assert!(
            resolve_review_response(
                &response(vec![revise("e1", "a"), revise("e2", "b")], vec![]),
                &context
            )
            .is_err()
        );
        let bad_memory = ReviewResponse {
            memory_updates: vec![MemoryUpdate {
                key: "k".to_owned(),
                value: String::new(),
            }],
            ..ReviewResponse::default()
        };
        assert!(resolve_review_response(&bad_memory, &context).is_err());
    }

    #[test]
    fn a_repeated_translation_is_not_a_change() {
        let elements = [element("e1", "Oi", Some("Olá"), json!({}))];
        let decision = resolve_review_response(
            &response(vec![revise("e1", " Olá ")], vec![]),
            &batch(&elements),
        )
        .unwrap();
        assert!(
            decision.updates.is_empty(),
            "echoing text must not mint a revision"
        );
    }

    #[test]
    fn corroborated_lettering_is_erased_when_the_reviewer_names_it() {
        let elements = [element("e1", "ドン", Some("ドドン"), lettering_evidence())];
        let decision = resolve_review_response(
            &response(vec![erase("e1")], vec![flag("e1", SfxKind::DecorativeSfx)]),
            &batch(&elements),
        )
        .unwrap();
        assert_eq!(decision.updates, vec![("e1".to_owned(), None)]);
        assert_eq!(decision.cleared.len(), 1);
        assert!(
            decision.flags.is_empty(),
            "the flag was the argument for a change that happened"
        );
    }

    #[test]
    fn short_dialogue_survives_a_request_to_erase_it() {
        // Machine origin plus a short one-word source is exactly the shape the gate
        // must never act on: these lines are someone speaking.
        for line in ["NO!", "STOP!", "HELP!", "RUN!"] {
            let elements = [element(
                "e1",
                line,
                Some(line),
                json!({ "origin": "gen", "role": "dialogue" }),
            )];
            let decision = resolve_review_response(
                &response(vec![erase("e1")], vec![flag("e1", SfxKind::Vocalization)]),
                &batch(&elements),
            )
            .unwrap();
            assert!(decision.updates.is_empty(), "{line} must keep its text");
            assert!(decision.cleared.is_empty());
            assert_eq!(decision.flags.len(), 1, "{line} is reported instead");
            assert!(
                decision.flags[0]
                    .reason
                    .contains("fewer than two strong signals"),
                "{}",
                decision.flags[0].reason
            );
        }
    }

    #[test]
    fn a_persons_translation_is_never_erased_here() {
        let mut evidence = lettering_evidence();
        evidence["origin"] = json!("user");
        let elements = [element("e1", "ドン", Some("ドドン"), evidence)];
        let decision = resolve_review_response(
            &response(vec![erase("e1")], vec![flag("e1", SfxKind::Vocalization)]),
            &batch(&elements),
        )
        .unwrap();
        assert!(decision.updates.is_empty());
        assert!(
            decision.flags[0].reason.contains("not machine-written"),
            "{}",
            decision.flags[0].reason
        );
    }

    #[test]
    fn one_strong_signal_is_not_corroboration() {
        // No balloon holds it, and that is all the scene says.
        let elements = [element(
            "e1",
            "ゴゴゴ",
            Some("Gogogo"),
            json!({ "origin": "gen", "role": "free" }),
        )];
        let decision = resolve_review_response(
            &response(vec![erase("e1")], vec![flag("e1", SfxKind::Vocalization)]),
            &batch(&elements),
        )
        .unwrap();
        assert!(decision.updates.is_empty());
        assert_eq!(decision.flags.len(), 1);
    }

    #[test]
    fn an_unnamed_erasure_is_reported_instead_of_applied() {
        let elements = [element("e1", "ドン", Some("ドドン"), lettering_evidence())];
        let decision =
            resolve_review_response(&response(vec![erase("e1")], vec![]), &batch(&elements))
                .unwrap();
        assert!(decision.updates.is_empty());
        assert_eq!(decision.flags[0].kind, SfxKind::Ambiguous);
        assert!(
            decision.flags[0]
                .reason
                .contains("without flagging the element"),
            "{}",
            decision.flags[0].reason
        );
    }

    #[test]
    fn lettering_with_content_stays_translated() {
        // The reviewer itself called it meaningful: that is an argument for keeping it.
        let elements = [element(
            "e1",
            "「助けて」",
            Some("Help!"),
            lettering_evidence(),
        )];
        let decision = resolve_review_response(
            &response(vec![erase("e1")], vec![flag("e1", SfxKind::MeaningfulSfx)]),
            &batch(&elements),
        )
        .unwrap();
        assert!(decision.updates.is_empty());
        assert!(
            decision.flags[0]
                .reason
                .contains("did not call it a vocalization"),
            "{}",
            decision.flags[0].reason
        );
    }

    #[test]
    fn erasing_text_that_was_never_translated_does_nothing() {
        let elements = [element("e1", "ドン", None, lettering_evidence())];
        let decision = resolve_review_response(
            &response(vec![erase("e1")], vec![flag("e1", SfxKind::Vocalization)]),
            &batch(&elements),
        )
        .unwrap();
        // There is nothing to remove, so no write is minted — but the finding that
        // this text should stay untranslated is still worth reporting.
        assert!(decision.updates.is_empty());
        assert!(decision.cleared.is_empty());
        assert_eq!(decision.flags.len(), 1);
    }

    #[test]
    fn lowest_reasoning_prefers_low_when_supported() {
        let model = CodexModel {
            id: "a".to_owned(),
            name: "a".to_owned(),
            reasoning: vec![Reasoning::Medium, Reasoning::High, Reasoning::Low],
        };
        assert_eq!(lowest_reasoning(&model), Reasoning::Low);
        let model = CodexModel {
            id: "b".to_owned(),
            name: "b".to_owned(),
            reasoning: vec![Reasoning::Medium, Reasoning::High],
        };
        assert_eq!(lowest_reasoning(&model), Reasoning::Medium);
        let model = CodexModel {
            id: "c".to_owned(),
            name: "c".to_owned(),
            reasoning: vec![],
        };
        assert_eq!(lowest_reasoning(&model), Reasoning::Low);
    }

    /// Hints the reviewer would attach to the last element of a batch, with the
    /// batch's target scripts estimated from every element in it.
    fn hints(batch: &[(&str, &str)], target: &str) -> Vec<&'static str> {
        let target_scripts = batch_target_scripts(batch);
        let (source, translation) = batch
            .last()
            .copied()
            .expect("a hinted batch has at least one element");
        quality_hints(
            &residue_scripts(source, &target_scripts),
            Some(translation),
            target,
        )
    }

    #[test]
    fn minimal_payload_excludes_geometry_and_metadata() {
        let page = ReviewPage {
            id: "p1".to_owned(),
            label: "Page 1".to_owned(),
            elements: vec![
                ReviewElement {
                    id: "e1".to_owned(),
                    order: 0,
                    source: "Hello".to_owned(),
                    source_language: Some("en-US".to_owned()),
                    translation: Some("Olá".to_owned()),
                    ..ReviewElement::default()
                },
                ReviewElement {
                    id: "e2".to_owned(),
                    order: 1,
                    source: "World".to_owned(),
                    translation: None,
                    ..ReviewElement::default()
                },
            ],
        };
        let body = serde_json::to_string(&page).unwrap();
        for forbidden in [
            "geometry",
            "typography",
            "visibility",
            "provider",
            "pipeline",
            "image",
            // Evidence keys stay off the wire until the scene records something.
            "origin",
            "role",
            "region",
            "score",
        ] {
            assert!(
                !body.contains(forbidden),
                "payload must not contain {forbidden}"
            );
        }
        for required in [
            "Hello",
            "Olá",
            "World",
            "en-US",
            "\"order\":0",
            "\"order\":1",
        ] {
            assert!(body.contains(required), "payload must contain {required}");
        }
        // An element without a recorded source language omits the key.
        assert_eq!(body.matches("\"lang\"").count(), 1);
    }

    #[test]
    fn quality_hints_flag_source_script_left_behind() {
        let leak = [
            ("ちょっと待って", "Espera aí!"),
            ("行こう", "Vamos lá!"),
            ("世界を見ろ", "Olá, 世界!"),
        ];
        assert!(
            hints(&leak, "Portuguese").contains(&"untranslated_source_script"),
            "Han left among Portuguese translations must be flagged"
        );
        // Hiragana-only and Hangul residue were invisible to the old range table.
        let kana = [
            ("ちょっと待って", "Espera aí!"),
            ("行こう", "Vamos lá!"),
            ("さようなら", "さようなら"),
        ];
        assert!(hints(&kana, "Portuguese").contains(&"untranslated_source_script"));
        let hangul = [
            ("ちょっと待って", "Espera aí!"),
            ("행복합니다", "행복합니다"),
        ];
        assert!(
            hints(&hangul, "Portuguese").contains(&"untranslated_source_script"),
            "Hangul left in the batch must be flagged"
        );

        let repeated = [
            ("ちょっと待って", "Espera aí!"),
            ("行こう", "Vamos lá!"),
            ("何度も言う", "Vamos ver vamos ver."),
        ];
        assert!(hints(&repeated, "Portuguese").contains(&"repeated_fragment"));
        let orphan = [
            ("ちょっと待って", "Espera aí!"),
            ("行こう", "Vamos lá!"),
            ("あれをちょうだい", "Me dá aquilo! M"),
        ];
        let flagged = hints(&orphan, "Portuguese");
        assert!(flagged.contains(&"possible_orphan_character"));
        assert_eq!(flagged.len(), 1);
        assert!(
            hints(
                &[("ちょっと待って", "Espera aí!"), ("行こう", "Oi!!")],
                "Portuguese"
            )
            .contains(&"suspicious_punctuation")
        );
        assert!(
            hints(
                &[("ちょっと待って", "Espera aí!"), ("行こう", "sério,,")],
                "Portuguese"
            )
            .contains(&"suspicious_punctuation")
        );
    }

    #[test]
    fn quality_hints_follow_the_target_language() {
        let clean = [
            ("ちょっと待って", "Espera aí!"),
            ("行こう", "Vamos lá!"),
            ("世界を見ろ", "Veja o mundo!"),
        ];
        assert!(hints(&clean, "Portuguese").is_empty());

        // A Chinese target legitimately keeps Han; only the kana it does not use
        // would be residue there.
        let chinese = [
            ("ちょっと待って", "等等"),
            ("行こう", "走吧"),
            ("世界を見ろ", "看世界"),
        ];
        assert!(
            !hints(&chinese, "Chinese").contains(&"untranslated_source_script"),
            "Han is the Chinese target's own script"
        );

        // A Latin source into a Latin target leaks nothing.
        assert!(
            hints(&[("Hello there", "Olá, forasteiro")], "Portuguese").is_empty(),
            "sharing an alphabet is not untranslated text"
        );

        // The isolated-letter list is Portuguese-only, so a target this module does
        // not model gets no such hint instead of a wrong one.
        let orphan = [
            ("ちょっと待って", "Espera aí!"),
            ("行こう", "Vamos lá!"),
            ("あれをちょうだい", "Me dá aquilo! M"),
        ];
        assert!(hints(&orphan, "Portuguese").contains(&"possible_orphan_character"));
        assert!(
            !hints(&orphan, "English").contains(&"possible_orphan_character"),
            "an unknown target must not be judged by Portuguese word lists"
        );
    }

    #[test]
    fn residue_is_an_element_fact_not_a_batch_fact() {
        let batch = [
            ("ちょっと待って", "Espera aí!"),
            ("Hello there", "Olá, forasteiro"),
        ];
        let target_scripts = batch_target_scripts(&batch);
        assert!(residue_scripts("ちょっと待って", &target_scripts).contains(&IcuScript::Hiragana));
        assert!(residue_scripts("Hello there", &target_scripts).is_empty());
    }

    #[test]
    fn quality_hints_do_not_flag_legitimate_portuguese() {
        for text in [
            "Este texto está correto.",
            "Olha essa cena! É",
            "Permaneceu calado. Sim!Muito calado",
            "Foi ontem à noite! K.",
            "— J. saiu correndo.",
            "Muito muito legal.",
            "Cer...? E depois!",
            "Sim!Muito",
            "Ela disse... né!",
            "Foi mesmo!?",
        ] {
            let batch = [("ちょっと待って", "Espera aí!"), ("行こう", text)];
            assert!(
                hints(&batch, "Portuguese").is_empty(),
                "{text} must not be flagged"
            );
        }
        assert!(
            quality_hints(&scripts("ちょっと待って"), None, "Portuguese").is_empty(),
            "an element without a translation has nothing to hint about"
        );
    }

    #[test]
    fn quality_hints_serialize_only_when_present() {
        let plain = ReviewElement {
            id: "e1".to_owned(),
            order: 0,
            source: "Hello".to_owned(),
            source_language: None,
            translation: Some("Olá".to_owned()),
            ..ReviewElement::default()
        };
        let plain_body = serde_json::to_string(&plain).unwrap();
        assert!(!plain_body.contains("quality_hints"));

        let hinted = ReviewElement {
            id: "e2".to_owned(),
            order: 1,
            source: "ちょっと待って".to_owned(),
            source_language: Some("ja-JP".to_owned()),
            translation: Some("Espera aí! M".to_owned()),
            quality_hints: hints(
                &[("行こう", "Vamos lá!"), ("ちょっと待って", "Espera aí! M")],
                "Portuguese",
            ),
            ..ReviewElement::default()
        };
        let hinted_body = serde_json::to_string(&hinted).unwrap();
        assert!(hinted_body.contains("quality_hints"));
        assert!(hinted_body.contains("possible_orphan_character"));
    }

    #[tokio::test]
    async fn payload_embeds_pages_and_quality_hints() {
        let mut host = MockHost::default();
        host.add_element(
            "p1",
            "p1e1",
            "ちょうどいい",
            Some("ja-JP"),
            Some("Perfeito! M"),
        );
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[])));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert!(!invocation.changed);
        assert_eq!(client.calls(), 1);
        let prompts = client.prompts();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("\"id\":\"p1e1\""));
        assert!(prompts[0].contains("Perfeito! M"));
        assert!(prompts[0].contains("possible_orphan_character"));
        assert!(invocation.value["metrics"]["hints"].as_u64() == Some(1));
    }

    #[tokio::test]
    async fn scene_evidence_reaches_the_reviewer() {
        let mut host = MockHost::default();
        host.add_element_with_evidence(
            "p1",
            "p1e1",
            "ドン",
            Some("ドドン"),
            json!({
                "origin": "gen",
                "role": "free",
                "region": "onomatopoeia",
                "score": 0.91,
            }),
        );
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[])));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        let prompt = &client.prompts()[0];
        for key in [
            "\"origin\":\"gen\"",
            "\"role\":\"free\"",
            "\"region\":\"onomatopoeia\"",
            "\"score\":0.91",
        ] {
            assert!(prompt.contains(key), "payload must carry {key}: {prompt}");
        }
        assert_eq!(
            invocation.value["metrics"]["lettering_candidates"].as_u64(),
            Some(1)
        );
    }

    #[tokio::test]
    async fn a_page_without_scene_evidence_reports_no_candidate() {
        let mut host = MockHost::default();
        host.add_element_with_evidence(
            "p1",
            "p1e1",
            "NO!",
            Some("Não!"),
            json!({ "origin": "gen", "role": "dialogue" }),
        );
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[])));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert!(
            !client.prompts()[0].contains("\"region\""),
            "an element with no lettering evidence keeps the payload bare"
        );
        assert_eq!(
            invocation.value["metrics"]["lettering_candidates"].as_u64(),
            Some(0)
        );
    }

    #[test]
    fn lettering_evidence_separates_scene_facts_from_text_shape() {
        let evidence_of =
            |source: &str, evidence: Value| SfxEvidence::of(&element("e", source, None, evidence));

        // A one-word line inside a balloon is someone speaking, not lettering.
        for line in ["NO!", "STOP!", "HELP!", "RUN!"] {
            let evidence = evidence_of(line, json!({ "origin": "gen", "role": "dialogue" }));
            assert_eq!(evidence.strong().count(), 0, "{line} is story text");
            assert!(
                !evidence.suspicious(),
                "{line} must not look like lettering"
            );
        }

        // Repetition earns a closer look and nothing more.
        let vocalization = evidence_of("アアア", json!({ "origin": "gen", "role": "dialogue" }));
        assert!(vocalization.has(SfxSignal::SingleToken));
        assert!(vocalization.has(SfxSignal::RepeatedChar));
        assert_eq!(vocalization.strong().count(), 0);
        assert!(vocalization.suspicious());

        // The scene classifying the region is what makes evidence strong.
        let lettering = evidence_of("ドン", lettering_evidence());
        assert!(lettering.has(SfxSignal::RegionOnomatopoeia));
        assert!(lettering.has(SfxSignal::DetectorScore));
        assert!(lettering.has(SfxSignal::NotDialogue));
        assert_eq!(lettering.strong().count(), 3);
        assert!(lettering.may_clear());

        // The detector doubting a text region tells the model to look harder; it is
        // not evidence that the text is lettering.
        let unsure = evidence_of(
            "行こう",
            json!({ "origin": "gen", "role": "dialogue", "score": 0.62 }),
        );
        assert!(!unsure.has(SfxSignal::DetectorScore));
        assert_eq!(unsure.strong().count(), 0);
        assert!(!unsure.may_clear());

        // Text a person wrote never looks suspicious to a shape heuristic.
        assert!(!evidence_of("!!!", json!({ "origin": "user", "role": "dialogue" })).suspicious());
    }

    /// A response that erases one element, arguing it is a given kind of lettering.
    fn review_erasure(element: &str, kind: &str) -> String {
        json!({
            "changes": [{ "element": element, "translation": null }],
            "flags": [{
                "element": element,
                "reason": "a drawn sound effect, not speech",
                "kind": kind,
            }],
            "memory_updates": [],
        })
        .to_string()
    }

    #[tokio::test]
    async fn a_corroborated_erasure_reaches_the_host_as_removal() {
        let mut host = MockHost::default();
        host.add_element_with_evidence("p1", "p1e1", "ドン", Some("ドドン"), lettering_evidence());
        let client = MockClient::new();
        client.push(client.respond_ok(review_erasure("p1e1", "decorative_sfx")));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert_eq!(host.applied(), vec![("p1e1".to_owned(), None)]);
        assert!(invocation.changed);
        assert_eq!(invocation.value["cleared"]["total"].as_u64(), Some(1));
        assert_eq!(invocation.value["flags"]["total"].as_u64(), Some(0));
        assert_eq!(
            invocation.value["cleared"]["reported"][0]["page"].as_str(),
            Some("p1"),
            "the report has to say where the text was"
        );
    }

    #[tokio::test]
    async fn an_uncorroborated_erasure_is_reported_and_the_text_stays() {
        let mut host = MockHost::default();
        host.add_element_with_evidence(
            "p1",
            "p1e1",
            "NO!",
            Some("Não!"),
            json!({ "origin": "gen", "role": "dialogue" }),
        );
        let client = MockClient::new();
        client.push(client.respond_ok(review_erasure("p1e1", "vocalization")));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert!(host.applied().is_empty(), "nothing may be written");
        assert!(!invocation.changed);
        assert_eq!(invocation.value["flags"]["total"].as_u64(), Some(1));
        assert_eq!(invocation.value["cleared"]["total"].as_u64(), Some(0));
        assert!(
            invocation.value["flags"]["reported"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("not removed"),
            "the report has to say why the text stayed"
        );
    }

    #[tokio::test]
    async fn memory_carries_across_batches_in_every_payload() {
        let mut host = MockHost::default();
        for page in ["p1", "p2", "p3"] {
            host.add_page(page, 8_900);
        }
        let client = MockClient::new();
        client
            .push(client.respond_ok(review_response(&[("p1e1", "mudou")], &[("Terra", "Terra")])));
        client.push(client.respond_ok(review_text(&[])));

        let invocation = run_job(&client, &host, &["p1", "p2", "p3"]).await.unwrap();
        assert!(invocation.changed);
        // ~9k chars per page → [p1, p2] + [p3].
        assert_eq!(client.calls(), 2);
        let prompts = client.prompts();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].contains("(none)"));
        // The second batch prompt must include the memory recorded in batch 1.
        assert!(prompts[1].contains("Terra: Terra"));
        assert!(
            invocation.value["memory"]
                .as_str()
                .unwrap()
                .contains("Terra: Terra")
        );
    }

    #[test]
    fn schema_is_strict_compatible() {
        let schema = review_response_schema();
        let required = schema["required"].as_array().unwrap();
        let names = required
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["changes", "flags", "memory_updates"]);
        assert_eq!(schema["additionalProperties"], false);
        for property in schema["properties"].as_object().unwrap().values() {
            if let Some(items) = property.get("items") {
                assert_eq!(items["additionalProperties"], false);
            }
        }
        // Removing a translation must be expressible, and every kind the gate reads
        // has to be named for the reviewer to choose.
        assert_eq!(
            schema["properties"]["changes"]["items"]["properties"]["translation"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            schema["properties"]["flags"]["items"]["properties"]["kind"]["enum"],
            json!(SFX_KIND_NAMES)
        );
    }

    #[tokio::test]
    async fn the_reviewer_is_told_the_projects_target_language() {
        let mut host = MockHost::default().with_target("English");
        host.add_element(
            "p1",
            "p1e1",
            "こんにちは",
            Some("ja-JP"),
            Some("Hello there"),
        );
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[])));

        run_job(&client, &host, &["p1"]).await.unwrap();
        let instructions = client.instructions();
        assert_eq!(instructions.len(), 1);
        assert!(
            instructions[0].contains("into English"),
            "{}",
            instructions[0]
        );
        for assumed in ["Portuguese", "Japanese"] {
            assert!(
                !instructions[0].contains(assumed),
                "instructions must not hardcode {assumed}"
            );
        }
    }

    #[tokio::test]
    async fn one_batch_keeps_every_elements_source_language() {
        let mut host = MockHost::default();
        host.add_element(
            "p1",
            "e1",
            "ちょっと待って",
            Some("ja-JP"),
            Some("Espera aí!"),
        );
        host.add_element(
            "p1",
            "e2",
            "Hello there",
            Some("en-US"),
            Some("Olá, forasteiro"),
        );
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[])));

        run_job(&client, &host, &["p1"]).await.unwrap();
        let prompt = client.prompts()[0].clone();
        assert!(prompt.contains("\"lang\":\"ja-JP\""), "{prompt}");
        assert!(prompt.contains("\"lang\":\"en-US\""), "{prompt}");
    }

    #[tokio::test]
    async fn one_successful_batch_is_one_model_request() {
        let mut host = MockHost::default();
        for page in ["p1", "p2", "p3"] {
            host.add_page(page, 30);
        }
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[("p1e1", "Olá mundo")])));

        let invocation = run_job(&client, &host, &["p1", "p2", "p3"]).await.unwrap();
        assert!(invocation.changed);
        assert_eq!(client.calls(), 1);
        assert_eq!(
            host.applied(),
            vec![("p1e1".to_owned(), Some("Olá mundo".to_owned()))]
        );
    }

    #[tokio::test]
    async fn every_batch_of_a_job_shares_one_cache_key() {
        let mut host = MockHost::default();
        for page in ["p1", "p2", "p3", "p4"] {
            host.add_page(page, 8_900);
        }
        let client = MockClient::new();
        for _ in 0..2 {
            client.push(client.respond_ok(review_text(&[])));
        }

        run_job(&client, &host, &["p1", "p2", "p3", "p4"])
            .await
            .unwrap();
        let keys = client.cache_keys();
        assert_eq!(keys.len(), 2);
        // Distinct batches, one cached prefix: the instructions and schema are
        // identical, only the pages differ.
        assert_eq!(keys[0], keys[1]);
        assert!(keys[0].starts_with("review:"), "{}", keys[0]);
    }

    #[tokio::test]
    async fn an_echoed_translation_writes_nothing() {
        let mut host = MockHost::default();
        host.add_element("p1", "p1e1", "こんにちは", Some("ja-JP"), Some("Olá"));
        let client = MockClient::new();
        client.push(client.respond_ok(review_text(&[("p1e1", "Olá")])));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert_eq!(client.calls(), 1);
        assert!(host.applied().is_empty());
        assert!(!invocation.changed);
        assert_eq!(invocation.value["metrics"]["changes"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn job_history_keeps_outcome_and_drops_details() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        let client = MockClient::new();
        client.push(client.respond_ok(review_response(
            &[("p1e1", "revisado")],
            &[("Tanaka", "Tanaka")],
        )));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert_eq!(invocation.history["total"], 1);
        assert_eq!(invocation.history["completed"], 1);
        assert!(
            invocation.history.get("memory").is_none(),
            "the glossary dump must not ride along in every later request"
        );
        assert!(invocation.history.get("metrics").is_none());
        // The turn that ran the job still sees the full result.
        assert!(invocation.value["memory"].is_string());
        assert!(invocation.value["metrics"].is_object());
    }

    #[tokio::test]
    async fn three_successful_batches_are_three_model_requests() {
        let mut host = MockHost::default();
        for page in ["p1", "p2", "p3", "p4", "p5", "p6"] {
            // ~9k chars per page → two pages per batch under the 20k budget.
            host.add_page(page, 8_900);
        }
        let client = MockClient::new();
        for (page, text) in [("p1e1", "one"), ("p3e1", "two"), ("p6e1", "three")] {
            client.push(client.respond_ok(review_text(&[(page, text)])));
        }

        let invocation = run_job(&client, &host, &["p1", "p2", "p3", "p4", "p5", "p6"])
            .await
            .unwrap();
        assert!(invocation.changed);
        assert_eq!(client.calls(), 3);
        assert_eq!(host.applied().len(), 3);
        let applied = host.applied();
        assert!(applied.contains(&("p1e1".to_owned(), Some("one".to_owned()))));
        assert!(applied.contains(&("p3e1".to_owned(), Some("two".to_owned()))));
        assert!(applied.contains(&("p6e1".to_owned(), Some("three".to_owned()))));
    }

    #[tokio::test]
    async fn context_overflow_splits_batches_until_success() {
        let mut host = MockHost::default();
        for page in ["p1", "p2", "p3", "p4"] {
            host.add_page(page, 50);
        }
        let client = MockClient::new();
        client.push(Err(provider_failure(
            "Codex response failed",
            "your input exceeds the context window of this model",
        )));
        client.push(client.respond_ok(review_text(&[("p1e1", "a"), ("p2e1", "b")])));
        client.push(client.respond_ok(review_text(&[("p3e1", "c"), ("p4e1", "d")])));

        let invocation = run_job(&client, &host, &["p1", "p2", "p3", "p4"])
            .await
            .unwrap();
        assert!(invocation.changed);
        // First attempt + two halved batches.
        assert_eq!(client.calls(), 3);
        assert_eq!(host.applied().len(), 4);
        // Splitting preserves page order across halves.
        let applied = host.applied();
        assert_eq!(applied[0].0, "p1e1");
        assert_eq!(applied[3].0, "p4e1");
    }

    #[tokio::test]
    async fn transient_errors_are_retried_before_apply() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        let client = MockClient::new();
        client.push(Err(response_error(
            StatusCode::TOO_MANY_REQUESTS,
            "slow down".to_owned(),
        )));
        client.push(client.respond_ok(review_text(&[("p1e1", "fixed")])));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert!(invocation.changed);
        assert_eq!(client.calls(), 2);
        assert_eq!(
            host.applied(),
            vec![("p1e1".to_owned(), Some("fixed".to_owned()))]
        );
    }

    #[tokio::test]
    async fn an_unrelated_refusal_ends_the_job_instead_of_splitting_batches() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        host.add_page("p2", 30);
        let client = MockClient::new();
        // A refusal about tokens that is not about the context window: guessing from
        // message text used to halve every batch and ask again, grinding through the
        // whole page list on a problem no smaller batch can solve.
        client.push(Err(response_error(
            StatusCode::UNAUTHORIZED,
            "refresh token reuse detected".to_owned(),
        )));

        let error = run_job(&client, &host, &["p1", "p2"]).await.unwrap_err();
        assert_eq!(client.calls(), 1);
        assert!(host.applied().is_empty());
        let message = format!("{error:#}");
        assert!(message.contains("every batch"), "{message}");
    }

    #[tokio::test]
    async fn invalid_response_applies_nothing() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        let client = MockClient::new();
        // References an element that is not part of the batch.
        client.push(client.respond_ok(review_text(&[("ghost", "nope")])));

        let error = run_job(&client, &host, &["p1"]).await.unwrap_err();
        assert_eq!(client.calls(), 1);
        assert!(host.applied().is_empty());
        // Nothing was reviewed, so the job reports failure rather than a clean
        // result the model would summarize as success.
        assert!(format!("{error:#}").contains("every batch"), "{error:#}");
    }

    #[tokio::test]
    async fn cancelled_job_stops_before_work() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        let client = MockClient::new();
        let control = Control::default();
        control.cancel();
        let reviewer = BulkReviewer::new(&client, &host);
        let result = reviewer
            .run_job(
                job_context(),
                &job_arguments(&["p1"]),
                &model(),
                &control,
                &mut |_event| {},
            )
            .await;
        assert!(result.is_err());
        assert_eq!(client.calls(), 0);
        assert!(host.applied().is_empty());
    }

    #[test]
    fn delta_is_produced_by_protocol_message() {
        let message = message("user", "hello");
        assert_eq!(message["role"], "user");
        assert!(message["content"][0]["text"].as_str().is_some());
    }
}
