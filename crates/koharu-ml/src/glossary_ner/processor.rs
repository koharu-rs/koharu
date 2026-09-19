use std::{ops::Range, path::Path};

use anyhow::{Context, Result, ensure};
use icu_segmenter::{WordSegmenter, options::WordBreakInvariantOptions};
use tokenizers::{AddedToken, EncodeInput, InputSequence, Tokenizer};

use super::model::MAX_SPAN_WIDTH;
use super::{GlossaryEntity, GlossaryEntityKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TextToken {
    pub(super) start: usize,
    pub(super) end: usize,
}

/// Upstream GLiNER uses a Unicode-aware whitespace regex. ICU word boundaries
/// are an intentional multilingual divergence: they retain that behavior for
/// spaced scripts and provide usable word units for Japanese and Chinese OCR.
pub(super) fn split_text(text: &str) -> Vec<TextToken> {
    let segmenter = WordSegmenter::new_auto(WordBreakInvariantOptions::default());
    let boundaries = segmenter.segment_str(text).collect::<Vec<_>>();
    let mut tokens: Vec<TextToken> = Vec::new();
    for bounds in boundaries.windows(2) {
        let (start, end) = (bounds[0], bounds[1]);
        let surface = &text[start..end];
        if surface.chars().all(char::is_whitespace) {
            continue;
        }
        if surface.chars().all(is_han)
            && tokens.last().is_some_and(|previous| {
                previous.end == start && text[previous.start..previous.end].chars().all(is_han)
            })
        {
            tokens.last_mut().unwrap().end = end;
        } else {
            tokens.push(TextToken { start, end });
        }
    }
    tokens
}

fn is_han(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{2fa1f}'
    )
}

pub(super) fn token_windows(
    token_count: usize,
    max_tokens: usize,
    overlap: usize,
) -> Result<Vec<Range<usize>>> {
    ensure!(max_tokens > 0, "glossary NER window size must be positive");
    ensure!(
        overlap < max_tokens,
        "glossary NER window overlap must be smaller than the window size"
    );
    if token_count == 0 {
        return Ok(Vec::new());
    }

    let stride = max_tokens - overlap;
    let mut windows = Vec::new();
    let mut start = 0;
    loop {
        let end = (start + max_tokens).min(token_count);
        windows.push(start..end);
        if end == token_count {
            break;
        }
        start += stride;
    }
    Ok(windows)
}

pub(super) fn resolve_overlaps(spans: &mut Vec<GlossaryEntity>) {
    // Upstream `greedy_search` ranks confidence first and preserves tensor
    // traversal order for ties. Spell out that traversal order so merging
    // overlapping windows remains deterministic across Rust versions.
    spans.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| left.end.cmp(&right.end))
            .then_with(|| left.kind.cmp(&right.kind))
    });

    let mut selected: Vec<GlossaryEntity> = Vec::with_capacity(spans.len());
    for candidate in spans.drain(..) {
        if selected
            .iter()
            .all(|span| candidate.end <= span.start || span.end <= candidate.start)
        {
            selected.push(candidate);
        }
    }
    selected.sort_by_key(|span| (span.start, span.end, span.kind));
    selected.dedup_by(|left, right| {
        left.start == right.start && left.end == right.end && left.kind == right.kind
    });
    *spans = selected;
}

#[derive(Debug)]
pub(super) struct EncodedWindow {
    pub(super) input_ids: Vec<i64>,
    pub(super) first_subtokens: Vec<usize>,
    pub(super) prompt_words: usize,
}

#[derive(Debug)]
pub(super) struct Processor {
    tokenizer: Tokenizer,
}

