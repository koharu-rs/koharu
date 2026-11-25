//! A page's tree drawn onto vello_gpu: blend modes, opacity, masks and
//! pass-through groups, and each node's glyphs, path, or pixels. The editor
//! and every export draw through this one painter, so they show the same
//! page.

use anyhow::Result;
use koharu_scene::{Blend, MaskMode};
use kurbo::{Affine, BezPath, Rect, Shape as _};
use vello_gpu::{
    Image, PaintType,
    peniko::{BlendMode, Color, Compose, Extend, Fill, ImageQuality, ImageSampler, Mix},
};

use super::tree::{Content, MaskNode, Node};
use crate::{Drawing, text};

/// Paints `nodes` onto `drawing` through `to_output`, from page pixels to
/// output pixels, within `clip`, a rectangle of output pixels.
pub(super) fn paint(
    nodes: &[Node],
    to_output: Affine,
    clip: Rect,
    drawing: &mut Drawing,
) -> Result<()> {
    let mut painter = Painter {
        to_output,
        clip,
        drawing,
        scopes: vec![Vec::new()],
    };
    painter.nodes(nodes)
}

struct Painter<'d, 'g, 'n> {
    /// From page pixels to output pixels.
    to_output: Affine,
    /// The output rectangle.
    clip: Rect,
    drawing: &'d mut Drawing<'g>,
    /// What each isolated group drew so far, innermost last: a pass-through
    /// group that can't blend straight into its backdrop draws it again.
    scopes: Vec<Vec<&'n Node>>,
}

impl<'n> Painter<'_, '_, 'n> {
    fn nodes(&mut self, nodes: &'n [Node]) -> Result<()> {
        nodes.iter().try_for_each(|node| self.node(node))
    }

