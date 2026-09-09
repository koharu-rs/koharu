//! Safe wrapper around multimodal (MTMD) functionality in llama.cpp.
//!
//! This module provides Rust bindings for llama.cpp's multimodal support,
//! allowing processing of text, image, and audio inputs through a unified interface.
//!
//! # Warning
//! This API is experimental and subject to breaking changes.
use std::ffi::{CStr, CString};
use std::ptr::NonNull;
use std::slice;

use crate::context::LlamaContext;
use crate::model::LlamaModel;
use crate::token::LlamaToken;

/// Input chunk types for multimodal data
///
/// # Examples
///
/// ```
/// use koharu_llama::mtmd::MtmdInputChunkType;
///
/// let text_chunk = MtmdInputChunkType::Text;
/// let image_chunk = MtmdInputChunkType::Image;
/// let audio_chunk = MtmdInputChunkType::Audio;
///
/// assert_eq!(text_chunk, MtmdInputChunkType::Text);
/// assert_eq!(text_chunk, koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_TEXT.into());
/// assert_ne!(text_chunk, image_chunk);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MtmdInputChunkType {
    /// Text input chunk
    Text = koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_TEXT as _,
    /// Image input chunk
    Image = koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_IMAGE as _,
    /// Audio input chunk
    Audio = koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_AUDIO as _,
}

impl From<koharu_llama_sys::mtmd_input_chunk_type> for MtmdInputChunkType {
    fn from(chunk_type: koharu_llama_sys::mtmd_input_chunk_type) -> Self {
        match chunk_type {
            koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_TEXT => MtmdInputChunkType::Text,
            koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_IMAGE => MtmdInputChunkType::Image,
            koharu_llama_sys::MTMD_INPUT_CHUNK_TYPE_AUDIO => MtmdInputChunkType::Audio,
            _ => panic!("Unknown MTMD input chunk type: {chunk_type}"),
        }
    }
}

/// Configuration parameters for MTMD context
///
/// # Examples
///
/// ```no_run
/// use koharu_llama::mtmd::{MtmdContextParams, mtmd_default_marker};
/// use std::ffi::CString;
///
/// let params = MtmdContextParams {
///     use_gpu: false,
///     print_timings: true,
///     n_threads: 4,
///     media_marker: CString::new(mtmd_default_marker()).unwrap(),
///     image_min_tokens: -1,
///     image_max_tokens: -1,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct MtmdContextParams {
    /// Whether to use GPU acceleration
    pub use_gpu: bool,
    /// Whether to print timing information
    pub print_timings: bool,
    /// Number of threads to use for processing
    pub n_threads: i32,
    /// Media marker string used to identify media positions in text
    pub media_marker: CString,
    /// Minimum number of tokens used to represent an image.
    /// Controls the visual token budget lower bound. Use -1 for the model default.
    /// Gemma 4 supported budgets: 70, 140, 280, 560, 1120.
    pub image_min_tokens: i32,
    /// Maximum number of tokens used to represent an image.
    /// Controls the visual token budget upper bound. Use -1 for the model default.
    /// Lower values reduce memory and compute at the cost of visual detail.
    /// Gemma 4 supported budgets: 70, 140, 280, 560, 1120.
    pub image_max_tokens: i32,
}

impl Default for MtmdContextParams {
    fn default() -> Self {
        unsafe { koharu_llama_sys::mtmd_context_params_default() }.into()
    }
}

impl From<&MtmdContextParams> for koharu_llama_sys::mtmd_context_params {
    fn from(params: &MtmdContextParams) -> Self {
        let mut context = unsafe { koharu_llama_sys::mtmd_context_params_default() };
        let MtmdContextParams {
            use_gpu,
            print_timings,
            n_threads,
            media_marker,
            image_min_tokens,
            image_max_tokens,
        } = params;

        context.use_gpu = *use_gpu;
        context.print_timings = *print_timings;
        context.n_threads = *n_threads;
        context.media_marker = media_marker.as_ptr();
        context.image_min_tokens = *image_min_tokens;
        context.image_max_tokens = *image_max_tokens;

        context
    }
}

