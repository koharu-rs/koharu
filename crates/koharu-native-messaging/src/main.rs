use anyhow::{Context, Result, anyhow};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};

use async_trait::async_trait;
use koharu_pipeline::{Operation, Pipeline, PipelineConfig, Request, RunStatus, Stage};
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_renderer::Renderer;
use koharu_scene::{AssetInput, AssetMetadata, AssetRole, PageDraft, Session};
use koharu_translator::{Language, Model, ProvidersConfig, Translator};

#[derive(Deserialize, Debug)]
#[serde(tag = "action", content = "payload")]
enum ExtensionRequest {
    #[serde(rename = "UploadChunk")]
    UploadChunk {
        #[serde(rename = "transferId")]
        transfer_id: String,
        index: usize,
        total: usize,
        data: String,
    },
    #[serde(rename = "Process")]
    Process {
        #[serde(rename = "transferId")]
        transfer_id: String,
        stages: Vec<Stage>,
    },
    #[serde(rename = "GetSettings")]
    GetSettings,
    #[serde(rename = "UpdateSettings")]
    UpdateSettings { pipeline: PipelineConfig },
}

#[derive(Serialize, Debug)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ExtensionResponse {
    ChunkReceived {
        #[serde(rename = "transferId")]
        transfer_id: String,
        index: usize,
    },
    Progress {
        #[serde(rename = "transferId")]
        transfer_id: String,
        stage: String,
        message: String,
    },
    DownloadChunk {
        #[serde(rename = "transferId")]
        transfer_id: String,
        index: usize,
        total: usize,
        data: String,
    },
    Success {
        #[serde(rename = "transferId")]
        transfer_id: String,
        #[serde(rename = "inpaintedImage")]
        inpainted_image: Option<String>, // Base64 png
        texts: Vec<TextOverlay>,
    },
    Settings {
        pipeline: PipelineConfig,
        #[serde(rename = "translationModels")]
        translation_models: Vec<Model>,
        providers: Vec<Choice>,
        languages: Vec<Choice>,
    },
    Error {
        #[serde(rename = "transferId")]
        transfer_id: Option<String>,
        message: String,
    },
}

#[derive(Serialize, Debug)]
struct Choice {
    id: String,
    name: String,
}

#[derive(Serialize, Debug, Clone)]
struct TextOverlay {
    text: String,
    points: Vec<Point>,
}

#[derive(Serialize, Debug, Clone)]
struct Point {
    x: f64,
    y: f64,
}

struct UploadSession {
    total: usize,
    chunks: HashMap<usize, Vec<u8>>,
}

struct AppState {
    pipeline: Pipeline,
    renderer: Renderer,
    rasterizer: Rasterizer,
    config_handle: koharu_config::Config<PipelineConfig>,
    translation_models: tokio::sync::OnceCell<Vec<Model>>,
    uploads: Mutex<HashMap<String, UploadSession>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let home_dir = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_owned());
    let log_path = std::path::PathBuf::from(home_dir)
        .join(".koharu")
        .join("native-host.log");
    std::fs::create_dir_all(log_path.parent().unwrap()).ok();
    let log_file = std::fs::File::create(log_path)?;

    tracing_subscriber::fmt()
        .with_writer(move || log_file.try_clone().unwrap())
        .init();

    tracing::info!("Starting Koharu Chrome Native Messaging Host...");

    koharu_ml::init()
        .await
        .context("Failed to initialize ML runtime")?;

    let device = koharu_ml::device(false);
    tracing::info!(?device, "Initialized compute device");

    let active_config = PipelineConfig::load()?;
    let mut config = {
        let config_read = active_config.read()?;
        tracing::info!(
            detection = ?config_read.detection,
            ocr = ?config_read.ocr,
            translation_model = ?config_read.translation.model,
            translation_target = ?config_read.translation.target_language,
            inpainting = ?config_read.inpainting,
            "Loaded system configuration"
        );
        config_read.clone()
    };

    // Fallback/Force Local (llama.cpp) if using a remote provider that fails or isn't set up
    if config.translation.model.provider != koharu_translator::Provider::Local {
        tracing::warn!(
            current_provider = ?config.translation.model.provider,
            "Translation provider is not Local (llama.cpp). Forcing Local provider default model."
        );
        config.translation.model.provider = koharu_translator::Provider::Local;
        config.translation.model.model = Some("lfm2.5-1.2b-instruct".to_owned());
        config.translation.model.quantization = Some("Q4_K_M".to_owned());
    }

    let config_handle = koharu_config::Config::memory(config);

    let pipeline = Pipeline::from_config(config_handle.clone(), ProvidersConfig::load()?, device)?;

    let renderer = Renderer::new()?;
    let rasterizer = Rasterizer::new()?;

    let state = Arc::new(AppState {
        pipeline,
        renderer,
        rasterizer,
        config_handle,
        translation_models: tokio::sync::OnceCell::new(),
        uploads: Mutex::new(HashMap::new()),
    });

    let mut stdin = tokio::io::stdin();

    let (responses, mut outbound) = mpsc::unbounded_channel::<ExtensionResponse>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(response) = outbound.recv().await {
            let bytes = match serde_json::to_vec(&response) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to serialize response");
                    continue;
                }
            };
            if let Err(e) = write_frame(&mut stdout, &bytes).await {
                tracing::error!(error = %e, "Failed to write response frame");
                break;
            }
        }
    });

    loop {
        match read_frame(&mut stdin).await {
            Ok(Some(frame)) => {
                let state_clone = state.clone();
                let request: Result<ExtensionRequest, serde_json::Error> =
                    serde_json::from_slice(&frame);
                match request {
                    Ok(req) => handle_request(state_clone, req, &responses).await,
                    Err(e) => {
                        let _ = responses.send(ExtensionResponse::Error {
                            transfer_id: None,
                            message: format!("Failed to parse request JSON: {}", e),
                        });
                    }
                }
            }
            Ok(None) => {
                tracing::info!("Stdin channel closed. Exiting host process.");
                break;
            }
            Err(e) => {
                tracing::error!(error = %e, "Error reading from stdin");
                break;
            }
        }
    }

    drop(responses);
    let _ = writer.await;

    Ok(())
}

