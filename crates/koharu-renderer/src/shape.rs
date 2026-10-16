//! HarfRust shaping and grapheme-safe font fallback.

use anyhow::{Context, Result};
use harfrust::{Buffer, Direction, Feature, Script, ShapeOptions, ShaperFont};
use icu_properties::{CodePointMapData, props::Script as IcuScript};
use icu_segmenter::GraphemeClusterSegmenter;
use skrifa::raw::TableProvider;

use crate::fonts::Font;

/// A glyph positioned for layout while retaining its source font and text cluster.
#[derive(Debug, Clone)]
pub struct PositionedGlyph<'a> {
    /// The glyph ID as per the font's glyph set.
    pub glyph_id: u32,
    /// The cluster index in the original text that this glyph corresponds to.
    pub cluster: u32,
    /// Font used to shape this glyph.
    pub font: &'a Font,
    /// How much the line advances after drawing this glyph when setting text in
    /// horizontal direction.
    pub x_advance: f32,
    /// How much the line advances after drawing this glyph when setting text in
    /// vertical direction.
    pub y_advance: f32,
    /// How much the glyph moves on the X-axis before drawing it, this should
    /// not affect how much the line advances.
    pub x_offset: f32,
    /// How much the glyph moves on the Y-axis before drawing it, this should
    /// not affect how much the line advances.
    pub y_offset: f32,
}

/// A shaped run of text, containing positioned glyphs and overall advance.
#[derive(Debug, Clone)]
pub struct ShapedRun<'a> {
    pub glyphs: Vec<PositionedGlyph<'a>>,
    pub x_advance: f32,
    pub y_advance: f32,
}

/// Options for shaping text.
#[derive(Debug, Clone)]
pub struct ShapingOptions<'a> {
    pub direction: Direction,
    pub script: Option<Script>,
    pub font_size: f32,
    pub features: &'a [Feature],
}

/// Text shaper using HarfRust.
#[derive(Debug, Clone, Default)]
pub struct TextShaper;

impl TextShaper {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn shape<'a>(
        &self,
        text: &str,
        font: &'a Font,
        options: &ShapingOptions,
    ) -> Result<ShapedRun<'a>> {
        let mut buffer = Buffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(options.direction);
        if let Some(script) = options.script {
            buffer.set_script(Some(script));
        }

        let direction = buffer.direction();
        let script = buffer.script();
        let language = buffer.language();
        let plan = font.shape_plan(direction, script, language, options.features);
        harfrust::shape(
            &ShaperFont::new(font.shaper_font()),
            &mut buffer,
            ShapeOptions::new()
                .plan(Some(&plan))
                .features(options.features)
                .point_size(Some(options.font_size)),
        )
        .context("failed to shape text with HarfRust")?;

        let glyph_positions = buffer.glyph_positions();
        let glyph_infos = buffer.glyph_infos();

        // Scale factor to convert font units to pixels
        let upem = font.skrifa_ref()?.head()?.units_per_em() as f32;
        let scale = options.font_size / upem;

        let mut positioned_glyphs = Vec::with_capacity(glyph_infos.len());
        for (info, pos) in glyph_infos.iter().zip(glyph_positions.iter()) {
            positioned_glyphs.push(PositionedGlyph {
                glyph_id: info.glyph_id,
                cluster: info.cluster,
                font,
                x_offset: (pos.x_offset as f32) * scale,
                y_offset: (pos.y_offset as f32) * scale,
                x_advance: (pos.x_advance as f32) * scale,
                y_advance: (pos.y_advance as f32) * scale,
            });
        }

        Ok(ShapedRun {
            glyphs: positioned_glyphs,
            x_advance: glyph_positions
                .iter()
                .map(|p| (p.x_advance as f32) * scale)
                .sum(),
            y_advance: glyph_positions
                .iter()
                .map(|p| (p.y_advance as f32) * scale)
                .sum(),
        })
    }
}