impl From<koharu_llama_sys::mtmd_context_params> for MtmdContextParams {
    fn from(params: koharu_llama_sys::mtmd_context_params) -> Self {
        Self {
            use_gpu: params.use_gpu,
            print_timings: params.print_timings,
            n_threads: params.n_threads,
            media_marker: unsafe { CStr::from_ptr(params.media_marker) }.to_owned(),
            image_min_tokens: params.image_min_tokens,
            image_max_tokens: params.image_max_tokens,
        }
    }
}

/// Text input configuration
///
/// # Examples
///
/// ```
/// use koharu_llama::mtmd::MtmdInputText;
///
/// let input = MtmdInputText {
///     text: "Describe this image.".to_string(),
///     add_special: true,
///     parse_special: true,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct MtmdInputText {
    /// The input text string
    pub text: String,
    /// Whether to add special tokens
    pub add_special: bool,
    /// Whether to parse special tokens
    pub parse_special: bool,
}

/// Safe wrapper around `mtmd_context`.
///
/// This represents an initialized multimodal context that can process
/// text, images, and audio through llama.cpp's multimodal interface.
#[derive(Debug)]
pub struct MtmdContext {
    pub(crate) context: NonNull<koharu_llama_sys::mtmd_context>,
}

// MtmdContext is thread safe
unsafe impl Send for MtmdContext {}
unsafe impl Sync for MtmdContext {}

impl MtmdContext {
    /// Initialize MTMD context from a multimodal projection file.
    ///
    /// # Arguments
    ///
    /// * `mmproj_path` - Path to the multimodal projection file
    /// * `text_model` - Reference to the text model
    /// * `params` - Configuration parameters for the MTMD context
    ///
    /// # Returns
    ///
    /// Returns `Ok(MtmdContext)` on success, or `Err(MtmdInitError)` on failure.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The path cannot be converted to a C string
    /// - The underlying C function returns null (indicating initialization failure)
    pub fn init_from_file(
        mmproj_path: &str,
        text_model: &LlamaModel,
        params: &MtmdContextParams,
    ) -> Result<Self, MtmdInitError> {
        let path_cstr = CString::new(mmproj_path)?;
        let ctx_params = koharu_llama_sys::mtmd_context_params::from(params);

        let context = unsafe {
            koharu_llama_sys::mtmd_init_from_file(
                path_cstr.as_ptr(),
                text_model.model.as_ptr(),
                ctx_params,
            )
        };

        let context = NonNull::new(context).ok_or(MtmdInitError::NullResult)?;
        Ok(Self { context })
    }

    /// Check whether non-causal attention mask is needed before `llama_decode`.
    #[must_use]
    pub fn decode_use_non_causal(&self) -> bool {
        unsafe {
            koharu_llama_sys::mtmd_decode_use_non_causal(self.context.as_ptr(), std::ptr::null())
        }
    }

    /// Check whether the current model uses M-RoPE for `llama_decode`.
    ///
    /// M-RoPE (Multimodal Rotary Position Embedding) affects how positions
    /// are calculated for multimodal inputs.
    #[must_use]
    pub fn decode_use_mrope(&self) -> bool {
        unsafe { koharu_llama_sys::mtmd_decode_use_mrope(self.context.as_ptr()) }
    }

    /// Check whether the current model supports vision input.
    #[must_use]
    pub fn support_vision(&self) -> bool {
        unsafe { koharu_llama_sys::mtmd_support_vision(self.context.as_ptr()) }
    }

    /// Check whether the current model supports audio input.
    #[must_use]
    pub fn support_audio(&self) -> bool {
        unsafe { koharu_llama_sys::mtmd_support_audio(self.context.as_ptr()) }
    }

    /// Get audio sample rate in Hz (e.g., 16000 for Whisper).
    /// Returns None if audio is not supported.
    #[must_use]
    pub fn get_audio_sample_rate(&self) -> Option<u32> {
        let rate = unsafe { koharu_llama_sys::mtmd_get_audio_sample_rate(self.context.as_ptr()) };
        (rate > 0).then_some(rate.unsigned_abs())
    }

    /// Backward-compatible alias for the audio sample rate getter.
    #[must_use]
    pub fn get_audio_bitrate(&self) -> Option<u32> {
        self.get_audio_sample_rate()
    }

