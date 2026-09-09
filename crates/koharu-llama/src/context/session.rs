//! utilities for working with session files

use crate::context::LlamaContext;
use crate::token::LlamaToken;
use std::ffi::{CString, NulError};
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};

/// Flags for state sequence operations.
///
/// These flags control what parts of the state are included when saving/restoring
/// sequence state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlamaStateSeqFlags(pub(crate) koharu_llama_sys::llama_state_seq_flags);

impl LlamaStateSeqFlags {
    /// Work only with partial states, such as SWA KV cache or recurrent cache (e.g. Mamba).
    ///
    /// This flag is useful when you only want to save/restore the recurrent state
    /// without affecting the KV cache.
    pub const PARTIAL_ONLY: LlamaStateSeqFlags = LlamaStateSeqFlags(1);

    /// Keep copied tensor data in device memory.
    ///
    /// Capturing another on-device state for the same sequence invalidates the previous one.
    pub const ON_DEVICE: LlamaStateSeqFlags = LlamaStateSeqFlags(2);

    /// Create an empty flags set.
    pub const fn empty() -> LlamaStateSeqFlags {
        LlamaStateSeqFlags(0)
    }

    /// Get the raw flags value.
    pub const fn bits(&self) -> u32 {
        self.0
    }

    /// Construct flags from their raw representation.
    pub const fn from_bits(bits: u32) -> LlamaStateSeqFlags {
        LlamaStateSeqFlags(bits)
    }

    /// Check if a flag is set.
    pub const fn contains(&self, other: LlamaStateSeqFlags) -> bool {
        (self.0 & other.0) != 0
    }
}

