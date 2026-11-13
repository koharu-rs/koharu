//! A view of a page's tree as vello scenes: blend modes, opacity, masks and
//! pass-through groups, in strips when one pass can't hold the images.

use koharu_scene::{Blend, MaskMode};
use vello::{
    Scene,
    kurbo::{Affine, Rect},
    peniko::{BlendMode, Color, Compose, Fill, Mix},
};

use super::{
    Strip, View,
    tree::{Content, MaskNode, Node},
};
use crate::gpu::ATLAS;

/// The passes that draw `nodes` for `view`: one, unless their images need
/// more of vello's atlas than one pass has, when they are strips across it.
pub(super) fn strips(nodes: &[Node], view: View) -> Vec<Strip> {
    let output = Rect::new(
        0.0,
        0.0,
        (view.rect.width() * view.scale).ceil(),
        (view.rect.height() * view.scale).ceil(),
    );
    let mut strips = Vec::new();
    cut(nodes, view, output, &mut strips);
    strips
}

/// Pushes the strips that draw the output pixels `rect`, halving it while
/// its images take more than one pass has.
fn cut(nodes: &[Node], view: View, rect: Rect, out: &mut Vec<Strip>) {
    let to_output = Affine::translate((-rect.x0, -rect.y0))
        * Affine::scale(view.scale)
        * Affine::translate((-view.rect.x0, -view.rect.y0));
    let clip = Rect::new(0.0, 0.0, rect.width(), rect.height());
    let pass = Pass { to_output, clip };
    let cost = nodes.iter().map(|node| pass.cost(node)).sum::<u64>();
    // vello skips an image that doesn't fit its atlas, so a pass keeps an
    // eighth of it free for packing.
    if cost > ATLAS * 7 / 8 && rect.height() > 64.0 {
        let middle = (rect.y0 + (rect.height() * 0.5).floor()).floor();
        cut(
            nodes,
            view,
            Rect::new(rect.x0, rect.y0, rect.x1, middle),
            out,
        );
        cut(
            nodes,
            view,
            Rect::new(rect.x0, middle, rect.x1, rect.y1),
            out,
        );
        return;
    }
    out.push(Strip {
        rect,
        scene: pass.compose(nodes),
    });
}

struct Pass {
    /// From page pixels to this pass's output pixels.
    to_output: Affine,
    /// The pass's output rectangle.
    clip: Rect,
}

impl Pass {
    fn cost(&self, node: &Node) -> u64 {
        if !node.visible {
            return 0;
        }
        let own = self.content_cost(&node.content);
        let masks: u64 = node
            .masks
            .iter()
            .filter(|mask| mask.visible)
            .map(|mask| self.content_cost(&mask.content))
            .sum();
        own + masks
            + node
                .children
                .iter()
                .map(|child| self.cost(child))
                .sum::<u64>()
    }

    /// The image pixels drawing content takes.
    fn content_cost(&self, content: &Content) -> u64 {
        match content {
            Content::Pixels { tiled, transform } => {
                tiled.cost(self.to_output * *transform, self.clip)
            }
            _ => 0,
        }
    }

    /// Draws nodes into a fresh scene: an isolated group's content.
    fn compose(&self, nodes: &[Node]) -> Scene {
        let mut scene = Scene::new();
        for node in nodes {
            self.node(node, &mut scene);
        }
        scene
    }