    /// Tokenize input text and bitmaps into chunks.
    ///
    /// The input text must contain media markers (default: `<__media__>`) that will be
    /// replaced with the corresponding bitmap data from the `bitmaps` array.
    /// The number of bitmaps must equal the number of markers in the text.
    ///
    /// # Arguments
    ///
    /// * `text` - Text input configuration containing the text and tokenization options
    /// * `bitmaps` - Array of bitmaps (images/audio) to replace markers with
    ///
    /// # Returns
    ///
    /// Returns `Ok(MtmdInputChunks)` containing the tokenized chunks on success.
    ///
    /// # Errors
    ///
    /// * `BitmapCountMismatch` - Number of bitmaps doesn't match number of markers
    /// * `ImagePreprocessingError` - Error occurred during image preprocessing
    /// * `UnknownError` - Other tokenization error occurred
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use koharu_llama::mtmd::*;
    /// # fn example(ctx: &MtmdContext, bitmap: &MtmdBitmap) -> Result<(), Box<dyn std::error::Error>> {
    /// let text = MtmdInputText {
    ///     text: "Here is an image: <__media__>\nDescribe it.".to_string(),
    ///     add_special: true,
    ///     parse_special: true,
    /// };
    /// let chunks = ctx.tokenize(text, &[bitmap])?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn tokenize(
        &self,
        text: MtmdInputText,
        bitmaps: &[&MtmdBitmap],
    ) -> Result<MtmdInputChunks, MtmdTokenizeError> {
        let chunks = MtmdInputChunks::new();
        let text_cstring = CString::new(text.text)?;
        let input_text = koharu_llama_sys::mtmd_input_text {
            text: text_cstring.as_ptr(),
            text_len: text_cstring.as_bytes().len(),
            add_special: text.add_special,
            parse_special: text.parse_special,
        };

        // Create bitmap pointers
        let bitmap_ptrs: Vec<*const koharu_llama_sys::mtmd_bitmap> = bitmaps
            .iter()
            .map(|b| b.bitmap.as_ptr().cast_const())
            .collect();

        let result = unsafe {
            koharu_llama_sys::mtmd_tokenize(
                self.context.as_ptr(),
                chunks.chunks.as_ptr(),
                &raw const input_text,
                bitmap_ptrs.as_ptr().cast_mut(),
                bitmaps.len(),
            )
        };

        match result {
            0 => Ok(chunks),
            1 => Err(MtmdTokenizeError::BitmapCountMismatch),
            2 => Err(MtmdTokenizeError::ImagePreprocessingError),
            _ => Err(MtmdTokenizeError::UnknownError(result)),
        }
    }

    /// Encode a chunk for image/audio processing.
    ///
    /// This function processes image or audio chunks by encoding them into
    /// embeddings that can be used by the language model. The embeddings
    /// can be retrieved using `get_output_embeddings()`.
    ///
    /// # Arguments
    ///
    /// * `chunk` - The input chunk to encode (should be image or audio type)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success.
    ///
    /// # Errors
    ///
    /// Returns `MtmdEncodeError::EncodeFailure` if encoding fails.
    pub fn encode_chunk(&self, chunk: &MtmdInputChunk) -> Result<(), MtmdEncodeError> {
        let result = unsafe {
            koharu_llama_sys::mtmd_encode_chunk(self.context.as_ptr(), chunk.chunk.as_ptr())
        };

        if result == 0 {
            Ok(())
        } else {
            Err(MtmdEncodeError::EncodeFailure(result))
        }
    }
}

impl Drop for MtmdContext {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::mtmd_free(self.context.as_ptr()) }
    }
}

/// Safe wrapper around `mtmd_bitmap`.
///
/// Represents bitmap data for images or audio that can be processed
/// by the multimodal system. For images, data is stored in RGB format.
/// For audio, data is stored as PCM F32 samples.
#[derive(Debug, Clone)]
pub struct MtmdBitmap {
    pub(crate) bitmap: NonNull<koharu_llama_sys::mtmd_bitmap>,
}

// MtmdBitmap is thread safe
unsafe impl Send for MtmdBitmap {}
unsafe impl Sync for MtmdBitmap {}

