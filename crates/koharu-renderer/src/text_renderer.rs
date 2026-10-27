//! Text layout and portable glyph recording.

use std::sync::Arc;

use anyhow::Result;
use koharu_rasterizer::{
    FillRule, PathElement, PreparedGlyph, PreparedGlyphRun, PreparedPath, PreparedResource,
    PreparedScene, PreparedSceneCommand, ResourceId,
};
use skrifa::{
    GlyphId, MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
};
use vello::kurbo::{Affine, BezPath, Diagonal2, Join, PathEl, expand_path};

use crate::{
    Error, FontStyle, HyphenationPolicy, LayoutRun, RenderBounds, RenderDiagnostic,
    Result as RenderResult, TextAlign, TextLayout, WritingMode,
    bubble::LayoutBox,
    fonts::{Fonts, font_key},
    script::is_chinese_or_japanese_text,
    shape::PositionedGlyph,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextNodeDescriptor {
    pub(crate) entity: koharu_scene::EntityId,
    pub(crate) text: String,
    pub(crate) language: Option<koharu_scene::LanguageTag>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) balloon_contour: Option<Vec<(f32, f32)>>,
    pub(crate) preferred_font: Option<String>,
    pub(crate) font_families: Vec<String>,
    pub(crate) font_weight: Option<u16>,
    pub(crate) font_style: Option<FontStyle>,
    pub(crate) font_size: Option<f32>,
    pub(crate) minimum_font_size: f32,
    pub(crate) auto_fit: bool,
    pub(crate) alignment: TextAlign,
    pub(crate) writing_mode: WritingMode,
    pub(crate) foreground_color: [u8; 4],
    pub(crate) stroke: Option<StrokeOptions>,
    pub(crate) line_height: f32,
    pub(crate) letter_spacing: f32,
    pub(crate) word_spacing: f32,
    pub(crate) text_inset: [f32; 4],
    pub(crate) point_text: bool,
}

pub(crate) struct RenderedTextNode {
    pub(crate) scene: Arc<PreparedScene>,
    pub(crate) resources: Arc<[PreparedResource]>,
    pub(crate) local_bounds: RenderBounds,
    pub(crate) metadata: RenderedTextMetadata,
    pub(crate) diagnostics: Vec<RenderDiagnostic>,
}