    fn node(&self, node: &Node, scene: &mut Scene) {
        if !node.visible {
            return;
        }
        let masked = node.masks.iter().any(|mask| mask.visible);
        let group = matches!(node.content, Content::None);
        if group && node.blend == Blend::PassThrough {
            if !masked && node.opacity >= 1.0 {
                // Children blend straight into the backdrop.
                for child in &node.children {
                    self.node(child, scene);
                }
                return;
            }
            // Children that all blend normally give the same result isolated,
            // as SrcOver is associative, so their backdrop needn't be drawn
            // again under them. Drawing it again under each of a page's
            // masked panels doubled the scene's images with every panel,
            // until they overflowed vello's atlas and the page drew nothing.
            if node.children.iter().all(blends_normally) {
                let content = self.compose(&node.children);
                self.wrap(node, Mix::Normal, content, scene);
                return;
            }
            // The group's layer draws the backdrop again under its children,
            // as a non-isolated group includes its backdrop, so its opacity
            // and masks fade between the backdrop and the result.
            let mut content = Scene::new();
            content.append(scene, None);
            for child in &node.children {
                self.node(child, &mut content);
            }
            self.wrap(node, Mix::Normal, content, scene);
            return;
        }
        let content = if group {
            self.compose(&node.children)
        } else {
            let mut content = Scene::new();
            self.content(&node.content, &mut content);
            content
        };
        if !masked && node.opacity >= 1.0 && node.blend == Blend::Normal {
            scene.append(&content, None);
        } else {
            self.wrap(node, mix(node.blend), content, scene);
        }
    }

    fn content(&self, content: &Content, scene: &mut Scene) {
        match content {
            Content::None => {}
            Content::Vector {
                vector,
                strokes,
                transform,
            } => {
                let transform = Some(self.to_output * *transform);
                if let Some(drawn) = vector.strokes.as_ref().filter(|_| *strokes) {
                    scene.append(drawn, transform);
                }
                scene.append(&vector.scene, transform);
            }
            Content::Pixels { tiled, transform } => {
                tiled.draw(scene, self.to_output * *transform, self.clip);
            }
            Content::Coverage(shape) => {
                scene.fill(
                    Fill::NonZero,
                    self.to_output,
                    Color::WHITE,
                    None,
                    shape.as_ref(),
                );
            }
        }
    }

    /// Composites `content` onto `scene` with the node's blend mode, opacity,
    /// and masks, over the output pixels the node can draw: vello gives a
    /// layer every tile its clip covers, so layers clipped to the whole pass
    /// on a large page overflowed vello 0.11's fixed buffers, and the page
    /// drew nothing. Past its bounds, and the two pixels its antialiased and
    /// filtered edges reach, a node draws nothing, which every blend and mask
    /// leaves as it was.
    fn wrap(&self, node: &Node, mix: Mix, content: Scene, scene: &mut Scene) {
        let Some(bounds) = node.bounds else {
            return;
        };
        let clip = self
            .to_output
            .transform_rect_bbox(bounds)
            .inflate(2.0, 2.0)
            .expand()
            .intersect(self.clip);
        if clip.area() <= 0.0 {
            return;
        }
        scene.push_layer(
            Fill::NonZero,
            BlendMode::new(mix, Compose::SrcOver),
            node.opacity,
            Affine::IDENTITY,
            &clip,
        );
        scene.append(&content, None);
        let masks: Vec<&MaskNode> = node.masks.iter().filter(|mask| mask.visible).collect();
        if !masks.is_empty() {
            // The masks build one coverage, and the content keeps only what
            // it covers.
            scene.push_layer(
                Fill::NonZero,
                BlendMode::new(Mix::Normal, Compose::DestIn),
                1.0,
                Affine::IDENTITY,
                &clip,
            );
            if masks[0].mode != MaskMode::Add {
                scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &clip);
            }
            for mask in masks {
                let compose = match mask.mode {
                    MaskMode::Add => Compose::SrcOver,
                    MaskMode::Subtract => Compose::DestOut,
                    MaskMode::Intersect => Compose::DestIn,
                };
                scene.push_layer(
                    Fill::NonZero,
                    BlendMode::new(Mix::Normal, compose),
                    1.0,
                    Affine::IDENTITY,
                    &clip,
                );
                self.content(&mask.content, scene);
                scene.pop_layer();
            }
            scene.pop_layer();
        }
        scene.pop_layer();
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
