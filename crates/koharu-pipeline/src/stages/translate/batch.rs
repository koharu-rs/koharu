//! Model-aware token budgets and context formatting for multi-page
//! translation (combined JSON batches when text-only, and marked neighboring
//! dialogue when Vision is active on general models).

use std::ops::Range;

use koharu_llm::{
    TRANSLATION,
    rig::{completion::Reasoning, driver::DynModel, operation::Completion},
};

/// Upper ceiling for Auto mode on general LLMs so a single combined JSON call
/// stays reliable even on 1M-token models.
const GENERAL_AUTO_MAX_SOURCE_TOKENS: u64 = 6_000;
const GENERAL_USER_MAX_SOURCE_TOKENS: u64 = 12_000;
const GENERAL_AUTO_MAX_PAGES: usize = 16;
const GENERAL_UNKNOWN_LIMIT_SOURCE_TOKENS: u64 = 2_000;

/// Conservative Auto ceiling for specialized `TRANSLATION` models (such as
/// `Index-Translate 2B/9B/35B`), trained on numbered JSON objects.
const TRANSLATION_AUTO_MAX_SOURCE_TOKENS: u64 = 1_500;
const TRANSLATION_AUTO_MAX_PAGES: usize = 4;
const TRANSLATION_UNKNOWN_LIMIT_SOURCE_TOKENS: u64 = 1_200;

/// Read-only neighbor context ceilings for Vision per-page translation on
/// general models.
const VISION_NEIGHBOR_MAX_TOKENS: u64 = 4_000;
const VISION_NEIGHBOR_UNKNOWN_TOKENS: u64 = 1_500;
const VISION_NEIGHBOR_AUTO_MAX_PAGES: usize = 8;
/// Approximate token budget consumed by a single 1,280 px manga scan.
const IMAGE_TOKENS_ESTIMATE: u64 = 1_200;

/// Per-request packing limits for a combined multi-page JSON batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BatchBudget {
    pub(super) max_source_tokens: u64,
    pub(super) max_pages: usize,
}

/// Whether `model` is a specialized translation-only model (e.g.
/// `Index-Translate`), which uses bare JSON without extra prose wrappers.
pub(super) fn is_translation_only(model: &DynModel<Completion>) -> bool {
    model.spec().and_then(|spec| spec.details.kind.as_deref()) == Some(TRANSLATION)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3000..=0x30FF // CJK punctuation, Hiragana, Katakana
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF // CJK Extension A
            | 0x4E00..=0x9FFF // CJK Unified Ideographs
            | 0xAC00..=0xD7AF // Hangul Syllables
            | 0xF900..=0xFAFF // CJK Compatibility Ideographs
            | 0xFF00..=0xFFEF // Fullwidth forms
    )
}

/// Conservative token estimate for one dialogue string plus JSON key/framing
/// overhead. CJK/Kana/Hangul characters average ~1.5 tokens each, avoiding
/// rig's 4-chars/token undercount on Japanese and Chinese.
pub(super) fn estimate_text_tokens(text: &str) -> u64 {
    let mut cjk = 0u64;
    let mut other = 0u64;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    (cjk * 3).div_ceil(2) + (other * 7).div_ceil(20) + 12
}

pub(super) fn estimate_page_tokens(texts: &[String]) -> u64 {
    texts.iter().map(|text| estimate_text_tokens(text)).sum()
}

/// Computes how many pages and source tokens fit in one combined multi-page
/// JSON translation request for `model`.
pub(super) fn combined_budget(
    model: &DynModel<Completion>,
    user_max_pages: Option<u32>,
    reasoning: Option<Reasoning>,
) -> BatchBudget {
    let translation_only = is_translation_only(model);
    let spec = model.spec();
    let input_limit = spec.and_then(|s| s.input_limit()).map(u64::from);
    let output_limit = spec.and_then(|s| s.output_limit()).map(u64::from);

    let input_budget = match input_limit {
        Some(limit) if translation_only => limit / 12,
        Some(limit) => limit / 6,
        None if translation_only => TRANSLATION_UNKNOWN_LIMIT_SOURCE_TOKENS,
        None => GENERAL_UNKNOWN_LIMIT_SOURCE_TOKENS,
    };

    // Translated JSON typically expands to ~1.5-2x source tokens; reasoning
    // models also emit chain-of-thought tokens inside the output limit.
    let output_factor = if reasoning == Some(Reasoning::Off) || translation_only {
        2
    } else {
        3
    };
    let output_budget = output_limit.map(|limit| (limit / output_factor).max(256));

    let ceiling = if translation_only {
        TRANSLATION_AUTO_MAX_SOURCE_TOKENS
    } else if user_max_pages.is_some() {
        GENERAL_USER_MAX_SOURCE_TOKENS
    } else {
        GENERAL_AUTO_MAX_SOURCE_TOKENS
    };

    let mut max_source_tokens = input_budget.min(ceiling);
    if let Some(out) = output_budget {
        max_source_tokens = max_source_tokens.min(out);
    }
    max_source_tokens = max_source_tokens.max(256);

    let max_pages = match user_max_pages {
        Some(cap) => (cap as usize).max(1),
        None if translation_only => TRANSLATION_AUTO_MAX_PAGES,
        None => GENERAL_AUTO_MAX_PAGES,
    };

    BatchBudget {
        max_source_tokens,
        max_pages,
    }
}

