//! llama.cpp generation and MTMD evaluation.
//!
//! llama.cpp generation loop:
//! https://github.com/ggml-org/llama.cpp/blob/99f3dc32296f825fec94f202da1e9fede1e78cf9/examples/simple/simple.cpp
//! MTMD helper evaluation:
//! https://github.com/ggml-org/llama.cpp/blob/99f3dc32296f825fec94f202da1e9fede1e78cf9/tools/mtmd/mtmd-helper.cpp

use std::{
    fmt::{Debug, Formatter},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use anyhow::{Context, Result, bail, ensure};
use koharu_llama::{
    context::{
        LlamaContext,
        params::{LlamaAttentionType, LlamaContextParams},
    },
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel, params::LlamaModelParams},
    mtmd::{MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputChunks, MtmdInputText},
    sampling::LlamaSampler,
    token::LlamaToken,
};
use llguidance::{Matcher, ParserFactory, api::TopLevelGrammar};
use minijinja::{Environment, Error as TemplateError, ErrorKind as TemplateErrorKind, context};
use serde::Serialize;

use super::{
    Capabilities, ChatMessage, ChatTemplateOptions, FinishReason, Generation, GenerationControl,
    GenerationOptions, Input, LoadOptions, Media, TokenChunk,
};
use crate::Backend;

const DEFAULT_MAX_UBATCH: u32 = 512;
const CHAT_TEMPLATE_NAME: &str = "chat";

pub(super) struct Model {
    backend: &'static LlamaBackend,
    model: LlamaModel,
    mtmd: Option<Mutex<MtmdContext>>,
    capabilities: Capabilities,
    eos_token: LlamaToken,
    chat_template: ChatTemplate,
    llguidance_factory: OnceLock<std::result::Result<ParserFactory, String>>,
}

impl Debug for Model {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("model", &self.model)
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct ChatTemplate {
    environment: Environment<'static>,
    bos_token: String,
    eos_token: String,
}

#[derive(Serialize)]
struct TemplateMessage<'a> {
    role: &'a str,
    content: &'a str,
}

impl ChatTemplate {
    fn from_model(model: &LlamaModel, eos_token: LlamaToken) -> Result<Self> {
        let template = model
            .chat_template(None)
            .context("GGUF model does not contain a chat template")?;
        let source = template
            .to_string()
            .context("GGUF chat template is not valid UTF-8")?;
        let mut environment = Environment::new();
        minijinja_contrib::add_to_environment(&mut environment);
        environment
            .set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
        environment.add_function("raise_exception", raise_template_exception);
        environment
            .add_template_owned(CHAT_TEMPLATE_NAME, source)
            .map_err(anyhow::Error::msg)
            .context("failed to compile GGUF chat template")?;
        Ok(Self {
            environment,
            bos_token: token_text(model, model.token_bos())
                .context("failed to decode model BOS token for chat template")?,
            eos_token: token_text(model, eos_token)
                .context("failed to decode model EOS token for chat template")?,
        })
    }

    fn render(&self, messages: &[ChatMessage], options: ChatTemplateOptions) -> Result<String> {
        let messages = messages
            .iter()
            .map(|message| TemplateMessage {
                role: message.role.as_str(),
                content: &message.content,
            })
            .collect::<Vec<_>>();
        let template = self
            .environment
            .get_template(CHAT_TEMPLATE_NAME)
            .map_err(anyhow::Error::msg)
            .context("compiled GGUF chat template is unavailable")?;
        template
            .render(context! {
                messages => messages,
                bos_token => self.bos_token.as_str(),
                eos_token => self.eos_token.as_str(),
                add_generation_prompt => options.add_generation_prompt,
                enable_thinking => options.enable_thinking,
                tools => Vec::<()>::new(),
                documents => Vec::<()>::new(),
            })
            .map_err(anyhow::Error::msg)
            .context("failed to render GGUF chat template")
    }
}

fn raise_template_exception(message: String) -> std::result::Result<String, TemplateError> {
    Err(TemplateError::new(
        TemplateErrorKind::InvalidOperation,
        message,
    ))
}

