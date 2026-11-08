//! A layout's glyph runs painted as vello scenes: fills, and strokes apart,
//! with synthetic bold and slant.

use vello::{
    FontEmbolden, Scene,
    kurbo::{Affine, Diagonal2, Join},
    peniko::Fill,
};

use super::TextLayout;
use crate::frame::Vector;

/// `layout` painted. Strokes go in a scene of their own, drawn under every
/// fill as Photoshop draws an Outside stroke under its type.
pub(crate) fn paint(layout: &TextLayout) -> Vector {
    let mut scene = Scene::new();
    let mut strokes: Option<Scene> = None;
    for (glyphs, run) in layout.runs.iter().zip(&layout.resolved.runs) {
        let font = &glyphs.font;
        let data = font.font_data();
        let skew = font
            .synthetic_skew()
            .map(|angle| Affine::skew(-f64::from(angle.to_radians().tan()), 0.0));
        // vello transforms a glyph in its own space, where y runs up, and
        // there a quarter turn clockwise on the page maps (x, y) to (y, -x).
        let transform = match glyphs.sideways {
            true => Some(
                Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]) * skew.unwrap_or(Affine::IDENTITY),
            ),
            false => skew,
        };
        let bold = if font.synthetic_bold() {
            f64::from(run.size) * 0.02
        } else {
            0.0
        };
        if let Some((color, width)) = run.stroke {
            let amount = bold + f64::from(width);
            strokes
                .get_or_insert_with(Scene::new)
                .draw_glyphs(&data)
                .font_size(run.size)
                .normalized_coords(font.normalized_coords())
                .glyph_transform(transform)
                .font_embolden(
                    FontEmbolden::new(Diagonal2::new(amount, amount)).with_join(Join::Round),
                )
                .brush(color)
                .draw(Fill::NonZero, glyphs.glyphs.iter().copied());
        }
        scene
            .draw_glyphs(&data)
            .font_size(run.size)
            .normalized_coords(font.normalized_coords())
            .glyph_transform(transform)
            .font_embolden(FontEmbolden::new(Diagonal2::new(bold, bold)))
            .brush(run.fill)
            .draw(Fill::NonZero, glyphs.glyphs.iter().copied());
    }
    Vector {
        scene,
        strokes,
        bounds: layout.bounds,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use koharu_scene::{CharStyle, Rgba, Text, TextStyle};

    use super::*;
    use crate::{
        fonts::FontSystem,
        text::{layout, testing::primary},
    };

    #[test]
    fn strokes_paint_apart_from_fills_and_widen_the_bounds() -> Result<()> {
        let font = primary(&mut FontSystem::new());
        let painted = |stroke_width| -> Result<Vector> {
            let text = Text {
                content: "Stroked".into(),
                style: TextStyle {
                    chars: CharStyle {
                        stroke: Some(Rgba([1.0; 4])),
                        stroke_width,
                        ..CharStyle::default()
                    },
                    ..TextStyle::default()
                },
                ..Text::default()
            };
            let layout = layout(&text, |_, _| Ok(vec![font.clone()]))?.expect("a layout");
            Ok(paint(&layout))
        };
        let (plain, stroked) = (painted(None)?, painted(Some(3.0))?);
        assert!(plain.strokes.is_none() && stroked.strokes.is_some());
        assert_eq!(stroked.bounds, plain.bounds.inflate(3.0, 3.0));
        Ok(())
    }
}
