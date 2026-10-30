//! A safe wrapper around `llama_vocab`.

use std::ffi::{c_char, CStr};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::num::NonZeroU16;
use std::slice;

use crate::model::LlamaModel;
use crate::ptr::Ptr;
use crate::token::LlamaToken;
use crate::token_type::LlamaTokenAttrs;

/// A safe wrapper around `llama_vocab`.
///
/// The vocabulary is owned by the [`LlamaModel`] it was retrieved from, so
/// dropping a [`LlamaVocab`] does not free the underlying `llama_vocab`.
#[derive(Debug)]
#[repr(transparent)]
#[allow(clippy::module_name_repetitions)]
pub struct LlamaVocab<'model> {
    vocab: Ptr<koharu_llama_sys::llama_vocab>,
    _phantom: PhantomData<&'model LlamaModel>,
}

// SAFETY: The vocabulary is immutable.
unsafe impl Send for LlamaVocab<'_> {}
// SAFETY: Same as above.
unsafe impl Sync for LlamaVocab<'_> {}

impl<'model> LlamaVocab<'model> {
    pub(crate) fn as_ptr(&self) -> *const koharu_llama_sys::llama_vocab {
        self.vocab.as_ptr()
    }

    pub(crate) fn new(ptr: *const koharu_llama_sys::llama_vocab) -> Option<Self> {
        Some(Self {
            vocab: Ptr::new(ptr.cast_mut())?,
            _phantom: PhantomData,
        })
    }

    /// The type of the vocabulary.
    #[must_use]
    pub fn vocab_type(&self) -> VocabType {
        let vocab_type = unsafe { koharu_llama_sys::llama_vocab_type(self.vocab.as_ptr()) };
        VocabType::try_from(vocab_type).expect("invalid vocab type")
    }

    /// The number of tokens the model was trained on.
    #[must_use]
    pub fn n_tokens(&self) -> i32 {
        unsafe { koharu_llama_sys::llama_vocab_n_tokens(self.vocab.as_ptr()) }
    }

    /// The text representation of a token, if any.
    #[must_use]
    pub fn text(&self, token: LlamaToken) -> Option<&CStr> {
        let text = unsafe { koharu_llama_sys::llama_vocab_get_text(self.vocab.as_ptr(), token.0) };
        if text.is_null() {
            None
        } else {
            // SAFETY: The pointer is not NULL, and it is valid for as long as
            // the `llama_vocab` is (it's stored in that).
            Some(unsafe { CStr::from_ptr(text) })
        }
    }

    /// The score of a token.
    #[must_use]
    pub fn score(&self, token: LlamaToken) -> f32 {
        unsafe { koharu_llama_sys::llama_vocab_get_score(self.vocab.as_ptr(), token.0) }
    }

    /// The attributes of a token.
    #[must_use]
    pub fn attr(&self, token: LlamaToken) -> LlamaTokenAttrs {
        let attrs = unsafe { koharu_llama_sys::llama_vocab_get_attr(self.vocab.as_ptr(), token.0) };
        LlamaTokenAttrs::try_from(attrs).expect("token attr is valid")
    }

    /// Check if the token is supposed to end generation (end-of-generation, eg. EOS, EOT, etc.)
    #[must_use]
    pub fn is_eog(&self, token: LlamaToken) -> bool {
        unsafe { koharu_llama_sys::llama_vocab_is_eog(self.vocab.as_ptr(), token.0) }
    }

    /// Identify if the token is a control token or a render-able token.
    #[must_use]
    pub fn is_control(&self, token: LlamaToken) -> bool {
        unsafe { koharu_llama_sys::llama_vocab_is_control(self.vocab.as_ptr(), token.0) }
    }