type Responses = mpsc::UnboundedSender<ExtensionResponse>;

async fn handle_request(state: Arc<AppState>, request: ExtensionRequest, responses: &Responses) {
    match request {
        ExtensionRequest::UploadChunk {
            transfer_id,
            index,
            total,
            data,
        } => {
            let decoded = match BASE64_STANDARD.decode(data.trim()) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let _ = responses.send(ExtensionResponse::Error {
                        transfer_id: Some(transfer_id),
                        message: format!("Invalid base64: {}", e),
                    });
                    return;
                }
            };

            let mut uploads = state.uploads.lock().await;
            let entry = uploads
                .entry(transfer_id.clone())
                .or_insert_with(|| UploadSession {
                    total,
                    chunks: HashMap::new(),
                });

            entry.chunks.insert(index, decoded);

            let _ = responses.send(ExtensionResponse::ChunkReceived { transfer_id, index });
        }
        ExtensionRequest::Process {
            transfer_id,
            stages,
        } => {
            let image_bytes = {
                let mut uploads = state.uploads.lock().await;
                match uploads.remove(&transfer_id) {
                    Some(session) => {
                        let mut complete = Vec::new();
                        let mut missing = None;
                        for i in 0..session.total {
                            match session.chunks.get(&i) {
                                Some(chunk) => complete.extend_from_slice(chunk),
                                None => {
                                    missing = Some(i);
                                    break;
                                }
                            }
                        }
                        if let Some(i) = missing {
                            let _ = responses.send(ExtensionResponse::Error {
                                transfer_id: Some(transfer_id),
                                message: format!("Missing chunk index {}", i),
                            });
                            return;
                        }
                        complete
                    }
                    None => {
                        let _ = responses.send(ExtensionResponse::Error {
                            transfer_id: Some(transfer_id),
                            message: "No upload session found for this transferId".to_owned(),
                        });
                        return;
                    }
                }
            };

            if let Err(e) =
                run_pipeline(state.clone(), &transfer_id, image_bytes, stages, responses).await
            {
                let _ = responses.send(ExtensionResponse::Error {
                    transfer_id: Some(transfer_id),
                    message: format!("Pipeline execution failed: {:?}", e),
                });
            }
        }
        ExtensionRequest::GetSettings => send_settings(&state, responses).await,
        ExtensionRequest::UpdateSettings { pipeline } => match apply_settings(&state, pipeline) {
            Ok(()) => send_settings(&state, responses).await,
            Err(e) => {
                let _ = responses.send(ExtensionResponse::Error {
                    transfer_id: None,
                    message: format!("Failed to apply settings: {e:#}"),
                });
            }
        },
    }
}

fn apply_settings(state: &AppState, pipeline: PipelineConfig) -> Result<()> {
    pipeline.validate()?;
    *state.config_handle.write()? = pipeline;
    Ok(())
}

// Sends the current settings to the extension. Used in extension to configure the engine
async fn send_settings(state: &AppState, responses: &Responses) {
    let settings = async {
        let pipeline = state.config_handle.read()?.clone();
        let providers = ProvidersConfig::default()
            .entries()
            .into_iter()
            .map(|config| {
                let provider = config.provider();
                Choice {
                    id: provider.to_string(),
                    name: provider.name().to_owned(),
                }
            })
            .collect();
        let languages = Language::ALL
            .iter()
            .map(|language| Choice {
                id: language.tag().to_owned(),
                name: language.to_string(),
            })
            .collect();
        anyhow::Ok(ExtensionResponse::Settings {
            pipeline,
            translation_models: state
                .translation_models
                .get_or_try_init(Translator::models)
                .await?
                .clone(),
            providers,
            languages,
        })
    }
    .await;
    let _ = responses.send(settings.unwrap_or_else(|e| ExtensionResponse::Error {
        transfer_id: None,
        message: format!("Failed to read settings: {e:#}"),
    }));
}