impl Processor {
    pub(super) fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut tokenizer = Tokenizer::from_file(path)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .with_context(|| format!("failed to parse {}", path.display()))?;
        tokenizer
            .add_tokens([
                AddedToken::from("<<ENT>>", false),
                AddedToken::from("<<SEP>>", false),
            ])
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        ensure!(
            tokenizer.token_to_id("<<ENT>>") == Some(250_102)
                && tokenizer.token_to_id("<<SEP>>") == Some(250_103),
            "GLiNER tokenizer special token IDs do not match the checkpoint"
        );
        Ok(Self { tokenizer })
    }

    pub(super) fn encode(&self, text: &str, tokens: &[TextToken]) -> Result<EncodedWindow> {
        let mut words = Vec::with_capacity(GlossaryEntityKind::ALL.len() * 2 + 1 + tokens.len());
        for kind in GlossaryEntityKind::ALL {
            words.push("<<ENT>>");
            words.push(kind.model_label());
        }
        words.push("<<SEP>>");
        let prompt_words = words.len();
        words.extend(tokens.iter().map(|token| &text[token.start..token.end]));
        let input = EncodeInput::Single(InputSequence::PreTokenized(words.into()));
        let encoding = self
            .tokenizer
            .encode(input, true)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut first_subtokens = vec![None; prompt_words + tokens.len()];
        for (subtoken, word) in encoding.get_word_ids().iter().enumerate() {
            if let Some(word) = word {
                first_subtokens[*word as usize].get_or_insert(subtoken);
            }
        }
        let first_subtokens = first_subtokens
            .into_iter()
            .enumerate()
            .map(|(word, subtoken)| {
                subtoken.ok_or_else(|| anyhow::anyhow!("GLiNER tokenizer omitted word {word}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(EncodedWindow {
            input_ids: encoding.get_ids().iter().map(|&id| i64::from(id)).collect(),
            first_subtokens,
            prompt_words,
        })
    }
}

pub(super) fn decode_scores(
    text: &str,
    tokens: &[TextToken],
    probabilities: &[f32],
    threshold: f32,
) -> Result<Vec<GlossaryEntity>> {
    ensure!(
        threshold.is_finite() && (0.0..=1.0).contains(&threshold),
        "glossary NER confidence threshold must be between 0 and 1"
    );
    let class_count = GlossaryEntityKind::ALL.len();
    let expected_spans = (0..MAX_SPAN_WIDTH.min(tokens.len()))
        .map(|width| tokens.len() - width)
        .sum::<usize>();
    ensure!(
        probabilities.len() == expected_spans * class_count,
        "GLiNER returned {} scores for {expected_spans} spans and {class_count} labels",
        probabilities.len()
    );
    let mut entities = Vec::new();
    let mut offset = 0;
    for width in 0..MAX_SPAN_WIDTH.min(tokens.len()) {
        for start in 0..tokens.len() - width {
            let end = start + width;
            for (class, kind) in GlossaryEntityKind::ALL.into_iter().enumerate() {
                let confidence = probabilities[offset + class];
                if confidence >= threshold {
                    let byte_start = tokens[start].start;
                    let byte_end = tokens[end].end;
                    entities.push(GlossaryEntity::new(
                        byte_start,
                        byte_end,
                        text[byte_start..byte_end].to_owned(),
                        kind,
                        confidence,
                    ));
                }
            }
            offset += class_count;
        }
    }
    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary_ner::{GlossaryEntity, GlossaryEntityKind};

    #[test]
    fn tokenization_preserves_multibyte_byte_ranges() {
        let text = "蒼井レンは東京・新宿へ行く。";
        let tokens = split_text(text);
        assert_eq!(
            tokens
                .iter()
                .map(|token| (&text[token.start..token.end], token.start, token.end))
                .collect::<Vec<_>>(),
            vec![
                ("蒼井", 0, 6),
                ("レン", 6, 12),
                ("は", 12, 15),
                ("東京", 15, 21),
                ("・", 21, 24),
                ("新宿", 24, 30),
                ("へ", 30, 33),
                ("行く", 33, 39),
                ("。", 39, 42),
            ]
        );
    }

    #[test]
    fn windows_overlap_and_cover_all_tokens() {
        let windows = token_windows(10, 4, 2).unwrap();
        assert_eq!(windows, vec![0..4, 2..6, 4..8, 6..10]);
    }

    #[test]
    fn score_layout_maps_spans_to_utf8_offsets() {
        let text = "蒼井レン";
        let tokens = split_text(text);
        let classes = GlossaryEntityKind::ALL.len();
        let mut probabilities = vec![0.0; (2 + 1) * classes];
        probabilities[GlossaryEntityKind::Person as usize] = 0.7;
        probabilities[2 * classes + GlossaryEntityKind::Person as usize] = 0.9;
        let entities = decode_scores(text, &tokens, &probabilities, 0.5).unwrap();
        assert_eq!(
            entities,
            vec![
                GlossaryEntity::new(0, 6, "蒼井".into(), GlossaryEntityKind::Person, 0.7),
                GlossaryEntity::new(0, 12, "蒼井レン".into(), GlossaryEntityKind::Person, 0.9),
            ]
        );
    }

    #[test]
    fn overlapping_spans_resolve_deterministically() {
        let mut spans = vec![
            GlossaryEntity::new(0, 6, "蒼井".into(), GlossaryEntityKind::Person, 0.8),
            GlossaryEntity::new(0, 12, "蒼井レン".into(), GlossaryEntityKind::Person, 0.8),
            GlossaryEntity::new(6, 12, "レン".into(), GlossaryEntityKind::Person, 0.9),
            GlossaryEntity::new(15, 21, "東京".into(), GlossaryEntityKind::Place, 0.7),
            GlossaryEntity::new(15, 21, "東京".into(), GlossaryEntityKind::Place, 0.7),
        ];
        resolve_overlaps(&mut spans);
        assert_eq!(
            spans,
            vec![
                GlossaryEntity::new(0, 6, "蒼井".into(), GlossaryEntityKind::Person, 0.8,),
                GlossaryEntity::new(6, 12, "レン".into(), GlossaryEntityKind::Person, 0.9,),
                GlossaryEntity::new(15, 21, "東京".into(), GlossaryEntityKind::Place, 0.7,),
            ]
        );
    }
}
