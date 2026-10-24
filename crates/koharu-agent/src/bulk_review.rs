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

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    CodexModel, Control, Host, Invocation, Reasoning, Tool, ToolCall,
    agent::{Event, RunId},
    codex::{
        Request, ReviewClient, backoff, is_transient_error, message, retry_wait, should_retry,
    },
};

/// Upper bound (in characters) for a single batch's textual payload. Small pages
/// batch together well beyond V2's fixed 5-page cap; big pages stay isolated.
pub(crate) const BULK_INPUT_BUDGET_CHARS: usize = 20_000;

/// Per-volume overhead heuristic used when planning batches.
const ESTIMATED_JSON_OVERHEAD_PER_ELEMENT: usize = 64;

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

#[derive(Serialize)]
pub(crate) struct ReviewElement {
    /// Element entity ID
    pub(crate) id: String,
    /// Reading order within the page
    pub(crate) order: usize,
    /// Source text
    pub(crate) source: String,
    /// Current translation (may be absent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) translation: Option<String>,
    /// Conservative local quality hints (advisory signals only — never applied
    /// as edits by this module), serialized only when non-empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) quality_hints: Vec<&'static str>,
}

/// Structured response from the bulk reviewer.
#[derive(Debug, Deserialize)]
pub(crate) struct ReviewResponse {
    /// Changes to apply — only elements that need updating.
    pub(crate) changes: Vec<TranslationChange>,
    /// Memory updates (incremental: add/replace, not full rewrite).
    #[serde(default)]
    pub(crate) memory_updates: Vec<MemoryUpdate>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TranslationChange {
    /// Element entity ID to update
    pub(crate) element: String,
    /// New translation text
    pub(crate) translation: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryUpdate {
    pub(crate) key: String,
    pub(crate) value: String,
}

/// Compact translation memory — local merge, structured storage.
#[derive(Clone, Debug, Default)]
pub(crate) struct TranslationMemory {
    entries: HashMap<String, String>,
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
        let mut entries = self.entries.drain().collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        let mut size = 0_usize;
        self.entries.clear();
        for (key, value) in entries {
            let entry_size = key.len() + value.len() + 2; // "; " separator + opening bracket bookkeeping
            if size + entry_size > max_bytes {
                continue;
            }
            self.entries.insert(key, value);
            size += entry_size;
        }
        self
    }
}

/// Local quality hints for a review element. They are conservative signals
/// computed locally to guide the reviewer toward likely OCR noise or missing
/// content. They are advisory only — this module never edits text directly.
fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{303F}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{FF66}'..='\u{FFDC}'
    )
}

/// CJK characters left in a Portuguese translation.
fn has_untranslated_cjk(text: &str) -> bool {
    text.chars().any(is_cjk)
}

/// Latin and CJK glyphs mixed inside the same whitespace-separated token.
fn has_suspicious_mixed_script(text: &str) -> bool {
    text.split(char::is_whitespace)
        .any(|token| token.chars().any(|c| c.is_ascii_alphabetic()) && token.chars().any(is_cjk))
}