    /// `nodes` in a group of their own, after `backdrop`, drawn again.
    fn isolated(&mut self, backdrop: &[&'n Node], nodes: &'n [Node]) -> Result<()> {
        self.scopes.push(Vec::new());
        let drawn = backdrop
            .iter()
            .try_for_each(|node| self.node(node))
            .and_then(|()| self.nodes(nodes));
        self.scopes.pop();
        drawn
    }

    fn node(&mut self, node: &'n Node) -> Result<()> {
        if !node.visible {
            return Ok(());
        }
        let masked = node.masks.iter().any(|mask| mask.visible);
        let group = node.content.is_none();
        if group && node.blend == Blend::PassThrough {
            if !masked && node.opacity >= 1.0 {
                // Children blend straight into the backdrop.
                return self.nodes(&node.children);
            }
            // Children that all blend normally give the same result
            // isolated, as SrcOver is associative, so their backdrop needn't
            // be drawn again under them. Drawing it again under each of a
            // page's masked panels multiplied the work with every panel.
            // Otherwise the group's layer draws the backdrop again under its
            // children, as a non-isolated group includes its backdrop, so its
            // opacity and masks fade between the backdrop and the result.
            let backdrop = match node.children.iter().all(blends_normally) {
                true => Vec::new(),
                false => self.scopes.last().cloned().unwrap_or_default(),
            };
            self.wrap(node, Mix::Normal, |painter| {
                painter.isolated(&backdrop, &node.children)
            })?;
        } else if !masked && node.opacity >= 1.0 && node.blend == Blend::Normal {
            match group {
                true => self.nodes(&node.children)?,
                false => self.content(node.content.as_ref(), node.strokes)?,
            }
        } else {
            self.wrap(node, mix(node.blend), |painter| match group {
                true => painter.isolated(&[], &node.children),
                false => painter.content(node.content.as_ref(), node.strokes),
            })?;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(node);
        }
        Ok(())
    }

    /// Draws `content`, a text's strokes too when `strokes` says so.
    fn content(&mut self, content: Option<&(Content, Affine)>, strokes: bool) -> Result<()> {
        let Some((content, transform)) = content else {
            return Ok(());
        };
        let transform = self.to_output * *transform;
        match content {
            Content::Text(layout) => text::draw(self.drawing, layout, transform, strokes)?,
            Content::Shape(shape) => {
                let scene = &mut self.drawing.scene;
                scene.set_transform(transform);
                if let Some(color) = shape.fill {
                    scene.set_fill_rule(shape.fill_rule);
                    scene.set_paint(color);
                    scene.fill_path(&shape.path);
                }
                if let Some((stroke, color)) = &shape.stroke {
                    scene.set_stroke(stroke.clone());
                    scene.set_paint(*color);
                    scene.stroke_path(&shape.path);
                }
            }
            Content::Pixels(pixels) => {
                let scale = transform.determinant().abs().sqrt();
                let (level, to_layer) = pixels.level(scale, self.drawing.gpu.largest_texture());
                let image = self.drawing.image(&level)?;
                let scene = &mut self.drawing.scene;
                scene.set_transform(transform * to_layer);
                scene.set_paint(PaintType::Image(Image {
                    image,
                    sampler: ImageSampler {
                        x_extend: Extend::Pad,
                        y_extend: Extend::Pad,
                        quality: ImageQuality::Medium,
                        alpha: 1.0,
                    },
                }));
                // A level may reach past the image.
                scene.fill_rect(
                    &to_layer
                        .inverse()
                        .transform_rect_bbox(pixels.bounds(Affine::IDENTITY)),
                );
            }
            Content::Coverage(shape) => {
                let scene = &mut self.drawing.scene;
                scene.set_transform(transform);
                scene.set_fill_rule(Fill::NonZero);
                scene.set_paint(Color::WHITE);
                scene.fill_path(shape);
            }
        }
        Ok(())
    }

    /// A layer clipped to `clip`, in output pixels.
    fn push_layer(&mut self, clip: &BezPath, blend: BlendMode, opacity: f32) {
        let scene = &mut self.drawing.scene;
        scene.set_transform(Affine::IDENTITY);
        scene.push_layer(Some(clip), Some(blend), Some(opacity), None, None);
    }

    /// Composites what `draw` draws with the node's blend mode, opacity, and
    /// masks, over the output pixels the node can draw. Past its bounds, and
    /// the two pixels its antialiased and filtered edges reach, a node draws
    /// nothing, which every blend and mask leaves as it was.
    fn wrap(
        &mut self,
        node: &'n Node,
        mix: Mix,
        draw: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        let Some(bounds) = node.bounds else {
            return Ok(());
        };
        let clip = self
            .to_output
            .transform_rect_bbox(bounds)
            .inflate(2.0, 2.0)
            .expand()
            .intersect(self.clip);
        if clip.area() <= 0.0 {
            return Ok(());
        }
        let area = clip.to_path(0.1);
        self.push_layer(&area, BlendMode::new(mix, Compose::SrcOver), node.opacity);
        draw(self)?;
        let masks: Vec<&MaskNode> = node.masks.iter().filter(|mask| mask.visible).collect();
        if !masks.is_empty() {
            // The masks build one coverage, and the content keeps only what
            // it covers.
            self.push_layer(&area, BlendMode::new(Mix::Normal, Compose::DestIn), 1.0);
            if masks[0].mode != MaskMode::Add {
                let scene = &mut self.drawing.scene;
                scene.set_paint(Color::WHITE);
                scene.fill_rect(&clip);
            }
            for mask in masks {
                let compose = match mask.mode {
                    MaskMode::Add => Compose::SrcOver,
                    MaskMode::Subtract => Compose::DestOut,
                    MaskMode::Intersect => Compose::DestIn,
                };
                self.push_layer(&area, BlendMode::new(Mix::Normal, compose), 1.0);
                self.content(mask.content.as_ref(), true)?;
                self.drawing.scene.pop_layer();
            }
            self.drawing.scene.pop_layer();
        }
        self.drawing.scene.pop_layer();
        Ok(())
    }
}

/// Whether `node` draws onto its backdrop with normal blending alone, its
/// children included, so isolating it changes nothing.
fn blends_normally(node: &Node) -> bool {
    !node.visible
        || (matches!(node.blend, Blend::Normal | Blend::PassThrough)
            && node.children.iter().all(blends_normally))
}

fn mix(blend: Blend) -> Mix {
    match blend {
        Blend::Normal | Blend::PassThrough => Mix::Normal,
        Blend::Multiply => Mix::Multiply,
        Blend::Screen => Mix::Screen,
        Blend::Overlay => Mix::Overlay,
        Blend::Darken => Mix::Darken,
        Blend::Lighten => Mix::Lighten,
        Blend::ColorDodge => Mix::ColorDodge,
        Blend::ColorBurn => Mix::ColorBurn,
        Blend::HardLight => Mix::HardLight,
        Blend::SoftLight => Mix::SoftLight,
        Blend::Difference => Mix::Difference,
        Blend::Exclusion => Mix::Exclusion,
        Blend::Hue => Mix::Hue,
        Blend::Saturation => Mix::Saturation,
        Blend::Color => Mix::Color,
        Blend::Luminosity => Mix::Luminosity,
    }
}