impl MtmdBitmap {
    /// Create a bitmap from image data in RGB format.
    ///
    /// # Arguments
    ///
    /// * `nx` - Width of the image in pixels
    /// * `ny` - Height of the image in pixels
    /// * `data` - Image data in RGBRGBRGB... format (must be exactly `nx * ny * 3` bytes)
    ///
    /// # Returns
    ///
    /// Returns `Ok(MtmdBitmap)` on success.
    ///
    /// # Errors
    ///
    /// * `InvalidDataSize` - Data length doesn't match `nx * ny * 3`
    /// * `NullResult` - Underlying C function returned null
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use koharu_llama::mtmd::MtmdBitmap;
    ///
    /// // Create a 2x2 red image
    /// let red_pixel = [255, 0, 0]; // RGB values for red
    /// let image_data = red_pixel.repeat(4); // 2x2 = 4 pixels
    ///
    /// let bitmap = MtmdBitmap::from_image_data(2, 2, &image_data);
    /// assert!(bitmap.is_ok());
    /// ```
    pub fn from_image_data(nx: u32, ny: u32, data: &[u8]) -> Result<Self, MtmdBitmapError> {
        if data.len() != (nx * ny * 3) as usize {
            return Err(MtmdBitmapError::InvalidDataSize);
        }

        let bitmap = unsafe { koharu_llama_sys::mtmd_bitmap_init(nx, ny, data.as_ptr()) };

        let bitmap = NonNull::new(bitmap).ok_or(MtmdBitmapError::NullResult)?;
        Ok(Self { bitmap })
    }

    /// Create a bitmap from audio data in PCM F32 format.
    ///
    /// # Arguments
    ///
    /// * `data` - Audio samples as 32-bit floating point values
    ///
    /// # Returns
    ///
    /// Returns `Ok(MtmdBitmap)` on success.
    ///
    /// # Errors
    ///
    /// * `NullResult` - Underlying C function returned null
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use koharu_llama::mtmd::MtmdBitmap;
    ///
    /// // Create a simple sine wave audio sample
    /// let audio_data: Vec<f32> = (0..100)
    ///     .map(|i| (i as f32 * 0.1).sin())
    ///     .collect();
    ///
    /// let bitmap = MtmdBitmap::from_audio_data(&audio_data);
    /// // Note: This will likely fail without proper MTMD context setup
    /// ```
    pub fn from_audio_data(data: &[f32]) -> Result<Self, MtmdBitmapError> {
        let bitmap =
            unsafe { koharu_llama_sys::mtmd_bitmap_init_from_audio(data.len(), data.as_ptr()) };

        let bitmap = NonNull::new(bitmap).ok_or(MtmdBitmapError::NullResult)?;
        Ok(Self { bitmap })
    }

    /// Create a bitmap from a file.
    ///
    /// Supported formats:
    /// - Images: formats supported by `stb_image` (jpg, png, bmp, gif, etc.)
    /// - Audio: formats supported by miniaudio (wav, mp3, flac)
    ///
    /// Audio files are auto-detected based on magic bytes.
    ///
    /// # Arguments
    ///
    /// * `ctx` - MTMD context for processing
    /// * `path` - Path to the image or audio file
    /// * `placeholder` - If `true`, build a data-less bitmap (dimensions/length only, with no
    ///   decoded pixels or audio samples) — useful for counting tokens without loading the media.
    ///   If `false`, decode and load the actual data.
    ///
    /// # Returns
    ///
    /// Returns `Ok(MtmdBitmap)` on success.
    ///
    /// # Errors
    ///
    /// * `CStringError` - Path contains null bytes
    /// * `NullResult` - File could not be loaded or processed
    ///
    /// This function is thread-safe.
    pub fn from_file(
        ctx: &MtmdContext,
        path: &str,
        placeholder: bool,
    ) -> Result<Self, MtmdBitmapError> {
        let path_cstr = CString::new(path)?;
        let wrapper = unsafe {
            koharu_llama_sys::mtmd_helper_bitmap_init_from_file(
                ctx.context.as_ptr(),
                path_cstr.as_ptr(),
                placeholder,
                koharu_llama_sys::mtmd_helper_init_opt_default(),
            )
        };

        if !wrapper.video_ctx.is_null() {
            unsafe {
                if !wrapper.bitmap.is_null() {
                    koharu_llama_sys::mtmd_bitmap_free(wrapper.bitmap);
                }
                koharu_llama_sys::mtmd_helper_video_free(wrapper.video_ctx);
            }
            return Err(MtmdBitmapError::NullResult);
        }
        let bitmap = NonNull::new(wrapper.bitmap).ok_or(MtmdBitmapError::NullResult)?;
        Ok(Self { bitmap })
    }

