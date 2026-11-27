//! The model's pass over the page, window by window. The model is shown every
//! lettering in a window, and only the window's own come back, so each
//! lettering is redrawn once.

use anyhow::Result;
use image::{GrayImage, Luma, RgbImage, imageops};
use koharu_scene::Rect;
use tokio::runtime::Handle;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    Inpainter, Work, inside, model,
    windows::{self, Window},
};
use crate::{
    RunId,
    gpu::{Key, LoadOutcome, Models},
    stages::Stopped,
};

/// `work` with each window the model redraws pasted under its own letterings'
/// masks, and an edit model's windows kept whole.
#[allow(clippy::too_many_arguments)]
pub(super) fn windows(
    mut work: Work,
    models: &mut Models,
    key: &Key,
    run: RunId,
    model: Inpainter,
    prompt: &str,
    cancel: &CancellationToken,
    runtime: &Handle,
) -> Result<Work> {
    let (width, height) = work.image.dimensions();
    for window in windows::plan(&work.regions, width, height, model.pass()) {
        if cancel.is_cancelled() {
            return Err(Stopped.into());
        }
        let Rect {
            x,
            y,
            width: across,
            height: down,
        } = window.rect;
        let (x, y) = (x as u32, y as u32);
        let cut = imageops::crop_imm(&work.mask, x, y, across, down).to_image();
        let own = own(&work, &window, &cut);
        if own.pixels().all(|pixel| pixel.0[0] == 0) {
            continue;
        }
        let crop = imageops::crop_imm(&work.image, x, y, across, down).to_image();
        let crop_mp = f64::from(across) * f64::from(down) / 1_000_000.0;
        let (redrawn, generative) = models.run(
            key,
            &[run],
            Some(crop_mp),
            |device| async move {
                Ok(LoadOutcome {
                    model: model::load(model, device).await?,
                    teardown: None,
                    full_gpu: true,
                })
            },
            |inpainter| {
                // Stop ends the redraw under way as well as the windows after it.
                let _stop = inpainter.canceller().map(|canceller| {
                    let cancel = cancel.clone();
                    AbortOnDropHandle::new(runtime.spawn(async move {
                        cancel.cancelled().await;
                        canceller.cancel();
                    }))
                });
                let _span = tracing::info_span!("redraw", width = across, height = down).entered();
                Ok((
                    inpainter.inference(&crop, &cut, prompt)?,
                    inpainter.generative(),
                ))
            },
        )?;
        paste(&mut work.image, (x, y), &own, &redrawn);
        if generative {
            work.windows.push((window, redrawn));
        }
    }
    Ok(work)
}

/// The part of `cut`, the page's mask in the window, inside the letterings
/// the window holds.
fn own(work: &Work, window: &Window, cut: &GrayImage) -> GrayImage {
    let (x, y) = (window.rect.x as u32, window.rect.y as u32);
    GrayImage::from_fn(cut.width(), cut.height(), |cx, cy| {
        let held = window
            .regions
            .iter()
            .any(|&region| inside(work.regions[region].rect, x + cx, y + cy));
        if held {
            *cut.get_pixel(cx, cy)
        } else {
            Luma([0])
        }
    })
}

/// `redrawn` laid on `image` at `at` where `mask` is set.
fn paste(image: &mut RgbImage, (x, y): (u32, u32), mask: &GrayImage, redrawn: &RgbImage) {
    for (cx, cy, pixel) in mask.enumerate_pixels() {
        if pixel.0[0] != 0 {
            image.put_pixel(x + cx, y + cy, *redrawn.get_pixel(cx, cy));
        }
    }
}
