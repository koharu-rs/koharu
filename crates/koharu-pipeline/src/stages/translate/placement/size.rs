//! The size a page is lettered at, measured as the renderer draws it, and how
//! a text that draws smaller is brought up to it. One its room holds larger
//! still fills its room.

use anyhow::Result;
use koharu_renderer::Renderer;
use koharu_scene::{Frame, LanguageIdentifier, LayerId, Role, Text};
use kurbo::Rect;
use rayon::prelude::*;

use super::{Lettering, boxed};

/// The margins a balloon's text may have, in ems, the widest first. Half holds
/// it 15 to 17% larger than the whole, and less puts letters on the balloon's
/// border.
const MARGINS: [f64; 5] = [1.0, 0.875, 0.75, 0.625, 0.5];
/// The steps a frame widens in, along its lines and across them.
const ALONG: u32 = 16;
const ACROSS: u32 = 4;

/// The middle of `values`, the larger of two.
pub(super) fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_unstable_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The size of the page `letterings` are on: the median of the largest its
/// texts' rooms hold in whole words, each measured from its sources' size,
/// `source`. Page medians agree within a work, where a size over its pages
/// followed the tightest one.
pub(super) fn page(letterings: &[Lettering], source: Option<f64>) -> Result<Option<f64>> {
    let mut held = letterings
        .par_iter()
        .filter(|lettering| lettering.block.role == Role::Text)
        .map(|lettering| {
            let mut text = lettering.text.clone();
            if let Some(widest) = lettering.widest {
                text.frame = boxed(widest);
            }
            text.style.chars.size = source.or(text.style.chars.size);
            // A text of no language has no hyphens.
            text.language = LanguageIdentifier::UNKNOWN;
            drawn(lettering.id, &text)
        })
        .collect::<Result<Vec<_>>>()?;
    // A text of no characters draws at none.
    held.retain(|size| *size > 0.0);
    Ok(median(held))
}

/// Brings `lettering` up to `size` in the least of its rooms that holds it so,
/// in whole words if one does, or else in the most it may take.
pub(super) fn lift(lettering: &mut Lettering, size: f64) -> Result<()> {
    let rooms = rooms(lettering);
    let Some(most) = rooms.last() else {
        return Ok(());
    };
    let mut text = lettering.text.clone();
    for language in [LanguageIdentifier::UNKNOWN, text.language.clone()] {
        text.language = language;
        let mut holds = |room: &Frame| -> Result<bool> {
            text.frame = room.clone();
            Ok(drawn(lettering.id, &text)? >= 0.999 * size)
        };
        if !holds(most)? {
            continue;
        }
        for room in &rooms {
            if holds(room)? {
                lettering.text.frame = room.clone();
                return Ok(());
            }
        }
    }
    lettering.text.frame = most.clone();
    Ok(())
}

/// The rooms `lettering`'s text may take, from the one it has to the most: in a
/// balloon narrower margins, and outside one wider frames, along its lines
/// first and then across them.
fn rooms(lettering: &Lettering) -> Vec<Frame> {
    match (&lettering.text.frame, lettering.widest) {
        (
            Frame::Path {
                path,
                fit,
                anchor,
                inset,
            },
            _,
        ) => MARGINS
            .into_iter()
            .map(|margin| Frame::Path {
                path: path.clone(),
                fit: *fit,
                anchor: *anchor,
                inset: inset.min(margin),
            })
            .collect(),
        (Frame::Box { bounds, .. }, Some(widest)) => {
            let own = Rect::from(*bounds);
            let mix = |from: f64, to: f64, share: f64| from + (to - from) * share;
            (0..=ACROSS)
                .flat_map(|across| (0..=ALONG).map(move |along| (along, across)))
                .map(|(along, across)| {
                    let along = f64::from(along) / f64::from(ALONG);
                    let across = f64::from(across) / f64::from(ACROSS);
                    boxed(Rect::new(
                        mix(own.x0, widest.x0, along),
                        mix(own.y0, widest.y0, across),
                        mix(own.x1, widest.x1, along),
                        mix(own.y1, widest.y1, across),
                    ))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The size `text`, block `id`'s, draws at; 0 when it has no characters.
pub(super) fn drawn(id: LayerId, text: &Text) -> Result<f64> {
    let laid = Renderer::get().layout(id, text)?;
    Ok(laid.map_or(0.0, |laid| {
        laid.runs
            .iter()
            .map(|run| f64::from(run.size))
            .fold(0.0, f64::max)
    }))
}