    /// The beginning of stream token.
    #[must_use]
    pub fn bos(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_bos(self.vocab.as_ptr()) })
    }

    /// The end of stream token.
    #[must_use]
    pub fn eos(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_eos(self.vocab.as_ptr()) })
    }

    /// The end of turn token.
    #[must_use]
    pub fn eot(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_eot(self.vocab.as_ptr()) })
    }

    /// The separator token.
    #[must_use]
    pub fn sep(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_sep(self.vocab.as_ptr()) })
    }

    /// The newline token.
    #[must_use]
    pub fn nl(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_nl(self.vocab.as_ptr()) })
    }

    /// The padding token.
    #[must_use]
    pub fn pad(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_pad(self.vocab.as_ptr()) })
    }

    /// The mask token.
    #[must_use]
    pub fn mask(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_mask(self.vocab.as_ptr()) })
    }

    /// Whether to add the beginning of stream token when tokenizing.
    #[must_use]
    pub fn should_add_bos(&self) -> bool {
        unsafe { koharu_llama_sys::llama_vocab_get_add_bos(self.vocab.as_ptr()) }
    }

    /// Whether to add the end of stream token when tokenizing.
    #[must_use]
    pub fn should_add_eos(&self) -> bool {
        unsafe { koharu_llama_sys::llama_vocab_get_add_eos(self.vocab.as_ptr()) }
    }

    /// Whether to add the separator token when tokenizing.
    #[must_use]
    pub fn should_add_sep(&self) -> bool {
        unsafe { koharu_llama_sys::llama_vocab_get_add_sep(self.vocab.as_ptr()) }
    }

    /// Model-specific suppress tokens.
    ///
    /// (gguf key: `tokenizer.ggml.suppress_tokens`)
    #[must_use]
    pub fn suppress_tokens(&self) -> &[LlamaToken] {
        let mut n_suppress_tokens = 0_i32;
        let tokens = unsafe {
            koharu_llama_sys::llama_vocab_get_suppress_tokens(
                self.vocab.as_ptr(),
                &mut n_suppress_tokens,
            )
        };
        if tokens.is_null() || n_suppress_tokens == 0 {
            return &[];
        }
        // `LlamaToken` is `#[repr(transparent)]` over `llama_token`.
        let tokens = tokens.cast::<LlamaToken>();
        unsafe { slice::from_raw_parts(tokens, n_suppress_tokens as usize) }
    }

    /// The fill-in-the-middle prefix token.
    #[must_use]
    pub fn fim_pre(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_fim_pre(self.vocab.as_ptr()) })
    }

    /// The fill-in-the-middle suffix token.
    #[must_use]
    pub fn fim_suf(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_fim_suf(self.vocab.as_ptr()) })
    }

    /// The fill-in-the-middle middle token.
    #[must_use]
    pub fn fim_mid(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_fim_mid(self.vocab.as_ptr()) })
    }

    /// The fill-in-the-middle padding token.
    #[must_use]
    pub fn fim_pad(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_fim_pad(self.vocab.as_ptr()) })
    }

    /// The fill-in-the-middle repo token.
    #[must_use]
    pub fn fim_rep(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_fim_rep(self.vocab.as_ptr()) })
    }

    /// The fill-in-the-middle separator token.
    #[must_use]
    pub fn fim_sep(&self) -> LlamaToken {
        LlamaToken(unsafe { koharu_llama_sys::llama_vocab_fim_sep(self.vocab.as_ptr()) })
    }

    /// Tokenize text to a [`Vec`] of tokens.
    ///
    /// This is a convenience wrapper on top of [`tokenize_into`][Self::tokenize_into].
    ///
    /// # Example
    ///
    /// Tokenize a string of text.
    ///
    /// ```no_run
    /// # let vocab: koharu_llama::vocab::LlamaVocab<'_> = todo!();
    /// let tokens = vocab.tokenize("Hello, World!".as_bytes(), true, true);
    /// ```
    pub fn tokenize(&self, text: &[u8], add_special: bool, parse_special: bool) -> Vec<LlamaToken> {
        let tokens_estimation = std::cmp::max(8, (text.len() / 2) + usize::from(add_special));
        let mut output = Vec::with_capacity(tokens_estimation);
        self.tokenize_into(text, &mut output, add_special, parse_special);
        output
    }

    /// Tokenize text into a user-provided buffer.
    ///
    /// The output is placed into the vector's spare capacity, and the `Vec`
    /// will be resized if the output doesn't fit.
    ///
    /// Setting `add_special` allows this function to add BOS and EOS tokens
    /// if the model is configured to do so.
    ///
    /// Setting `parse_special` allows this function to tokenize special
    /// and/or control tokens which otherwise are not exposed and treated as
    /// plaintext. Setting it does not insert a leading space.
    pub fn tokenize_into(
        &self,
        text: &[u8],
        buffer: &mut Vec<LlamaToken>,
        add_special: bool,
        parse_special: bool,
    ) {
        let result = self.tokenize_raw(
            text,
            buffer.spare_capacity_mut(),
            add_special,
            parse_special,
        );

        let size = match result {
            // It's fine to panic here, having a context size that causes
            // this large an output string is never going to happen.
            i32::MIN => panic!("output string would've been larger than 2GB"),
            result if result.is_negative() => {
                // Cannot overflow, usize > i32, and `-written` is non-negative.
                let required_size = (-result) as usize;

                // If we fail the first time we resize the vector to the
                // correct size and try again.
                buffer.reserve_exact(required_size);

                let result = self.tokenize_raw(
                    text,
                    buffer.spare_capacity_mut(),
                    add_special,
                    parse_special,
                );

                // Should succeed, we know the vector was correctly sized now.
                debug_assert!(result.is_positive());

                result as usize
            }
            // Cannot overflow, usize > i32
            written => written as usize,
        };

        // SAFETY: llama.cpp has written `size` items into the spare capacity.
        unsafe { buffer.set_len(buffer.len() + size) };
    }

    fn tokenize_raw(
        &self,
        text: &[u8],
        buffer: &mut [MaybeUninit<LlamaToken>],
        add_special: bool,
        parse_special: bool,
    ) -> i32 {
        unsafe {
            koharu_llama_sys::llama_tokenize(
                self.vocab.as_ptr(),
                text.as_ptr().cast::<c_char>(),
                // Assume the text is short enough to fit in `llama_tokenize`.
                // If not, the user tried to tokenize a >2GB large text, which
                // is gonna fail horribly elsewhere anyhow.
                text.len().try_into().expect("input text was too large"),
                buffer.as_mut_ptr().cast::<koharu_llama_sys::llama_token>(),
                // It's fine to clamp the buffer length here, we'll get an
                // error from llama.cpp anyhow if there are too many tokens to
                // tokenize.
                buffer.len().try_into().unwrap_or(i32::MAX),
                add_special,
                parse_special,
            )
        }
    }

    /// Detokenize a list of tokens.
    ///
    /// This is a convenience wrapper on top of [`detokenize_into`][Self::detokenize_into].
    pub fn detokenize(
        &self,
        tokens: &[LlamaToken],
        remove_special: bool,
        unparse_special: bool,
    ) -> Vec<u8> {
        let tokens_estimation = std::cmp::max(8, (tokens.len() * 2) - usize::from(remove_special));
        let mut output = Vec::with_capacity(tokens_estimation);
        self.detokenize_into(tokens, &mut output, remove_special, unparse_special);
        output
    }

    /// Detokenize a list of tokens into a user-provided buffer.
    ///
    /// This is the inverse of [tokenization][Self::tokenize_into].
    ///
    /// The output is placed into the vector's spare capacity, and the `Vec`
    /// will be resized if the output doesn't fit.
    ///
    /// Setting `remove_special` allows this function to remove BOS and EOS
    /// tokens if the model is configured to do so.
    ///
    /// Special tokens are rendered in the output if `unparse_special` is set.
    ///
    /// # Example
    ///
    /// Convert a list of tokens to a string.
    ///
    /// When converting tokens while streaming, you probably want to use
    /// [`token_to_piece`][Self::token_to_piece] instead.
    ///
    /// ```no_run
    /// # let tokens: Vec<koharu_llama::token::LlamaToken> = todo!();
    /// # let vocab: koharu_llama::vocab::LlamaVocab<'_> = todo!();
    /// let token_bytes = vocab.detokenize(&tokens, true, true);
    /// let (token_string, _had_errors) = encoding_rs::UTF_8.decode_without_bom_handling(&token_bytes);
    /// print!("{token_string}");
    /// ```
    pub fn detokenize_into(
        &self,
        tokens: &[LlamaToken],
        buffer: &mut Vec<u8>,
        remove_special: bool,
        unparse_special: bool,
    ) {
        let result = self.detokenize_raw(
            tokens,
            buffer.spare_capacity_mut(),
            remove_special,
            unparse_special,
        );

        let size = match result {
            result if result.is_negative() => {
                // Cannot overflow, usize > i32, and `-written` is non-negative.
                let required_size = (-result) as usize;

                // If we fail the first time we resize the vector to the
                // correct size and try again.
                buffer.reserve_exact(required_size);

                let result = self.detokenize_raw(
                    tokens,
                    buffer.spare_capacity_mut(),
                    remove_special,
                    unparse_special,
                );

                // Should succeed, we know the vector was correctly sized now.
                debug_assert!(result.is_positive());

                result as usize
            }
            // Cannot overflow, usize > i32
            written => written as usize,
        };

        // SAFETY: llama.cpp has written `size` items into the spare capacity.
        unsafe { buffer.set_len(buffer.len() + size) };
    }

    fn detokenize_raw(
        &self,
        tokens: &[LlamaToken],
        buffer: &mut [MaybeUninit<u8>],
        remove_special: bool,
        unparse_special: bool,
    ) -> i32 {
        unsafe {
            koharu_llama_sys::llama_detokenize(
                self.vocab.as_ptr(),
                tokens.as_ptr().cast::<koharu_llama_sys::llama_token>(),
                // Assume the tokens are few enough to fit in `llama_detokenize`.
                // If not, the user tried to detokenize a >2GB large list of
                // tokens, which is gonna fail horribly elsewhere anyhow.
                tokens.len().try_into().expect("too many input tokens"),
                buffer.as_mut_ptr().cast::<c_char>(),
                // It's fine to clamp the buffer length here, we'll get an
                // error from llama.cpp anyhow if there are too many tokens to
                // detokenize.
                buffer.len().try_into().unwrap_or(i32::MAX),
                remove_special,
                unparse_special,
            )
        }
    }

    /// Detokenize a single token.
    ///
    /// This is a convenience method on top of [`token_to_piece_into`][Self::token_to_piece_into].
    pub fn token_to_piece(
        &self,
        token: LlamaToken,
        special: bool,
        lstrip: Option<NonZeroU16>,
    ) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(8);
        self.token_to_piece_into(token, &mut buffer, special, lstrip);
        buffer
    }

    /// Detokenize a single token into a caller-provided buffer.
    ///
    /// The output is placed into the vector's spare capacity, and the `Vec`
    /// will be resized if the output doesn't fit.
    ///
    /// Special tokens are rendered in the output if `special` is set.
    ///
    /// This will skip up to `lstrip` leading spaces before copying, which can
    /// be useful when encoding/decoding multiple tokens with
    /// 'add_space_prefix'.
    ///
    /// # Example
    ///
    /// Convert a stream of tokens to a string, with proper partial UTF-8
    /// decoding (such as for emoji support) and reusing buffers.
    ///
    /// ```no_run
    /// # let tokens: Vec<koharu_llama::token::LlamaToken> = todo!();
    /// # let vocab: koharu_llama::vocab::LlamaVocab<'_> = todo!();
    /// let mut buffer = Vec::new();
    /// let mut decoder = encoding_rs::UTF_8.new_decoder();
    /// let mut token_string = String::new();
    /// for token in tokens {
    ///     // Clear buffers.
    ///     buffer.clear();
    ///     token_string.clear();
    ///
    ///     // Get the token bytes.
    ///     vocab.token_to_piece_into(token, &mut buffer, true, None);
    ///
    ///     // Ensure the size of the string buffer is large enough.
    ///     let max_len = decoder.max_utf8_buffer_length(buffer.len()).unwrap();
    ///     token_string.reserve(max_len);
    ///
    ///     // Grab the parts of the buffer that's valid (and leave the rest
    ///     // for the next token to reuse).
    ///     let (result, read, _) = decoder.decode_to_string(&buffer, &mut token_string, false);
    ///     assert!(
    ///         matches!(result, encoding_rs::CoderResult::InputEmpty) && read == buffer.len(),
    ///         "UTF-8 decoder capacity bound must consume the complete token"
    ///     );
    ///
    ///     // Print the resulting partially decoded token.
    ///     println!("{token} --> {token_string}");
    /// }
    /// ```
    pub fn token_to_piece_into(
        &self,
        token: LlamaToken,
        buffer: &mut Vec<u8>,
        special: bool,
        lstrip: Option<NonZeroU16>,
    ) {
        let lstrip = lstrip.map_or(0, |it| i32::from(it.get()));
        let result = self.token_to_piece_raw(token, buffer.spare_capacity_mut(), special, lstrip);

        let size = match result {
            result if result.is_negative() => {
                // Cannot overflow, usize > i32, and `-written` is non-negative.
                let required_size = (-result) as usize;

                // If we fail the first time we resize the vector to the
                // correct size and try again.
                buffer.reserve_exact(required_size);

                let result =
                    self.token_to_piece_raw(token, buffer.spare_capacity_mut(), special, lstrip);

                // Should succeed, we know the vector was correctly sized now.
                debug_assert!(result.is_positive());

                result as usize
            }
            // Cannot overflow, usize > i32
            written => written as usize,
        };

        // SAFETY: llama.cpp has written `size` items into the spare capacity.
        unsafe { buffer.set_len(buffer.len() + size) };
    }

    fn token_to_piece_raw(
        &self,
        token: LlamaToken,
        buffer: &mut [MaybeUninit<u8>],
        special: bool,
        lstrip: i32,
    ) -> i32 {
        unsafe {
            koharu_llama_sys::llama_token_to_piece(
                self.vocab.as_ptr(),
                token.0,
                buffer.as_mut_ptr().cast::<c_char>(),
                // It's fine to clamp the buffer length here, we'll get an
                // error from llama.cpp anyhow if there are too many tokens to
                // detokenize.
                buffer.len().try_into().unwrap_or(i32::MAX),
                lstrip,
                special,
            )
        }
    }

    /// Get all tokens in the vocabulary.
    pub fn tokens(&self) -> impl Iterator<Item = LlamaToken> {
        (0..self.n_tokens()).map(LlamaToken::new)
    }
}