pub(crate) fn shape_script_runs<'a>(
    shaper: &TextShaper,
    text: &str,
    fonts: &[&'a Font],
    options: &ShapingOptions,
) -> Result<Vec<ShapedRun<'a>>> {
    if text.is_empty() || fonts.is_empty() {
        return Ok(Vec::new());
    }

    let script_map = CodePointMapData::<IcuScript>::new();
    let mut runs = Vec::new();

    let mut char_iter = text.char_indices().peekable();
    while let Some((start, ch)) = char_iter.next() {
        let mut script = script_map.get(ch);
        let mut end = start + ch.len_utf8();

        while let Some(&(next_start, next_ch)) = char_iter.peek() {
            let next_script = script_map.get(next_ch);
            if next_script == script
                || next_script == IcuScript::Common
                || next_script == IcuScript::Inherited
            {
                char_iter.next();
                end = next_start + next_ch.len_utf8();
            } else if script == IcuScript::Common || script == IcuScript::Inherited {
                script = next_script;
                char_iter.next();
                end = next_start + next_ch.len_utf8();
            } else {
                break;
            }
        }

        let script_run_text = &text[start..end];

        let mut run_opts = options.clone();
        run_opts.script = crate::script::harfrust_script(script);

        if let Some(font) = fonts
            .first()
            .copied()
            .filter(|font| font.covers(script_run_text))
        {
            push_font_run(shaper, script_run_text, font, &run_opts, start, &mut runs)?;
            continue;
        }

        // If no single face covers the script run, preserve extended grapheme
        // clusters. This keeps combining marks, variation selectors, and emoji
        // ZWJ sequences together instead of falling back per Unicode scalar.
        let boundaries = GraphemeClusterSegmenter::new()
            .segment_str(script_run_text)
            .collect::<Vec<_>>();
        let mut group_start = 0;
        let mut group_font = 0;
        for pair in boundaries.windows(2) {
            let cluster = &script_run_text[pair[0]..pair[1]];
            let font = fonts
                .iter()
                .position(|font| font.covers(cluster))
                .unwrap_or(0);
            if pair[0] == 0 {
                group_font = font;
                continue;
            }
            if font != group_font {
                push_font_run(
                    shaper,
                    &script_run_text[group_start..pair[0]],
                    fonts[group_font],
                    &run_opts,
                    start + group_start,
                    &mut runs,
                )?;
                group_start = pair[0];
                group_font = font;
            }
        }
        push_font_run(
            shaper,
            &script_run_text[group_start..],
            fonts[group_font],
            &run_opts,
            start + group_start,
            &mut runs,
        )?;
    }

    Ok(runs)
}

