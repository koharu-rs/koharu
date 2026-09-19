//! Safe wrapper around `llama_sampler`.

use std::borrow::Borrow;
use std::ffi::{CString, c_char};
use std::fmt::{Debug, Formatter};

use crate::GrammarError;
use crate::context::LlamaContext;
use crate::model::LlamaModel;
use crate::token::LlamaToken;
use crate::token::data_array::LlamaTokenDataArray;
use crate::token::logit_bias::LlamaLogitBias;

/// A safe wrapper around `llama_sampler`.
pub struct LlamaSampler {
    pub(crate) sampler: *mut koharu_llama_sys::llama_sampler,
}

impl Debug for LlamaSampler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaSamplerChain").finish()
    }
}

impl LlamaSampler {
    /// Sample and accept a token from the idx-th output of the last evaluation
    #[must_use]
    pub fn sample(&mut self, ctx: &LlamaContext, idx: i32) -> LlamaToken {
        let token = unsafe {
            koharu_llama_sys::llama_sampler_sample(self.sampler, ctx.context.as_ptr(), idx)
        };

        LlamaToken(token)
    }

    /// Applies this sampler to a [`LlamaTokenDataArray`].
    pub fn apply(&self, data_array: &mut LlamaTokenDataArray) {
        data_array.apply_sampler(self);
    }

    /// Accepts a token from the sampler, possibly updating the internal state of certain samplers
    /// (e.g. grammar, repetition, etc.)
    pub fn accept(&mut self, token: LlamaToken) {
        unsafe { koharu_llama_sys::llama_sampler_accept(self.sampler, token.0) };
    }

    /// Accepts several tokens from the sampler or context, possibly updating the internal state of
    /// certain samplers (e.g. grammar, repetition, etc.)
    pub fn accept_many(&mut self, tokens: impl IntoIterator<Item = impl Borrow<LlamaToken>>) {
        for token in tokens {
            self.accept(*token.borrow());
        }
    }

    /// Accepts several tokens from the sampler or context, possibly updating the internal state of
    /// certain samplers (e.g. grammar, repetition, etc.)
    #[must_use]
    pub fn with_tokens(
        mut self,
        tokens: impl IntoIterator<Item = impl Borrow<LlamaToken>>,
    ) -> Self {
        self.accept_many(tokens);
        self
    }

    /// Resets the internal state of the sampler.
    ///
    /// This can be useful when you want to start fresh with a sampler without creating a new instance.
    pub fn reset(&mut self) {
        unsafe {
            koharu_llama_sys::llama_sampler_reset(self.sampler);
        }
    }

    /// Gets the random seed used by this sampler.
    ///
    /// Returns:
    /// - For random samplers (dist, mirostat, mirostat_v2): returns their current seed
    /// - For sampler chains: returns the first non-default seed found in reverse order
    /// - For all other samplers: returns 0xFFFFFFFF
    #[must_use]
    pub fn get_seed(&self) -> u32 {
        unsafe { koharu_llama_sys::llama_sampler_get_seed(self.sampler) }
    }

    /// Combines a list of samplers into a single sampler that applies each component sampler one
    /// after another.
    ///
    /// If you are using a chain to select a token, the chain should always end with one of
    /// [`LlamaSampler::greedy`], [`LlamaSampler::dist`], [`LlamaSampler::mirostat`], and
    /// [`LlamaSampler::mirostat_v2`].
    #[must_use]
    pub fn chain(samplers: impl IntoIterator<Item = Self>, no_perf: bool) -> Self {
        unsafe {
            let chain = koharu_llama_sys::llama_sampler_chain_init(
                koharu_llama_sys::llama_sampler_chain_params { no_perf },
            );

            for sampler in samplers {
                koharu_llama_sys::llama_sampler_chain_add(chain, sampler.sampler);

                // Do not call `llama_sampler_free` on the sampler, as the internal sampler is now
                // owned by the chain
                std::mem::forget(sampler);
            }

            Self { sampler: chain }
        }
    }