/// Packs consecutive page token costs into contiguous batch ranges respecting
/// `budget`. Every page is guaranteed progress (an oversized single page goes
/// in its own 1-page batch).
pub(super) fn pack_indices(page_tokens: &[u64], budget: BatchBudget) -> Vec<Range<usize>> {
    let mut batches = Vec::new();
    let max_pages = budget.max_pages.max(1);
    let mut start = 0;
    while start < page_tokens.len() {
        let mut end = start + 1;
        let mut used = page_tokens[start];
        while end < page_tokens.len() && (end - start) < max_pages {
            let next = page_tokens[end];
            if used.saturating_add(next) > budget.max_source_tokens {
                break;
            }
            used = used.saturating_add(next);
            end += 1;
        }
        batches.push(start..end);
        start = end;
    }
    batches
}

/// Token and page budget for read-only `<previous-pages>` and
/// `<upcoming-pages>` context during Vision per-page translation. Returns
/// `None` for specialized `TRANSLATION` models whose fine-tuned prompt expects
/// bare JSON only.
pub(super) fn neighbor_budget(
    model: &DynModel<Completion>,
    user_max_pages: Option<u32>,
) -> Option<(u64, usize)> {
    if is_translation_only(model) {
        return None;
    }
    let input_limit = model.spec().and_then(|s| s.input_limit()).map(u64::from);
    let tokens = match input_limit {
        Some(limit) => {
            let after_image = limit.saturating_sub(IMAGE_TOKENS_ESTIMATE);
            (after_image / 8).clamp(256, VISION_NEIGHBOR_MAX_TOKENS)
        }
        None => VISION_NEIGHBOR_UNKNOWN_TOKENS,
    };
    let pages = match user_max_pages {
        Some(cap) => (cap as usize).max(1),
        None => VISION_NEIGHBOR_AUTO_MAX_PAGES,
    };
    Some((tokens, pages))
}

/// Formats a general-model Vision user prompt containing the current page's
/// JSON together with optional read-only previous and upcoming page dialogue.
/// Returns `(body, has_context)`; when neither section has entries, returns
/// `current_json` untouched and `false`.
pub(super) fn format_neighbor_prompt(
    previous: &[(usize, Vec<(String, Option<String>)>)],
    current_json: &str,
    upcoming: &[(usize, Vec<String>)],
) -> (String, bool) {
    if previous.is_empty() && upcoming.is_empty() {
        return (current_json.to_owned(), false);
    }
    let mut sections = Vec::new();
    if !previous.is_empty() {
        let mut lines = vec!["<previous-pages>".to_owned()];
        for (page_number, entries) in previous {
            lines.push(format!("Page {page_number}:"));
            for (source, translated) in entries {
                match translated
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(tr) => lines.push(format!("- {source} => {tr}")),
                    None => lines.push(format!("- {source}")),
                }
            }
        }
        lines.push("</previous-pages>".to_owned());
        sections.push(lines.join("\n"));
    }
    sections.push(format!("<current-page>\n{current_json}\n</current-page>"));
    if !upcoming.is_empty() {
        let mut lines = vec!["<upcoming-pages>".to_owned()];
        for (page_number, entries) in upcoming {
            lines.push(format!("Page {page_number}:"));
            for source in entries {
                lines.push(format!("- {source}"));
            }
        }
        lines.push("</upcoming-pages>".to_owned());
        sections.push(lines.join("\n"));
    }
    (sections.join("\n\n"), true)
}