pub(crate) struct RenderedTextMetadata {
    pub(crate) rendered_bounds: RenderBounds,
    pub(crate) layout_bounds: RenderBounds,
    pub(crate) post_script_fonts: Vec<String>,
    pub(crate) font_size: f32,
    pub(crate) color: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StrokeOptions {
    pub color: [u8; 4],
    pub width_px: f32,
}

/// Paint options used when recording one laid-out text run into a Vello scene.
#[derive(Debug, Clone)]
pub(crate) struct TextRenderOptions {
    pub color: [u8; 4],
    pub hint_glyphs: bool,
    pub padding: f32,
    pub baseline_shift: f32,
    pub stroke: Option<StrokeOptions>,
}

impl Default for TextRenderOptions {
    fn default() -> Self {
        Self {
            color: [0, 0, 0, 255],
            hint_glyphs: true,
            padding: 0.0,
            baseline_shift: 0.0,
            stroke: None,
        }
    }
}

/// Shapes text and records the resulting glyphs into vector scenes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextRenderer;

impl TextRenderer {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub(crate) fn layout<'a>(&self, builder: &TextLayout<'a>, text: &str) -> Result<LayoutRun<'a>> {
        builder.run(text)
    }

    pub(crate) fn render(
        &self,
        scene: &mut PreparedScene,
        resources: &mut Vec<PreparedResource>,
        layout: &LayoutRun<'_>,
        writing_mode: WritingMode,
        options: &TextRenderOptions,
        transform: Affine,
    ) {
        // The border is an outward dilation of each glyph in y-down pen space, recorded
        // before the fill. A centered stroke would erase small marks, and font embolden
        // shifts the halo toward one side as the width grows. Hinting and synthetic bold
        // stay on the fill run.
        if let Some(stroke) = options
            .stroke
            .filter(|stroke| stroke.width_px > 0.0 && stroke.color[3] > 0)
        {
            draw_layout(
                scene,
                resources,
                layout,
                writing_mode,
                options,
                transform,
                GlyphPaint {
                    color: stroke.color,
                    dilation_px: stroke.width_px,
                },
            );
        }
        draw_layout(
            scene,
            resources,
            layout,
            writing_mode,
            options,
            transform,
            GlyphPaint {
                color: options.color,
                dilation_px: 0.0,
            },
        );
    }

    pub(crate) fn render_descriptor(
        &self,
        descriptor: &TextNodeDescriptor,
        fonts: &Fonts,
    ) -> RenderResult<RenderedTextNode> {
        let is_bubble_text = descriptor.balloon_contour.is_some();
        let frame = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: descriptor.width,
            height: descriptor.height,
        };
        let bounds = if is_bubble_text {
            inset(frame, descriptor.text_inset)
        } else {
            frame
        };
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(Error::invalid(format!(
                "text inset leaves no layout area for entity {}",
                descriptor.entity
            )));
        }
        let fonts = fonts
            .resolve(
                descriptor.preferred_font.as_deref(),
                descriptor.font_weight,
                descriptor.font_style,
                &descriptor.font_families,
                &descriptor.text,
                descriptor
                    .language
                    .as_ref()
                    .map(koharu_scene::LanguageTag::as_str),
            )
            .map_err(|source| Error::Font {
                entity: descriptor.entity,
                source,
            })?;
        let automatic_maximum = || automatic_maximum(descriptor, bounds, is_bubble_text);
        let maximum = if descriptor.auto_fit {
            automatic_maximum()
        } else {
            descriptor.font_size.unwrap_or_else(automatic_maximum)
        };
        let minimum = descriptor.minimum_font_size.min(maximum);
        let mut layout = TextLayout::new(&fonts[0])
            .with_fallback_fonts(&fonts[1..])
            .with_writing_mode(descriptor.writing_mode)
            .with_alignment(descriptor.alignment)
            .with_line_height(descriptor.line_height)
            .with_spacing(descriptor.letter_spacing, descriptor.word_spacing)
            .with_cjk_punctuation_layout(
                is_chinese_or_japanese_text(&descriptor.text)
                    || descriptor
                        .language
                        .as_ref()
                        .is_some_and(|language| is_chinese_or_japanese_language(language.as_str())),
            );
        if !descriptor.point_text {
            layout = layout
                .with_max_width(bounds.width)
                .with_max_height(bounds.height);
        }
        if let Some(contour) = &descriptor.balloon_contour {
            let [top, _, _, left] = descriptor.text_inset;
            let contour = contour.iter().map(|&(x, y)| (x - left, y - top)).collect();
            layout = layout.with_comic_balloon(
                bounds.width,
                bounds.height,
                contour,
                descriptor.text_inset.into_iter().fold(0.0, f32::max),
            );
        }
        if let Some(language) = &descriptor.language {
            layout = layout.with_hyphenation_language_tag(language.as_str());
        }
        if is_bubble_text && descriptor.writing_mode == WritingMode::Horizontal {
            layout = layout.with_hyphenation_policy(HyphenationPolicy::LastResort);
        }
        let layout = if descriptor.auto_fit && !descriptor.point_text {
            layout
                .with_max_font_size(maximum)
                .with_min_font_size(minimum)
        } else {
            layout.with_font_size(descriptor.font_size.unwrap_or(maximum))
        };
        let layout = self
            .layout(&layout, &descriptor.text)
            .map_err(|source| Error::Layout {
                entity: descriptor.entity,
                source,
            })?;
        let (mut x, mut y) = if descriptor.point_text {
            (bounds.x, bounds.y)
        } else {
            placement(bounds, layout.width, layout.height)
        };
        x += layout.placement_offset_x();
        y += layout.placement_offset_y();
        let transform = Affine::translate((f64::from(x), f64::from(y)));
        let color = descriptor.foreground_color;
        let mut options = TextRenderOptions {
            color,
            stroke: None,
            ..TextRenderOptions::default()
        };
        let mut scene = PreparedScene::default();
        let mut resources = Vec::new();
        if let Some(stroke) = descriptor.stroke {
            options.stroke = Some(stroke);
        }
        self.render(
            &mut scene,
            &mut resources,
            &layout,
            descriptor.writing_mode,
            &options,
            transform,
        );
        let mut diagnostics = Vec::new();
        if layout.font_size + f32::EPSILON < descriptor.minimum_font_size {
            diagnostics.push(RenderDiagnostic::TextBelowReadableSize {
                entity: descriptor.entity,
                font_size: layout.font_size,
                minimum_font_size: descriptor.minimum_font_size,
            });
        }
        if layout.overflowed() {
            diagnostics.push(RenderDiagnostic::TextOverflow {
                entity: descriptor.entity,
                available: bounds.into(),
                actual_width: layout.width,
                actual_height: layout.height,
                font_size: layout.font_size,
            });
        }
        let rendered_bounds = RenderBounds {
            x,
            y,
            width: layout.width,
            height: layout.height,
        };
        let stroke_padding = descriptor
            .stroke
            .map_or(0.0, |stroke| stroke.width_px.max(0.0));
        Ok(RenderedTextNode {
            scene: Arc::new(scene),
            resources: resources.into(),
            local_bounds: RenderBounds {
                x: rendered_bounds.x - stroke_padding,
                y: rendered_bounds.y - stroke_padding,
                width: rendered_bounds.width + stroke_padding * 2.0,
                height: rendered_bounds.height + stroke_padding * 2.0,
            },
            metadata: RenderedTextMetadata {
                rendered_bounds,
                layout_bounds: if descriptor.point_text {
                    rendered_bounds
                } else {
                    bounds.into()
                },
                post_script_fonts: fonts
                    .iter()
                    .map(|font| font.post_script_name().to_owned())
                    .collect(),
                font_size: layout.font_size,
                color,
            },
            diagnostics,
        })
    }
}

