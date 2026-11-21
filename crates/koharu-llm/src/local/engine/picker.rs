//! How a sequence picks its tokens: llama.cpp's samplers, greedy decoding
//! held to a grammar, and reasoning that leads into either.

use anyhow::{Context, Result};
use koharu_llama::{
    context::LlamaContext, model::LlamaModel, sampling::LlamaSampler, token::LlamaToken,
};
use llguidance::{Matcher, ParserFactory, api::TopLevelGrammar};

use super::{Engine, Job};
use crate::local::{Sampling, markup};

pub(super) enum Picker {
    /// llama.cpp's sampler chain.
    Chain(LlamaSampler),
    /// Greedy decoding held to a grammar. The top token goes when the grammar
    /// allows it, and the full mask is built only when it doesn't, as
    /// llama.cpp's common sampler tries its sampled token before masking the
    /// vocabulary. Greedy, that picks what masking first would: the first
    /// token with the highest logit the grammar allows.
    Guided(Box<Matcher>),
    /// Reasoning by `first`'s picks until the reply writes `close`, then the
    /// answer by `then`'s. Reasoning the prompt didn't open comes only at the
    /// reply's start, so a reply whose first text past whitespace can't open
    /// it is answer from that token on, as llama.cpp's Gemma 4 grammar holds
    /// an optional thought and then the schema
    /// (common/parsers/gemma4.cpp at b12565).
    Reasoning {
        first: Box<Picker>,
        /// The marker the reply opens its reasoning with, until it has.
        open: Option<&'static str>,
        close: &'static str,
        /// The reply's last bytes, to find `open` and `close` across tokens.
        tail: Vec<u8>,
        then: Box<Picker>,
    },
}

impl Picker {
    /// Picks by `sampling`, held to `grammar` when there is one, after
    /// `history`, which the penalties count.
    fn sampled(
        sampling: &Sampling,
        grammar: Option<Matcher>,
        model: &LlamaModel,
        history: &[LlamaToken],
    ) -> Result<Self> {
        let repeat = sampling.repeat_penalty.unwrap_or(1.0);
        let frequency = sampling.frequency_penalty.unwrap_or(0.0);
        let presence = sampling.presence_penalty.unwrap_or(0.0);
        let penalized = repeat != 1.0 || frequency != 0.0 || presence != 0.0;
        let temperature = sampling.temperature.unwrap_or(1.0);
        let mut samplers = Vec::new();
        if let Some(grammar) = grammar {
            if temperature == 0.0 && !penalized {
                return Ok(Self::Guided(Box::new(grammar)));
            }
            samplers.push(LlamaSampler::from(grammar));
        }
        if penalized {
            let mut penalties = LlamaSampler::penalties(
                model.n_vocab(),
                // llama.cpp's default window.
                sampling.repeat_last_n.unwrap_or(64),
                repeat,
                frequency,
                presence,
            );
            penalties.accept_many(history.iter().copied());
            samplers.push(penalties);
        }
        if temperature == 0.0 {
            samplers.push(LlamaSampler::greedy());
        } else {
            if let Some(top_k) = sampling.top_k {
                samplers.push(LlamaSampler::top_k(i32::try_from(top_k)?));
            }
            if let Some(top_p) = sampling.top_p {
                samplers.push(LlamaSampler::top_p(top_p as f32, 1));
            }
            if let Some(min_p) = sampling.min_p {
                samplers.push(LlamaSampler::min_p(min_p, 1));
            }
            samplers.push(LlamaSampler::temp(temperature as f32));
            // llama.h's LLAMA_DEFAULT_SEED, a random seed.
            let seed = sampling.seed.map_or(Ok(u32::MAX), u32::try_from)?;
            samplers.push(LlamaSampler::dist(seed));
        }
        Ok(Self::Chain(LlamaSampler::chain_simple(samplers)))
    }