fn push_font_run<'a>(
    shaper: &TextShaper,
    text: &str,
    font: &'a Font,
    options: &ShapingOptions<'_>,
    text_offset: usize,
    runs: &mut Vec<ShapedRun<'a>>,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let mut shaped = shaper.shape(text, font, options)?;
    for glyph in &mut shaped.glyphs {
        glyph.cluster += text_offset as u32;
    }
    runs.push(shaped);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontSystem;

    const PRIMARY_FAMILIES: &[&str] = &[
        "Arial",
        "Segoe UI",
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
        "Yu Gothic",
        "MS Gothic",
        "Times New Roman",
    ];
    const FALLBACK_FAMILIES: &[&str] = &[
        "Segoe UI Symbol",
        "Segoe UI Emoji",
        "Noto Sans Symbols",
        "Noto Sans Symbols2",
        "Noto Color Emoji",
        "Apple Color Emoji",
        "Apple Symbols",
        "Symbola",
        "Arial Unicode MS",
    ];
    const SYMBOLS: &[char] = &[
        '\u{1F4A9}',
        '\u{1F642}',
        '\u{2665}',
        '\u{2605}',
        '\u{2713}',
        '\u{2603}',
        '\u{262F}',
        '\u{2691}',
        '\u{26A0}',
    ];

    fn query_font(fonts: &mut FontSystem, name: &str) -> Option<Font> {
        fonts.query_family(name).ok()
    }

    #[test]
    fn shape_segment_uses_fallback_font() -> Result<()> {
        let mut fonts = FontSystem::new();
        let primary = PRIMARY_FAMILIES
            .iter()
            .find_map(|name| query_font(&mut fonts, name))
            .expect("no primary font available for fallback test");

        let mut chosen = None;
        for ch in SYMBOLS {
            if primary.has_glyph(*ch) {
                continue;
            }
            if let Some(fallback) = FALLBACK_FAMILIES.iter().find_map(|name| {
                let font = query_font(&mut fonts, name)?;
                (font.post_script_name() != primary.post_script_name() && font.has_glyph(*ch))
                    .then_some(font)
            }) {
                chosen = Some((fallback, *ch));
                break;
            }
        }

        let (fallback, symbol) = chosen.expect("no fallback font with missing primary glyph found");

        let text = format!("A{}!", symbol);
        let shaper = TextShaper::new();
        let options = ShapingOptions {
            direction: harfrust::Direction::LeftToRight,
            script: None,
            font_size: 16.0,
            features: &[],
        };
        let runs = shape_script_runs(&shaper, &text, &[&primary, &fallback], &options)?;

        let glyph_count: usize = runs.iter().map(|r| r.glyphs.len()).sum();
        assert!(glyph_count >= 3);
        assert!(
            runs.iter()
                .flat_map(|run| &run.glyphs)
                .all(|glyph| glyph.glyph_id != 0)
        );
        assert!(
            runs.iter()
                .flat_map(|run| &run.glyphs)
                .any(|glyph| std::ptr::eq(glyph.font, &fallback))
        );
        assert!(
            runs.iter()
                .flat_map(|run| &run.glyphs)
                .any(|glyph| glyph.cluster == 0 && std::ptr::eq(glyph.font, &primary))
        );

        Ok(())
    }

    #[test]
    fn shape_arabic_with_fallback_preserves_context() -> Result<()> {
        let mut fonts = FontSystem::new();
        let primary = PRIMARY_FAMILIES
            .iter()
            .find_map(|name| query_font(&mut fonts, name))
            .expect("no primary font available");
        let fallback = FALLBACK_FAMILIES
            .iter()
            .find_map(|name| query_font(&mut fonts, name))
            .expect("no fallback font available");

        let shaper = TextShaper::new();
        let options = ShapingOptions {
            direction: harfrust::Direction::RightToLeft,
            script: Some(harfrust::Script::from_iso15924_tag(harfrust::Tag::new(b"Arab")).unwrap()),
            font_size: 16.0,
            features: &[],
        };

        // "مرحبا" (Marhaba)
        let text = "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627}";
        let shaped = shape_script_runs(&shaper, text, &[&primary, &fallback], &options)?;

        assert!(!shaped.is_empty());
        // Verify it used a consistent font for the whole run to ensure joining.
        let font_at_start = shaped[0].glyphs[0].font;
        assert!(
            shaped[0]
                .glyphs
                .iter()
                .all(|g| std::ptr::eq(g.font, font_at_start))
        );

        Ok(())
    }

    #[test]
    fn shape_rtl_multi_run_preserves_visual_order() -> Result<()> {
        let mut fonts = FontSystem::new();
        let font = PRIMARY_FAMILIES
            .iter()
            .find_map(|name| query_font(&mut fonts, name))
            .expect("no font available");

        let shaper = TextShaper::new();
        let options = ShapingOptions {
            direction: harfrust::Direction::RightToLeft,
            script: Some(harfrust::Script::from_iso15924_tag(harfrust::Tag::new(b"Arab")).unwrap()),
            font_size: 16.0,
            features: &[],
        };

        // Mixed script: Arabic + Hebrew (will be detected as separate script runs).
        let text = "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} \u{05e9}\u{05dc}\u{05d5}\u{05dd}";
        let shaped = shape_script_runs(&shaper, text, &[&font], &options)?;

        // Now returns separate script runs in logical order.
        assert!(shaped.len() >= 2); // Arabic+space, Hebrew (or maybe space separate)
        assert!(shaped[0].glyphs[0].cluster < shaped[shaped.len() - 1].glyphs[0].cluster);

        Ok(())
    }
}