impl Default for LlamaStateSeqFlags {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::ops::BitOr for LlamaStateSeqFlags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Failed to save a sequence state file
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum SaveSeqStateError {
    /// llama.cpp failed to save the sequence state file
    #[error("Failed to save sequence state file")]
    FailedToSave,

    /// null byte in string
    #[error("null byte in string {0}")]
    NullError(#[from] NulError),

    /// failed to convert path to str
    #[error("failed to convert path {0} to str")]
    PathToStrError(PathBuf),
}

/// Failed to load a sequence state file
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum LoadSeqStateError {
    /// llama.cpp failed to load the sequence state file
    #[error("Failed to load sequence state file")]
    FailedToLoad,

    /// null byte in string
    #[error("null byte in string {0}")]
    NullError(#[from] NulError),

    /// failed to convert path to str
    #[error("failed to convert path {0} to str")]
    PathToStrError(PathBuf),

    /// Insufficient max length
    #[error("max_length is not large enough to hold {n_out} (was {max_tokens})")]
    InsufficientMaxLength {
        /// The length of the loaded sequence
        n_out: usize,
        /// The maximum length
        max_tokens: usize,
    },
}

/// Failed to save a Session file
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum SaveSessionError {
    /// llama.cpp failed to save the session file
    #[error("Failed to save session file")]
    FailedToSave,

    /// null byte in string
    #[error("null byte in string {0}")]
    NullError(#[from] NulError),

    /// failed to convert path to str
    #[error("failed to convert path {0} to str")]
    PathToStrError(PathBuf),
}

/// Failed to load a Session file
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum LoadSessionError {
    /// llama.cpp failed to load the session file
    #[error("Failed to load session file")]
    FailedToLoad,

    /// null byte in string
    #[error("null byte in string {0}")]
    NullError(#[from] NulError),

    /// failed to convert path to str
    #[error("failed to convert path {0} to str")]
    PathToStrError(PathBuf),

    /// Insufficient max length
    #[error("max_length is not large enough to hold {n_out} (was {max_tokens})")]
    InsufficientMaxLength {
        /// The length of the session file
        n_out: usize,
        /// The maximum length
        max_tokens: usize,
    },
}

impl LlamaContext<'_> {
    /// Save the current session to a file.
    ///
    /// # Parameters
    ///
    /// * `path_session` - The file to save to.
    /// * `tokens` - The tokens to associate the session with. This should be a prefix of a sequence of tokens that the context has processed, so that the relevant KV caches are already filled.
    ///
    /// # Errors
    ///
    /// Fails if the path is not a valid utf8, is not a valid c string, or llama.cpp fails to save the session file.
    #[deprecated(since = "0.1.136", note = "Use `state_save_file` instead")]
    pub fn save_session_file(
        &self,
        path_session: impl AsRef<Path>,
        tokens: &[LlamaToken],
    ) -> Result<(), SaveSessionError> {
        let path = path_session.as_ref();
        let path = path
            .to_str()
            .ok_or_else(|| SaveSessionError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;

        if unsafe {
            koharu_llama_sys::llama_save_session_file(
                self.context.as_ptr(),
                cstr.as_ptr(),
                tokens.as_ptr().cast::<koharu_llama_sys::llama_token>(),
                tokens.len(),
            )
        } {
            Ok(())
        } else {
            Err(SaveSessionError::FailedToSave)
        }
    }
    /// Load a session file into the current context.
    ///
    /// You still need to pass the returned tokens to the context for inference to work. What this function buys you is that the KV caches are already filled with the relevant data.
    ///
    /// # Parameters
    ///
    /// * `path_session` - The file to load from. It must be a session file from a compatible context, otherwise the function will error.
    /// * `max_tokens` - The maximum token length of the loaded session. If the session was saved with a longer length, the function will error.
    ///
    /// # Errors
    ///
    /// Fails if the path is not a valid utf8, is not a valid c string, or llama.cpp fails to load the session file. (e.g. the file does not exist, is not a session file, etc.)
    #[deprecated(since = "0.1.136", note = "Use `state_load_file` instead")]
    pub fn load_session_file(
        &mut self,
        path_session: impl AsRef<Path>,
        max_tokens: usize,
    ) -> Result<Vec<LlamaToken>, LoadSessionError> {
        let path = path_session.as_ref();
        let path = path
            .to_str()
            .ok_or(LoadSessionError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;
        let mut tokens: Vec<LlamaToken> = Vec::with_capacity(max_tokens);
        let mut n_out = 0;

        // SAFETY: cast is valid as LlamaToken is repr(transparent)
        let tokens_out = tokens.as_mut_ptr().cast::<koharu_llama_sys::llama_token>();

        let load_session_success = unsafe {
            koharu_llama_sys::llama_load_session_file(
                self.context.as_ptr(),
                cstr.as_ptr(),
                tokens_out,
                max_tokens,
                &mut n_out,
            )
        };
        if load_session_success {
            if n_out > max_tokens {
                return Err(LoadSessionError::InsufficientMaxLength { n_out, max_tokens });
            }
            // SAFETY: we checked that n_out <= max_tokens and llama.cpp promises that n_out tokens will be written
            unsafe {
                tokens.set_len(n_out);
            }
            Ok(tokens)
        } else {
            Err(LoadSessionError::FailedToLoad)
        }
    }

    /// Save the full state to a file.
    ///
    /// This is the non-deprecated replacement for [`save_session_file`](Self::save_session_file).
    ///
    /// # Parameters
    ///
    /// * `path_session` - The file to save to.
    /// * `tokens` - The tokens to associate the state with. This should be a prefix of a sequence
    ///   of tokens that the context has processed, so that the relevant KV caches are already filled.
    ///
    /// # Errors
    ///
    /// Fails if the path is not a valid utf8, is not a valid c string, or llama.cpp fails to save
    /// the state file.
    pub fn state_save_file(
        &self,
        path_session: impl AsRef<Path>,
        tokens: &[LlamaToken],
    ) -> Result<(), SaveSessionError> {
        let path = path_session.as_ref();
        let path = path
            .to_str()
            .ok_or_else(|| SaveSessionError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;

        if unsafe {
            koharu_llama_sys::llama_state_save_file(
                self.context.as_ptr(),
                cstr.as_ptr(),
                tokens.as_ptr().cast::<koharu_llama_sys::llama_token>(),
                tokens.len(),
            )
        } {
            Ok(())
        } else {
            Err(SaveSessionError::FailedToSave)
        }
    }

    /// Load a state file into the current context.
    ///
    /// This is the non-deprecated replacement for [`load_session_file`](Self::load_session_file).
    ///
    /// You still need to pass the returned tokens to the context for inference to work. What this
    /// function buys you is that the KV caches are already filled with the relevant data.
    ///
    /// # Parameters
    ///
    /// * `path_session` - The file to load from. It must be a state file from a compatible context,
    ///   otherwise the function will error.
    /// * `max_tokens` - The maximum token length of the loaded state. If the state was saved with a
    ///   longer length, the function will error.
    ///
    /// # Errors
    ///
    /// Fails if the path is not a valid utf8, is not a valid c string, or llama.cpp fails to load
    /// the state file.
    pub fn state_load_file(
        &mut self,
        path_session: impl AsRef<Path>,
        max_tokens: usize,
    ) -> Result<Vec<LlamaToken>, LoadSessionError> {
        let path = path_session.as_ref();
        let path = path
            .to_str()
            .ok_or(LoadSessionError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;
        let mut tokens: Vec<LlamaToken> = Vec::with_capacity(max_tokens);
        let mut n_out = 0;

        // SAFETY: cast is valid as LlamaToken is repr(transparent)
        let tokens_out = tokens.as_mut_ptr().cast::<koharu_llama_sys::llama_token>();

        let success = unsafe {
            koharu_llama_sys::llama_state_load_file(
                self.context.as_ptr(),
                cstr.as_ptr(),
                tokens_out,
                max_tokens,
                &mut n_out,
            )
        };
        if success {
            if n_out > max_tokens {
                return Err(LoadSessionError::InsufficientMaxLength { n_out, max_tokens });
            }
            // SAFETY: we checked that n_out <= max_tokens and llama.cpp promises that n_out tokens will be written
            unsafe {
                tokens.set_len(n_out);
            }
            Ok(tokens)
        } else {
            Err(LoadSessionError::FailedToLoad)
        }
    }

    /// Save state for a single sequence to a file.
    ///
    /// This enables saving state for individual sequences, which is useful for multi-sequence
    /// inference scenarios.
    ///
    /// # Parameters
    ///
    /// * `filepath` - The file to save to.
    /// * `seq_id` - The sequence ID whose state to save.
    /// * `tokens` - The tokens to associate with the saved state.
    ///
    /// # Errors
    ///
    /// Fails if the path is not a valid utf8, is not a valid c string, or llama.cpp fails to save
    /// the sequence state file.
    ///
    /// # Returns
    ///
    /// The number of bytes written on success.
    pub fn state_seq_save_file(
        &self,
        filepath: impl AsRef<Path>,
        seq_id: i32,
        tokens: &[LlamaToken],
    ) -> Result<usize, SaveSeqStateError> {
        let path = filepath.as_ref();
        let path = path
            .to_str()
            .ok_or_else(|| SaveSeqStateError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;

        let bytes_written = unsafe {
            koharu_llama_sys::llama_state_seq_save_file(
                self.context.as_ptr(),
                cstr.as_ptr(),
                seq_id,
                tokens.as_ptr().cast::<koharu_llama_sys::llama_token>(),
                tokens.len(),
            )
        };

        if bytes_written == 0 {
            Err(SaveSeqStateError::FailedToSave)
        } else {
            Ok(bytes_written)
        }
    }

    /// Load state for a single sequence from a file.
    ///
    /// This enables loading state for individual sequences, which is useful for multi-sequence
    /// inference scenarios.
    ///
    /// # Parameters
    ///
    /// * `filepath` - The file to load from.
    /// * `dest_seq_id` - The destination sequence ID to load the state into.
    /// * `max_tokens` - The maximum number of tokens to read.
    ///
    /// # Errors
    ///
    /// Fails if the path is not a valid utf8, is not a valid c string, or llama.cpp fails to load
    /// the sequence state file.
    ///
    /// # Returns
    ///
    /// A tuple of `(tokens, bytes_read)` on success.
    pub fn state_seq_load_file(
        &mut self,
        filepath: impl AsRef<Path>,
        dest_seq_id: i32,
        max_tokens: usize,
    ) -> Result<(Vec<LlamaToken>, usize), LoadSeqStateError> {
        let path = filepath.as_ref();
        let path = path
            .to_str()
            .ok_or(LoadSeqStateError::PathToStrError(path.to_path_buf()))?;

        let cstr = CString::new(path)?;
        let mut tokens: Vec<LlamaToken> = Vec::with_capacity(max_tokens);
        let mut n_out = 0;

        // SAFETY: cast is valid as LlamaToken is repr(transparent)
        let tokens_out = tokens.as_mut_ptr().cast::<koharu_llama_sys::llama_token>();

        let bytes_read = unsafe {
            koharu_llama_sys::llama_state_seq_load_file(
                self.context.as_ptr(),
                cstr.as_ptr(),
                dest_seq_id,
                tokens_out,
                max_tokens,
                &mut n_out,
            )
        };

        if bytes_read == 0 {
            return Err(LoadSeqStateError::FailedToLoad);
        }

        if n_out > max_tokens {
            return Err(LoadSeqStateError::InsufficientMaxLength { n_out, max_tokens });
        }

        // SAFETY: we checked that n_out <= max_tokens and llama.cpp promises that n_out tokens will be written
        unsafe {
            tokens.set_len(n_out);
        }

        Ok((tokens, bytes_read))
    }

    /// Returns the maximum size in bytes of the state (rng, logits, embedding
    /// and `kv_cache`) - will often be smaller after compacting tokens
    #[must_use]
    pub fn get_state_size(&self) -> usize {
        unsafe { koharu_llama_sys::llama_get_state_size(self.context.as_ptr()) }
    }

    /// Copies the state to the specified destination address.
    ///
    /// Returns the number of bytes copied
    ///
    /// # Safety
    ///
    /// Destination needs to have allocated enough memory.
    pub unsafe fn copy_state_data(&self, dest: *mut u8) -> usize {
        unsafe { koharu_llama_sys::llama_copy_state_data(self.context.as_ptr(), dest) }
    }

    /// Set the state reading from the specified address
    /// Returns the number of bytes read
    ///
    /// # Safety
    ///
    /// help wanted: not entirely sure what the safety requirements are here.
    pub unsafe fn set_state_data(&mut self, src: &[u8]) -> usize {
        unsafe { koharu_llama_sys::llama_set_state_data(self.context.as_ptr(), src.as_ptr()) }
    }

    /// Get the size of the state for a single sequence with optional flags.
    ///
    /// This is the extended version that supports flags for partial state operations.
    ///
    /// # Parameters
    ///
    /// * `seq_id` - The sequence ID to get the state size for.
    /// * `flags` - Optional flags (e.g., [`LlamaStateSeqFlags::PARTIAL_ONLY`]).
    ///
    /// # Returns
    ///
    /// The size in bytes needed to store the sequence state.
    #[must_use]
    pub fn state_seq_get_size_ext(&self, seq_id: i32, flags: LlamaStateSeqFlags) -> usize {
        unsafe {
            koharu_llama_sys::llama_state_seq_get_size_ext(self.context.as_ptr(), seq_id, flags.0)
        }
    }

    /// Copy the state of a single sequence into the specified buffer with optional flags.
    ///
    /// This is the extended version that supports flags for partial state operations.
    ///
    /// # Parameters
    ///
    /// * `dest` - Destination buffer to copy state into.
    /// * `seq_id` - The sequence ID to get the state for.
    /// * `flags` - Optional flags (e.g., [`LlamaStateSeqFlags::PARTIAL_ONLY`]).
    ///
    /// # Safety
    ///
    /// Destination needs to have allocated enough memory.
    ///
    /// # Returns
    ///
    /// The number of bytes copied.
    pub unsafe fn state_seq_get_data_ext(
        &self,
        dest: *mut u8,
        seq_id: i32,
        flags: LlamaStateSeqFlags,
    ) -> usize {
        unsafe {
            koharu_llama_sys::llama_state_seq_get_data_ext(
                self.context.as_ptr(),
                dest,
                usize::MAX,
                seq_id,
                flags.0,
            )
        }
    }

    /// Set the state for a single sequence from the specified buffer with optional flags.
    ///
    /// This is the extended version that supports flags for partial state operations.
    /// Useful for restoring only the recurrent/partial state without affecting the KV cache.
    ///
    /// # Parameters
    ///
    /// * `src` - Source buffer containing the state data.
    /// * `dest_seq_id` - The destination sequence ID to load the state into.
    /// * `flags` - Optional flags (e.g., [`LlamaStateSeqFlags::PARTIAL_ONLY`]).
    ///
    /// # Safety
    ///
    /// The source buffer must contain valid state data.
    ///
    /// # Returns
    ///
    /// Positive on success, zero on failure.
    pub unsafe fn state_seq_set_data_ext(
        &mut self,
        src: &[u8],
        dest_seq_id: i32,
        flags: LlamaStateSeqFlags,
    ) -> bool {
        unsafe {
            koharu_llama_sys::llama_state_seq_set_data_ext(
                self.context.as_ptr(),
                src.as_ptr(),
                src.len(),
                dest_seq_id,
                flags.0,
            ) > 0
        }
    }

    /// Capture an opaque snapshot of a single sequence's state.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StateSeqError::SizeMismatch`] if llama.cpp writes a different
    /// number of bytes than it reported.
    pub fn state_seq_get(
        &self,
        seq_id: i32,
        flags: LlamaStateSeqFlags,
    ) -> Result<SeqState, crate::StateSeqError> {
        let size = unsafe {
            koharu_llama_sys::llama_state_seq_get_size_ext(self.context.as_ptr(), seq_id, flags.0)
        };
        let mut bytes = vec![0; size];
        let actual = unsafe {
            koharu_llama_sys::llama_state_seq_get_data_ext(
                self.context.as_ptr(),
                bytes.as_mut_ptr(),
                size,
                seq_id,
                flags.0,
            )
        };
        if actual != size {
            return Err(crate::StateSeqError::SizeMismatch {
                expected: size,
                actual,
            });
        }
        Ok(SeqState { bytes, flags })
    }

    /// Restore a sequence state captured by [`Self::state_seq_get`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::StateSeqError::SizeMismatch`] if llama.cpp does not consume
    /// the complete snapshot.
    pub fn state_seq_set(
        &mut self,
        state: &SeqState,
        seq_id: i32,
    ) -> Result<(), crate::StateSeqError> {
        let actual = unsafe {
            koharu_llama_sys::llama_state_seq_set_data_ext(
                self.context.as_ptr(),
                state.bytes.as_ptr(),
                state.bytes.len(),
                seq_id,
                state.flags.0,
            )
        };
        if actual != state.bytes.len() {
            return Err(crate::StateSeqError::SizeMismatch {
                expected: state.bytes.len(),
                actual,
            });
        }
        Ok(())
    }
}

/// Opaque, immutable snapshot of a single sequence's state.
#[derive(Clone)]
pub struct SeqState {
    bytes: Vec<u8>,
    flags: LlamaStateSeqFlags,
}

impl SeqState {
    /// Flags used to capture this state.
    #[must_use]
    pub fn flags(&self) -> LlamaStateSeqFlags {
        self.flags
    }

    /// Serialized state size in bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

impl Debug for SeqState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeqState")
            .field("byte_len", &self.bytes.len())
            .field("flags", &self.flags)
            .finish()
    }
}