/// Trailing isolated ASCII letter after sentence-ending punctuation, e.g. the
/// stray "M" in "Me dá aquilo! M". Conservative: single-letter Portuguese
/// words, initials with a period, and attached cases ("Sim!Muito") are skipped.
fn has_possible_orphan_character(text: &str) -> bool {
    const PORTUGUESE_SINGLE_LETTER_WORDS: &[&str] =
        &["a", "e", "i", "o", "u", "á", "à", "é", "í", "ó", "ú", "ê"];
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
    if PORTUGUESE_SINGLE_LETTER_WORDS.contains(&lower.as_str()) {
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

/// Compute conservative local quality hints for an element's translation.
/// Returns an empty list when the element has no translation or looks clean.
pub(crate) fn quality_hints(_source: &str, translation: Option<&str>) -> Vec<&'static str> {
    let Some(text) = translation else {
        return Vec::new();
    };
    let text = text.trim();
    let mut hints = Vec::new();
    if has_untranslated_cjk(text) {
        hints.push("untranslated_cjk");
    }
    if has_suspicious_mixed_script(text) {
        hints.push("suspicious_mixed_script");
    }
    if has_possible_orphan_character(text) {
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

pub(crate) const BULK_REVIEW_INSTRUCTIONS: &str = r#"You are a specialized bulk translation reviewer for manga pages (source text in Japanese, translation into Portuguese).
You receive minimal textual data: page labels, element IDs, source text, current translations, and optional local quality hints on an element.
Review every provided translation in this order:
1. SOURCE FIDELITY — does the translation carry the source meaning? Check for lost or added information, wrong subject/object, inverted negation, wrong verb tense, and swapped pronouns or relationships. Accuracy failures are the most serious errors to fix.
2. OCR/TEXT NOISE — translation pipelines leave OCR artifacts. Look for isolated single letters after sentence punctuation ("Me dá aquilo! M"), duplicated or cut fragments, misplaced punctuation (doubled marks, commas next to periods), and CJK characters left untranslated in the Portuguese text. Do not preserve a character just because it exists. Do NOT remove real initials ("K.", "J."), acronyms, units, or onomatopoeia.
3. NAMES/TERMINOLOGY — keep names and terms consistent. The provided translation memory is your canonical glossary: reuse existing keys and spellings and prefer them over new variants. If two spellings of the same name appear in one batch, normalize them to one. Add durable decisions (names, pronouns, relationships, tone) to "memory_updates" only when they are genuinely useful for later batches.
4. NATURAL PORTUGUESE — only after steps 1–3, fix flow, register, and readability. Do not restyle translations that are already accurate.
Local "hints" are conservative advisory flags for possible OCR noise or untranslated text. Verify each one against the source before acting; some hints are false positives and identical text is not necessarily an error.
Output ONLY a structured JSON response — no conversational text, no explanations outside the response object.
In "changes" include ONLY elements that need updating. If a translation is already correct, do NOT echo it.
In "memory_updates" provide incremental additions or replacements for terminology, names, pronouns, relationships, tone decisions — not a full memory dump."#;

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
                        "translation": { "type": "string" }
                    },
                    "required": ["element", "translation"],
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
        "required": ["changes", "memory_updates"],
        "additionalProperties": false
    })
}

/// Estimate payload size for text-based batch planning.
pub(crate) fn estimate_text_size(summary: &PageTextSummary) -> usize {
    summary.source_chars
        + summary.translation_chars
        + summary.element_count * ESTIMATED_JSON_OVERHEAD_PER_ELEMENT
}

#[derive(Clone, Debug)]
pub(crate) struct PageTextSummary {
    pub(crate) element_count: usize,
    pub(crate) source_chars: usize,
    pub(crate) translation_chars: usize,
}

/// Extract measured page sizes from a `bulk_review_pages` result.
pub(crate) fn summaries_from_bulk(data: &Value) -> Result<HashMap<String, PageTextSummary>> {
    let pages = data
        .get("pages")
        .and_then(Value::as_array)
        .context("bulk review data is missing its pages array")?;
    let mut summaries = HashMap::new();
    for page in pages {
        let id = page
            .get("id")
            .and_then(Value::as_str)
            .context("bulk review page is missing its id")?
            .to_owned();
        let mut element_count = 0_usize;
        let mut source_chars = 0_usize;
        let mut translation_chars = 0_usize;
        for element in page
            .get("elements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            element_count = element_count.saturating_add(1);
            source_chars = source_chars.saturating_add(
                element
                    .get("source")
                    .and_then(Value::as_str)
                    .map_or(0, |text| text.chars().count()),
            );
            translation_chars = translation_chars.saturating_add(
                element
                    .get("translation")
                    .and_then(Value::as_str)
                    .map_or(0, |text| text.chars().count()),
            );
        }
        summaries.insert(
            id,
            PageTextSummary {
                element_count,
                source_chars,
                translation_chars,
            },
        );
    }
    Ok(summaries)
}