fn automatic_maximum(
    descriptor: &TextNodeDescriptor,
    bounds: LayoutBox,
    is_bubble_text: bool,
) -> f32 {
    if is_bubble_text {
        if descriptor.writing_mode.is_vertical() {
            bounds.height
        } else {
            bounds.width
        }
    } else {
        24.0
    }
}

fn is_chinese_or_japanese_language(language: &str) -> bool {
    language
        .split(['-', '_'])
        .next()
        .is_some_and(|primary| matches!(primary.to_ascii_lowercase().as_str(), "ja" | "zh"))
}

fn inset(rect: LayoutBox, [top, right, bottom, left]: [f32; 4]) -> LayoutBox {
    LayoutBox {
        x: rect.x + left,
        y: rect.y + top,
        width: (rect.width - left - right).max(0.0),
        height: (rect.height - top - bottom).max(0.0),
    }
}

fn placement(rect: LayoutBox, width: f32, height: f32) -> (f32, f32) {
    let x = rect.x + (rect.width - width) * 0.5;
    let remaining = rect.height - height;
    let y = rect.y + remaining * 0.5;
    (x, y)
}

/// Color and outward dilation for one glyph draw pass (border or ordinary fill).
struct GlyphPaint {
    color: [u8; 4],
    dilation_px: f32,
}

fn glyph_pen_outline(
    glyph: &PositionedGlyph<'_>,
    font_size: f32,
    x: f32,
    y: f32,
) -> Option<BezPath> {
    let font = glyph.font.skrifa_ref().ok()?;
    let outline = font.outline_glyphs().get(GlyphId::new(glyph.glyph_id))?;
    let mut path = BezPath::new();
    outline
        .draw(
            DrawSettings::unhinted(Size::new(font_size), glyph.font.location()),
            &mut PenOutline(&mut path),
        )
        .ok()?;
    if !path_has_segments(&path) {
        return None;
    }
    let mut placed =
        Affine::translate((f64::from(x), f64::from(y))) * Affine::scale_non_uniform(1.0, -1.0);
    if let Some(angle) = glyph.font.synthetic_skew() {
        placed *= Affine::skew(-(angle.to_radians().tan() as f64), 0.0);
    }
    path.apply_affine(placed);
    path.is_finite().then_some(path)
}

fn finite_path_elements(path: &BezPath) -> Option<Vec<PathElement>> {
    if !path.is_finite() || !path_has_segments(path) {
        return None;
    }
    Some(
        path.elements()
            .iter()
            .copied()
            .map(|element| match element {
                PathEl::MoveTo(point) => PathElement::MoveTo([point.x, point.y]),
                PathEl::LineTo(point) => PathElement::LineTo([point.x, point.y]),
                PathEl::QuadTo(control, point) => {
                    PathElement::QuadTo([control.x, control.y, point.x, point.y])
                }
                PathEl::CurveTo(control_a, control_b, point) => PathElement::CurveTo([
                    control_a.x,
                    control_a.y,
                    control_b.x,
                    control_b.y,
                    point.x,
                    point.y,
                ]),
                PathEl::ClosePath => PathElement::Close,
            })
            .collect(),
    )
}

fn path_has_segments(path: &BezPath) -> bool {
    path.elements().iter().any(|element| {
        matches!(
            element,
            PathEl::LineTo(_) | PathEl::QuadTo(_, _) | PathEl::CurveTo(_, _, _)
        )
    })
}

