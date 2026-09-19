use thiserror::Error;

use crate::Language;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{provider} does not support target language {language}")]
    UnsupportedLanguage {
        provider: &'static str,
        language: Language,
    },
    #[error("{provider} does not support source language {language}")]
    UnsupportedSourceLanguage {
        provider: &'static str,
        language: Language,
    },
    #[error("{provider} returned {actual} segments; expected {expected}")]
    SegmentCount {
        provider: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("{provider} does not support translation requests that require a system prompt")]
    UnsupportedSystemPrompt { provider: &'static str },
    #[error("serialized terminology is {actual} bytes; maximum is {max} bytes")]
    TerminologyPromptTooLarge { actual: usize, max: usize },
    #[error("{provider} quota or rate limit was exceeded")]
    QuotaExceeded { provider: &'static str },
    #[error("{provider} API request failed with HTTP {status}: {message}")]
    Api {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
