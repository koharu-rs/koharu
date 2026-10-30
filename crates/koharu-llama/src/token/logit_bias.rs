//! Safe wrapper around `llama_logit_bias`.
use crate::token::LlamaToken;

/// A transparent wrapper around `llama_logit_bias`.
///
/// Represents a bias to be applied to a specific token during text generation.
/// The bias modifies the likelihood of the token being selected.
///
/// Do not rely on `repr(transparent)` for this type. It should be considered an implementation
/// detail and may change across minor versions.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(transparent)]
#[allow(clippy::module_name_repetitions)]
pub struct LlamaLogitBias {
    logit_bias: koharu_llama_sys::llama_logit_bias,
}

impl LlamaLogitBias {
    /// Creates a new logit bias for a specific token with the given bias value.
    ///
    /// # Examples
    /// ```
    /// # use koharu_llama::token::{LlamaToken, logit_bias::LlamaLogitBias};
    /// let token = LlamaToken::new(1);
    /// let bias = LlamaLogitBias::new(token, 1.5);
    /// ```
    #[must_use]
    pub fn new(LlamaToken(token): LlamaToken, bias: f32) -> Self {
        Self {
            logit_bias: koharu_llama_sys::llama_logit_bias { token, bias },
        }
    }

    /// Gets the token this bias applies to.
    ///
    /// # Examples
    /// ```
    /// # use koharu_llama::token::{LlamaToken, logit_bias::LlamaLogitBias};
    /// let token = LlamaToken::new(1);
    /// let bias = LlamaLogitBias::new(token, 1.5);
    /// assert_eq!(bias.token(), token);
    /// ```
    #[must_use]
    pub fn token(&self) -> LlamaToken {
        LlamaToken(self.logit_bias.token)
    }

    /// Gets the bias value.
    ///
    /// # Examples
    /// ```
    /// # use koharu_llama::token::{LlamaToken, logit_bias::LlamaLogitBias};
    /// let token = LlamaToken::new(1);
    /// let bias = LlamaLogitBias::new(token, 1.5);
    /// assert_eq!(bias.bias(), 1.5);
    /// ```
    #[must_use]
    pub fn bias(&self) -> f32 {
        self.logit_bias.bias
    }

    /// Sets the token this bias applies to.
    ///
    /// # Examples
    /// ```
    /// # use koharu_llama::token::{LlamaToken, logit_bias::LlamaLogitBias};
    /// let token = LlamaToken::new(1);
    /// let mut bias = LlamaLogitBias::new(token, 1.5);
    /// let new_token = LlamaToken::new(2);
    /// bias.set_token(new_token);
    /// assert_eq!(bias.token(), new_token);
    /// ```
    pub fn set_token(&mut self, token: LlamaToken) {
        self.logit_bias.token = token.0;
    }

    /// Sets the bias value.
    ///
    /// # Examples
    /// ```
    /// # use koharu_llama::token::{LlamaToken, logit_bias::LlamaLogitBias};
    /// let token = LlamaToken::new(1);
    /// let mut bias = LlamaLogitBias::new(token, 1.5);
    /// bias.set_bias(2.0);
    /// assert_eq!(bias.bias(), 2.0);
    /// ```
    pub fn set_bias(&mut self, bias: f32) {
        self.logit_bias.bias = bias;
    }
}