#[cfg(test)]
mod tests {
    use rig_core::{
        catalog::ModelSpec, providers::registry::ProviderId, test_utils::MockCompletionModel,
    };

    use super::*;

    fn model_with(
        input_limit: Option<u32>,
        output_limit: Option<u32>,
        translation_only: bool,
    ) -> DynModel<Completion> {
        let mut spec = ModelSpec::new(ProviderId::declare("test").unwrap(), "mock");
        if let Some(ctx) = input_limit {
            spec = spec.with_context_window(ctx);
        }
        if let Some(out) = output_limit {
            spec = spec.with_max_output_tokens(out);
        }
        if translation_only {
            spec.details.kind = Some(TRANSLATION.to_owned());
        }
        MockCompletionModel::from_turns([]).priced_as(spec).erase()
    }

    #[test]
    fn budget_scales_with_model_window_and_differentiates_translation_models() {
        // Tight VRAM local general model (4K ctx) -> small source budget (~682 tokens).
        let tight = combined_budget(&model_with(Some(4_096), None, false), None, None);
        assert_eq!(tight.max_source_tokens, 682);
        assert_eq!(tight.max_pages, GENERAL_AUTO_MAX_PAGES);

        // Large hosted general model (200K ctx, 8K output, reasoning default) -> output-capped at 8192/3 = 2730.
        let hosted_small_out =
            combined_budget(&model_with(Some(200_000), Some(8_192), false), None, None);
        assert_eq!(hosted_small_out.max_source_tokens, 2_730);

        // Reasoning off allows 2x output ratio -> 8192/2 = 4096.
        let hosted_no_reason = combined_budget(
            &model_with(Some(200_000), Some(8_192), false),
            None,
            Some(Reasoning::Off),
        );
        assert_eq!(hosted_no_reason.max_source_tokens, 4_096);

        // Specialized Index-Translate (32K ctx) -> tighter Option A ceiling (1500 tokens, 4 pages).
        let index_tr = combined_budget(&model_with(Some(32_768), None, true), None, None);
        assert_eq!(index_tr.max_source_tokens, 1_500);
        assert_eq!(index_tr.max_pages, 4);

        // User cap overrides page count while preserving token guard.
        let capped = combined_budget(
            &model_with(Some(200_000), Some(64_000), false),
            Some(3),
            None,
        );
        assert_eq!(capped.max_pages, 3);
    }

    #[test]
    fn neighbor_budget_is_none_for_specialized_translation_models() {
        assert!(neighbor_budget(&model_with(Some(32_768), None, true), None).is_none());
        let general = neighbor_budget(&model_with(Some(128_000), None, false), None).unwrap();
        assert_eq!(
            general,
            (VISION_NEIGHBOR_MAX_TOKENS, VISION_NEIGHBOR_AUTO_MAX_PAGES)
        );
    }

    #[test]
    fn pack_indices_respects_token_and_page_limits_and_never_stalls_on_huge_page() {
        let budget = BatchBudget {
            max_source_tokens: 1_000,
            max_pages: 3,
        };
        assert_eq!(
            pack_indices(&[300, 300, 300, 300, 1_500, 400, 700], budget),
            vec![0..3, 3..4, 4..5, 5..6, 6..7]
        );
    }

    #[test]
    fn cjk_estimator_counts_japanese_conservatively() {
        let jp = estimate_text_tokens("こんにちは世界");
        let en = estimate_text_tokens("Hello world");
        assert!(jp >= 20, "expected conservative CJK token count, got {jp}");
        assert!(
            jp > en,
            "expected 7 CJK chars ({jp}) > 11 ASCII chars ({en})"
        );
    }

    #[test]
    fn format_neighbor_prompt_marks_previous_current_and_upcoming_sections() {
        let json = "{\n  \"1\": \"元気？\"\n}";
        assert_eq!(
            format_neighbor_prompt(&[], json, &[]),
            (json.to_owned(), false)
        );
        let (body, has_ctx) = format_neighbor_prompt(
            &[(2, vec![("やあ".to_owned(), Some("Hey".to_owned()))])],
            json,
            &[(4, vec!["うん！".to_owned()])],
        );
        assert!(has_ctx);
        assert!(body.contains("<previous-pages>\nPage 2:\n- やあ => Hey\n</previous-pages>"));
        assert!(body.contains(&format!("<current-page>\n{json}\n</current-page>")));
        assert!(body.contains("<upcoming-pages>\nPage 4:\n- うん！\n</upcoming-pages>"));
    }
}