    /// Create a bitmap from a buffer containing file data.
    ///
    /// Supported formats:
    /// - Images: formats supported by `stb_image` (jpg, png, bmp, gif, etc.)
    /// - Audio: formats supported by miniaudio (wav, mp3, flac)
    ///
    /// Audio files are auto-detected based on magic bytes.
    ///
    /// # Arguments
    ///
    /// * `ctx` - MTMD context for processing
    /// * `data` - Buffer containing the file data
    /// * `placeholder` - If `true`, build a data-less bitmap (dimensions/length only, with no
    ///   decoded pixels or audio samples) — useful for counting tokens without loading the media.
    ///   If `false`, decode and load the actual data.
    ///
    /// # Returns
    ///
    /// Returns `Ok(MtmdBitmap)` on success.
    ///
    /// # Errors
    ///
    /// * `NullResult` - Buffer could not be processed
    ///
    /// This function is thread-safe.
    pub fn from_buffer(
        ctx: &MtmdContext,
        data: &[u8],
        placeholder: bool,
    ) -> Result<Self, MtmdBitmapError> {
        let wrapper = unsafe {
            koharu_llama_sys::mtmd_helper_bitmap_init_from_buf(
                ctx.context.as_ptr(),
                data.as_ptr(),
                data.len(),
                placeholder,
                koharu_llama_sys::mtmd_helper_init_opt_default(),
            )
        };

        if !wrapper.video_ctx.is_null() {
            unsafe {
                if !wrapper.bitmap.is_null() {
                    koharu_llama_sys::mtmd_bitmap_free(wrapper.bitmap);
                }
                koharu_llama_sys::mtmd_helper_video_free(wrapper.video_ctx);
            }
            return Err(MtmdBitmapError::NullResult);
        }
        let bitmap = NonNull::new(wrapper.bitmap).ok_or(MtmdBitmapError::NullResult)?;
        Ok(Self { bitmap })
    }

    /// Get bitmap width in pixels.
    #[must_use]
    pub fn nx(&self) -> u32 {
        unsafe { koharu_llama_sys::mtmd_bitmap_get_nx(self.bitmap.as_ptr()) }
    }

    /// Get bitmap height in pixels.
    #[must_use]
    pub fn ny(&self) -> u32 {
        unsafe { koharu_llama_sys::mtmd_bitmap_get_ny(self.bitmap.as_ptr()) }
    }

    /// Get bitmap data as a byte slice.
    ///
    /// For images: RGB format with length `nx * ny * 3`
    /// For audio: PCM F32 format with length `n_samples * 4`
    #[must_use]
    pub fn data(&self) -> &[u8] {
        let ptr = unsafe { koharu_llama_sys::mtmd_bitmap_get_data(self.bitmap.as_ptr()) };
        let len = unsafe { koharu_llama_sys::mtmd_bitmap_get_n_bytes(self.bitmap.as_ptr()) };
        unsafe { slice::from_raw_parts(ptr, len) }
    }

    /// Check if this bitmap contains audio data (vs image data).
    #[must_use]
    pub fn is_audio(&self) -> bool {
        unsafe { koharu_llama_sys::mtmd_bitmap_is_audio(self.bitmap.as_ptr()) }
    }

