use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use clap::{Parser, ValueEnum};
use koharu_config::Config;
use koharu_pipeline::{
    Committer, DetectionModel, Flux2KleinConfig, InpaintingModel, KoharuLayoutRFDetrSeg2XLConfig,
    OcrModel, Operation, Pipeline, PipelineConfig, Progress, Request, RoremMixedConfig, Scope,
    StageOutput, TranslationConfig,
};
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_renderer::Renderer;
use koharu_scene::{AssetInput, AssetMetadata, AssetRole, At, PageDraft, Session};
use koharu_translator::{GenerationConfig, Language, ModelSelection, Provider, ProvidersConfig};

#[derive(Debug, Parser)]
#[command(version, about = "Run Koharu's complete in-process pipeline")]
struct Arguments {
    #[arg(short, long, value_name = "INPUT")]
    input: PathBuf,

    #[arg(short, long, value_name = "OUTPUT")]
    output: PathBuf,

    #[arg(long, value_enum, default_value = "koharu-layout-rfdetr-seg-2xl")]
    detection: DetectionChoice,

    #[arg(long, value_enum, default_value = "paddleocr-vl-1.6")]
    ocr: OcrChoice,

    #[arg(long, value_enum, default_value = "lama")]
    inpainting: InpaintingChoice,

    #[arg(long, default_value = "en-US")]
    target_language: Language,

    #[arg(long)]
    translation_instructions: Option<String>,

    #[arg(long, default_value = "gemma4-12b-it")]
    llm: String,

    #[arg(long)]
    cpu: bool,
}

struct SessionCommitter<'a>(&'a mut Session);

