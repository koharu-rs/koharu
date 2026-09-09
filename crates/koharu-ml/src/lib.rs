use anyhow::Context;
use koharu_llama::llama_backend::LlamaBackend;
use koharu_runtime::{Feature, Hardware, Runtime};
use tokio::sync::OnceCell;

mod backend;

pub mod aot_inpainting;
pub mod baberu_ocr;
pub mod comic_layout_yolo26s;
pub mod comic_onomatopoeia;
pub mod comic_text_bubble_detector;
pub mod comic_text_detector;
pub mod flux2_klein;
pub mod font_detector;
pub mod hayai_ocr;
pub mod koharu_layout_rfdetr_seg_2xl;
pub mod lama;
pub mod llm;
pub mod manga_ocr;
pub mod manga_text_mask;
pub mod paddle_ocr_vl;
pub mod paddle_ocr_vl_quantized;
pub mod pp_doclayout_v3;
pub mod pp_ocr_v6;
pub mod rorem_mixed;
pub mod speech_bubble_yolo11n;
pub mod speech_bubble_yolov8m;

pub use koharu_diffusion as diffusion;
pub use koharu_llama as llama;
pub use koharu_runtime::{Backend, Device, DeviceType};
pub use koharu_torch as torch;

static LLAMA: OnceCell<LlamaBackend> = OnceCell::const_new();
static DIFFUSION: OnceCell<()> = OnceCell::const_new();
static READY: OnceCell<Device> = OnceCell::const_new();

/// Initializes every process-wide native runtime used by Koharu.
///
/// Concurrent callers share one attempt. A failed attempt may be retried; any
/// backend that completed successfully is retained and is not initialized
/// again. Retry timing belongs to the application bootstrap rather than this
/// deterministic, single-attempt API.
#[tracing::instrument(skip_all)]
pub async fn init() -> anyhow::Result<()> {
    READY
        .get_or_try_init(|| async {
            let runtime = Runtime::discover([Feature::Torch, Feature::Llama, Feature::Diffusion])?;
            let device = runtime
                .initialize()
                .await
                .context("failed to initialize runtimes")?;

            LLAMA
                .get_or_try_init(|| async {
                    koharu_llama::send_logs_to_tracing(koharu_llama::LogOptions::default());
                    LlamaBackend::init().context("failed to initialize llama.cpp backend")
                })
                .await?;
            DIFFUSION
                .get_or_try_init(|| async {
                    koharu_diffusion::send_logs_to_tracing()
                        .context("failed to redirect stable-diffusion.cpp logs")?;
                    Ok::<(), anyhow::Error>(())
                })
                .await?;
            Ok::<Device, anyhow::Error>(device)
        })
        .await?;
    Ok(())
}

/// Returns the runtime-owned device selected for every model backend.
#[must_use]
pub fn device(cpu: bool) -> Device {
    if cpu {
        Device::cpu()
    } else if let Some(device) = READY.get() {
        device.clone()
    } else if let Some(device) = Hardware::discover().device() {
        device.clone()
    } else {
        tracing::warn!("GPU is not available, falling back to CPU");
        Device::cpu()
    }
}

/// Returns the initialized process-wide llama.cpp backend.
#[must_use]
pub fn llama_backend() -> Option<&'static LlamaBackend> {
    LLAMA.get()
}

macro_rules! model_repository {
    ($repository:literal @ $revision:literal { $($name:ident = $filename:literal),+ $(,)? }) => {
        $(
            const $name: koharu_runtime::HuggingFaceFile<'static> =
                koharu_runtime::HuggingFaceFile::pinned(
                    $repository,
                    $revision,
                    $filename,
                );
        )+
    };
}

pub(crate) use model_repository;