    /// Get the bitmap's optional ID string.
    ///
    /// Bitmap ID is useful for KV cache tracking and can e.g. be calculated
    /// based on a hash of the bitmap data.
    #[must_use]
    pub fn id(&self) -> Option<String> {
        let ptr = unsafe { koharu_llama_sys::mtmd_bitmap_get_id(self.bitmap.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            let id = unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned();
            Some(id)
        }
    }

    /// Set the bitmap's ID string.
    ///
    /// Bitmap ID is useful for KV cache tracking and can e.g. be calculated
    /// based on a hash of the bitmap data.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID string to set
    ///
    /// # Errors
    ///
    /// Returns an error if the ID string contains null bytes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use koharu_llama::mtmd::MtmdBitmap;
    /// # fn example(bitmap: &MtmdBitmap) -> Result<(), Box<dyn std::error::Error>> {
    /// bitmap.set_id("image_001")?;
    /// assert_eq!(bitmap.id(), Some("image_001".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_id(&self, id: &str) -> Result<(), std::ffi::NulError> {
        let id_cstr = CString::new(id)?;
        unsafe {
            koharu_llama_sys::mtmd_bitmap_set_id(self.bitmap.as_ptr(), id_cstr.as_ptr());
        }
        Ok(())
    }
}

impl Drop for MtmdBitmap {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::mtmd_bitmap_free(self.bitmap.as_ptr()) }
    }
}

/// Safe wrapper around `mtmd_input_chunks`.
///
/// This is a collection of input chunks created from tokenizing text and media.
/// The chunks represent the tokenized input that can be processed by the model,
/// with text chunks containing tokens and media chunks containing embeddings.
#[derive(Debug)]
pub struct MtmdInputChunks {
    pub(crate) chunks: NonNull<koharu_llama_sys::mtmd_input_chunks>,
}

impl Default for MtmdInputChunks {
    fn default() -> Self {
        Self::new()
    }
}

impl MtmdInputChunks {
    /// Create a new empty input chunks collection
    /// # Panics
    /// This function will panic if the underlying llama.cpp function returns null,
    /// which should not happen.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use koharu_llama::mtmd::MtmdInputChunks;
    ///
    /// let chunks = MtmdInputChunks::new();
    /// assert_eq!(chunks.len(), 0);
    /// assert!(chunks.is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        let chunks = unsafe { koharu_llama_sys::mtmd_input_chunks_init() };
        let chunks = NonNull::new(chunks).unwrap();
        Self { chunks }
    }

    /// Get the number of chunks
    #[must_use]
    pub fn len(&self) -> usize {
        unsafe { koharu_llama_sys::mtmd_input_chunks_size(self.chunks.as_ptr()) }
    }

    /// Check if chunks collection is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get a chunk by index
    #[must_use]
    pub fn get(&self, index: usize) -> Option<MtmdInputChunk> {
        if index >= self.len() {
            return None;
        }

        let chunk_ptr =
            unsafe { koharu_llama_sys::mtmd_input_chunks_get(self.chunks.as_ptr(), index) };

        // Note: We don't own this chunk, it's owned by the chunks collection
        NonNull::new(chunk_ptr.cast_mut()).map(|ptr| MtmdInputChunk {
            chunk: ptr,
            owned: false,
        })
    }

    /// Get total number of tokens across all chunks.
    ///
    /// This is useful for keeping track of KV cache size.
    #[must_use]
    pub fn total_tokens(&self) -> usize {
        unsafe { koharu_llama_sys::mtmd_helper_get_n_tokens(self.chunks.as_ptr()) }
    }

    /// Get total position count across all chunks.
    ///
    /// This is useful to keep track of `n_past`. Normally `n_pos` equals `n_tokens`,
    /// but for M-RoPE it is different.
    #[must_use]
    pub fn total_positions(&self) -> i32 {
        unsafe { koharu_llama_sys::mtmd_helper_get_n_pos(self.chunks.as_ptr()) }
    }

    /// Evaluate chunks using the multimodal context and LLAMA context.
    ///
    /// This helper function automatically:
    /// 1. Runs `llama_decode()` on text chunks
    /// 2. Runs `mtmd_encode()` on image chunks, then `mtmd_get_output_embd()` and then `llama_decode()`
    ///
    /// If any of the `mtmd_encode()` or `llama_decode()` calls return non-zero, the function
    /// stops and forwards the error.
    ///
    /// # Arguments
    ///
    /// * `mtmd_ctx` - The multimodal context
    /// * `llama_ctx` - The LLAMA context
    /// * `n_past` - Current position in the sequence
    /// * `seq_id` - Sequence ID for the batch
    /// * `n_batch` - Batch size for processing
    /// * `logits_last` - Whether to compute logits for the last token only
    ///
    /// # Returns
    ///
    /// Returns the new `n_past` value on success.
    ///
    /// # Errors
    ///
    /// Returns `MtmdEvalError::EvalFailure` if any encoding or decoding operation fails.
    ///
    /// This function is NOT thread-safe.
    pub fn eval_chunks(
        &self,
        mtmd_ctx: &MtmdContext,
        llama_ctx: &LlamaContext,
        n_past: koharu_llama_sys::llama_pos,
        seq_id: koharu_llama_sys::llama_seq_id,
        n_batch: i32,
        logits_last: bool,
    ) -> Result<koharu_llama_sys::llama_pos, MtmdEvalError> {
        let mut new_n_past: koharu_llama_sys::llama_pos = 0;

        let result = unsafe {
            koharu_llama_sys::mtmd_helper_eval_chunks(
                mtmd_ctx.context.as_ptr(),
                llama_ctx.context.as_ptr(),
                self.chunks.as_ptr(),
                n_past,
                seq_id,
                n_batch,
                logits_last,
                &raw mut new_n_past,
            )
        };

        if result == 0 {
            Ok(new_n_past)
        } else {
            Err(MtmdEvalError::EvalFailure(result))
        }
    }
}