impl Model {
    pub(super) fn new(
        device: &crate::Device,
        model_path: PathBuf,
        options: LoadOptions,
    ) -> Result<Self> {
        ensure_file(&model_path, "GGUF model")?;
        let backend = crate::llama_backend().context("llama.cpp backend is not initialized")?;
        let params = model_params(backend, device, &options)?;
        let model = LlamaModel::load_from_file(backend, &model_path, &params)
            .with_context(|| format!("failed to load GGUF model {}", model_path.display()))?;
        ensure!(
            model.has_decoder(),
            "GGUF model does not advertise a decoder graph"
        );

        let eos_token = options
            .eos_token_id
            .map_or_else(|| model.token_eos(), LlamaToken::new);
        let chat_template = ChatTemplate::from_model(&model, eos_token)?;
        let (mtmd, capabilities) = match options.mtmd {
            Some(options) => {
                ensure_file(&options.projector_path, "MTMD projector")?;
                validate_image_token_range(options.image_min_tokens, options.image_max_tokens)?;
                let projector_path = options.projector_path.to_str().with_context(|| {
                    format!(
                        "invalid MTMD projector path {}",
                        options.projector_path.display()
                    )
                })?;
                let mtmd = MtmdContext::init_from_file(
                    projector_path,
                    &model,
                    &MtmdContextParams {
                        use_gpu: device.backend != Backend::Cpu && backend.supports_gpu_offload(),
                        print_timings: false,
                        n_threads: available_threads(),
                        image_min_tokens: options.image_min_tokens,
                        image_max_tokens: options.image_max_tokens,
                        ..MtmdContextParams::default()
                    },
                )
                .context("failed to initialize MTMD projector")?;
                let capabilities = Capabilities {
                    vision: mtmd.support_vision(),
                    audio: mtmd.support_audio(),
                    audio_sample_rate: mtmd.get_audio_sample_rate(),
                };
                ensure!(
                    capabilities.multimodal(),
                    "MTMD projector advertises neither vision nor audio support"
                );
                (Some(Mutex::new(mtmd)), capabilities)
            }
            None => (None, Capabilities::default()),
        };

        tracing::info!(
            path = %model_path.display(),
            has_encoder = model.has_encoder(),
            has_decoder = model.has_decoder(),
            vision = capabilities.vision,
            audio = capabilities.audio,
            "loaded llama.cpp model"
        );
        Ok(Self {
            backend,
            model,
            mtmd,
            capabilities,
            eos_token,
            chat_template,
            llguidance_factory: OnceLock::new(),
        })
    }

    pub(super) fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    pub(super) fn bos_token(&self) -> Result<String> {
        token_text(&self.model, self.model.token_bos()).context("failed to decode model BOS token")
    }

    pub(super) fn eos_token(&self) -> Result<String> {
        token_text(&self.model, self.eos_token).context("failed to decode model EOS token")
    }

    pub(super) fn render_chat_prompt(
        &self,
        messages: &[ChatMessage],
        options: ChatTemplateOptions,
    ) -> Result<String> {
        self.chat_template.render(messages, options)
    }

