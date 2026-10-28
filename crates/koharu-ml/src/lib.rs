mod backend;
mod nn;

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
pub mod manga_ocr;
pub mod manga_text_mask;
pub mod paddle_ocr_vl;
pub mod paddle_ocr_vl_quantized;
pub mod pp_doclayout_v3;
pub mod pp_ocr_v6;
pub mod rorem_mixed;
pub mod speech_bubble_yolo11n;
pub mod speech_bubble_yolov8m;

pub use koharu_runtime::Device;

use tch::Tensor;

/// Transformers' `RepetitionPenaltyLogitsProcessor`: each score of a token in
/// its row of `ids`, `[batch, n]` on `scores`' device, is multiplied by
/// `penalty` when negative and divided by it otherwise.
/// https://github.com/huggingface/transformers/blob/598d8ba8baaec7fec5a22da0e2844c7bf4ea20e1/src/transformers/generation/logits_process.py#L409-L412
pub(crate) fn repetition_penalty(
    scores: &Tensor,
    ids: &Tensor,
    penalty: f64,
) -> tch::Result<Tensor> {
    let score = scores.f_gather(1, ids, false)?;
    let score = score
        .f_mul_scalar(penalty)?
        .f_where_self(&score.f_lt(0.0)?, &score.f_div_scalar(penalty)?)?;
    scores.f_scatter(1, ids, &score)
}

/// The rows' ids as one `[rows, n]` tensor on `device` for
/// [`repetition_penalty`], each row padded with its first id, which the
/// penalty then reads twice to the same effect.
pub(crate) fn penalty_ids(rows: &[Vec<i64>], device: tch::Device) -> tch::Result<Tensor> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut padded = Vec::with_capacity(rows.len() * width);
    for row in rows {
        let first = *row.first().ok_or_else(|| {
            tch::TchError::Shape("a repetition penalty row has no ids".to_owned())
        })?;
        padded.extend(
            row.iter()
                .copied()
                .chain(std::iter::repeat_n(first, width - row.len())),
        );
    }
    Tensor::f_from_slice(&padded)?
        .f_view([rows.len() as i64, width as i64])?
        .f_to_device(device)
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