impl Drop for MtmdInputChunks {
    fn drop(&mut self) {
        unsafe { koharu_llama_sys::mtmd_input_chunks_free(self.chunks.as_ptr()) }
    }
}

/// Safe wrapper around `mtmd_input_chunk`.
///
/// Represents a single chunk of input data, which can be either text tokens,
/// image tokens, or audio tokens. The chunk type determines what kind of
/// data and operations are available.
#[derive(Debug)]
pub struct MtmdInputChunk {
    pub(crate) chunk: NonNull<koharu_llama_sys::mtmd_input_chunk>,
    owned: bool,
}

impl MtmdInputChunk {
    /// Get the type of this chunk
    #[must_use]
    pub fn chunk_type(&self) -> MtmdInputChunkType {
        let chunk_type =
            unsafe { koharu_llama_sys::mtmd_input_chunk_get_type(self.chunk.as_ptr()) };
        MtmdInputChunkType::from(chunk_type)
    }

    /// Get text tokens from this chunk.
    ///
    /// Only valid for text chunks. Returns `None` for image or audio chunks.
    ///
    /// # Returns
    ///
    /// Returns `Some(&[LlamaToken])` for text chunks, `None` otherwise.
    #[must_use]
    pub fn text_tokens(&self) -> Option<&[LlamaToken]> {
        if self.chunk_type() != MtmdInputChunkType::Text {
            return None;
        }

        let mut n_tokens = 0usize;
        let tokens_ptr = unsafe {
            koharu_llama_sys::mtmd_input_chunk_get_tokens_text(
                self.chunk.as_ptr(),
                &raw mut n_tokens,
            )
        };

        if tokens_ptr.is_null() || n_tokens == 0 {
            None
        } else {
            unsafe {
                Some(slice::from_raw_parts(
                    tokens_ptr.cast::<LlamaToken>(),
                    n_tokens,
                ))
            }
        }
    }

    /// Get the number of tokens in this chunk
    #[must_use]
    pub fn n_tokens(&self) -> usize {
        unsafe { koharu_llama_sys::mtmd_input_chunk_get_n_tokens(self.chunk.as_ptr()) }
    }

    /// Get the number of positions in this chunk.
    ///
    /// Returns the number of temporal positions (always 1 for M-RoPE, `n_tokens` otherwise).
    #[must_use]
    pub fn n_positions(&self) -> i32 {
        unsafe { koharu_llama_sys::mtmd_input_chunk_get_n_pos(self.chunk.as_ptr()) }
    }