/// Text-based batch planner — groups pages to stay under the input budget while
/// preserving order and removing duplicate IDs.
pub(crate) fn plan_text_batches(
    pages: Vec<String>,
    summaries: HashMap<String, PageTextSummary>,
) -> Vec<Vec<String>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_size = 0_usize;
    let mut seen = HashSet::new();

    for page in pages {
        if !seen.insert(page.clone()) {
            continue;
        }
        let summary = summaries.get(&page);
        let page_size = summary.map_or(1000, estimate_text_size);

        if !current_batch.is_empty()
            && current_size.saturating_add(page_size) > BULK_INPUT_BUDGET_CHARS
        {
            batches.push(std::mem::take(&mut current_batch));
            current_size = 0;
        }

        current_batch.push(page);
        current_size = current_size.saturating_add(page_size);

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

/// Validate a full structured response before applying anything. If any change
/// is invalid, zero mutations are applied.
pub(crate) fn validate_review_response(
    response: &ReviewResponse,
    target_pages: &HashSet<String>,
    element_to_page: &HashMap<String, String>,
) -> Result<()> {
    if response.changes.len() > element_to_page.len() {
        bail!("more changes than available elements");
    }
    let mut seen = HashSet::new();
    for change in &response.changes {
        if change.element.trim().is_empty() {
            bail!("change element IDs cannot be empty");
        }
        let page = element_to_page
            .get(&change.element)
            .with_context(|| format!("unknown element ID: {}", change.element))?;
        if !target_pages.contains(page) {
            bail!(
                "element {} belongs to page {}, which is not in the target batch",
                change.element,
                page
            );
        }
        if !seen.insert(change.element.clone()) {
            bail!("duplicate change for element {}", change.element);
        }
        let translation_chars = change.translation.chars().count();
        if translation_chars == 0 || change.translation.trim().is_empty() {
            bail!("translation for element {} cannot be empty", change.element);
        }
        if translation_chars > MAX_CHANGE_CHARS {
            bail!(
                "translation for element {} exceeds {} characters",
                change.element,
                MAX_CHANGE_CHARS
            );
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
    Ok(())
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

/// Detect context-window failures so a batch can be split instead of aborted.
pub(crate) fn is_context_error(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_lowercase();
    message.contains("context") || message.contains("token") || message.contains("too long")
}

/// Arguments for the `process_pages_job` tool.
#[derive(Deserialize)]
pub(crate) struct PageJob {
    pub(crate) pages: Vec<String>,
    pub(crate) instruction: String,
}

pub(crate) fn page_job_tool() -> Tool {
    Tool::new(
        "process_pages_job",
        "Process a large ordered set of pages autonomously in small sequential batches. Use for tasks spanning more than five pages; the user does not need to request each batch.",
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

/// Deterministic, mockable V3 orchestrator. Each successful batch is exactly
/// one model request followed by local validation and a single bulk apply.
pub(crate) struct BulkReviewer<'a> {
    client: &'a dyn ReviewClient,
    host: &'a dyn Host,
}

impl<'a> BulkReviewer<'a> {
    pub(crate) fn new(client: &'a dyn ReviewClient, host: &'a dyn Host) -> Self {
        Self { client, host }
    }

    pub(crate) async fn run_job<F>(
        &self,
        run: RunId,
        call_id: &str,
        arguments: &str,
        model: &CodexModel,
        control: &Control,
        publish: &mut F,
    ) -> Result<Invocation>
    where
        F: FnMut(Event),
    {
        control.ensure_running()?;
        let job: PageJob =
            serde_json::from_str(arguments).context("invalid arguments for process_pages_job")?;
        anyhow::ensure!(
            !job.pages.is_empty(),
            "page job requires at least one page ID"
        );

        // Measure the real page sizes locally for planning — never in the model loop.
        let pages_data = self
            .host
            .bulk_review_pages(&job.pages)
            .await
            .context("failed to read project pages for review while planning batches")?;
        let summaries = summaries_from_bulk(&pages_data)?;
        for page in &job.pages {
            if !summaries.contains_key(page) {
                bail!("page job contains unknown page ID {page}");
            }
        }

        let queue = plan_text_batches(job.pages, summaries);
        let total = queue.iter().map(Vec::len).sum::<usize>();
        let mut queue = queue.into_iter().collect::<VecDeque<_>>();
        let mut completed = Vec::with_capacity(total);
        let mut failed = Vec::new();
        let mut memory = TranslationMemory::new().bounded(MAX_MEMORY_BYTES);
        let mut changed = false;
        let mut metrics = JobMetrics::default();
        let mut sequence = 0_usize;
        // Stable per-job cache key: the instructions and schema prefix are
        // identical across batches, only the pages differ.
        let session = format!("bulk-review:{call_id}");

        while let Some(batch) = queue.pop_front() {
            control.ensure_running()?;
            sequence += 1;
            let batch_call_id = format!("{call_id}-batch-{sequence}");
            publish(Event::ToolStarted {
                run,
                call_id: batch_call_id.clone(),
                name: "process_page_batch".to_owned(),
            });
            match self
                .run_review_batch(
                    &session,
                    &job.instruction,
                    &batch,
                    &memory,
                    model,
                    control,
                    sequence,
                )
                .await
            {
                Ok(result) => {
                    changed |= result.changed;
                    memory = result.memory;
                    completed.extend(batch.iter().cloned());
                    metrics.add_batch(&result.metrics);
                    publish(Event::ToolFinished {
                        run,
                        call_id: batch_call_id,
                        name: "process_page_batch".to_owned(),
                        changed: result.changed,
                        output: json!({
                            "pages": batch,
                            "summary": result.metrics.log_summary(),
                            "completed": completed.len(),
                            "total": total,
                        })
                        .to_string(),
                    });
                }
                Err(error) if error.context_limit && batch.len() > 1 => {
                    // Split a too-large batch in half and reprocess both halves
                    // with the same V3 worker. A single page is terminal.
                    let middle = batch.len().div_ceil(2);
                    let left = batch[..middle].to_vec();
                    let right = batch[middle..].to_vec();
                    queue.push_front(right);
                    queue.push_front(left);
                    publish(Event::ToolFinished {
                        run,
                        call_id: batch_call_id,
                        name: "process_page_batch".to_owned(),
                        changed: false,
                        output: json!({ "pages": batch, "status": "split" }).to_string(),
                    });
                }
                Err(error) => {
                    failed.extend(
                        batch
                            .iter()
                            .map(|page| json!({ "page": page, "error": format!("{error:#}") })),
                    );
                    publish(Event::ToolFinished {
                        run,
                        call_id: batch_call_id,
                        name: "process_page_batch".to_owned(),
                        changed: false,
                        output: json!({
                            "pages": batch,
                            "status": "failed",
                            "completed": completed.len(),
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

        metrics.log();
        let value = json!({
            "total": total,
            "completed": completed.len(),
            "completed_pages": completed,
            "failed": failed,
            "memory": memory.serialize(),
            "metrics": json!({
                "batches": metrics.total_batches,
                "changes": metrics.total_changes,
                "hints": metrics.total_hints,
                "model_requests": metrics.total_model_requests,
                "elapsed_ms": metrics.total_elapsed_ms,
            }),
        });
        if changed {
            Ok(Invocation::changed(value)?)
        } else {
            Ok(Invocation::read(value)?)
        }
    }

    /// One model request, local validation, local bulk apply.
    #[allow(clippy::too_many_arguments)]
    async fn run_review_batch(
        &self,
        session: &str,
        instruction: &str,
        batch: &[String],
        memory: &TranslationMemory,
        model: &CodexModel,
        control: &Control,
        batch_index: usize,
    ) -> std::result::Result<ReviewBatchResult, ReviewBatchError> {
        let started = Instant::now();
        control.ensure_running().map_err(ReviewBatchError::from)?;

        let pages_data = self
            .host
            .bulk_review_pages(batch)
            .await
            .context(format!(
                "failed to read project pages for review (batch {batch_index})"
            ))
            .map_err(ReviewBatchError::from)?;
        let pages_array = pages_data
            .get("pages")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ReviewBatchError::from(anyhow!(
                    "bulk_review_pages returned invalid shape (batch {batch_index})"
                ))
            })?;

        let mut element_to_page = HashMap::new();
        let mut target_pages = HashSet::new();
        let mut review_pages = Vec::with_capacity(pages_array.len());
        let mut element_count = 0_usize;
        let mut hints_count = 0_usize;
        let mut source_chars = 0_usize;
        let mut translation_chars = 0_usize;

        for page in pages_array {
            let page_id = page
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ReviewBatchError::from(anyhow!("page missing id in bulk review data"))
                })?
                .to_owned();
            let label = page
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            target_pages.insert(page_id.clone());
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
                let translation = element
                    .get("translation")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                source_chars = source_chars.saturating_add(source.chars().count());
                translation_chars = translation_chars.saturating_add(
                    translation
                        .as_deref()
                        .map_or(0, |text| text.chars().count()),
                );
                element_count = element_count.saturating_add(1);
                element_to_page.insert(id.clone(), page_id.clone());
                let hints = quality_hints(&source, translation.as_deref());
                hints_count = hints_count.saturating_add(hints.len());
                elements.push(ReviewElement {
                    id,
                    order,
                    source,
                    translation,
                    quality_hints: hints,
                });
            }
            review_pages.push(ReviewPage {
                id: page_id,
                label,
                elements,
            });
        }

        // The pages themselves are the payload: the model must see every element's
        // source and current translation, not just the page IDs.
        let pages_json = serde_json::to_string(&review_pages)
            .map_err(|error| ReviewBatchError::from(anyhow!(error)))?;
        let payload_bytes = pages_json.len();
        let memory_text = memory.serialize();
        let prompt = format!(
            "Objective: {instruction}\nPages in this batch: {}\nTranslation memory (incremental updates only, do not repeat):\n{}\nReview the pages below and return ONLY the structured JSON response.\n\n{pages_json}",
            batch.join(", "),
            if memory_text.is_empty() {
                "(none)"
            } else {
                &memory_text
            },
        );
        let input = vec![message("user", prompt)];
        let request = Request::new(
            model.id.clone(),
            BULK_REVIEW_INSTRUCTIONS.to_owned(),
            input,
            Vec::new(),
            lowest_reasoning(model),
            session.to_owned(),
        )
        .with_json_schema(Some(review_response_schema()), "review_response");

        let mut attempts = 0_usize;
        let turn = loop {
            attempts = attempts.saturating_add(1);
            control.ensure_running().map_err(ReviewBatchError::from)?;
            match self.client.respond_once(&request, control).await {
                Ok(turn) => break turn,
                Err(error) if should_retry(attempts - 1, is_transient_error(&error)) => {
                    retry_wait(backoff(attempts - 1), control)
                        .await
                        .map_err(ReviewBatchError::from)?;
                }
                Err(source) => return Err(ReviewBatchError::from(source)),
            }
        };

        let response_bytes = turn.text.len();
        let response_json: ReviewResponse = serde_json::from_str(&turn.text)
            .with_context(|| format!("invalid structured review response (batch {batch_index})"))
            .map_err(ReviewBatchError::from)?;
        validate_review_response(&response_json, &target_pages, &element_to_page)
            .map_err(ReviewBatchError::from)?;

        let mut changed = false;
        let mut applied = 0_usize;
        if !response_json.changes.is_empty() {
            let updates = response_json
                .changes
                .iter()
                .map(|change| json!({ "element": change.element, "text": change.translation }))
                .collect::<Vec<_>>();
            let invocation = self
                .host
                .invoke(
                    ToolCall {
                        call_id: "bulk-apply".to_owned(),
                        name: "set_translations".to_owned(),
                        arguments: serde_json::to_string(&json!({ "updates": updates }))
                            .unwrap_or_default(),
                    },
                    control,
                )
                .await
                .map_err(ReviewBatchError::from)?;
            changed = invocation.changed;
            applied = updates.len();
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
            source_chars,
            translation_chars,
            payload_bytes,
            model_requests: attempts,
            response_bytes,
            changes_applied: applied,
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
        })
    }
}

struct ReviewBatchResult {
    changed: bool,
    memory: TranslationMemory,
    metrics: BatchMetrics,
}

#[derive(Debug)]
struct ReviewBatchError {
    source: anyhow::Error,
    context_limit: bool,
}

impl From<anyhow::Error> for ReviewBatchError {
    fn from(source: anyhow::Error) -> Self {
        let context_limit = is_context_error(&source);
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
    pub(crate) source_chars: usize,
    pub(crate) translation_chars: usize,
    pub(crate) payload_bytes: usize,
    pub(crate) model_requests: usize,
    pub(crate) response_bytes: usize,
    pub(crate) changes_applied: usize,
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
            source_chars = self.source_chars,
            translation_chars = self.translation_chars,
            payload_bytes = self.payload_bytes,
            model_requests = self.model_requests,
            response_bytes = self.response_bytes,
            changes = self.changes_applied,
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
            "batch {}: {} pages, {} elements, {} changes, {} hints, {} request(s), {} ms",
            self.batch_index,
            self.page_count,
            self.element_count,
            self.changes_applied,
            self.hints_count,
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
    pub(crate) total_changes: usize,
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
        self.total_changes = self.total_changes.saturating_add(batch.changes_applied);
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
            changes = self.total_changes,
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
    use serde_json::json;

    use crate::{
        Invocation,
        codex::{Turn, message},
    };

    use super::*;

    fn empty_turn() -> Turn {
        Turn {
            output: Vec::new(),
            calls: Vec::new(),
            text: r#"{"changes":[],"memory_updates":[]}"#.to_owned(),
            usage: None,
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
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                script: Mutex::new(VecDeque::new()),
                prompts: Mutex::new(Vec::new()),
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
            let mut script = self.script.lock().unwrap();
            if let Some(result) = script.pop_front() {
                return result;
            }
            Ok(empty_turn())
        }
    }

    #[derive(Default)]
    struct MockHost {
        pages: HashMap<String, Vec<Value>>,
        applied: Mutex<Vec<(String, Option<String>)>>,
    }

    impl MockHost {
        fn add_element(
            &mut self,
            page_id: &str,
            element_id: &str,
            source: &str,
            translation: Option<&str>,
        ) {
            self.pages
                .entry(page_id.to_owned())
                .or_default()
                .push(json!({
                    "id": element_id,
                    "source": source,
                    "translation": translation,
                }));
        }

        fn add_page(&mut self, id: &str, source_chars: usize) {
            self.add_element(id, &format!("{id}e1"), &"x".repeat(source_chars), None);
        }

        fn applied(&self) -> Vec<(String, Option<String>)> {
            self.applied.lock().unwrap().clone()
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

        async fn invoke(&self, call: ToolCall, _control: &Control) -> Result<Invocation> {
            if call.name != "set_translations" {
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

    async fn run_job(
        client: &MockClient,
        host: &MockHost,
        pages: &[&str],
    ) -> std::result::Result<Invocation, anyhow::Error> {
        let control = Control::default();
        let reviewer = BulkReviewer::new(client, host);
        reviewer
            .run_job(
                RunId::default(),
                "job-1",
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
        let summaries = HashMap::from([
            ("p1".to_owned(), small.clone()),
            ("p2".to_owned(), small),
            ("p3".to_owned(), big),
        ]);
        let batches = plan_text_batches(
            vec!["p1".to_owned(), "p2".to_owned(), "p3".to_owned()],
            summaries,
        );
        // p1+p2 batch together (~3.5k), p3 alone (~41k, exceeds the 20k budget).
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], vec!["p1", "p2"]);
        assert_eq!(batches[1], vec!["p3"]);
    }

    #[test]
    fn text_planner_removes_duplicates_and_preserves_order() {
        let summaries = HashMap::from([(
            "p1".to_owned(),
            PageTextSummary {
                element_count: 5,
                source_chars: 100,
                translation_chars: 120,
            },
        )]);
        let batches = plan_text_batches(
            vec![
                "p1".to_owned(),
                "p1".to_owned(),
                "p2".to_owned(),
                "p1".to_owned(),
            ],
            summaries,
        );
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0], vec!["p1", "p2"]);
    }

    #[test]
    fn summaries_from_bulk_measure_real_text() {
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
        let summaries = summaries_from_bulk(&data).unwrap();
        let p1 = &summaries["p1"];
        assert_eq!(p1.element_count, 2);
        assert_eq!(p1.source_chars, 4);
        assert_eq!(p1.translation_chars, 8);
        assert_eq!(summaries["p2"].element_count, 0);
    }

    #[test]
    fn validates_response_structure() {
        let response = ReviewResponse {
            changes: vec![
                TranslationChange {
                    element: "e1".to_owned(),
                    translation: "revised".to_owned(),
                },
                TranslationChange {
                    element: "e2".to_owned(),
                    translation: "fixed".to_owned(),
                },
            ],
            memory_updates: vec![],
        };
        let target_pages = ["p1".to_owned()].into_iter().collect();
        let element_to_page = HashMap::from([
            ("e1".to_owned(), "p1".to_owned()),
            ("e2".to_owned(), "p1".to_owned()),
        ]);
        assert!(validate_review_response(&response, &target_pages, &element_to_page).is_ok());
    }

    #[test]
    fn rejects_element_outside_batch() {
        let response = ReviewResponse {
            changes: vec![TranslationChange {
                element: "e_other".to_owned(),
                translation: "bad".to_owned(),
            }],
            memory_updates: vec![],
        };
        let target_pages = ["p1".to_owned()].into_iter().collect();
        let element_to_page = HashMap::from([("e_other".to_owned(), "p99".to_owned())]);
        assert!(validate_review_response(&response, &target_pages, &element_to_page).is_err());
    }

    #[test]
    fn rejects_unknown_or_duplicate_or_empty_changes() {
        let target_pages = ["p1".to_owned()].into_iter().collect();
        let element_to_page = HashMap::from([("e1".to_owned(), "p1".to_owned())]);
        let unknown = ReviewResponse {
            changes: vec![TranslationChange {
                element: "ghost".to_owned(),
                translation: "x".to_owned(),
            }],
            memory_updates: vec![],
        };
        assert!(validate_review_response(&unknown, &target_pages, &element_to_page).is_err());
        let duplicate = ReviewResponse {
            changes: vec![
                TranslationChange {
                    element: "e1".to_owned(),
                    translation: "first".to_owned(),
                },
                TranslationChange {
                    element: "e1".to_owned(),
                    translation: "second".to_owned(),
                },
            ],
            memory_updates: vec![],
        };
        assert!(validate_review_response(&duplicate, &target_pages, &element_to_page).is_err());
        let empty = ReviewResponse {
            changes: vec![TranslationChange {
                element: "e1".to_owned(),
                translation: "   ".to_owned(),
            }],
            memory_updates: vec![],
        };
        assert!(validate_review_response(&empty, &target_pages, &element_to_page).is_err());
    }

    #[test]
    fn rejects_excessive_changes_and_bad_memory() {
        let target_pages = ["p1".to_owned()].into_iter().collect();
        let element_to_page = HashMap::from([("e1".to_owned(), "p1".to_owned())]);
        let too_many = ReviewResponse {
            changes: vec![
                TranslationChange {
                    element: "e1".to_owned(),
                    translation: "a".to_owned(),
                },
                TranslationChange {
                    element: "e2".to_owned(),
                    translation: "b".to_owned(),
                },
            ],
            memory_updates: vec![],
        };
        assert!(validate_review_response(&too_many, &target_pages, &element_to_page).is_err());
        let bad_memory = ReviewResponse {
            changes: vec![],
            memory_updates: vec![MemoryUpdate {
                key: "k".to_owned(),
                value: "".to_owned(),
            }],
        };
        assert!(validate_review_response(&bad_memory, &target_pages, &element_to_page).is_err());
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
                    translation: Some("Olá".to_owned()),
                    quality_hints: Vec::new(),
                },
                ReviewElement {
                    id: "e2".to_owned(),
                    order: 1,
                    source: "World".to_owned(),
                    translation: None,
                    quality_hints: Vec::new(),
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
        ] {
            assert!(
                !body.contains(forbidden),
                "payload must not contain {forbidden}"
            );
        }
        for required in ["Hello", "Olá", "World", "\"order\":0", "\"order\":1"] {
            assert!(body.contains(required), "payload must contain {required}");
        }
    }

    #[test]
    fn quality_hints_flag_ocr_and_cjk_noise() {
        let orphan = quality_hints("src", Some("Me dá aquilo! M"));
        assert!(orphan.contains(&"possible_orphan_character"));
        assert_eq!(orphan.len(), 1);

        let cjk = quality_hints("src", Some("Olá, 世界!"));
        assert!(cjk.contains(&"untranslated_cjk"));
        assert!(!cjk.contains(&"possible_orphan_character"));

        let mixed = quality_hints("src", Some("Brasil東京"));
        assert!(mixed.contains(&"untranslated_cjk"));
        assert!(mixed.contains(&"suspicious_mixed_script"));

        let repeated = quality_hints("src", Some("Vamos ver vamos ver."));
        assert!(repeated.contains(&"repeated_fragment"));

        assert!(quality_hints("src", Some("Oi!!")).contains(&"suspicious_punctuation"));
        assert!(quality_hints("src", Some("sério,,")).contains(&"suspicious_punctuation"));
    }

    #[test]
    fn quality_hints_do_not_flag_legitimate_portuguese() {
        assert!(quality_hints("src", Some("Este texto está correto.")).is_empty());
        assert!(quality_hints("src", Some("Olha essa cena! É")).is_empty());
        assert!(quality_hints("src", Some("Permaneceu calado. Sim!Muito calado")).is_empty());
        assert!(quality_hints("src", Some("Foi ontem à noite! K.")).is_empty());
        assert!(quality_hints("src", Some("— J. saiu correndo.")).is_empty());
        assert!(quality_hints("src", Some("Muito muito legal.")).is_empty());
        assert!(quality_hints("src", Some("Cer...? E depois!")).is_empty());
        assert!(quality_hints("src", None).is_empty());
        assert!(quality_hints("src", Some("")).is_empty());
    }

    #[test]
    fn quality_hints_skip_attached_cases_and_single_cjk() {
        let attached = quality_hints("src", Some("Sim!Muito"));
        assert!(attached.is_empty());

        let cjk_end = quality_hints("src", Some("Vamos! 死"));
        assert!(cjk_end.contains(&"untranslated_cjk"));
        assert!(!cjk_end.contains(&"possible_orphan_character"));

        let ellipsis = quality_hints("src", Some("Ela disse... né!"));
        assert!(!ellipsis.contains(&"suspicious_punctuation"));

        let interjection = quality_hints("src", Some("Foi mesmo!?"));
        assert!(!interjection.contains(&"suspicious_punctuation"));
    }

    #[test]
    fn quality_hints_serialize_only_when_present() {
        let plain = ReviewElement {
            id: "e1".to_owned(),
            order: 0,
            source: "Hello".to_owned(),
            translation: Some("Olá".to_owned()),
            quality_hints: Vec::new(),
        };
        let plain_body = serde_json::to_string(&plain).unwrap();
        assert!(!plain_body.contains("quality_hints"));

        let hinted = ReviewElement {
            id: "e2".to_owned(),
            order: 1,
            source: "ちょっと待って".to_owned(),
            translation: Some("Espera aí! M".to_owned()),
            quality_hints: quality_hints("ちょっと待って", Some("Espera aí! M")),
        };
        let hinted_body = serde_json::to_string(&hinted).unwrap();
        assert!(hinted_body.contains("quality_hints"));
        assert!(hinted_body.contains("possible_orphan_character"));
    }

    #[tokio::test]
    async fn payload_embeds_pages_and_quality_hints() {
        let mut host = MockHost::default();
        host.add_element("p1", "p1e1", "ちょうどいい", Some("Perfeito! M"));
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
        assert_eq!(names, vec!["changes", "memory_updates"]);
        assert_eq!(schema["additionalProperties"], false);
        for property in schema["properties"].as_object().unwrap().values() {
            if let Some(items) = property.get("items") {
                assert_eq!(items["additionalProperties"], false);
            }
        }
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
        client.push(Err(anyhow!(
            "your input exceeds the context window of this model"
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
        client.push(Err(anyhow!("connection reset by peer (os error 10054)")));
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
    async fn invalid_response_applies_nothing() {
        let mut host = MockHost::default();
        host.add_page("p1", 30);
        let client = MockClient::new();
        // References an element that is not part of the batch.
        client.push(client.respond_ok(review_text(&[("ghost", "nope")])));

        let invocation = run_job(&client, &host, &["p1"]).await.unwrap();
        assert!(!invocation.changed);
        assert_eq!(client.calls(), 1);
        assert!(host.applied().is_empty());
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
                RunId::default(),
                "job-1",
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