    pub(super) fn inference<F>(
        &self,
        input: &Input<'_>,
        options: &GenerationOptions,
        json_schema: Option<&serde_json::Value>,
        mut on_token: F,
    ) -> Result<Generation>
    where
        F: FnMut(TokenChunk<'_>) -> Result<GenerationControl>,
    {
        validate_generation_options(options)?;
        if options.max_tokens == 0 {
            return Ok(Generation::empty(FinishReason::Length));
        }

        let mtmd = if input.media().is_empty() {
            None
        } else {
            Some(
                self.mtmd
                    .as_ref()
                    .context("media input requires an MTMD projector")?
                    .lock()
                    .map_err(|_| anyhow::anyhow!("MTMD projector lock is poisoned"))?,
            )
        };
        let prepared = self.prepare_prompt(input, options.add_special, mtmd.as_deref())?;
        let context_config = context_config(&prepared, options)?;
        let mut context = self
            .model
            .new_context(self.backend, context_config.params)
            .context("failed to create llama.cpp context")?;
        let mut sampler = self.build_sampler(options, json_schema, &prepared.history_tokens)?;

        let prompt_start = Instant::now();
        let (mut next_token, mut position) = self.prefill(
            &prepared,
            mtmd.as_deref(),
            &mut context,
            &mut sampler,
            context_config.n_batch,
        )?;
        let prompt_duration = prompt_start.elapsed();
        drop(mtmd);

        let generation_start = Instant::now();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut text = String::new();
        let mut generated_tokens = 0;
        let mut finish_reason = FinishReason::Length;

        while generated_tokens < options.max_tokens {
            if self.should_stop(next_token) {
                finish_reason = FinishReason::StopToken;
                break;
            }

            let piece = self
                .model
                .token_to_piece(next_token, &mut decoder, true, None)
                .context("failed to decode generated token")?;
            text.push_str(&piece);
            generated_tokens += 1;

            if matches!(
                on_token(TokenChunk {
                    token: next_token,
                    piece: &piece,
                    text: &text,
                    generated_tokens,
                })?,
                GenerationControl::Stop
            ) {
                finish_reason = FinishReason::Callback;
                break;
            }
            if generated_tokens >= options.max_tokens {
                break;
            }

            let mut batch = LlamaBatch::new(1, 1);
            batch
                .add(next_token, position, &[0], true)
                .context("failed to build generated-token batch")?;
            context
                .decode(&mut batch)
                .context("failed to decode generated token")?;
            position = position
                .checked_add(1)
                .context("generated position exceeds i32")?;
            // llama_sampler_sample applies the chain and accepts the selected token.
            next_token = sampler.sample(&context, -1);
        }

        Ok(Generation {
            text,
            prompt_tokens: prepared.prompt_tokens,
            generated_tokens,
            prompt_duration,
            generation_duration: generation_start.elapsed(),
            finish_reason,
        })
    }

    fn prepare_prompt(
        &self,
        input: &Input<'_>,
        add_special: bool,
        mtmd: Option<&MtmdContext>,
    ) -> Result<PreparedPrompt> {
        let Some(mtmd) = mtmd else {
            let tokens = self
                .model
                .str_to_token(
                    input.prompt(),
                    if add_special {
                        AddBos::Always
                    } else {
                        AddBos::Never
                    },
                )
                .context("failed to tokenize prompt")?;
            ensure!(!tokens.is_empty(), "prompt produced no tokens");
            let prompt_tokens = tokens.len();
            return Ok(PreparedPrompt {
                history_tokens: tokens.clone(),
                kind: PreparedPromptKind::Text(tokens),
                prompt_positions: prompt_tokens,
                prompt_tokens,
                batch_tokens: prompt_tokens,
                non_causal: false,
            });
        };

        let mut bitmaps = Vec::with_capacity(input.media().len());
        for media in input.media() {
            bitmaps.push(match media {
                Media::Image(image) => {
                    ensure!(
                        self.capabilities.vision,
                        "MTMD projector does not support images"
                    );
                    ensure!(
                        image.width() > 0 && image.height() > 0,
                        "image dimensions must be non-zero"
                    );
                    let rgb = image.to_rgb8();
                    let (width, height) = rgb.dimensions();
                    MtmdBitmap::from_image_data(width, height, &rgb.into_raw())
                        .context("failed to create MTMD image bitmap")?
                }
                Media::Audio(samples) => {
                    ensure!(
                        self.capabilities.audio,
                        "MTMD projector does not support audio"
                    );
                    ensure!(!samples.is_empty(), "audio input must not be empty");
                    MtmdBitmap::from_audio_data(samples)
                        .context("failed to create MTMD audio bitmap")?
                }
            });
        }
        let bitmap_refs = bitmaps.iter().collect::<Vec<_>>();
        let chunks = mtmd
            .tokenize(
                MtmdInputText {
                    text: input.prompt().to_owned(),
                    add_special,
                    parse_special: true,
                },
                &bitmap_refs,
            )
            .context("failed to tokenize multimodal input")?;
        ensure!(
            !chunks.is_empty(),
            "multimodal tokenization produced no chunks"
        );

        let prompt_positions = usize::try_from(chunks.total_positions())
            .context("multimodal prompt positions overflow usize")?;
        let prompt_tokens = chunks.total_tokens();
        let batch_tokens = max_chunk_tokens(&chunks).max(1);
        let history_tokens = text_chunk_tokens(&chunks);
        Ok(PreparedPrompt {
            kind: PreparedPromptKind::Multimodal {
                chunks,
                _bitmaps: bitmaps,
            },
            history_tokens,
            prompt_positions,
            prompt_tokens,
            batch_tokens,
            non_causal: mtmd.decode_use_non_causal(),
        })
    }

    fn prefill(
        &self,
        prepared: &PreparedPrompt,
        mtmd: Option<&MtmdContext>,
        context: &mut LlamaContext<'_>,
        sampler: &mut LlamaSampler,
        n_batch: u32,
    ) -> Result<(LlamaToken, i32)> {
        match &prepared.kind {
            PreparedPromptKind::Text(tokens) => {
                let mut batch = LlamaBatch::new(tokens.len(), 1);
                batch
                    .add_sequence(tokens, 0, false)
                    .context("failed to build prompt batch")?;
                if self.model.has_encoder() {
                    context
                        .encode(&mut batch)
                        .context("failed to encode prompt batch")?;
                    self.start_decoder(context, sampler)
                } else {
                    context
                        .decode(&mut batch)
                        .context("failed to decode prompt batch")?;
                    let position = i32::try_from(tokens.len()).context("prompt is too long")?;
                    Ok((sampler.sample(context, -1), position))
                }
            }
            PreparedPromptKind::Multimodal { chunks, .. } => {
                let mtmd = mtmd.context("multimodal prompt requires an MTMD projector")?;
                let past = chunks
                    .eval_chunks(
                        mtmd,
                        context,
                        0,
                        0,
                        i32::try_from(n_batch).context("batch size exceeds i32")?,
                        true,
                    )
                    .context("failed to evaluate multimodal prompt")?;
                if self.model.has_encoder() {
                    self.start_decoder(context, sampler)
                } else {
                    Ok((sampler.sample(context, -1), past))
                }
            }
        }
    }

    fn start_decoder(
        &self,
        context: &mut LlamaContext<'_>,
        sampler: &mut LlamaSampler,
    ) -> Result<(LlamaToken, i32)> {
        let decoder_start = self.model.decode_start_token();
        let decoder_start = if decoder_start.0 >= 0 {
            decoder_start
        } else {
            self.model.token_bos()
        };
        ensure!(
            decoder_start.0 >= 0,
            "encoder-decoder model has no decoder start or BOS token"
        );
        let mut batch = LlamaBatch::new(1, 1);
        batch
            .add(decoder_start, 0, &[0], true)
            .context("failed to build decoder start batch")?;
        context
            .decode(&mut batch)
            .context("failed to decode model start token")?;
        Ok((sampler.sample(context, -1), 1))
    }

    fn build_sampler(
        &self,
        options: &GenerationOptions,
        json_schema: Option<&serde_json::Value>,
        history_tokens: &[LlamaToken],
    ) -> Result<LlamaSampler> {
        let mut samplers = Vec::new();

        if let Some(schema) = json_schema {
            let mut schema = schema.clone();
            if let Some(schema) = schema.as_object_mut() {
                schema.entry("x-guidance").or_insert_with(|| {
                    serde_json::json!({
                        "item_separator": ", ",
                        "key_separator": ": ",
                        "whitespace_flexible": false
                    })
                });
            }
            let schema = serde_json::to_string(&schema)
                .context("failed to serialize structured output schema")?;
            let factory = self
                .llguidance_factory
                .get_or_init(|| {
                    let tok_env = LlamaSampler::llguidance_tok_env(&self.model);
                    ParserFactory::new_simple(&tok_env).map_err(|error| error.to_string())
                })
                .as_ref()
                .map_err(|error| anyhow::Error::msg(error.clone()))
                .context("failed to initialize structured output tokenizer")?;
            let grammar = TopLevelGrammar::from_tagged_str("json_schema", &schema)
                .context("failed to compile structured output schema")?;
            let parser = factory
                .create_parser(grammar)
                .context("failed to create structured output parser")?;
            samplers.push(LlamaSampler::from(Matcher::new(Ok(parser))));
        }

        let has_repeat =
            (options.repeat_penalty - 1.0).abs() >= f32::EPSILON && options.repeat_last_n != 0;
        let has_frequency = options.frequency_penalty.abs() >= f32::EPSILON;
        let has_presence = options.presence_penalty.abs() >= f32::EPSILON;
        if has_repeat || has_frequency || has_presence {
            let mut penalties = LlamaSampler::penalties(
                &self.model,
                options.repeat_last_n,
                if has_repeat {
                    options.repeat_penalty
                } else {
                    1.0
                },
                options.frequency_penalty,
                options.presence_penalty,
            );
            penalties.accept_many(history_tokens);
            samplers.push(penalties);
        }

        if options.temperature <= 0.0 {
            samplers.push(LlamaSampler::greedy());
            return Ok(LlamaSampler::chain_simple(samplers));
        }
        if let Some(top_k) = options.top_k.filter(|value| *value > 0) {
            samplers.push(LlamaSampler::top_k(
                i32::try_from(top_k).unwrap_or(i32::MAX),
            ));
        }
        if let Some(top_p) = options.top_p {
            samplers.push(LlamaSampler::top_p(top_p, 1));
        }
        if let Some(min_p) = options.min_p.filter(|value| *value > 0.0) {
            samplers.push(LlamaSampler::min_p(min_p, 1));
        }
        samplers.push(LlamaSampler::temp(options.temperature));
        samplers.push(LlamaSampler::dist(options.seed));
        Ok(LlamaSampler::chain_simple(samplers))
    }

    fn should_stop(&self, token: LlamaToken) -> bool {
        token == self.eos_token || self.model.is_eog_token(token)
    }
}

struct PreparedPrompt {
    kind: PreparedPromptKind,
    history_tokens: Vec<LlamaToken>,
    prompt_positions: usize,
    prompt_tokens: usize,
    batch_tokens: usize,
    non_causal: bool,
}

enum PreparedPromptKind {
    Text(Vec<LlamaToken>),
    Multimodal {
        chunks: MtmdInputChunks,
        _bitmaps: Vec<MtmdBitmap>,
    },
}

struct ContextConfig {
    params: LlamaContextParams,
    n_batch: u32,
}

fn model_params(
    backend: &LlamaBackend,
    device: &crate::Device,
    options: &LoadOptions,
) -> Result<LlamaModelParams> {
    let gpu_layers = if device.backend == Backend::Cpu || !backend.supports_gpu_offload() {
        0
    } else {
        options.gpu_layers
    };
    let mut params = LlamaModelParams::default()
        .with_n_gpu_layers(gpu_layers)
        .with_load_mode(options.load_mode);
    if gpu_layers > 0 {
        params = params
            .with_devices(&[device.index])
            .context("failed to select llama.cpp accelerator")?;
    }
    Ok(params)
}

fn context_config(prepared: &PreparedPrompt, options: &GenerationOptions) -> Result<ContextConfig> {
    let values = context_values(prepared, options)?;
    let mut params = LlamaContextParams::default()
        .with_n_ctx(Some(values.n_ctx))
        .with_n_batch(values.n_batch)
        .with_n_ubatch(values.n_ubatch);
    if prepared.non_causal {
        params = params.with_attention_type(LlamaAttentionType::NonCausal);
    }
    if let Some(n_threads) = options.n_threads {
        params = params.with_n_threads(n_threads);
    }
    if let Some(n_threads_batch) = options.n_threads_batch {
        params = params.with_n_threads_batch(n_threads_batch);
    }
    Ok(ContextConfig {
        params,
        n_batch: values.n_batch,
    })
}

struct ContextValues {
    n_ctx: NonZeroU32,
    n_batch: u32,
    n_ubatch: u32,
}

fn context_values(prepared: &PreparedPrompt, options: &GenerationOptions) -> Result<ContextValues> {
    let required = prepared
        .prompt_positions
        .saturating_add(options.max_tokens)
        .saturating_add(1)
        .max(
            prepared
                .prompt_tokens
                .saturating_add(options.max_tokens)
                .saturating_add(1),
        )
        .max(prepared.batch_tokens.saturating_add(1));
    let n_ctx = match options.n_ctx {
        Some(n_ctx) if usize::try_from(n_ctx.get()).unwrap_or(usize::MAX) >= required => n_ctx,
        Some(_) => bail!(
            "n_ctx is too small: need at least {required} positions for prompt and generation"
        ),
        None => NonZeroU32::new(u32::try_from(required).context("context size exceeds u32")?)
            .expect("required context size is non-zero"),
    };

    let required_batch =
        u32::try_from(prepared.batch_tokens.max(1)).context("prompt batch size exceeds u32")?;
    let n_batch = options.n_batch.unwrap_or(required_batch).max(1);
    ensure!(
        n_batch >= required_batch,
        "n_batch is too small: need at least {required_batch} tokens"
    );
    let n_ubatch = if prepared.non_causal {
        n_batch
    } else {
        let n_ubatch = options
            .n_ubatch
            .unwrap_or_else(|| n_batch.min(DEFAULT_MAX_UBATCH))
            .max(1);
        ensure!(n_ubatch <= n_batch, "n_ubatch must not exceed n_batch");
        n_ubatch
    };

    Ok(ContextValues {
        n_ctx,
        n_batch,
        n_ubatch,
    })
}

fn validate_generation_options(options: &GenerationOptions) -> Result<()> {
    ensure!(
        options.temperature.is_finite() && options.temperature >= 0.0,
        "temperature must be finite and non-negative"
    );
    if let Some(top_p) = options.top_p {
        ensure!(
            top_p.is_finite() && (0.0..=1.0).contains(&top_p),
            "top_p must be between 0 and 1"
        );
    }
    if let Some(min_p) = options.min_p {
        ensure!(
            min_p.is_finite() && (0.0..=1.0).contains(&min_p),
            "min_p must be between 0 and 1"
        );
    }
    ensure!(
        options.repeat_penalty.is_finite() && options.repeat_penalty > 0.0,
        "repeat_penalty must be finite and positive"
    );
    ensure!(
        options.frequency_penalty.is_finite(),
        "frequency_penalty must be finite"
    );
    ensure!(
        options.presence_penalty.is_finite(),
        "presence_penalty must be finite"
    );
    ensure!(
        options.repeat_last_n >= 0,
        "repeat_last_n must be non-negative"
    );
    if let Some(n_threads) = options.n_threads {
        ensure!(n_threads > 0, "n_threads must be positive");
    }
    if let Some(n_threads_batch) = options.n_threads_batch {
        ensure!(n_threads_batch > 0, "n_threads_batch must be positive");
    }
    Ok(())
}

fn max_chunk_tokens(chunks: &MtmdInputChunks) -> usize {
    (0..chunks.len())
        .filter_map(|index| chunks.get(index))
        .map(|chunk| chunk.n_tokens())
        .max()
        .unwrap_or(0)
}

fn text_chunk_tokens(chunks: &MtmdInputChunks) -> Vec<LlamaToken> {
    let mut tokens = Vec::new();
    for index in 0..chunks.len() {
        if let Some(chunk) = chunks.get(index)
            && let Some(text_tokens) = chunk.text_tokens()
        {
            tokens.extend_from_slice(text_tokens);
        }
    }
    tokens
}

fn token_text(model: &LlamaModel, token: LlamaToken) -> Result<String> {
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    model
        .token_to_piece(token, &mut decoder, true, None)
        .map_err(Into::into)
}

fn ensure_file(path: &Path, description: &str) -> Result<()> {
    ensure!(
        path.exists(),
        "{description} does not exist: {}",
        path.display()
    );
    ensure!(
        path.is_file(),
        "{description} is not a file: {}",
        path.display()
    );
    Ok(())
}

fn validate_image_token_range(min: i32, max: i32) -> Result<()> {
    ensure!(min >= -1, "image_min_tokens must be -1 or non-negative");
    ensure!(max >= -1, "image_max_tokens must be -1 or non-negative");
    if min >= 0 && max >= 0 {
        ensure!(
            min <= max,
            "image_min_tokens must not exceed image_max_tokens"
        );
    }
    Ok(())
}

fn available_threads() -> i32 {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .try_into()
        .unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(tokens: usize, non_causal: bool) -> PreparedPrompt {
        PreparedPrompt {
            kind: PreparedPromptKind::Text(vec![LlamaToken(0); tokens]),
            history_tokens: Vec::new(),
            prompt_positions: tokens,
            prompt_tokens: tokens,
            batch_tokens: tokens,
            non_causal,
        }
    }

    #[test]
    fn context_includes_prompt_and_generation() {
        let options = GenerationOptions {
            max_tokens: 10,
            ..Default::default()
        };
        let config = context_values(&prepared(5, false), &options).unwrap();
        assert_eq!(config.n_ctx, NonZeroU32::new(16).unwrap());
        assert_eq!(config.n_batch, 5);
    }

    #[test]
    fn context_rejects_small_explicit_limits() {
        let options = GenerationOptions {
            max_tokens: 10,
            n_ctx: NonZeroU32::new(15),
            n_batch: Some(4),
            ..Default::default()
        };
        assert!(context_values(&prepared(5, false), &options).is_err());
    }

    #[test]
    fn non_causal_context_uses_full_ubatch() {
        let config = context_values(
            &prepared(1024, true),
            &GenerationOptions {
                max_tokens: 1,
                n_ubatch: Some(32),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(config.n_ubatch, 1024);
    }
}