fn progress_response(transfer_id: &str, progress: koharu_pipeline::Progress) -> ExtensionResponse {
    use koharu_pipeline::Progress;

    let (stage, message) = match progress {
        Progress::Started { stages, .. } => {
            let names: Vec<String> = stages.iter().map(ToString::to_string).collect();
            ("started".to_owned(), format!("Queued {}", names.join(", ")))
        }
        Progress::Loading { stage, model, .. } => (stage.to_string(), format!("Loading {}", model)),
        Progress::Running { stage, model, .. } => (stage.to_string(), format!("Running {}", model)),
        Progress::Finished { stage, elapsed, .. } => (
            stage.to_string(),
            format!("Done in {:.1}s", elapsed.as_secs_f64()),
        ),
        Progress::Skipped { stage, .. } => (stage.to_string(), "Skipped".to_owned()),
    };

    ExtensionResponse::Progress {
        transfer_id: transfer_id.to_owned(),
        stage,
        message,
    }
}

async fn run_pipeline(
    state: Arc<AppState>,
    transfer_id: &str,
    image_bytes: Vec<u8>,
    stages: Vec<Stage>,
    responses: &Responses,
) -> Result<()> {
    // 1. Create a scene session
    let mut session = Session::memory().await?;
    let mut edit = session.snapshot().edit();

    // Decode image dimensions to set up page correctly
    let (width, height) = (|| -> Result<(f64, f64), image::ImageError> {
        let reader =
            image::ImageReader::new(std::io::Cursor::new(&image_bytes)).with_guessed_format()?;
        let (w, h) = reader.into_dimensions()?;
        Ok((w as f64, h as f64))
    })()
    .unwrap_or((1000.0, 1000.0));

    // 2. Add page and source image asset
    let page = edit.add_page(
        PageDraft::new("web-capture", width, height),
        koharu_scene::At::End,
    )?;

    edit.set_asset(
        page,
        &AssetRole::new("source")?,
        AssetInput::new(
            Arc::<[u8]>::from(image_bytes),
            "image/png",
            AssetMetadata {
                width: Some(width as u32),
                height: Some(height as u32),
                attributes: std::collections::BTreeMap::new(),
            },
        ),
    )?;

    let patch = edit.finish()?;
    session.commit(patch).await?;

    // 3. Prepare execution request
    let progress_sink: koharu_pipeline::ProgressSink = {
        let responses = responses.clone();
        let transfer_id = transfer_id.to_owned();
        Arc::new(move |progress| {
            let _ = responses.send(progress_response(&transfer_id, progress));
        })
    };

    let request = Request {
        operation: Operation::Stages { stages },
        progress: Some(progress_sink),
        ..Request::default()
    };

    struct PipelineCommitter {
        session: Session,
    }

    #[async_trait]
    impl koharu_pipeline::Committer for PipelineCommitter {
        async fn commit(
            &mut self,
            output: koharu_pipeline::StageOutput,
        ) -> Result<koharu_scene::Snapshot> {
            self.session.commit(output.patch).await?;
            Ok(self.session.snapshot())
        }
    }

    let mut committer = PipelineCommitter { session };

    // 4. Run the pipeline
    let report = state
        .pipeline
        .execute(committer.session.snapshot(), request, &mut committer)
        .await
        .map_err(|e| anyhow!("Pipeline run error: {:?}", e))?;

    if report.status == RunStatus::Stopped {
        return Err(anyhow!("Pipeline execution was stopped."));
    }

    // 5. Render and rasterize the final page (containing both inpainted canvas and translated text)
    let final_snapshot = committer.session.snapshot();
    let frame = state.renderer.render(&final_snapshot, page).await?;
    let raster = state
        .rasterizer
        .rasterize(&frame.raster_frame()?, RasterOptions::default())?;

    // Encode to PNG bytes in memory
    let mut png_bytes = Vec::new();
    let mut writer = std::io::Cursor::new(&mut png_bytes);
    raster
        .image
        .write_to(&mut writer, image::ImageFormat::Png)?;
    let encoded = BASE64_STANDARD.encode(&png_bytes);

    // Chunk size: 500 KB (base64 characters = 500,000 bytes)
    const CHUNK_SIZE: usize = 500 * 1024;
    let total_chunks = (encoded.len() + CHUNK_SIZE - 1) / CHUNK_SIZE;

    for i in 0..total_chunks {
        let start = i * CHUNK_SIZE;
        let end = std::cmp::min(start + CHUNK_SIZE, encoded.len());
        let chunk = encoded[start..end].to_owned();

        let _ = responses.send(ExtensionResponse::DownloadChunk {
            transfer_id: transfer_id.to_owned(),
            index: i,
            total: total_chunks,
            data: chunk,
        });
    }

    Ok(())
}

async fn read_frame<R>(reader: &mut R) -> Result<Option<Vec<u8>>>
where
    R: AsyncReadExt + Unpin,
{
    let mut length_buf = [0u8; 4];
    match reader.read_exact(&mut length_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let length = u32::from_ne_bytes(length_buf) as usize;
    let mut data = vec![0u8; length];
    reader.read_exact(&mut data).await?;
    Ok(Some(data))
}

async fn write_frame<W>(writer: &mut W, data: &[u8]) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let len = data.len() as u32;
    writer.write_all(&len.to_ne_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}
