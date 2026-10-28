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
use serde_json::json;

use crate::paddle_ocr_vl::{
    MAX_NEW_TOKENS, PaddleOCRVLResult, PaddleOCRVLTask, REPETITION_PENALTY,
};

crate::model_repository!("PaddlePaddle/PaddleOCR-VL-1.6-GGUF" @ "511b09642bb324401f15f97cc23bc67e8f0a291d" {
    MODEL = "PaddleOCR-VL-1.6-GGUF.gguf",
    PROJECTOR = "PaddleOCR-VL-1.6-GGUF-mmproj.gguf",
});

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
        let (model, projector) = tokio::try_join!(MODEL.resolve(), PROJECTOR.resolve())
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