struct PenOutline<'a>(&'a mut BezPath);

impl OutlinePen for PenOutline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0
            .quad_to((f64::from(cx), f64::from(cy)), (f64::from(x), f64::from(y)));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}

fn draw_layout(
    scene: &mut PreparedScene,
    resources: &mut Vec<PreparedResource>,
    layout: &LayoutRun<'_>,
    writing_mode: WritingMode,
    options: &TextRenderOptions,
    transform: Affine,
    paint: GlyphPaint,
) {
    for line in &layout.lines {
        let (baseline_x, baseline_y) = match writing_mode {
            WritingMode::Horizontal | WritingMode::VerticalRl => line.baseline,
        };
        let mut pen_x = 0.0;
        let mut pen_y = 0.0;
        let mut start = 0;

        while start < line.glyphs.len() {
            let font = line.glyphs[start].font;
            let key = font_key(font);
            let mut end = start + 1;
            while end < line.glyphs.len() && font_key(line.glyphs[end].font) == key {
                end += 1;
            }

            let synthetic_bold = if font.synthetic_bold() { 1.0 } else { 0.0 };
            if paint.dilation_px > 0.0 {
                let radius = f64::from(synthetic_bold + paint.dilation_px);
                for glyph in &line.glyphs[start..end] {
                    let x = options.padding + baseline_x + pen_x + glyph.x_offset;
                    let y = options.padding + baseline_y + pen_y
                        - glyph.y_offset
                        - options.baseline_shift;
                    if let Some(outline) = glyph_pen_outline(glyph, layout.font_size, x, y) {
                        let dilated = expand_path(
                            &outline,
                            Diagonal2::new(radius, radius),
                            Join::Round,
                            4.0,
                            0.1,
                        );
                        if let Some(elements) = finite_path_elements(&dilated) {
                            scene
                                .commands
                                .push(PreparedSceneCommand::FillPath(PreparedPath {
                                    fill: FillRule::NonZero,
                                    transform: transform.as_coeffs(),
                                    color: paint.color,
                                    elements,
                                }));
                        }
                    }
                    pen_x += glyph.x_advance;
                    pen_y -= glyph.y_advance;
                }
            } else {
                let mut glyphs = Vec::with_capacity(end - start);
                for glyph in &line.glyphs[start..end] {
                    glyphs.push(PreparedGlyph {
                        id: glyph.glyph_id,
                        x: options.padding + baseline_x + pen_x + glyph.x_offset,
                        y: options.padding + baseline_y + pen_y
                            - glyph.y_offset
                            - options.baseline_shift,
                    });
                    pen_x += glyph.x_advance;
                    pen_y -= glyph.y_advance;
                }

                let font_id = ResourceId::for_font(font.bytes());
                if !resources.iter().any(|candidate| candidate.id() == font_id) {
                    resources.push(PreparedResource::font_shared(font.shared_bytes()));
                }
                let glyph_transform = font
                    .synthetic_skew()
                    .map(|angle| Affine::skew(-(angle.to_radians().tan() as f64), 0.0).as_coeffs());
                scene
                    .commands
                    .push(PreparedSceneCommand::GlyphRun(PreparedGlyphRun {
                        font: font_id,
                        font_index: font.index(),
                        font_size: layout.font_size,
                        normalized_coords: font.normalized_coords().to_vec(),
                        transform: transform.as_coeffs(),
                        glyph_transform,
                        hint: options.hint_glyphs,
                        embolden: [synthetic_bold; 2],
                        color: paint.color,
                        glyphs,
                    }));
            }
            start = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use koharu_rasterizer::{
        Bounds, CompositionCommand, LayerId, LayerKind as PreparedLayerKind, PathElement, Point,
        PreparedContent, PreparedFrame, PreparedFrameBundle, PreparedFrameManifest, PreparedLayer,
        PreparedPath, PreparedResourcePacket, PreparedResourceStore, Presentation, Revision,
    };
    use koharu_scene::EntityId;
    use skrifa::{
        GlyphId, MetadataProvider,
        instance::Size,
        outline::{DrawSettings, OutlinePen},
    };
    use vello::kurbo::{BezPath, PathEl, Rect, Shape};

    use super::*;
    use crate::fonts::FontSystem;
    use crate::shape::PositionedGlyph;

    #[test]
    fn automatic_size_preserves_free_text_default_and_balloon_extent() {
        let descriptor = TextNodeDescriptor {
            entity: EntityId::new(),
            text: "Hi".to_owned(),
            language: None,
            width: 240.0,
            height: 120.0,
            balloon_contour: None,
            preferred_font: Some("Arial".to_owned()),
            font_families: vec!["Arial".to_owned()],
            font_weight: None,
            font_style: None,
            font_size: Some(6.0),
            minimum_font_size: 9.0,
            auto_fit: true,
            alignment: TextAlign::Center,
            writing_mode: WritingMode::Horizontal,
            foreground_color: [0, 0, 0, 255],
            stroke: None,
            line_height: 1.2,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_inset: [0.0; 4],
            point_text: false,
        };
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 240.0,
            height: 120.0,
        };

        assert_eq!(automatic_maximum(&descriptor, bounds, false), 24.0);
        assert_eq!(automatic_maximum(&descriptor, bounds, true), 240.0);
    }

    #[test]
    fn rendered_text_survives_prepared_packet_round_trip() {
        let font = FontSystem::new().first_font().unwrap();
        let layout = TextLayout::new(&font)
            .with_font_size(24.0)
            .run("Koharu")
            .unwrap();
        let mut scene = PreparedScene::default();
        let mut resources = Vec::new();
        TextRenderer::new().render(
            &mut scene,
            &mut resources,
            &layout,
            WritingMode::Horizontal,
            &TextRenderOptions::default(),
            Affine::IDENTITY,
        );
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0],
            PreparedResource::Font { bytes, .. } if !bytes.is_empty()
        ));

        let layer = LayerId::from_bytes([2; 16]);
        let bundle = PreparedFrameBundle {
            frame: PreparedFrame {
                revision: Revision::new(7),
                page: LayerId::from_bytes([1; 16]),
                width: 160,
                height: 48,
                origin: (0, 0),
                normalization: Affine::IDENTITY.as_coeffs(),
                layers: vec![PreparedLayer {
                    id: layer,
                    geometry: vec![
                        Point { x: 0.0, y: 0.0 },
                        Point { x: 160.0, y: 0.0 },
                        Point { x: 160.0, y: 48.0 },
                        Point { x: 0.0, y: 48.0 },
                    ],
                    bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 160.0,
                        height: 48.0,
                    },
                    local_bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 160.0,
                        height: 48.0,
                    },
                    presentation: Presentation {
                        visible: true,
                        opacity: 1.0,
                    },
                    kind: PreparedLayerKind::Text,
                    placement: Affine::IDENTITY.as_coeffs(),
                    content: PreparedContent::Vector(scene),
                    element_frame: None,
                }],
            },
            resources,
        };

        let encoded = bundle.manifest().unwrap().encode().unwrap();
        let manifest = PreparedFrameManifest::decode(&encoded).unwrap();
        let mut resources = PreparedResourceStore::default();
        for reference in manifest.required_resources() {
            let encoded = bundle
                .resource_packet(reference.id)
                .unwrap()
                .encode()
                .unwrap();
            resources.insert(PreparedResourcePacket::decode(&encoded).unwrap());
        }
        let compiled = manifest.compile(&resources).unwrap();
        assert_eq!(compiled.revision(), Revision::new(7));
        assert_eq!(compiled.layers()[0].id(), layer);
        assert!(matches!(
            compiled.composition_commands(1).as_slice(),
            [CompositionCommand::Vector(_)]
        ));
    }

    #[test]
    fn text_border_dilates_evenly_on_every_side() {
        let font = FontSystem::new().first_font().unwrap();
        assert!(font.synthetic_skew().is_none());
        let layout = TextLayout::new(&font)
            .with_font_size(64.0)
            .run("H.")
            .unwrap();
        let padding = 4.0;
        let baseline_shift = 3.0;
        let transform = Affine::translate((12.0, -5.0));
        let placed = |width: f32| {
            stroke_paths(
                &layout,
                WritingMode::Horizontal,
                padding,
                baseline_shift,
                [20, 40, 220, 255],
                width,
                transform,
            )
        };

        let narrow = placed(8.0);
        let wide = placed(16.0);
        let glyphs = outlined_glyphs(&layout, padding, baseline_shift);
        assert_eq!(narrow.len(), glyphs.len());
        assert_eq!(wide.len(), glyphs.len());
        assert!(glyphs.len() >= 2, "expected a path per outlined glyph");
        for ((origin, glyph), (narrow_path, wide_path)) in
            glyphs.iter().copied().zip(narrow.iter().zip(&wide))
        {
            let outline = unhinted_outline_bounds(glyph, origin, layout.font_size, transform);
            let narrow_sides = side_deltas(&outline, path_bounds(narrow_path));
            let wide_sides = side_deltas(&outline, path_bounds(wide_path));
            assert_even(narrow_sides, 8.0);
            assert_even(wide_sides, 16.0);
            for (before, after) in narrow_sides.values().into_iter().zip(wide_sides.values()) {
                let growth = after - before;
                assert!(
                    (growth - 8.0).abs() <= 1.25,
                    "doubling the width should grow every side, grew by {growth}"
                );
            }
        }

        let (origin, period) = glyphs.last().copied().unwrap();
        let outline = unhinted_outline(period, origin, layout.font_size, transform);
        let center = Shape::bounding_box(&outline).center();
        let period_stroke = path_bez(narrow.last().unwrap());
        assert_ne!(
            Shape::winding(&period_stroke, center),
            0,
            "a small mark stays a solid dilated blob"
        );
        let outline_bounds = Shape::bounding_box(&outline);
        let stroke_bounds = Shape::bounding_box(&period_stroke);
        assert!(stroke_bounds.union(outline_bounds) == stroke_bounds);
    }

    #[test]
    fn invisible_stroke_emits_no_border() {
        let font = FontSystem::new().first_font().unwrap();
        let layout = TextLayout::new(&font)
            .with_font_size(32.0)
            .run("H")
            .unwrap();
        for stroke in [
            Some(StrokeOptions {
                color: [0, 0, 0, 255],
                width_px: 0.0,
            }),
            Some(StrokeOptions {
                color: [0, 0, 0, 255],
                width_px: -4.0,
            }),
            Some(StrokeOptions {
                color: [0, 0, 0, 0],
                width_px: 8.0,
            }),
            None,
        ] {
            let mut scene = PreparedScene::default();
            TextRenderer::new().render(
                &mut scene,
                &mut Vec::new(),
                &layout,
                WritingMode::Horizontal,
                &TextRenderOptions {
                    stroke,
                    ..TextRenderOptions::default()
                },
                Affine::IDENTITY,
            );
            assert!(
                scene
                    .commands
                    .iter()
                    .all(|command| matches!(command, PreparedSceneCommand::GlyphRun(run) if run.embolden == [0.0, 0.0])),
                "width {stroke:?} should not record a border"
            );
        }
    }

    #[test]
    fn bold_and_vertical_borders_keep_the_fill_inside() {
        let font = faux_bold_font();
        assert!(font.synthetic_bold());
        assert!(font.synthetic_skew().is_none());
        let horizontal = TextLayout::new(&font)
            .with_font_size(48.0)
            .run("H.")
            .unwrap();
        let vertical = TextLayout::new(&font)
            .with_font_size(48.0)
            .with_writing_mode(WritingMode::VerticalRl)
            .run("H.")
            .unwrap();
        for (layout, mode) in [
            (&horizontal, WritingMode::Horizontal),
            (&vertical, WritingMode::VerticalRl),
        ] {
            let mut scene = PreparedScene::default();
            TextRenderer::new().render(
                &mut scene,
                &mut Vec::new(),
                layout,
                mode,
                &TextRenderOptions {
                    padding: 2.0,
                    baseline_shift: 1.0,
                    stroke: Some(StrokeOptions {
                        color: [255, 0, 0, 255],
                        width_px: 8.0,
                    }),
                    ..TextRenderOptions::default()
                },
                Affine::IDENTITY,
            );
            let (paths, runs): (Vec<_>, Vec<_>) = scene
                .commands
                .iter()
                .partition(|command| matches!(command, PreparedSceneCommand::FillPath(_)));
            assert!(!paths.is_empty());
            assert!(runs.iter().all(|command| matches!(
                command,
                PreparedSceneCommand::GlyphRun(run)
                    if run.embolden == [1.0, 1.0] && run.color == [0, 0, 0, 255]
            )));
            let glyphs = outlined_glyphs(layout, 2.0, 1.0);
            assert_eq!(paths.len(), glyphs.len());
            for ((origin, glyph), command) in glyphs.iter().copied().zip(&paths) {
                let PreparedSceneCommand::FillPath(path) = command else {
                    unreachable!();
                };
                let outline =
                    unhinted_outline_bounds(glyph, origin, layout.font_size, Affine::IDENTITY);
                assert_even(side_deltas(&outline, path_bounds(path)), 9.0);
            }
        }

        let spaced = TextLayout::new(&font)
            .with_font_size(48.0)
            .run("A A")
            .unwrap();
        let mut scene = PreparedScene::default();
        TextRenderer::new().render(
            &mut scene,
            &mut Vec::new(),
            &spaced,
            WritingMode::Horizontal,
            &TextRenderOptions {
                stroke: Some(StrokeOptions {
                    color: [0, 0, 255, 255],
                    width_px: 6.0,
                }),
                ..TextRenderOptions::default()
            },
            Affine::IDENTITY,
        );
        let path_count = scene
            .commands
            .iter()
            .filter(|command| matches!(command, PreparedSceneCommand::FillPath(_)))
            .count();
        let fill = scene.commands.iter().find_map(|command| match command {
            PreparedSceneCommand::GlyphRun(run) => Some(run),
            PreparedSceneCommand::FillPath(_) => None,
        });
        let fill = fill.unwrap();
        assert_eq!(fill.embolden, [1.0, 1.0]);
        let outlined = outlined_glyphs(&spaced, 0.0, 0.0).len();
        assert_eq!(path_count, outlined);
        assert!(path_count < fill.glyphs.len());
    }

    fn faux_bold_font() -> crate::fonts::Font {
        let mut system = FontSystem::new();
        for family in [
            "Times New Roman",
            "Times",
            "Georgia",
            "Courier New",
            "Courier",
            "Verdana",
            "Trebuchet MS",
            "Comic Sans MS",
            "Arial",
            "Helvetica",
            "DejaVu Sans",
            "Liberation Sans",
            "Noto Sans",
        ] {
            if let Some(font) = synthetic_bold_face(&mut system, &[family.to_owned()]) {
                return font;
            }
        }
        synthetic_bold_face(&mut system, &[]).expect("no faux-bold system font")
    }

    fn synthetic_bold_face(
        system: &mut FontSystem,
        families: &[String],
    ) -> Option<crate::fonts::Font> {
        system
            .resolve(
                families,
                fontique::Attributes {
                    weight: fontique::FontWeight::new(1000.0),
                    ..fontique::Attributes::default()
                },
                &[fontique::Script::from_bytes(*b"Latn")],
                None,
            )
            .ok()?
            .into_iter()
            .find(|font| font.synthetic_bold())
    }

    fn stroke_paths(
        layout: &LayoutRun<'_>,
        writing_mode: WritingMode,
        padding: f32,
        baseline_shift: f32,
        color: [u8; 4],
        width_px: f32,
        transform: Affine,
    ) -> Vec<PreparedPath> {
        let mut scene = PreparedScene::default();
        TextRenderer::new().render(
            &mut scene,
            &mut Vec::new(),
            layout,
            writing_mode,
            &TextRenderOptions {
                padding,
                baseline_shift,
                stroke: Some(StrokeOptions { color, width_px }),
                ..TextRenderOptions::default()
            },
            transform,
        );
        let mut paths = Vec::new();
        let mut seen_fill = false;
        for command in scene.commands {
            match command {
                PreparedSceneCommand::FillPath(path) => {
                    assert!(!seen_fill, "stroke paths are recorded before the fill");
                    assert_eq!(path.color, color);
                    paths.push(path);
                }
                PreparedSceneCommand::GlyphRun(run) => {
                    seen_fill = true;
                    assert_eq!(run.color, [0, 0, 0, 255]);
                    assert_eq!(run.embolden, [0.0, 0.0]);
                }
            }
        }
        assert!(seen_fill);
        paths
    }

    fn outlined_glyphs<'a>(
        layout: &'a LayoutRun<'a>,
        padding: f32,
        baseline_shift: f32,
    ) -> Vec<((f32, f32), &'a PositionedGlyph<'a>)> {
        let mut glyphs = Vec::new();
        for line in &layout.lines {
            let (baseline_x, baseline_y) = line.baseline;
            let mut pen_x = 0.0;
            let mut pen_y = 0.0;
            for glyph in &line.glyphs {
                let origin = (
                    padding + baseline_x + pen_x + glyph.x_offset,
                    padding + baseline_y + pen_y - glyph.y_offset - baseline_shift,
                );
                if has_outline(glyph, layout.font_size) {
                    glyphs.push((origin, glyph));
                }
                pen_x += glyph.x_advance;
                pen_y -= glyph.y_advance;
            }
        }
        glyphs
    }

    fn has_outline(glyph: &PositionedGlyph<'_>, font_size: f32) -> bool {
        let Ok(font) = glyph.font.skrifa_ref() else {
            return false;
        };
        let Some(outline) = font.outline_glyphs().get(GlyphId::new(glyph.glyph_id)) else {
            return false;
        };
        let mut path = BezPath::new();
        outline
            .draw(
                DrawSettings::unhinted(Size::new(font_size), glyph.font.location()),
                &mut TestOutline(&mut path),
            )
            .is_ok_and(|_| {
                path.elements().iter().any(|element| {
                    matches!(
                        element,
                        PathEl::LineTo(_) | PathEl::QuadTo(_, _) | PathEl::CurveTo(_, _, _)
                    )
                })
            })
    }

    fn unhinted_outline_bounds(
        glyph: &PositionedGlyph<'_>,
        origin: (f32, f32),
        font_size: f32,
        transform: Affine,
    ) -> Rect {
        Shape::bounding_box(&unhinted_outline(glyph, origin, font_size, transform))
    }

    fn unhinted_outline(
        glyph: &PositionedGlyph<'_>,
        (x, y): (f32, f32),
        font_size: f32,
        transform: Affine,
    ) -> BezPath {
        let font = glyph.font.skrifa_ref().unwrap();
        let outline = font
            .outline_glyphs()
            .get(GlyphId::new(glyph.glyph_id))
            .unwrap();
        let mut path = BezPath::new();
        outline
            .draw(
                DrawSettings::unhinted(Size::new(font_size), glyph.font.location()),
                &mut TestOutline(&mut path),
            )
            .unwrap();
        let mut placed =
            Affine::translate((f64::from(x), f64::from(y))) * Affine::scale_non_uniform(1.0, -1.0);
        if let Some(angle) = glyph.font.synthetic_skew() {
            placed *= Affine::skew(-(angle.to_radians().tan() as f64), 0.0);
        }
        path.apply_affine(transform * placed);
        path
    }

    struct TestOutline<'a>(&'a mut BezPath);

    impl OutlinePen for TestOutline<'_> {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to((f64::from(x), f64::from(y)));
        }

        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to((f64::from(x), f64::from(y)));
        }

        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
            self.0
                .quad_to((f64::from(cx), f64::from(cy)), (f64::from(x), f64::from(y)));
        }

        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.curve_to(
                (f64::from(cx0), f64::from(cy0)),
                (f64::from(cx1), f64::from(cy1)),
                (f64::from(x), f64::from(y)),
            );
        }

        fn close(&mut self) {
            self.0.close_path();
        }
    }

    fn path_bounds(path: &PreparedPath) -> Rect {
        Shape::bounding_box(&path_bez(path))
    }

    fn path_bez(path: &PreparedPath) -> BezPath {
        let mut bez = BezPath::new();
        for element in &path.elements {
            match *element {
                PathElement::MoveTo([x, y]) => bez.move_to((x, y)),
                PathElement::LineTo([x, y]) => bez.line_to((x, y)),
                PathElement::QuadTo([x1, y1, x2, y2]) => bez.quad_to((x1, y1), (x2, y2)),
                PathElement::CurveTo([x1, y1, x2, y2, x3, y3]) => {
                    bez.curve_to((x1, y1), (x2, y2), (x3, y3));
                }
                PathElement::Close => bez.close_path(),
            }
        }
        bez.apply_affine(Affine::new(path.transform));
        bez
    }

    #[derive(Clone, Copy, Debug)]
    struct Sides {
        left: f64,
        right: f64,
        top: f64,
        bottom: f64,
    }

    impl Sides {
        fn values(self) -> [f64; 4] {
            [self.left, self.right, self.top, self.bottom]
        }
    }

    fn side_deltas(outline: &Rect, stroke: Rect) -> Sides {
        Sides {
            left: outline.min_x() - stroke.min_x(),
            right: stroke.max_x() - outline.max_x(),
            top: outline.min_y() - stroke.min_y(),
            bottom: stroke.max_y() - outline.max_y(),
        }
    }

    fn assert_even(sides: Sides, radius: f64) {
        let values = sides.values();
        for value in values {
            assert!(
                (value - radius).abs() <= 1.0,
                "expected about {radius}px on every side, got {sides:?}"
            );
        }
        let min = values.into_iter().fold(f64::INFINITY, f64::min);
        let max = values.into_iter().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max - min <= 1.0,
            "the four sides should match, got {sides:?}"
        );
    }
}
