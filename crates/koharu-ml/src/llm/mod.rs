//! High-level local LLM inference backed by llama.cpp.

mod model;

use std::{num::NonZeroU32, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use image::DynamicImage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use self::model::Model;

pub use koharu_llama::model::params::LlamaLoadMode;

const DEFAULT_GPU_LAYERS: u32 = 1000;
const DEFAULT_MAX_TOKENS: usize = 512;
const DEFAULT_SEED: u32 = 299_792_458;

/// Marker consumed by llama.cpp's multimodal tokenizer for one attached media input.
#[must_use]
pub fn media_marker() -> &'static str {
    koharu_llama::mtmd::mtmd_default_marker()
}

/// A loaded GGUF language model with optional multimodal support.
#[derive(Debug)]
pub struct Llm {
    model: Model,
}

impl Llm {
    /// Loads a text-only GGUF model.
    pub async fn load(device: crate::Device, model_path: impl Into<PathBuf>) -> Result<Self> {
        Self::load_with_options(device, model_path, LoadOptions::default()).await
    }

    /// Loads a GGUF model with model and optional MTMD projector settings.
    pub async fn load_with_options(
        device: crate::Device,
        model_path: impl Into<PathBuf>,
        options: LoadOptions,
    ) -> Result<Self> {
        let model_path = model_path.into();
        let model = tokio_rayon::spawn(move || Model::new(&device, model_path, options)).await?;
        Ok(Self { model })
    }

    /// Returns the input modalities advertised by the loaded model and projector.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.model.capabilities()
    }

    /// Converts the model's beginning-of-sequence token to its textual form.
    pub fn bos_token(&self) -> Result<String> {
        self.model.bos_token()
    }

    /// Converts the model's end-of-sequence token to its textual form.
    pub fn eos_token(&self) -> Result<String> {
        self.model.eos_token()
    }

    /// Applies the GGUF chat template.
    pub fn render_chat_prompt(&self, messages: &[ChatMessage]) -> Result<String> {
        self.render_chat_prompt_with_options(messages, ChatTemplateOptions::default())
    }

    /// Applies the GGUF chat template with explicit generation and reasoning behavior.
    pub fn render_chat_prompt_with_options(
        &self,
        messages: &[ChatMessage],
        options: ChatTemplateOptions,
    ) -> Result<String> {
        self.model.render_chat_prompt(messages, options)
    }

    /// Generates unconstrained text for an input.
    pub fn inference(&self, input: &Input<'_>, options: &GenerationOptions) -> Result<Generation> {
        self.inference_with_callback(input, options, |_| Ok(GenerationControl::Continue))
    }

    /// Generates text and invokes `on_token` after each decoded token.
    pub fn inference_with_callback<F>(
        &self,
        input: &Input<'_>,
        options: &GenerationOptions,
        on_token: F,
    ) -> Result<Generation>
    where
        F: FnMut(TokenChunk<'_>) -> Result<GenerationControl>,
    {
        self.model.inference(input, options, None, on_token)
    }

    /// Generates JSON constrained by the supplied JSON Schema.
    pub fn inference_with_json_schema(
        &self,
        input: &Input<'_>,
        options: &GenerationOptions,
        schema: &serde_json::Value,
    ) -> Result<Generation> {
        self.model.inference(input, options, Some(schema), |_| {
            Ok(GenerationControl::Continue)
        })
    }

    /// Generates and deserializes JSON constrained to the schema of `T`.
    pub fn inference_structured<T>(
        &self,
        input: &Input<'_>,
        options: &GenerationOptions,
    ) -> Result<StructuredGeneration<T>>
    where
        T: DeserializeOwned + JsonSchema,
    {
        let schema = serde_json::to_value(schemars::schema_for!(T))
            .context("failed to serialize structured output schema")?;
        let generation = self.inference_with_json_schema(input, options, &schema)?;
        let value = serde_json::from_str(generation.text.trim())
            .context("failed to deserialize structured LLM output")?;
        Ok(StructuredGeneration { value, generation })
    }
}

/// Options used while loading a GGUF model.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// Number of model layers to offload when an accelerator is selected.
    pub gpu_layers: u32,
    /// Controls how llama.cpp reads and retains model data.
    pub load_mode: LlamaLoadMode,
    /// Overrides an incorrect end-of-sequence token advertised by a GGUF file.
    pub eos_token_id: Option<i32>,
    /// Optional multimodal projector configuration.
    pub mtmd: Option<MtmdOptions>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            gpu_layers: DEFAULT_GPU_LAYERS,
            load_mode: LlamaLoadMode::Auto,
            eos_token_id: None,
            mtmd: None,
        }
    }
}

/// Configuration for llama.cpp's multimodal (MTMD) projector.
#[derive(Debug, Clone)]
pub struct MtmdOptions {
    pub projector_path: PathBuf,
    /// `-1` selects the projector default.
    pub image_min_tokens: i32,
    /// `-1` selects the projector default.
    pub image_max_tokens: i32,
}

impl MtmdOptions {
    #[must_use]
    pub fn new(projector_path: impl Into<PathBuf>) -> Self {
        Self {
            projector_path: projector_path.into(),
            image_min_tokens: -1,
            image_max_tokens: -1,
        }
    }

    #[must_use]
    pub fn with_image_token_range(mut self, min: i32, max: i32) -> Self {
        self.image_min_tokens = min;
        self.image_max_tokens = max;
        self
    }
}

/// Input modalities supported by an [`Llm`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub vision: bool,
    pub audio: bool,
    /// PCM sample rate expected by an audio projector, when advertised.
    pub audio_sample_rate: Option<u32>,
}

