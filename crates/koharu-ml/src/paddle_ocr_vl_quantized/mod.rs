//! Quantized PaddleOCR-VL-1.6 on `koharu-llm`'s llama.cpp engine, so the
//! crops of a page decode together.

use std::io::Cursor;

use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};
use koharu_llm::{
    Size,
    rig::completion::{
        CompletionRequest, Message,
        message::{DocumentSourceKind, Image, ImageMediaType, UserContent},
    },
};
use koharu_runtime::Repository;
use serde::{Deserialize, Serialize};
use serde_json::json;

const REPOSITORY: Repository = Repository::model(
    "PaddlePaddle/PaddleOCR-VL-1.6-GGUF",
    "511b09642bb324401f15f97cc23bc67e8f0a291d",
);

const MAX_NEW_TOKENS: usize = 512;
const REPETITION_PENALTY: f32 = 1.2;

/// The task prompts the model card lists:
/// https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6/blob/66317acc4c9fc17bd154591ce650735cd2855f3e/README.md
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaddleOCRVLTask {
    Ocr,
    Table,
    Formula,
    Chart,
    Spotting,
    Seal,
}

impl PaddleOCRVLTask {
    fn prompt(self) -> &'static str {
        match self {
            Self::Ocr => "OCR:",
            Self::Table => "Table Recognition:",
            Self::Formula => "Formula Recognition:",
            Self::Chart => "Chart Recognition:",
            Self::Spotting => "Spotting:",
            Self::Seal => "Seal Recognition:",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaddleOCRVLResult {
    pub text: String,
}

/// The tokens one crop's request holds: the largest image PaddleOCR-VL
/// reads, 1,280 tokens at its processor's `max_pixels`, the task prompt, and
/// the answer.
const CONTEXT: u32 = 2048;

#[derive(Debug)]
pub struct PaddleOCRVLQuantized {
    model: koharu_llm::Model,
}

impl PaddleOCRVLQuantized {
    /// Loads the model to decode `sequences` crops together.
    pub async fn load(sequences: u32) -> Result<Self> {
        let [model, projector] = REPOSITORY
            .resolve([
                "PaddleOCR-VL-1.6-GGUF.gguf",
                "PaddleOCR-VL-1.6-GGUF-mmproj.gguf",
            ])
            .await
            .context("failed to resolve quantized PaddleOCR-VL-1.6 files")?;
        let model = koharu_llm::load(
            &model,
            Some(&projector),
            Size {
                sequences,
                context: CONTEXT,
            },
        )
        .await
        .context("failed to load quantized PaddleOCR-VL-1.6")?;
        Ok(Self { model })
    }

    /// Reads each image, the requests decoding together.
    pub async fn inference(
        &self,
        images: &[DynamicImage],
        task: PaddleOCRVLTask,
    ) -> Result<Vec<PaddleOCRVLResult>> {
        let requests = images
            .iter()
            .map(|image| request(image, task))
            .collect::<Result<Vec<_>>>()?;
        let replies = futures::future::try_join_all(
            requests
                .into_iter()
                .map(|request| self.model.completion.call(request)),
        )
        .await?;
        Ok(replies
            .into_iter()
            .map(|reply| PaddleOCRVLResult { text: reply.text() })
            .collect())
    }
}

/// The image, then the task prompt, greedy, with the libtorch port's
/// repetition penalty over the answer's length.
fn request(image: &DynamicImage, task: PaddleOCRVLTask) -> Result<CompletionRequest> {
    let mut png = Vec::new();
    image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
    let content = vec![
        UserContent::Image(Image {
            data: DocumentSourceKind::Raw(png),
            media_type: Some(ImageMediaType::PNG),
            detail: None,
            native: None,
        }),
        UserContent::text(task.prompt()),
    ];
    let mut request = CompletionRequest::new(Message::User { content });
    request.temperature = Some(0.0);
    request.max_tokens = Some(MAX_NEW_TOKENS as u64);
    request.additional_params = Some(json!({
        "repeat_penalty": REPETITION_PENALTY,
        "repeat_last_n": MAX_NEW_TOKENS,
    }));
    Ok(request)
}
