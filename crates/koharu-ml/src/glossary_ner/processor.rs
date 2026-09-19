use std::{ops::Range, path::Path};

use anyhow::{Context, Result, ensure};
use icu_segmenter::{WordSegmenter, options::WordBreakInvariantOptions};
use tokenizers::{AddedToken, EncodeInput, InputSequence, Tokenizer};

use super::model::MAX_SPAN_WIDTH;
use super::{Cancellation, GlossaryEntity, GlossaryEntityKind};

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
        if !surface.chars().all(char::is_whitespace) {
            tokens.push(TextToken { start, end });
        }
    }
    tokens
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
    pub(super) word_subtokens: Vec<Range<usize>>,
    pub(super) prompt_words: usize,
}

impl EncodedWindow {
    pub(super) fn fits(&self, max_encoder_tokens: usize) -> bool {
        self.input_ids.len() <= max_encoder_tokens
            && self
                .word_subtokens
                .iter()
                .all(|subtokens| subtokens.end <= max_encoder_tokens)
    }
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
            .with_truncation(None)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
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
        let mut word_subtokens: Vec<Option<Range<usize>>> = vec![None; prompt_words + tokens.len()];
        for (subtoken, word) in encoding.get_word_ids().iter().enumerate() {
            if let Some(word) = word {
                let word = *word as usize;
                let range = word_subtokens.get_mut(word).ok_or_else(|| {
                    anyhow::anyhow!("GLiNER tokenizer returned unknown word {word}")
                })?;
                if let Some(range) = range {
                    ensure!(
                        range.end == subtoken,
                        "GLiNER tokenizer returned non-contiguous subtokens for word {word}"
                    );
                    range.end = subtoken + 1;
                } else {
                    *range = Some(subtoken..subtoken + 1);
                }
            }
        }
        let word_subtokens = word_subtokens
            .into_iter()
            .enumerate()
            .map(|(word, subtokens)| {
                subtokens.ok_or_else(|| anyhow::anyhow!("GLiNER tokenizer omitted word {word}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(EncodedWindow {
            input_ids: encoding.get_ids().iter().map(|&id| i64::from(id)).collect(),
            word_subtokens,
            prompt_words,
        })
    }

    pub(super) fn split_oversized_tokens(
        &self,
        text: &str,
        tokens: &[TextToken],
        max_encoder_tokens: usize,
        cancellation: &Cancellation<'_>,
    ) -> Result<Option<Vec<TextToken>>> {
        ensure!(
            max_encoder_tokens > 0,
            "glossary NER encoder capacity must be positive"
        );
        let mut fitted = Vec::with_capacity(tokens.len());
        for token in tokens {
            let mut pending = vec![token.clone()];
            while let Some(candidate) = pending.pop() {
                if cancellation.cancelled() {
                    return Ok(None);
                }
                let encoded = self.encode(text, std::slice::from_ref(&candidate))?;
                if encoded.fits(max_encoder_tokens) {
                    fitted.push(candidate);
                    continue;
                }

                let surface = &text[candidate.start..candidate.end];
                let character_count = surface.chars().count();
                ensure!(
                    character_count > 1,
                    "glossary NER prompt or one text character exceeds the encoder capacity"
                );
                let split = candidate.start
                    + surface
                        .char_indices()
                        .nth(character_count / 2)
                        .map(|(offset, _)| offset)
                        .ok_or_else(|| {
                            anyhow::anyhow!("failed to split oversized glossary token")
                        })?;
                pending.push(TextToken {
                    start: split,
                    end: candidate.end,
                });
                pending.push(TextToken {
                    start: candidate.start,
                    end: split,
                });
            }
        }
        Ok(Some(fitted))
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
    use std::collections::BTreeSet;

    use tokenizers::models::bpe::BPE;

    use super::*;
    use crate::glossary_ner::{GlossaryEntity, GlossaryEntityKind, model::MAX_ENCODER_TOKENS};

    fn character_processor() -> Processor {
        let alphabet = GlossaryEntityKind::ALL
            .into_iter()
            .flat_map(|kind| kind.model_label().chars())
            .chain(['界'])
            .collect::<BTreeSet<_>>();
        let vocabulary = alphabet
            .into_iter()
            .enumerate()
            .map(|(index, character)| (character.to_string(), index as u32))
            .collect();
        let mut tokenizer = Tokenizer::new(BPE::new(vocabulary, Vec::new()));
        tokenizer
            .add_tokens([
                AddedToken::from("<<ENT>>", false),
                AddedToken::from("<<SEP>>", false),
            ])
            .unwrap();
        Processor { tokenizer }
    }

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
                ("蒼", 0, 3),
                ("井", 3, 6),
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
    fn adjacent_han_word_span_can_cover_only_part_of_run() {
        let text = "東京大学教授";
        let tokens = split_text(text);
        assert_eq!(
            tokens
                .iter()
                .map(|token| &text[token.start..token.end])
                .collect::<Vec<_>>(),
            vec!["東京", "大学", "教授"]
        );

        let classes = GlossaryEntityKind::ALL.len();
        let mut probabilities = vec![0.0; 6 * classes];
        probabilities[3 * classes + GlossaryEntityKind::Organization as usize] = 0.9;
        assert_eq!(
            decode_scores(text, &tokens, &probabilities, 0.5).unwrap(),
            vec![GlossaryEntity::new(
                0,
                12,
                "東京大学".into(),
                GlossaryEntityKind::Organization,
                0.9,
            )]
        );
    }

    #[test]
    fn oversized_token_is_split_at_utf8_boundaries_before_encoding() {
        let processor = character_processor();
        let text = "界".repeat(600);
        let tokens = processor
            .split_oversized_tokens(
                &text,
                &[TextToken {
                    start: 0,
                    end: text.len(),
                }],
                MAX_ENCODER_TOKENS,
                &Cancellation::never(),
            )
            .unwrap()
            .unwrap();

        assert!(tokens.len() > 1);
        assert_eq!(tokens.first().unwrap().start, 0);
        assert_eq!(tokens.last().unwrap().end, text.len());
        assert!(tokens.windows(2).all(|pair| pair[0].end == pair[1].start));
        for token in &tokens {
            assert!(text.is_char_boundary(token.start));
            assert!(text.is_char_boundary(token.end));
            let encoded = processor
                .encode(&text, std::slice::from_ref(token))
                .unwrap();
            assert!(encoded.input_ids.len() <= MAX_ENCODER_TOKENS);
            assert_eq!(encoded.word_subtokens.len(), encoded.prompt_words + 1);
            assert!(
                encoded
                    .word_subtokens
                    .iter()
                    .all(|subtokens| subtokens.end <= MAX_ENCODER_TOKENS)
            );
        }
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
        let mut probabilities = vec![0.0; 6 * classes];
        probabilities[3 * classes + GlossaryEntityKind::Person as usize] = 0.7;
        probabilities[5 * classes + GlossaryEntityKind::Person as usize] = 0.9;
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