#[async_trait::async_trait]
impl Committer for SessionCommitter<'_> {
    async fn commit(&mut self, output: StageOutput) -> Result<koharu_scene::Snapshot> {
        Ok(self.0.commit(output.patch).await?.snapshot)
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DetectionChoice {
    #[value(name = "koharu-layout-rfdetr-seg-2xl")]
    KoharuLayoutRFDetrSeg2XL,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OcrChoice {
    #[value(name = "paddleocr-vl-1.6")]
    PaddleOcrVl1_6,
    #[value(name = "manga-ocr")]
    MangaOcr,
    #[value(name = "baberu-ocr")]
    BaberuOcr,
    #[value(name = "hayai-ocr")]
    HayaiOcr,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InpaintingChoice {
    #[value(name = "lama")]
    LaMa,
    #[value(name = "aot-inpainting")]
    AotInpainting,
    #[value(name = "flux2-klein")]
    Flux2Klein,
    #[value(name = "rorem-mixed")]
    RoremMixed,
}

impl Arguments {
    fn pipeline_config(&self) -> PipelineConfig {
        PipelineConfig {
            detection: match self.detection {
                DetectionChoice::KoharuLayoutRFDetrSeg2XL => {
                    DetectionModel::KoharuLayoutRFDetrSeg2XL(
                        KoharuLayoutRFDetrSeg2XLConfig::default(),
                    )
                }
            },
            ocr: match self.ocr {
                OcrChoice::PaddleOcrVl1_6 => OcrModel::PaddleOcrVl1_6,
                OcrChoice::MangaOcr => OcrModel::MangaOcr,
                OcrChoice::BaberuOcr => OcrModel::BaberuOcr,
                OcrChoice::HayaiOcr => OcrModel::HayaiOcr,
            },
            translation: TranslationConfig {
                model: ModelSelection {
                    provider: Provider::Local,
                    model: Some(self.llm.clone()),
                    quantization: None,
                    vision: true,
                    reasoning: true,
                },
                generation: GenerationConfig::default(),
                target_language: self.target_language,
                instructions: self.translation_instructions.clone(),
            },
            inpainting: match self.inpainting {
                InpaintingChoice::LaMa => InpaintingModel::LaMa {},
                InpaintingChoice::AotInpainting => InpaintingModel::AotInpainting {},
                InpaintingChoice::Flux2Klein => {
                    InpaintingModel::Flux2Klein(Flux2KleinConfig::default())
                }
                InpaintingChoice::RoremMixed => {
                    InpaintingModel::RoremMixed(RoremMixedConfig::default())
                }
            },
            processor: Default::default(),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    initialize_with_retry().await;

    let source = fs::read(&arguments.input)
        .with_context(|| format!("failed to read {}", arguments.input.display()))?;
    let decoded = image::load_from_memory(&source).context("failed to decode input image")?;
    let name = arguments
        .input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("input");
    let mut session = Session::memory().await?;
    let mut page = None;
    let patch = session.snapshot().patch(|edit| {
        let id = edit.add_page(
            PageDraft::new(
                name,
                f64::from(decoded.width()),
                f64::from(decoded.height()),
            ),
            At::End,
        )?;
        edit.set_asset(
            id,
            &AssetRole::new("source")?,
            AssetInput::new(
                Arc::<[u8]>::from(source),
                image_media_type(&arguments.input),
                AssetMetadata {
                    width: Some(decoded.width()),
                    height: Some(decoded.height()),
                    attributes: BTreeMap::new(),
                },
            ),
        )?;
        page = Some(id);
        Ok(())
    })?;
    session.commit(patch).await?;
    let page = page.expect("page ID is assigned by the edit");

    let device = koharu_ml::device(arguments.cpu);
    let pipeline = Pipeline::from_config(
        Config::memory(arguments.pipeline_config()),
        Config::memory(ProvidersConfig::default()),
        device,
    )?;
    let snapshot = session.snapshot();
    let mut committer = SessionCommitter(&mut session);
    let report = pipeline
        .execute(
            snapshot,
            Request {
                operation: Operation::Full,
                scope: Scope::Pages(vec![page]),
                progress: Some(Arc::new(|event| {
                    if let Progress::Finished { stage, elapsed, .. } = event {
                        eprintln!("{stage} finished in {:.2}s", elapsed.as_secs_f64());
                    }
                })),
                ..Request::default()
            },
            &mut committer,
        )
        .await?;
    eprintln!("pipeline finished in {:.2}s", report.elapsed.as_secs_f64());

    let renderer = Renderer::new()?;
    let render_started = Instant::now();
    let snapshot = session.snapshot();
    let frame = renderer.render(&snapshot, page).await?;
    let rasterizer = Rasterizer::new()?;
    let raster = rasterizer.rasterize(&frame.raster_frame()?, RasterOptions::default())?;
    let render_elapsed = render_started.elapsed();
    raster
        .image
        .save(&arguments.output)
        .with_context(|| format!("failed to write {}", arguments.output.display()))?;
    eprintln!(
        "rendered {} in {:.2}s",
        arguments.output.display(),
        render_elapsed.as_secs_f64()
    );
    Ok(())
}

async fn initialize_with_retry() {
    let mut delay = Duration::from_secs(1);
    let mut attempt = 0_u64;
    loop {
        attempt += 1;
        match koharu_ml::init().await {
            Ok(()) => return,
            Err(error) => {
                let jitter = Duration::from_millis((attempt.wrapping_mul(137)) % 251);
                let wait = delay + jitter;
                eprintln!(
                    "runtime initialization attempt {attempt} failed: {error}; retrying in {:.1}s",
                    wait.as_secs_f64()
                );
                tokio::time::sleep(wait).await;
                delay = delay.saturating_mul(2).min(Duration::from_secs(30));
            }
        }
    }
}

fn image_media_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_flags_select_models() {
        let arguments = Arguments::try_parse_from([
            "run",
            "--input",
            "input.png",
            "--output",
            "output.png",
            "--ocr",
            "manga-ocr",
            "--inpainting",
            "flux2-klein",
        ])
        .unwrap();
        assert!(matches!(
            arguments.pipeline_config().ocr,
            OcrModel::MangaOcr
        ));
        assert!(matches!(
            arguments.pipeline_config().inpainting,
            InpaintingModel::Flux2Klein(_)
        ));
    }
}
