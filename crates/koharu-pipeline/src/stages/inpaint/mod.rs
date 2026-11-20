//! Inpainting: each block's clean-up, the scan redrawn without its
//! lettering under its mask, and a painted area's new raster layer. The fill
//! tiers rebuild what they can; the inpainting model redraws the rest in
//! windows. LaMa's redraw is kept under the mask. An edit model redraws its
//! whole window, and the clean-up or raster keeps it all, shown through the
//! mask, as Photoshop keeps a Generative Fill result whole under its layer
//! mask, so painting or hiding the mask shows more of it.

mod flat;
mod model;
mod read;
mod redraw;
mod telea;
mod tone;
mod windows;
mod write;

use std::sync::Arc;

use anyhow::Result;
use image::{GrayImage, RgbImage};
use indoc::indoc;
use koharu_scene::{LayerId, Rect, Tiles};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

pub use self::model::Inpainter;
use self::{
    flat::Fill,
    windows::{Region, Window},
};
use super::{Task, action, key};
use crate::{Area, PipelineConfig, Stage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct InpainterConfig {
    /// What cleans pages and the Remove brush's areas.
    pub model: Inpainter,
    /// What Generative Fill's bar picks and keeps for the next fill, as
    /// Photoshop's Generative Fill keeps its own model; a fill brings the one
    /// its bar shows, in `model`'s place for its run.
    pub fill: Inpainter,
    /// What the model is asked when a run brings no prompt of its own, as an
    /// area's can; LaMa, which takes none, ignores it.
    pub prompt: String,
}

impl Default for InpainterConfig {
    fn default() -> Self {
        Self {
            model: Inpainter::default(),
            // A fill draws what its prompt asks, which LaMa, the clean-up's
            // default, can't.
            fill: Inpainter::Flux3Klein,
            // An instruction, which Qwen-Image's edits read best. FLUX.3's
            // edits removed as much lettering with it as with a caption of the
            // result on seven page windows, and one sound effect less of seven
            // over busy art.
            prompt: indoc! {"
                Remove all the text, including dialogue, sound effects, and handwritten notes.
                Leave speech bubbles empty and keep everything else exactly the same."}
            .to_owned(),
        }
    }
}

/// A painted area to inpaint, and its own prompt.
pub(crate) struct AreaWork {
    pub(crate) area: Area,
    pub(crate) prompt: Option<String>,
}

/// A page's scan as it's being filled, and what the model redraws in it.
struct Work {
    image: RgbImage,
    /// What the model redraws, over the page.
    mask: GrayImage,
    regions: Vec<Region>,
    target: Target,
    /// Each window an edit model redrew, whole, as it drew it.
    windows: Vec<(Window, RgbImage)>,
}

enum Target {
    Blocks(Vec<Cleanup>),
    /// The area's rectangle and its mask over it.
    Area {
        rect: Rect,
        mask: GrayImage,
    },
}

/// A block's clean-up: its grown mask over `rect`, the clean-up's tiles now,
/// and whether the model redraws it.
struct Cleanup {
    block: LayerId,
    cleanup: LayerId,
    old: Tiles,
    rect: Rect,
    mask: GrayImage,
    /// The tier that filled it, or `None` when the model redraws it.
    fill: Option<Fill>,
    /// Its region, when the model redraws it.
    region: Option<usize>,
}

/// Reads what the run inpaints, redraws it on the GPU, and writes the
/// clean-ups or the area's layer.
pub(crate) async fn inpaint(
    task: &Task,
    config: &PipelineConfig,
    area: Option<Arc<AreaWork>>,
) -> Result<()> {
    let member = &task.members[0];
    let (reader, painted) = (member.clone(), area.clone());
    let work = tokio::task::spawn_blocking(move || {
        let _span =
            tracing::info_span!("read", stage = %Stage::Inpaint, page = reader.index + 1).entered();
        match &painted {
            Some(area) => read::area(&reader, &area.area),
            None => read::blocks(&reader),
        }
    })
    .await??;
    let Some(work) = work else {
        return Ok(());
    };
    let key = key(Stage::Inpaint, config).expect("an inpainter is Koharu's");
    let model = config.inpainter.model;
    // An area's own prompt names its layer, as Photoshop names a Generative
    // Fill's layer after its prompt.
    let own = area.and_then(|area| area.prompt.clone());
    let name = own.clone().unwrap_or_default();
    let prompt = own.unwrap_or_else(|| config.inpainter.prompt.clone());
    let asked = prompt.clone();
    let (cancel, runtime) = (member.cancel.clone(), Handle::current());
    let work = task
        .inference(&[member], move |models| {
            // What the fill tiers rebuilt all of needs no model.
            if work.regions.is_empty() {
                return Ok(work);
            }
            let inpainter = models.get(&key, |device| model::load(model, device))?;
            // Stop ends the redraw under way as well as the windows after it.
            let _stop = inpainter.canceller().map(|canceller| {
                let cancel = cancel.clone();
                AbortOnDropHandle::new(runtime.spawn(async move {
                    cancel.cancelled().await;
                    canceller.cancel();
                }))
            });
            redraw::windows(work, inpainter.as_ref(), &asked, &cancel)
        })
        .await?;
    let page = member.page;
    let parameters = (!prompt.trim().is_empty()).then(|| serde_json::json!({"prompt": prompt}));
    let made = action("c2pa.created", &model.to_string(), parameters);
    member
        .apply(Stage::Inpaint, move |project| {
            write::ops(project, page, work, &made, &name)
        })
        .await
}

/// The page pixels `mask`, over `rect`, sets.
fn masked(rect: Rect, mask: &GrayImage) -> impl Iterator<Item = (u32, u32)> + '_ {
    let (x0, y0) = (rect.x as u32, rect.y as u32);
    mask.enumerate_pixels()
        .filter(|(_, _, pixel)| pixel.0[0] != 0)
        .map(move |(x, y, _)| (x0 + x, y0 + y))
}

/// Whether the page pixel at `x`, `y` is in `rect`.
fn inside(rect: Rect, x: u32, y: u32) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    (i64::from(rect.x)..i64::from(rect.x) + i64::from(rect.width)).contains(&x)
        && (i64::from(rect.y)..i64::from(rect.y) + i64::from(rect.height)).contains(&y)
}