    /// Get chunk ID if available.
    ///
    /// Returns `None` for text chunks, may return an ID for image/audio chunks.
    #[must_use]
    pub fn id(&self) -> Option<String> {
        let ptr = unsafe { koharu_llama_sys::mtmd_input_chunk_get_id(self.chunk.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
                .into()
        }
    }

    /// Create a copy of this chunk that you own.
    ///
    /// This is useful if you want to use custom logic to handle the chunk
    /// (e.g., KV cache management) by moving the chunk ownership to your own code.
    /// Remember to ensure the copied chunk is properly freed when you're done with it.
    ///
    /// # Returns
    ///
    /// Returns an owned copy of the chunk.
    ///
    /// # Errors
    ///
    /// Returns `MtmdInputChunkError::NullResult` if copying fails.
    pub fn copy(&self) -> Result<Self, MtmdInputChunkError> {
        let chunk = unsafe { koharu_llama_sys::mtmd_input_chunk_copy(self.chunk.as_ptr()) };
        let chunk = NonNull::new(chunk).ok_or(MtmdInputChunkError::NullResult)?;
        Ok(Self { chunk, owned: true })
    }
}

impl Drop for MtmdInputChunk {
    fn drop(&mut self) {
        if self.owned {
            unsafe { koharu_llama_sys::mtmd_input_chunk_free(self.chunk.as_ptr()) }
        }
    }
}

/// Get the default media marker string.
///
/// Returns the default marker used to identify media positions in text
/// (typically `"<__media__>"`). This marker should be used in your input text
/// to indicate where media content should be inserted.
///
/// # Returns
///
/// Returns the default media marker as a string slice.
///
/// # Examples
///
/// ```no_run
/// use koharu_llama::mtmd::mtmd_default_marker;
///
/// let marker = mtmd_default_marker();
/// assert!(!marker.is_empty());
///
/// let text = format!("Describe this image: {}", marker);
/// assert!(text.contains(marker));
/// ```
#[must_use]
pub fn mtmd_default_marker() -> &'static str {
    unsafe {
        let c_str = koharu_llama_sys::mtmd_default_marker();
        CStr::from_ptr(c_str).to_str().unwrap_or("<__media__>")
    }
}

// Error types
/// Errors that can occur when initializing MTMD context
#[derive(thiserror::Error, Debug)]
pub enum MtmdInitError {
    /// Failed to create `CString` from input
    #[error("Failed to create CString: {0}")]
    CStringError(#[from] std::ffi::NulError),
    /// MTMD context initialization returned null
    #[error("MTMD context initialization returned null")]
    NullResult,
}

/// Errors that can occur when working with MTMD bitmaps
#[derive(thiserror::Error, Debug)]
pub enum MtmdBitmapError {
    /// Failed to create `CString` from input
    #[error("Failed to create CString: {0}")]
    CStringError(#[from] std::ffi::NulError),
    /// Invalid data size for bitmap
    #[error("Invalid data size for bitmap")]
    InvalidDataSize,
    /// Bitmap creation returned null
    #[error("Bitmap creation returned null")]
    NullResult,
}

/// Errors that can occur when working with MTMD input chunks collections
#[derive(thiserror::Error, Debug)]
pub enum MtmdInputChunksError {
    /// Input chunks creation returned null
    #[error("Input chunks creation returned null")]
    NullResult,
}

/// Errors that can occur when working with individual MTMD input chunks
#[derive(thiserror::Error, Debug)]
pub enum MtmdInputChunkError {
    /// Input chunk operation returned null
    #[error("Input chunk operation returned null")]
    NullResult,
}

/// Errors that can occur during tokenization
#[derive(thiserror::Error, Debug)]
pub enum MtmdTokenizeError {
    /// Number of bitmaps does not match number of markers in text
    #[error("Number of bitmaps does not match number of markers")]
    BitmapCountMismatch,
    /// Image preprocessing error occurred
    #[error("Image preprocessing error")]
    ImagePreprocessingError,
    /// Text contains characters that cannot be converted to C string
    #[error("Failed to create CString from text: {0}")]
    CStringError(#[from] std::ffi::NulError),
    /// Unknown error occurred during tokenization
    #[error("Unknown error: {0}")]
    UnknownError(i32),
}

/// Errors that can occur during encoding
#[derive(thiserror::Error, Debug)]
pub enum MtmdEncodeError {
    /// Encode operation failed
    #[error("Encode failed with code: {0}")]
    EncodeFailure(i32),
}

/// Errors that can occur during evaluation
#[derive(thiserror::Error, Debug)]
pub enum MtmdEvalError {
    /// Evaluation operation failed
    #[error("Eval failed with code: {0}")]
    EvalFailure(i32),
}