    /// Same as [`Self::chain`] with `no_perf = false`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use koharu_llama::token::{
    ///    LlamaToken,
    ///    data::LlamaTokenData,
    ///    data_array::LlamaTokenDataArray
    /// };
    /// use koharu_llama::sampling::LlamaSampler;
    /// use koharu_llama::llama_backend::LlamaBackend;
    /// let backend = LlamaBackend::init().unwrap();
    ///
    /// let mut data_array = LlamaTokenDataArray::new(vec![
    ///     LlamaTokenData::new(LlamaToken(0), 0., 0.),
    ///     LlamaTokenData::new(LlamaToken(1), 1., 0.),
    ///     LlamaTokenData::new(LlamaToken(2), 2., 0.),
    /// ], false);
    ///
    /// data_array.apply_sampler(&mut LlamaSampler::chain_simple([
    ///     LlamaSampler::temp(0.5),
    ///     LlamaSampler::greedy(),
    /// ]));
    ///
    /// assert_eq!(data_array.data[0].logit(), 0.);
    /// assert_eq!(data_array.data[1].logit(), 2.);
    /// assert_eq!(data_array.data[2].logit(), 4.);
    ///
    /// assert_eq!(data_array.data.len(), 3);
    /// assert_eq!(data_array.selected_token(), Some(LlamaToken(2)));
    /// ```
    #[must_use]
    pub fn chain_simple(samplers: impl IntoIterator<Item = Self>) -> Self {
        Self::chain(samplers, false)
    }

