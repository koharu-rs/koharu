//! The size a page is lettered at, measured as the renderer draws it, and how
//! a text that draws smaller is brought up to it. One its room holds larger
//! still fills its room.

use anyhow::Result;
use koharu_renderer::Renderer;
use koharu_scene::{Frame, LanguageIdentifier, LayerId, Role, Text};
use kurbo::Rect;
use rayon::prelude::*;

use super::{Lettering, boxed};

/// The margins a balloon's text narrows to, in ems, the widest first. Half
/// the one it starts with holds it 15 to 17% larger, and less puts letters on
/// the balloon's border.
const MARGINS: [f64; 4] = [0.875, 0.75, 0.625, 0.5];

/// The middle of `values`, the larger of two.
pub(super) fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_unstable_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The size of the page `letterings` are on: the median of the largest its
/// texts' rooms hold in whole words, and at most `source`, its sources' own.
/// Page medians agree within a work, where a size over its pages followed the
/// tightest one.
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
    Ok(median(held).map(|size| source.map_or(size, |source| size.min(source))))
}

/// Brings `lettering` up to `size`, as far as its room allows: in a balloon
/// by its margin, and outside one by its frame, each by no more than that
/// takes.
pub(super) fn lift(lettering: &mut Lettering, size: f64) -> Result<()> {
    if holds(lettering.id, &lettering.text, size)? {
        return Ok(());
    }
    match (&lettering.text.frame, lettering.widest) {
        (Frame::Path { .. }, _) => narrow(lettering.id, &mut lettering.text, size),
        (Frame::Box { bounds, .. }, Some(widest)) => {
            let from = Rect::from(*bounds);
            widen(lettering.id, &mut lettering.text, from, widest, size)
        }
        _ => Ok(()),
    }
}

/// Narrows `text`'s margin to the widest that holds it at `size`, or the
/// narrowest.
fn narrow(id: LayerId, text: &mut Text, size: f64) -> Result<()> {
    for margin in MARGINS {
        if let Frame::Path { inset, .. } = &mut text.frame {
            *inset = inset.min(margin);
        }
        if holds(id, text, size)? {
            break;
        }
    }
    Ok(())
}

/// Widens `text`'s box from `from` toward `widest`, along its lines first and
/// then across them, until it holds the text at `size`.
fn widen(id: LayerId, text: &mut Text, from: Rect, widest: Rect, size: f64) -> Result<()> {
    let between = |along: f64, across: f64| {
        let mix = |from: f64, to: f64, share: f64| from + (to - from) * share;
        boxed(Rect::new(
            mix(from.x0, widest.x0, along),
            mix(from.y0, widest.y0, across),
            mix(from.x1, widest.x1, along),
            mix(from.y1, widest.y1, across),
        ))
    };
    for across in 0..=4 {
        let across = f64::from(across) / 4.0;
        text.frame = between(1.0, across);
        if !holds(id, text, size)? {
            continue;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..5 {
            let middle = (low + high) / 2.0;
            text.frame = between(middle, across);
            match holds(id, text, size)? {
                true => high = middle,
                false => low = middle,
            }
        }
        text.frame = between(high, across);
        break;
    }
    Ok(())
}

/// Whether `text` draws at `size` or larger.
fn holds(id: LayerId, text: &Text, size: f64) -> Result<bool> {
    Ok(drawn(id, text)? >= 0.999 * size)
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