/// a rusty equivalent of `llama_vocab_type`
#[repr(u32)]
#[derive(Debug, Eq, Copy, Clone, PartialEq)]
pub enum VocabType {
    /// For models without vocab.
    NONE = koharu_llama_sys::LLAMA_VOCAB_TYPE_NONE as _,
    /// LLaMA tokenizer based on byte-level BPE with byte fallback.
    SPM = koharu_llama_sys::LLAMA_VOCAB_TYPE_SPM as _,
    /// GPT-2 tokenizer based on byte-level BPE.
    BPE = koharu_llama_sys::LLAMA_VOCAB_TYPE_BPE as _,
    /// BERT tokenizer based on WordPiece
    WPM = koharu_llama_sys::LLAMA_VOCAB_TYPE_WPM as _,
    /// T5 tokenizer based on Unigram
    UGM = koharu_llama_sys::LLAMA_VOCAB_TYPE_UGM as _,
    /// RWKV tokenizer based on greedy tokenization
    RWKV = koharu_llama_sys::LLAMA_VOCAB_TYPE_RWKV as _,
    /// PLaMo-2 tokenizer based on Aho-Corasick with dynamic programming
    PLAMO2 = koharu_llama_sys::LLAMA_VOCAB_TYPE_PLAMO2 as _,
    /// Dummy tokenizer for testing: rolling hash of fixed-size chunks -> tokens, tokens -> hex
    TEST = koharu_llama_sys::LLAMA_VOCAB_TYPE_TEST as _,
}

/// There was an error converting a `llama_vocab_type` to a `VocabType`.
#[derive(thiserror::Error, Debug, Eq, PartialEq)]
pub enum LlamaTokenTypeFromIntError {
    /// The value is not a valid `llama_token_type`. Contains the int value that was invalid.
    #[error("Unknown Value {0}")]
    UnknownValue(koharu_llama_sys::llama_vocab_type),
}

impl TryFrom<koharu_llama_sys::llama_vocab_type> for VocabType {
    type Error = LlamaTokenTypeFromIntError;

    fn try_from(value: koharu_llama_sys::llama_vocab_type) -> Result<Self, Self::Error> {
        match value {
            koharu_llama_sys::LLAMA_VOCAB_TYPE_BPE => Ok(VocabType::BPE),
            koharu_llama_sys::LLAMA_VOCAB_TYPE_SPM => Ok(VocabType::SPM),
            unknown => Err(LlamaTokenTypeFromIntError::UnknownValue(unknown)),
        }
    }
}