impl Capabilities {
    #[must_use]
    pub fn multimodal(self) -> bool {
        self.vision || self.audio
    }
}

/// A raw prompt and any media referenced by its MTMD markers.
#[derive(Debug)]
pub struct Input<'a> {
    prompt: &'a str,
    media: Vec<Media<'a>>,
}

impl<'a> Input<'a> {
    #[must_use]
    pub fn new(prompt: &'a str) -> Self {
        Self {
            prompt,
            media: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_image(mut self, image: &'a DynamicImage) -> Self {
        self.media.push(Media::Image(image));
        self
    }

    #[must_use]
    pub fn with_audio(mut self, samples: &'a [f32]) -> Self {
        self.media.push(Media::Audio(samples));
        self
    }

    #[must_use]
    pub fn prompt(&self) -> &str {
        self.prompt
    }

    #[must_use]
    pub fn media(&self) -> &[Media<'a>] {
        &self.media
    }
}

/// Image or PCM F32 audio attached to an [`Input`].
#[derive(Debug, Clone, Copy)]
pub enum Media<'a> {
    Image(&'a DynamicImage),
    Audio(&'a [f32]),
}

/// Sampling and context settings for one inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationOptions {
    pub max_tokens: usize,
    pub temperature: f32,
    pub top_k: Option<usize>,
    pub top_p: Option<f32>,
    pub min_p: Option<f32>,
    pub seed: u32,
    pub repeat_penalty: f32,
    /// Number of recent tokens to penalize; `0` disables repeat penalties.
    pub repeat_last_n: i32,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
    /// Whether llama.cpp or MTMD should add the model's special prompt tokens.
    pub add_special: bool,
    pub n_ctx: Option<NonZeroU32>,
    pub n_batch: Option<u32>,
    pub n_ubatch: Option<u32>,
    pub n_threads: Option<i32>,
    pub n_threads_batch: Option<i32>,
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: 0.1,
            top_k: None,
            top_p: None,
            min_p: None,
            seed: DEFAULT_SEED,
            repeat_penalty: 1.2,
            repeat_last_n: 64,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            add_special: false,
            n_ctx: None,
            n_batch: None,
            n_ubatch: None,
            n_threads: None,
            n_threads_batch: None,
        }
    }
}

/// Text and accounting information produced by one inference call.
#[derive(Debug, Clone)]
pub struct Generation {
    pub text: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    pub prompt_duration: Duration,
    pub generation_duration: Duration,
    pub finish_reason: FinishReason,
}

impl Generation {
    pub(super) fn empty(finish_reason: FinishReason) -> Self {
        Self {
            text: String::new(),
            prompt_tokens: 0,
            generated_tokens: 0,
            prompt_duration: Duration::ZERO,
            generation_duration: Duration::ZERO,
            finish_reason,
        }
    }

    #[must_use]
    pub fn prompt_tokens_per_second(&self) -> f64 {
        rate(self.prompt_tokens, self.prompt_duration)
    }

    #[must_use]
    pub fn generated_tokens_per_second(&self) -> f64 {
        rate(self.generated_tokens, self.generation_duration)
    }
}

/// Why generation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    StopToken,
    Length,
    Callback,
}

/// Return value from a token callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationControl {
    Continue,
    Stop,
}

/// One newly generated token and the complete text generated through that token.
#[derive(Debug, Clone, Copy)]
pub struct TokenChunk<'a> {
    pub token: koharu_llama::token::LlamaToken,
    pub piece: &'a str,
    pub text: &'a str,
    pub generated_tokens: usize,
}

/// A schema-constrained generation and its deserialized value.
#[derive(Debug, Clone)]
pub struct StructuredGeneration<T> {
    pub value: T,
    pub generation: Generation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    #[must_use]
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(ChatRole::System, content)
    }

    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(ChatRole::User, content)
    }

    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Assistant, content)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
    /// A model-specific role name used by specialized translation templates.
    Named(String),
}

impl ChatRole {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Named(name) => name,
        }
    }
}

/// Options controlling GGUF chat-template application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatTemplateOptions {
    pub add_generation_prompt: bool,
    /// Exposes the standard `enable_thinking` variable to compatible templates.
    pub enable_thinking: bool,
}

impl Default for ChatTemplateOptions {
    fn default() -> Self {
        Self {
            add_generation_prompt: true,
            enable_thinking: false,
        }
    }
}

fn rate(tokens: usize, duration: Duration) -> f64 {
    if duration.as_secs_f64() > 0.0 {
        tokens as f64 / duration.as_secs_f64()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
    struct ExampleOutput {
        translated: String,
    }

    #[test]
    fn generated_schema_round_trips_structured_type() {
        let schema = serde_json::to_value(schemars::schema_for!(ExampleOutput)).unwrap();
        assert_eq!(schema["type"], "object");
        let value: ExampleOutput = serde_json::from_str(r#"{"translated":"hello"}"#).unwrap();
        assert_eq!(value.translated, "hello");
    }

    #[test]
    fn mtmd_defaults_to_projector_token_budget() {
        let options = MtmdOptions::new("projector.gguf");
        assert_eq!(options.image_min_tokens, -1);
        assert_eq!(options.image_max_tokens, -1);
    }

    #[test]
    fn load_options_do_not_override_eos_by_default() {
        assert_eq!(LoadOptions::default().eos_token_id, None);
    }

    #[test]
    fn named_chat_roles_preserve_model_specific_names() {
        let role = ChatRole::Named("Japanese".to_owned());
        assert_eq!(role.as_str(), "Japanese");
    }
}
