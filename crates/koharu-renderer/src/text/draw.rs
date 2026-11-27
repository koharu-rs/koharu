//! A text layout drawn: its glyph runs filled, over their strokes, with the
//! bold and slant their faces lack.

use anyhow::Result;
use kurbo::{Affine, Join, Stroke};
use vello_gpu::peniko::Color;

use super::TextLayout;
use crate::Drawing;

/// Draws `layout`'s glyph runs onto `drawing` through `transform`, its
/// strokes too when `strokes` says so. Strokes draw first, under every fill,
/// as Photoshop draws an Outside stroke under its type: as the glyphs grown
/// by the stroke's width with round joins.
pub(crate) fn draw(
    drawing: &mut Drawing,
    layout: &TextLayout,
    transform: Affine,
    strokes: bool,
) -> Result<()> {
    drawing.scene.set_transform(transform);
    let passes: &[bool] = if strokes { &[true, false] } else { &[false] };
    for &stroking in passes {
        let resolved = &layout.resolved;
        let runs = (resolved.runs.iter())
            .chain(&resolved.ruby)
            .chain(&resolved.emphasis);
        for (glyphs, run) in layout.runs.iter().zip(runs) {
            let (paint, width, join) = match (stroking, run.stroke) {
                (true, Some((color, width))) => (color, f64::from(width), Join::Round),
                (true, None) => continue,
                (false, _) => (run.fill, 0.0, Join::Miter),
            };
            let font = &glyphs.font;
            // Glyphs grow under a stroke twice as wide, as Skia fakes a bold,
            // not by glifo's embolden: kurbo 0.13.1's `expand_path` fits one
            // cubic to each segment's offset, so a short handle turning back,
            // as many comic fonts have, sends the outline far off the glyph,
            // and it grows every contour one way, by the glyph's area, so a
            // contour wound against the rest, like some fonts' dots on i and
            // j, shrinks instead (koharu-rs/koharu#1073).
            let grow = f64::from(font.growth(run.size)) + width;
            let stroke = (grow > 0.0).then(|| Stroke::new(2.0 * grow).with_join(join));
            // The stroke overlaps the fill inside the outline, so a
            // translucent paint draws opaque in a layer of its alpha, or the
            // overlap would show darker.
            let color = Color::from(paint);
            let alpha = color.components[3];
            let layered = stroke.is_some() && alpha < 1.0;
            if layered {
                drawing.scene.push_opacity_layer(alpha);
            }
            drawing.scene.set_paint(match layered {
                true => color.with_alpha(1.0),
                false => color,
            });
            // y runs down where a glyph turns: glifo flips an outline before
            // this transform (`glyph_transform * FLIP_Y` in glifo 0.5's
            // `glyph.rs`).
            drawing.glyphs(font, run.size, glyphs.turn(), stroke, &glyphs.glyphs)?;
            if layered {
                drawing.scene.pop_layer();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use image::RgbaImage;
    use koharu_scene::{CharStyle, Rgba, Text, TextStyle};

    use super::*;
    use crate::{Gpu, fonts::Library};

    #[test]
    fn strokes_widen_a_text_and_draw_under_its_fill() -> Result<()> {
        let mut library = Library::new();
        let font = ["Arial", "DejaVu Sans", "Helvetica"]
            .into_iter()
            .find_map(|family| library.family(family).ok())
            .expect("an installed sans-serif");
        let laid = |stroke_width| -> Result<TextLayout> {
            let text = Text {
                content: "Stroked".into(),
                style: TextStyle {
                    chars: CharStyle {
                        stroke: Some(Rgba([1.0, 0.0, 0.0, 1.0])),
                        stroke_width,
                        ..CharStyle::default()
                    },
                    ..TextStyle::default()
                },
                ..Text::default()
            };
            Ok(crate::text::layout(&text, |_, _| Ok(vec![font.clone()]))?.expect("a layout"))
        };
        let (plain, stroked) = (laid(None)?, laid(Some(3.0))?);
        assert_eq!(stroked.bounds, plain.bounds.inflate(3.0, 3.0));
        let Ok(gpu) = Gpu::get() else {
            eprintln!("skipped drawing without a GPU");
            return Ok(());
        };
        // Drawn on white, the stroked text is red around a black fill.
        let draw = |layout: &TextLayout| -> Result<RgbaImage> {
            let size = layout.bounds.size();
            let mut drawing = gpu.drawing(size.width.ceil() as u16, size.height.ceil() as u16);
            draw(
                &mut drawing,
                layout,
                Affine::translate((-layout.bounds.x0, -layout.bounds.y0)),
                true,
            )?;
            drawing.read()
        };
        let red = |image: &RgbaImage| {
            image
                .pixels()
                .filter(|pixel| pixel.0[0] > 200 && pixel.0[1] < 60 && pixel.0[3] > 200)
                .count()
        };
        let (plain, stroked) = (draw(&plain)?, draw(&stroked)?);
        assert_eq!(red(&plain), 0);
        assert!(red(&stroked) > 100, "{} red pixels", red(&stroked));
        assert!(
            stroked
                .pixels()
                .any(|pixel| pixel.0[..3] == [0, 0, 0] && pixel.0[3] == 255)
        );
        Ok(())
    }

    #[test]
    fn a_sideways_glyph_turns_a_quarter_clockwise_inside_its_bounds() -> Result<()> {
        let mut library = Library::new();
        let font = ["Arial", "DejaVu Sans", "Helvetica"]
            .into_iter()
            .find_map(|family| library.family(family).ok())
            .expect("an installed sans-serif");
        let text = Text {
            content: "L".into(),
            direction: koharu_scene::Direction::Vertical,
            ..Text::default()
        };
        let layout = crate::text::layout(&text, |_, _| Ok(vec![font.clone()]))?.expect("a layout");
        assert!(layout.runs[0].sideways);
        let Ok(gpu) = Gpu::get() else {
            eprintln!("skipped drawing without a GPU");
            return Ok(());
        };
        // Drawn with a margin round its bounds, so ink a wrong turn moves out
        // of them still shows.
        let margin = 32.0;
        let size = layout.bounds.size();
        let mut drawing = gpu.drawing(
            (size.width + margin * 2.0).ceil() as u16,
            (size.height + margin * 2.0).ceil() as u16,
        );
        let origin = (margin - layout.bounds.x0, margin - layout.bounds.y0);
        draw(&mut drawing, &layout, Affine::translate(origin), false)?;
        let image = drawing.read()?;
        let ink: Vec<(u32, u32)> = image
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel.0[3] > 128 && pixel.0[0] < 128)
            .map(|(x, y, _)| (x, y))
            .collect();
        let left = ink.iter().map(|at| at.0).min().expect("ink");
        let right = ink.iter().map(|at| at.0).max().expect("ink");
        let top = ink.iter().map(|at| at.1).min().expect("ink");
        let bottom = ink.iter().map(|at| at.1).max().expect("ink");
        let inside = |low: u32, high: u32, from: f64, length: f64| {
            f64::from(low) >= from - 1.0 && f64::from(high) <= from + length + 1.0
        };
        assert!(
            inside(left, right, margin, size.width),
            "{left}..{right} across"
        );
        assert!(
            inside(top, bottom, margin, size.height),
            "{top}..{bottom} down"
        );
        // An L turned clockwise is a Γ: its stem runs along the top, which
        // turned the other way would be a ┘'s foot at the right.
        let along_top = ink.iter().filter(|at| at.1 <= top + 1).count() as u32;
        assert!(
            along_top * 2 > right - left,
            "{along_top} of {}",
            right - left
        );
        Ok(())
    }
}