    #[allow(clippy::doc_markdown)]
    /// Updates the logits l_i' = l_i/t. When t <= 0.0f, the maximum logit is kept at it's original
    /// value, the rest are set to -inf
    ///
    /// # Example:
    /// ```rust,no_run
    /// use koharu_llama::token::{
    ///    LlamaToken,
    ///    data::LlamaTokenData,
    ///    data_array::LlamaTokenDataArray
    /// };
    /// use koharu_llama::sampling::LlamaSampler;
    ///
    /// let mut data_array = LlamaTokenDataArray::new(vec![
    ///     LlamaTokenData::new(LlamaToken(0), 0., 0.),
    ///     LlamaTokenData::new(LlamaToken(1), 1., 0.),
    ///     LlamaTokenData::new(LlamaToken(2), 2., 0.),
    /// ], false);
    ///
    /// data_array.apply_sampler(&mut LlamaSampler::temp(0.5));
    ///
    /// assert_eq!(data_array.data[0].logit(), 0.);
    /// assert_eq!(data_array.data[1].logit(), 2.);
    /// assert_eq!(data_array.data[2].logit(), 4.);
    /// ```
    #[must_use]
    pub fn temp(t: f32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_temp(t) };
        Self { sampler }
    }

    /// Dynamic temperature implementation (a.k.a. entropy) described in the paper
    /// <https://arxiv.org/abs/2309.02772>.
    #[must_use]
    pub fn temp_ext(t: f32, delta: f32, exponent: f32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_temp_ext(t, delta, exponent) };
        Self { sampler }
    }

    /// Top-K sampling described in academic paper "The Curious Case of Neural Text Degeneration"
    /// <https://arxiv.org/abs/1904.09751>
    ///
    /// # Example:
    /// ```rust,no_run
    /// use koharu_llama::token::{
    ///    LlamaToken,
    ///    data::LlamaTokenData,
    ///    data_array::LlamaTokenDataArray
    /// };
    /// use koharu_llama::sampling::LlamaSampler;
    ///
    /// let mut data_array = LlamaTokenDataArray::new(vec![
    ///     LlamaTokenData::new(LlamaToken(0), 0., 0.),
    ///     LlamaTokenData::new(LlamaToken(1), 1., 0.),
    ///     LlamaTokenData::new(LlamaToken(2), 2., 0.),
    ///     LlamaTokenData::new(LlamaToken(3), 3., 0.),
    /// ], false);
    ///
    /// data_array.apply_sampler(&mut LlamaSampler::top_k(2));
    ///
    /// assert_eq!(data_array.data.len(), 2);
    /// assert_eq!(data_array.data[0].id(), LlamaToken(3));
    /// assert_eq!(data_array.data[1].id(), LlamaToken(2));
    /// ```
    #[must_use]
    pub fn top_k(k: i32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_top_k(k) };
        Self { sampler }
    }

    /// Top-nÏƒ sampling as described in academic paper "Top-nÏƒ: Not All Logits Are You Need"
    /// <https://arxiv.org/pdf/2411.07641>
    ///
    /// This method filters logits by selecting only those within *n* standard deviations of the mean.
    ///
    /// # Parameters
    /// - `n`: Number of standard deviations from the mean to include in sampling
    ///
    /// # Example
    /// ```rust,no_run
    /// use koharu_llama::sampling::LlamaSampler;
    /// use koharu_llama::token::{
    ///     LlamaToken,
    ///     data::LlamaTokenData,
    ///     data_array::LlamaTokenDataArray
    /// };
    ///
    /// let mut data_array = LlamaTokenDataArray::new(vec![
    ///     LlamaTokenData::new(LlamaToken(0), 0.0, 0.0),
    ///     LlamaTokenData::new(LlamaToken(1), 1.0, 0.0),
    ///     LlamaTokenData::new(LlamaToken(2), 2.0, 0.0),
    /// ], false);
    ///
    /// data_array.apply_sampler(&mut LlamaSampler::top_n_sigma(2.0));
    /// ```
    #[must_use]
    pub fn top_n_sigma(n: f32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_top_n_sigma(n) };
        Self { sampler }
    }

    /// Locally Typical Sampling implementation described in the paper <https://arxiv.org/abs/2202.00666>.
    #[must_use]
    pub fn typical(p: f32, min_keep: usize) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_typical(p, min_keep) };
        Self { sampler }
    }

    /// Nucleus sampling described in academic paper "The Curious Case of Neural Text Degeneration"
    /// <https://arxiv.org/abs/1904.09751>
    #[must_use]
    pub fn top_p(p: f32, min_keep: usize) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_top_p(p, min_keep) };
        Self { sampler }
    }

    /// Minimum P sampling as described in <https://github.com/ggerganov/llama.cpp/pull/3841>
    #[must_use]
    pub fn min_p(p: f32, min_keep: usize) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_min_p(p, min_keep) };
        Self { sampler }
    }

    /// XTC sampler as described in <https://github.com/oobabooga/text-generation-webui/pull/6335>
    #[must_use]
    pub fn xtc(p: f32, t: f32, min_keep: usize, seed: u32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_xtc(p, t, min_keep, seed) };
        Self { sampler }
    }

    /// Grammar sampler
    pub fn grammar(
        model: &LlamaModel,
        grammar_str: &str,
        grammar_root: &str,
    ) -> Result<Self, GrammarError> {
        let (grammar_str, grammar_root) =
            Self::sanitize_grammar_strings(grammar_str, grammar_root)?;

        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init_grammar(
                model.vocab_ptr(),
                grammar_str.as_ptr(),
                grammar_root.as_ptr(),
            )
        };

        if sampler.is_null() {
            Err(GrammarError::NullGrammar)
        } else {
            Ok(Self { sampler })
        }
    }

    /// Lazy grammar sampler, introduced in <https://github.com/ggerganov/llama.cpp/pull/9639>
    ///
    /// This sampler enforces grammar rules only when specific trigger words or tokens are encountered.
    pub fn grammar_lazy(
        model: &LlamaModel,
        grammar_str: &str,
        grammar_root: &str,
        trigger_words: impl IntoIterator<Item = impl AsRef<[u8]>>,
        trigger_tokens: &[LlamaToken],
    ) -> Result<Self, GrammarError> {
        let (grammar_str, grammar_root) =
            Self::sanitize_grammar_strings(grammar_str, grammar_root)?;
        let trigger_words = Self::sanitize_trigger_words(trigger_words)?;

        let mut trigger_word_ptrs: Vec<*const c_char> =
            trigger_words.iter().map(|cs| cs.as_ptr()).collect();

        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init_grammar_lazy(
                model.vocab_ptr(),
                grammar_str.as_ptr(),
                grammar_root.as_ptr(),
                trigger_word_ptrs.as_mut_ptr(),
                trigger_word_ptrs.len(),
                trigger_tokens.as_ptr().cast(),
                trigger_tokens.len(),
            )
        };

        if sampler.is_null() {
            Err(GrammarError::NullGrammar)
        } else {
            Ok(Self { sampler })
        }
    }

    /// Lazy grammar sampler using regex trigger patterns.
    ///
    /// Trigger patterns are regular expressions matched from the start of the
    /// generation output. The grammar sampler will be fed content starting from
    /// the first match group.
    pub fn grammar_lazy_patterns(
        model: &LlamaModel,
        grammar_str: &str,
        grammar_root: &str,
        trigger_patterns: &[String],
        trigger_tokens: &[LlamaToken],
    ) -> Result<Self, GrammarError> {
        let (grammar_str, grammar_root) =
            Self::sanitize_grammar_strings(grammar_str, grammar_root)?;
        let trigger_patterns = Self::sanitize_trigger_patterns(trigger_patterns)?;

        let mut trigger_pattern_ptrs: Vec<*const c_char> =
            trigger_patterns.iter().map(|cs| cs.as_ptr()).collect();

        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init_grammar_lazy_patterns(
                model.vocab_ptr(),
                grammar_str.as_ptr(),
                grammar_root.as_ptr(),
                trigger_pattern_ptrs.as_mut_ptr(),
                trigger_pattern_ptrs.len(),
                trigger_tokens.as_ptr().cast(),
                trigger_tokens.len(),
            )
        };

        if sampler.is_null() {
            Err(GrammarError::NullGrammar)
        } else {
            Ok(Self { sampler })
        }
    }

    /// Build and return the reusable llguidance tokenizer environment for `model`.
    ///
    /// Use it to construct an `llguidance::ParserFactory`, create a matcher for each
    /// generation, and convert that matcher into a sampler with `LlamaSampler::from`.
    #[cfg(feature = "llguidance")]
    #[must_use]
    pub fn llguidance_tok_env(model: &LlamaModel) -> toktrie::TokEnv {
        crate::llguidance_sampler::llguidance_build_tok_env(model)
    }

    fn sanitize_grammar_strings(
        grammar_str: &str,
        grammar_root: &str,
    ) -> Result<(CString, CString), GrammarError> {
        if !grammar_str.contains(grammar_root) {
            return Err(GrammarError::RootNotFound);
        }

        if grammar_str.contains('\0') || grammar_root.contains('\0') {
            return Err(GrammarError::GrammarNullBytes);
        }

        Ok((
            CString::new(grammar_str).unwrap(),
            CString::new(grammar_root).unwrap(),
        ))
    }

    fn sanitize_trigger_words(
        trigger_words: impl IntoIterator<Item = impl AsRef<[u8]>>,
    ) -> Result<Vec<CString>, GrammarError> {
        let trigger_words: Vec<_> = trigger_words.into_iter().collect();
        if trigger_words
            .iter()
            .any(|word| word.as_ref().contains(&b'\0'))
        {
            return Err(GrammarError::TriggerWordNullBytes);
        }
        Ok(trigger_words
            .into_iter()
            .map(|word| CString::new(word.as_ref()).unwrap())
            .collect())
    }

    fn sanitize_trigger_patterns(
        trigger_patterns: &[String],
    ) -> Result<Vec<CString>, GrammarError> {
        let mut patterns = Vec::with_capacity(trigger_patterns.len());
        for pattern in trigger_patterns {
            if pattern.contains('\0') {
                return Err(GrammarError::GrammarNullBytes);
            }
            patterns.push(CString::new(pattern.as_str()).unwrap());
        }
        Ok(patterns)
    }

    /// DRY sampler, designed by p-e-w, as described in:
    /// <https://github.com/oobabooga/text-generation-webui/pull/5677>, porting Koboldcpp
    /// implementation authored by pi6am: <https://github.com/LostRuins/koboldcpp/pull/982>
    ///
    /// # Panics
    /// If any string in ``seq_breakers`` contains null bytes.
    #[allow(missing_docs)]
    #[must_use]
    pub fn dry(
        model: &LlamaModel,
        multiplier: f32,
        base: f32,
        allowed_length: i32,
        penalty_last_n: i32,
        seq_breakers: impl IntoIterator<Item = impl AsRef<[u8]>>,
    ) -> Self {
        let seq_breakers: Vec<CString> = seq_breakers
            .into_iter()
            .map(|s| CString::new(s.as_ref()).expect("A sequence breaker contains null bytes"))
            .collect();
        let mut seq_breaker_pointers: Vec<*const c_char> =
            seq_breakers.iter().map(|s| s.as_ptr()).collect();

        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init_dry(
                model.vocab_ptr(),
                multiplier,
                base,
                allowed_length,
                penalty_last_n,
                seq_breaker_pointers.as_mut_ptr(),
                seq_breaker_pointers.len(),
            )
        };
        Self { sampler }
    }

    /// Penalizes tokens for being present in the context.
    ///
    /// Parameters:
    /// - ``penalty_last_n``: last n tokens to penalize (0 = disable penalty)
    /// - ``penalty_repeat``: must be positive; 1.0 disables repeat penalties
    /// - ``penalty_freq``: must be finite; 0.0 disables frequency penalties
    /// - ``penalty_present``: must be finite; 0.0 disables presence penalties
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn penalties(
        model: &LlamaModel,
        penalty_last_n: i32,
        penalty_repeat: f32,
        penalty_freq: f32,
        penalty_present: f32,
    ) -> Self {
        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init_penalties(
                model.n_vocab(),
                penalty_last_n,
                penalty_repeat,
                penalty_freq,
                penalty_present,
            )
        };
        Self { sampler }
    }

    /// Mirostat 1.0 algorithm described in the paper <https://arxiv.org/abs/2007.14966>. Uses tokens instead of words.
    ///
    /// # Parameters:
    /// - ``n_vocab``: [`LlamaModel::n_vocab`]
    /// - ``seed``: Seed to initialize random generation with.
    /// - ``tau``: The target cross-entropy (or surprise) value you want to achieve for the
    ///   generated text. A higher value corresponds to more surprising or less predictable text,
    ///   while a lower value corresponds to less surprising or more predictable text.
    /// - ``eta``: The learning rate used to update `mu` based on the error between the target and
    ///   observed surprisal of the sampled word. A larger learning rate will cause `mu` to be
    ///   updated more quickly, while a smaller learning rate will result in slower updates.
    /// - ``m``: The number of tokens considered in the estimation of `s_hat`. This is an arbitrary
    ///   value that is used to calculate `s_hat`, which in turn helps to calculate the value of `k`.
    ///   In the paper, they use `m = 100`, but you can experiment with different values to see how
    ///   it affects the performance of the algorithm.
    #[must_use]
    pub fn mirostat(n_vocab: i32, seed: u32, tau: f32, eta: f32, m: i32) -> Self {
        let sampler =
            unsafe { koharu_llama_sys::llama_sampler_init_mirostat(n_vocab, seed, tau, eta, m) };
        Self { sampler }
    }

    /// Mirostat 2.0 algorithm described in the paper <https://arxiv.org/abs/2007.14966>. Uses tokens instead of words.
    ///
    /// # Parameters:
    /// - ``seed``: Seed to initialize random generation with.
    /// - ``tau``: The target cross-entropy (or surprise) value you want to achieve for the
    ///   generated text. A higher value corresponds to more surprising or less predictable text,
    ///   while a lower value corresponds to less surprising or more predictable text.
    /// - ``eta``: The learning rate used to update `mu` based on the error between the target and
    ///   observed surprisal of the sampled word. A larger learning rate will cause `mu` to be
    ///   updated more quickly, while a smaller learning rate will result in slower updates.
    #[must_use]
    pub fn mirostat_v2(seed: u32, tau: f32, eta: f32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_mirostat_v2(seed, tau, eta) };
        Self { sampler }
    }

    /// Selects a token at random based on each token's probabilities
    #[must_use]
    pub fn dist(seed: u32) -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_dist(seed) };
        Self { sampler }
    }

    /// Selects the most likely token
    ///
    /// # Example:
    /// ```rust,no_run
    /// use koharu_llama::token::{
    ///    LlamaToken,
    ///    data::LlamaTokenData,
    ///    data_array::LlamaTokenDataArray
    /// };
    /// use koharu_llama::sampling::LlamaSampler;
    ///
    /// let mut data_array = LlamaTokenDataArray::new(vec![
    ///     LlamaTokenData::new(LlamaToken(0), 0., 0.),
    ///     LlamaTokenData::new(LlamaToken(1), 1., 0.),
    /// ], false);
    ///
    /// data_array.apply_sampler(&mut LlamaSampler::greedy());
    ///
    /// assert_eq!(data_array.data.len(), 2);
    /// assert_eq!(data_array.selected_token(), Some(LlamaToken(1)));
    /// ```
    #[must_use]
    pub fn greedy() -> Self {
        let sampler = unsafe { koharu_llama_sys::llama_sampler_init_greedy() };
        Self { sampler }
    }

    /// Creates a sampler that applies bias values to specific tokens during sampling.
    ///
    /// # Parameters
    /// - ``n_vocab``: [`LlamaModel::n_vocab`]
    /// - ``biases``: Slice of [`LlamaLogitBias`] values specifying token-bias pairs
    ///
    /// # Example
    /// ```rust,no_run
    /// use koharu_llama::token::{LlamaToken, logit_bias::LlamaLogitBias};
    /// use koharu_llama::sampling::LlamaSampler;
    ///
    /// let biases = vec![
    ///     LlamaLogitBias::new(LlamaToken(1), 1.5),  // Increase probability of token 1
    ///     LlamaLogitBias::new(LlamaToken(2), -1.0), // Decrease probability of token 2
    /// ];
    ///
    /// // Assuming vocab_size of 32000
    /// let sampler = LlamaSampler::logit_bias(32000, &biases);
    /// ```
    #[must_use]
    pub fn logit_bias(n_vocab: i32, biases: &[LlamaLogitBias]) -> Self {
        let data = biases.as_ptr().cast::<koharu_llama_sys::llama_logit_bias>();

        let sampler = unsafe {
            koharu_llama_sys::llama_sampler_init_logit_bias(n_vocab, biases.len() as i32, data)
        };

        Self { sampler }
    }
}

impl Drop for LlamaSampler {
    fn drop(&mut self) {
        unsafe {
            koharu_llama_sys::llama_sampler_free(self.sampler);
        }
    }
}
