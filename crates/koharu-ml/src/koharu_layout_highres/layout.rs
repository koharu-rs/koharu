//! A page's layout: its lettering in reading order, the balloons it sits in,
//! and the panels around both, each related by index.

use kurbo::{BezPath, Rect};

use super::{Mask, order, processor::Detection, rooms};

/// A text's share of its box a balloon must hold for the text to be in it.
/// On Manga109's test pages, no text had a fifth of its box in a second
/// balloon, so the balloon holding most of it is the only one.
const IN_BUBBLE: f32 = 0.5;

/// A page's layout.
#[derive(Debug, Clone)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// Text and onomatopoeia in reading order.
    pub lettering: Vec<Lettering>,
    pub bubbles: Vec<Bubble>,
    /// In reading order.
    pub panels: Vec<Panel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Onomatopoeia,
}

#[derive(Debug, Clone)]
pub struct Lettering {
    pub kind: Kind,
    pub score: f32,
    pub bbox: [f32; 4],
    /// The pixels of its glyphs.
    pub mask: Mask,
    /// Index into `Layout::bubbles` of the bubble holding most of its glyphs.
    pub bubble: Option<usize>,
    /// Index into `Layout::panels` of the panel it is read in.
    pub panel: Option<usize>,
    /// A text's room in its bubble, in page pixels: the whole outline when it
    /// is the bubble's only text, else its share from `rooms`.
    pub room: Option<BezPath>,
}

#[derive(Debug, Clone)]
pub struct Bubble {
    pub score: f32,
    pub bbox: [f32; 4],
    pub mask: Mask,
    /// The mask's outer contour, simplified and closed, in page pixels.
    pub outline: BezPath,
    /// Indices into `Layout::lettering` of what it holds, in reading order.
    pub lettering: Vec<usize>,
    pub panel: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Panel {
    pub score: f32,
    pub bbox: [f32; 4],
    pub mask: Mask,
}

/// The layout of `detections`, whose lettering comes first in reading order.
pub(super) fn build(detections: Vec<Detection>, width: u32, height: u32) -> Layout {
    let panels: Vec<usize> = (0..detections.len())
        .filter(|&index| detections[index].label == "panel")
        .collect();
    let panel = |index: usize| {
        order::holder(&detections, index, &panels)
            .and_then(|holder| panels.iter().position(|&panel| panel == holder))
    };

    let mut bubbles: Vec<Bubble> = Vec::new();
    let mut lettering: Vec<Lettering> = Vec::new();
    for (index, detection) in detections.iter().enumerate() {
        match detection.label.as_str() {
            "bubble" => {
                // A bubble with no outline has no room to give.
                if let Some(outline) = rooms::outline(&detection.mask) {
                    bubbles.push(Bubble {
                        score: detection.score,
                        bbox: detection.bbox,
                        mask: detection.mask.clone(),
                        outline,
                        lettering: Vec::new(),
                        panel: panel(index),
                    });
                }
            }
            label @ ("text" | "onomatopoeia") => lettering.push(Lettering {
                kind: if label == "text" {
                    Kind::Text
                } else {
                    Kind::Onomatopoeia
                },
                score: detection.score,
                bbox: detection.bbox,
                mask: detection.mask.clone(),
                bubble: None,
                panel: panel(index),
                room: None,
            }),
            _ => {}
        }
    }

    for (index, item) in lettering.iter_mut().enumerate() {
        item.bubble = holder(&item.mask, &bubbles);
        if let Some(bubble) = item.bubble {
            bubbles[bubble].lettering.push(index);
        }
    }
    for bubble in &bubbles {
        let texts: Vec<usize> = bubble
            .lettering
            .iter()
            .copied()
            .filter(|&index| lettering[index].kind == Kind::Text)
            .collect();
        let groups: Vec<Vec<Rect>> = texts
            .iter()
            .map(|&index| {
                let [x0, y0, x1, y1] = lettering[index].bbox.map(f64::from);
                vec![Rect::new(x0, y0, x1, y1)]
            })
            .collect();
        for (index, room) in texts
            .into_iter()
            .zip(rooms::rooms(&bubble.outline, &groups))
        {
            lettering[index].room = Some(room);
        }
    }

    Layout {
        width,
        height,
        lettering,
        bubbles,
        panels: panels
            .into_iter()
            .map(|index| Panel {
                score: detections[index].score,
                bbox: detections[index].bbox,
                mask: detections[index].mask.clone(),
            })
            .collect(),
    }
}

/// The bubble whose mask holds most of `glyphs`, at least `IN_BUBBLE` of
/// them; the smaller of two holding as much. The glyphs, not the box: a
/// lettering beside a bubble, as a side remark written over its edge, has a
/// box that can reach well into it.
fn holder(glyphs: &Mask, bubbles: &[Bubble]) -> Option<usize> {
    let ink: Vec<(u32, u32)> = glyphs
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, value)| **value != 0)
        .map(|(at, _)| {
            let at = at as u32;
            (glyphs.x + at % glyphs.width, glyphs.y + at / glyphs.width)
        })
        .collect();
    let area = ink.len();
    if area == 0 {
        return None;
    }
    bubbles
        .iter()
        .enumerate()
        .map(|(index, bubble)| {
            let inside = ink
                .iter()
                .filter(|&&(x, y)| bubble.mask.contains(x, y))
                .count();
            (index, inside as f32 / area as f32)
        })
        .filter(|&(_, share)| share >= IN_BUBBLE)
        .max_by(|&(left, left_share), &(right, right_share)| {
            left_share.total_cmp(&right_share).then_with(|| {
                let size = |index: usize| {
                    let [x0, y0, x1, y1] = bubbles[index].bbox;
                    (x1 - x0) * (y1 - y0)
                };
                size(right).total_cmp(&size(left))
            })
        })
        .map(|(index, _)| index)
}