    /// The token this picks from the logits of output `index`.
    pub(super) fn pick(
        &mut self,
        model: &LlamaModel,
        context: &LlamaContext,
        index: i32,
    ) -> LlamaToken {
        match self {
            Self::Chain(sampler) => sampler.sample(context, index),
            Self::Guided(matcher) => guided(matcher, model, context, index),
            Self::Reasoning {
                first,
                open,
                close,
                tail,
                ..
            } => {
                let token = first.pick(model, context, index);
                tail.extend(model.vocab().token_to_piece(token, true, None));
                if let Some(marker) = *open {
                    // Whitespace before the marker isn't kept, so `tail`
                    // stays as long as the marker.
                    let space = tail.len() - tail.trim_ascii_start().len();
                    tail.drain(..space);
                    match markup::opens(marker, tail) {
                        Some(true) => *open = None,
                        None => return token,
                        // This token is the answer's first, so the schema
                        // picks it.
                        Some(false) => {
                            self.answer();
                            return self.pick(model, context, index);
                        }
                    }
                }
                let done = tail.ends_with(close.as_bytes());
                if tail.len() > 2 * close.len() {
                    tail.drain(..tail.len() - close.len());
                }
                if done {
                    self.answer();
                }
                token
            }
        }
    }

    /// Ends reasoning: the answer's picks follow.
    fn answer(&mut self) {
        *self = match std::mem::replace(self, Self::Chain(LlamaSampler::greedy())) {
            Self::Reasoning { then, .. } => *then,
            picker => picker,
        };
    }
}

impl Engine<'_> {
    /// How the reply to `job` picks its tokens after `history`.
    pub(super) fn picker(&mut self, job: &Job, history: &[LlamaToken]) -> Result<Picker> {
        let grammar = job
            .schema
            .as_deref()
            .map(|schema| self.grammar(schema))
            .transpose()?;
        let answer = Picker::sampled(&job.sampling, grammar, self.model, history)?;
        let Some((open, close)) = job.answer_after else {
            return Ok(answer);
        };
        Ok(Picker::Reasoning {
            first: Box::new(Picker::sampled(&job.sampling, None, self.model, history)?),
            open: (!markup::opened(open, &job.prompt)).then_some(open),
            close,
            tail: Vec::new(),
            then: Box::new(answer),
        })
    }

    /// `schema` compiled for the model's vocabulary, whose grammar factory is
    /// made once.
    fn grammar(&mut self, schema: &str) -> Result<Matcher> {
        if self.grammars.is_none() {
            let tokens = LlamaSampler::llguidance_tok_env(self.model);
            self.grammars = Some(
                ParserFactory::new_simple(&tokens).map_err(|error| anyhow::anyhow!("{error}"))?,
            );
        }
        let factory = self.grammars.as_ref().context("no grammar factory")?;
        let grammar = TopLevelGrammar::from_tagged_str("json_schema", schema)
            .context("couldn't compile the JSON schema")?;
        let parser = factory
            .create_parser(grammar)
            .context("couldn't compile the JSON schema")?;
        Ok(Matcher::new(Ok(parser)))
    }
}

/// Greedy decoding held to `matcher`, which accepts the token it picks.
fn guided(
    matcher: &mut Matcher,
    model: &LlamaModel,
    context: &LlamaContext,
    index: i32,
) -> LlamaToken {
    let logits = context.get_logits_ith(index);
    let token = LlamaToken(first_highest(logits.iter().copied()) as i32);
    let allowed = !matcher.is_stopped()
        && !model.vocab().is_eog(token)
        && matcher
            .validate_tokens(&[token.0.cast_unsigned()])
            .is_ok_and(|valid| valid == 1);
    let token = if allowed {
        token
    } else {
        // The grammar sampler leaves the logits unmasked when it can't
        // build the mask.
        match matcher.compute_mask_or_eos() {
            Ok(mask) => {
                let masked = logits.iter().enumerate().map(|(at, &logit)| {
                    match mask.is_allowed(at as u32) {
                        true => logit,
                        false => f32::NEG_INFINITY,
                    }
                });
                LlamaToken(first_highest(masked) as i32)
            }
            Err(_) => token,
        }
    };
    // As the grammar sampler accepts a token: never into a stopped parser,
    // which consuming would poison.
    if !matcher.is_stopped() {
        let _ = matcher.consume_token(token.0.cast_unsigned());
    }
    token
}

/// llama.cpp's greedy sampler: the index of the first of the highest logits.
fn first_highest(logits: impl Iterator<Item = f32>) -> usize {
    let (mut best, mut high) = (0, f32::NEG_INFINITY);
    for (at, logit) in logits.enumerate() {
        if at == 0 || logit > high {
            (best, high) = (at, logit);
        }
    }
    best
}
