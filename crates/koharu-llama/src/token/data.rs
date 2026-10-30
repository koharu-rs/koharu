//! Safe wrapper around `llama_token_data`.
use crate::token::LlamaToken;

/// A transparent wrapper around `llama_token_data`.
///
/// Do not rely on `repr(transparent)` for this type. It should be considered an implementation
/// detail and may change across minor versions.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(transparent)]
#[allow(clippy::module_name_repetitions)]
pub struct LlamaTokenData {
    data: koharu_llama_sys::llama_token_data,
}

impl LlamaTokenData {
    /// Create a new token data from a token, logit, and probability.
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let token_data = LlamaTokenData::new(token, 1.0, 1.0);
    #[must_use]
    pub fn new(LlamaToken(id): LlamaToken, logit: f32, p: f32) -> Self {
        LlamaTokenData {
            data: koharu_llama_sys::llama_token_data { id, logit, p },
        }
    }
    /// Get the token's id
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let token_data = LlamaTokenData::new(token, 1.0, 1.0);
    /// assert_eq!(token_data.id(), token);
    /// ```
    #[must_use]
    pub fn id(&self) -> LlamaToken {
        LlamaToken(self.data.id)
    }

    /// Get the token's logit
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let token_data = LlamaTokenData::new(token, 1.0, 1.0);
    /// assert_eq!(token_data.logit(), 1.0);
    /// ```
    #[must_use]
    pub fn logit(&self) -> f32 {
        self.data.logit
    }

    /// Get the token's probability
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let token_data = LlamaTokenData::new(token, 1.0, 1.0);
    /// assert_eq!(token_data.p(), 1.0);
    /// ```
    #[must_use]
    pub fn p(&self) -> f32 {
        self.data.p
    }

    /// Set the token's id
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let mut token_data = LlamaTokenData::new(token, 1.0, 1.0);
    /// token_data.set_id(LlamaToken::new(2));
    /// assert_eq!(token_data.id(), LlamaToken::new(2));
    /// ```
    pub fn set_id(&mut self, id: LlamaToken) {
        self.data.id = id.0;
    }

    /// Set the token's logit
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let mut token_data = LlamaTokenData::new(token, 1.0, 1.0);
    /// token_data.set_logit(2.0);
    /// assert_eq!(token_data.logit(), 2.0);
    /// ```
    pub fn set_logit(&mut self, logit: f32) {
        self.data.logit = logit;
    }

    /// Set the token's probability
    /// ```
    /// # use koharu_llama::token::LlamaToken;
    /// # use koharu_llama::token::data::LlamaTokenData;
    /// let token = LlamaToken::new(1);
    /// let mut token_data = LlamaTokenData::new(token, 1.0, 1.0);
    /// token_data.set_p(2.0);
    /// assert_eq!(token_data.p(), 2.0);
    /// ```
    pub fn set_p(&mut self, p: f32) {
        self.data.p = p;
    }
}
