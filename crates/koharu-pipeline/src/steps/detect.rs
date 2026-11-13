//! Detection: the page's panels, balloons, and blocks in reading order, in
//! place of the ones there, whoever made them. A block gets its source's
//! frame, tilt, and direction, and its mask.

use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma};
use koharu_ml::koharu_layout_highres::{
    Detected, Kind, Layout, Lettering, Mask, Options, Typography,
};
use koharu_scene::{Bounds, Content, Layer, Op, PageId, Project, Rect, Ref, Rgba, Transform};
use kurbo::{Affine, Point};
use rayon::prelude::*;
use serde_json::{Value, json};

use super::{Member, Stages, action, dilate, op, scan};
use crate::{DetectorConfig, Step, models::Models};

pub(crate) struct Detect {
    config: DetectorConfig,
}

impl Detect {
    pub(crate) fn new(config: DetectorConfig) -> Self {
        Self { config }
    }
}

impl Stages for Detect {
    type Input = (PageId, DynamicImage);
    /// The page, its scan, and what the model found on it.
    type Output = (PageId, DynamicImage, Detected);
    /// The page and the ops adding what was found.
    type Ready = (PageId, Vec<Op>);

    const STEP: Step = Step::Detect;

    fn read(&self, member: &Member) -> Result<Option<Self::Input>> {
        Ok(scan(member)?.map(|(_, scan)| (member.page, scan)))
    }

    fn infer(
        &mut self,
        models: &mut Models,
        inputs: Vec<Self::Input>,
    ) -> Result<Vec<Self::Output>> {
        let options = Options {
            text: self.config.text,
            onomatopoeia: self.config.onomatopoeia,
            bubble: self.config.bubble,
            panel: self.config.panel,
        };
        let detector = models.detector(self.config.model)?;
        inputs
            .into_iter()
            .map(|(page, scan)| {
                let detected = detector.detect(&scan, &options)?;
                Ok((page, scan, detected))
            })
            .collect()
    }

    fn finish(&mut self, (page, scan, detected): Self::Output) -> Result<Self::Ready> {
        let pixels = scan.to_rgb8();
        let layout = {
            let _span = tracing::info_span!("layout").entered();
            detected.layout(&pixels)
        };
        let _span = tracing::info_span!("typography").entered();
        let typography: Vec<Option<Typography>> = layout
            .lettering
            .par_iter()
            .map(|lettering| lettering.typography(&pixels))
            .collect();
        let model = self.config.model.to_string();
        Ok((page, additions(&layout, &typography, page, &model)?))
    }

    fn write(&self, project: &Project, (page, additions): Self::Ready) -> Result<Vec<Op>> {
        let Ok(page) = project.page(page) else {
            return Ok(Vec::new());
        };
        let mut ops = Vec::new();
        removals(&page.layers, &mut ops);
        ops.extend(additions);
        Ok(ops)
    }
}

/// Removes the page's panels, balloons, and blocks, each with what's inside
/// it, but a locked one, which the user keeps.
fn removals(layers: &[Layer], ops: &mut Vec<Op>) {
    for layer in layers.iter().filter(|layer| !layer.locked) {
        match layer.content {
            Content::Panel(_) | Content::Balloon(_) | Content::Block(_) => {
                ops.push(Op::RemoveLayer {
                    layer: layer.id.into(),
                })
            }
            _ => removals(&layer.children, ops),
        }
    }
}

/// A detected part of the page and the parts inside it.
struct Node {
    part: Part,
    children: Vec<Node>,
}

#[derive(Clone, Copy)]
enum Part {
    Panel(usize),
    Bubble(usize),
    Lettering(usize),
}

/// The layout as a tree in reading order: panels in their order, each
/// balloon in its panel, and each lettering in its balloon, else free in its
/// panel. A panel clips what it holds, so a balloon or a free lettering that
/// reaches past its panel's border sits on the page, where it's read.
struct Tree<'l> {
    layout: &'l Layout,
    top: Vec<Node>,
    /// Where each panel and balloon went: its index on the page, and a
    /// balloon's panel.
    panels: Vec<Option<usize>>,
    bubbles: Vec<Option<(Option<usize>, usize)>>,
}

impl<'l> Tree<'l> {
    fn new(layout: &'l Layout) -> Self {
        let mut tree = Self {
            layout,
            top: Vec::new(),
            panels: vec![None; layout.panels.len()],
            bubbles: vec![None; layout.bubbles.len()],
        };
        for (at, lettering) in layout.lettering.iter().enumerate() {
            let node = Node {
                part: Part::Lettering(at),
                children: Vec::new(),
            };
            match lettering.bubble {
                Some(bubble) => tree.bubble(bubble).push(node),
                None => {
                    let panel = within(layout, lettering.bbox, &lettering.mask, lettering.panel);
                    tree.holder(panel).push(node);
                }
            }
        }
        // Panels and balloons holding no lettering.
        if let Some(last) = layout.panels.len().checked_sub(1) {
            tree.panel(last);
        }
        for bubble in 0..layout.bubbles.len() {
            tree.bubble(bubble);
        }
        tree
    }

    /// Panel `at`'s index on the page, the panels before it placed first.
    fn panel(&mut self, at: usize) -> usize {
        for panel in 0..=at {
            if self.panels[panel].is_none() {
                self.top.push(Node {
                    part: Part::Panel(panel),
                    children: Vec::new(),
                });
                self.panels[panel] = Some(self.top.len() - 1);
            }
        }
        self.panels[at].expect("a panel placed above")
    }

    /// What a part in `panel`, or on the page, goes in.
    fn holder(&mut self, panel: Option<usize>) -> &mut Vec<Node> {
        match panel {
            Some(panel) => {
                let at = self.panel(panel);
                &mut self.top[at].children
            }
            None => &mut self.top,
        }
    }

    /// What a lettering in balloon `at` goes in.
    fn bubble(&mut self, at: usize) -> &mut Vec<Node> {
        let (panel, index) = match self.bubbles[at] {
            Some(place) => place,
            None => {
                let bubble = &self.layout.bubbles[at];
                let panel = within(self.layout, bubble.bbox, &bubble.mask, bubble.panel);
                let holder = self.holder(panel);
                holder.push(Node {
                    part: Part::Bubble(at),
                    children: Vec::new(),
                });
                let place = (panel, holder.len() - 1);
                self.bubbles[at] = Some(place);
                place
            }
        };
        &mut self.holder(panel)[index].children
    }
}

/// `panel`, when a part with `bbox` and `mask` lies wholly inside the box
/// its outline is drawn from.
fn within(layout: &Layout, bbox: [f32; 4], mask: &Mask, panel: Option<usize>) -> Option<usize> {
    let [left, top, right, bottom] = layout.panels[panel?].bbox;
    let inside =
        |[x0, y0, x1, y1]: [f32; 4]| x0 >= left && y0 >= top && x1 <= right && y1 <= bottom;
    let masked = mask.width == 0
        || mask.height == 0
        || inside(
            [mask.x, mask.y, mask.x + mask.width, mask.y + mask.height].map(|edge| edge as f32),
        );
    (inside(bbox) && masked).then_some(panel?)
}

/// The ops adding the layout's parts at the top of the page, before the
/// layers left there, with each block's mask.
fn additions(
    layout: &Layout,
    typography: &[Option<Typography>],
    page: PageId,
    model: &str,
) -> Result<Vec<Op>> {
    let tree = Tree::new(layout);
    let mut adding = Adding {
        layout,
        typography,
        page,
        model,
        ops: Vec::new(),
        labels: 0,
    };
    adding.add(&tree.top, None)?;
    Ok(adding.ops)
}

struct Adding<'a> {
    layout: &'a Layout,
    typography: &'a [Option<Typography>],
    page: PageId,
    model: &'a str,
    ops: Vec<Op>,
    labels: usize,
}

impl Adding<'_> {
    fn add(&mut self, nodes: &[Node], parent: Option<&str>) -> Result<()> {
        for (index, node) in nodes.iter().enumerate() {
            let label = format!("d{}", self.labels);
            self.labels += 1;
            let (mut layer, score) = match node.part {
                Part::Panel(at) => {
                    let panel = &self.layout.panels[at];
                    (
                        json!({"kind": "panel", "outline": rectangle(panel.bbox)}),
                        panel.score,
                    )
                }
                Part::Bubble(at) => {
                    let bubble = &self.layout.bubbles[at];
                    (
                        json!({"kind": "balloon", "outline": bubble.outline.to_svg()}),
                        bubble.score,
                    )
                }
                Part::Lettering(at) => {
                    let lettering = &self.layout.lettering[at];
                    (
                        block(lettering, self.typography[at].as_ref()),
                        lettering.score,
                    )
                }
            };
            layer["attributes"] = json!({
                "c2pa": {"detection": action("c2pa.created", self.model, None)},
                "koharu": {"detection": {"score": score}},
            });
            self.ops.push(op(json!({
                "op": "add_layer",
                "id": format!("${label}"),
                "page": self.page,
                "parent": parent.map(|parent| format!("${parent}")),
                "index": index,
                "layer": layer,
            }))?);
            if let Part::Lettering(at) = node.part
                && let Some((rect, pixels)) = grown(
                    &self.layout.lettering[at].mask,
                    self.layout.lettering[at].bbox,
                    // Along its lines: a glyph's size, past either end.
                    self.typography[at]
                        .as_ref()
                        .map_or([0.0; 2], |found| match found.vertical {
                            true => [0.0, found.size],
                            false => [found.size, 0.0],
                        }),
                    &self
                        .layout
                        .lettering
                        .iter()
                        .enumerate()
                        .filter(|(other, _)| *other != at)
                        .map(|(_, other)| other.bbox)
                        .collect::<Vec<_>>(),
                    self.layout.width,
                    self.layout.height,
                )
            {
                self.ops.push(Op::WritePixels {
                    layer: Ref::try_from(format!("${label}.mask")).map_err(anyhow::Error::msg)?,
                    rect,
                    pixels,
                });
            }
            self.add(&node.children, Some(&label))?;
        }
        Ok(())
    }
}

/// How far a block's mask reaches past its glyph mask: the clean-up shows
/// only inside the mask, and slivers of letter outlines lie just outside the
/// glyphs.
const GROW: u32 = 3;

/// `mask` inside `bbox`, or within `reach` of it across and down but in none
/// of `others`, grown by `GROW` on a `width` by `height` page, and the
/// rectangle it covers; `None` when it's empty. RF-DETR's mask can spill onto
/// a neighbouring lettering, whose translation this block's clean-up would
/// then paint over: on Manga109 pages masks reached 70 pixels into a
/// neighbour's box, and one three glyphs past its own. The checkpoint learned
/// masks cut to their boxes, but a box can stop short of its lines' ends, as
/// one ended inside a trailing ellipsis, whose rest the inpainter then drew
/// back in; so a mask reaches a glyph past the ends, though not past the
/// sides, where a neighbour's lines stand, nor into another box.
fn grown(
    mask: &Mask,
    bbox: [f32; 4],
    reach: [f32; 2],
    others: &[[f32; 4]],
    width: u32,
    height: u32,
) -> Option<(Rect, Vec<u8>)> {
    if mask.width == 0 || mask.height == 0 {
        return None;
    }
    let within = |[left, top, right, bottom]: [f32; 4], [across, down]: [f32; 2], (x, y)| {
        (left - across..=right + across).contains(&x) && (top - down..=bottom + down).contains(&y)
    };
    let inside = |x: u32, y: u32| {
        let at = ((mask.x + x) as f32 + 0.5, (mask.y + y) as f32 + 0.5);
        within(bbox, [0.0; 2], at)
            || (within(bbox, reach, at) && !others.iter().any(|other| within(*other, [0.0; 2], at)))
    };
    let mut canvas = GrayImage::new(mask.width + 2 * GROW, mask.height + 2 * GROW);
    for (at, &value) in mask.pixels.iter().enumerate() {
        let (x, y) = (at as u32 % mask.width, at as u32 / mask.width);
        if inside(x, y) {
            canvas.put_pixel(x + GROW, y + GROW, Luma([value]));
        }
    }
    let canvas = dilate(&canvas, GROW);
    // The canvas's corner on the page, and the part of it on the page.
    let (left, top) = (
        i64::from(mask.x) - i64::from(GROW),
        i64::from(mask.y) - i64::from(GROW),
    );
    let (x0, y0) = (left.max(0), top.max(0));
    let x1 = (left + i64::from(canvas.width())).min(i64::from(width));
    let y1 = (top + i64::from(canvas.height())).min(i64::from(height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let rect = Rect {
        x: x0 as i32,
        y: y0 as i32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    };
    let pixels = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| canvas.get_pixel((x - left) as u32, (y - top) as u32).0[0])
        .collect();
    Some((rect, pixels))
}

fn rectangle([x0, y0, x1, y1]: [f32; 4]) -> String {
    format!("M{x0} {y0} L{x1} {y0} L{x1} {y1} L{x0} {y1} Z")
}

/// A block whose source is the lettering: its box turned back by its tilt,
/// and the turn, in its direction, at its size, its outline as its stroke.
fn block(lettering: &Lettering, typography: Option<&Typography>) -> Value {
    let [x0, y0, x1, y1] = lettering.bbox;
    let (vertical, tilt) = typography.map_or((y1 - y0 > x1 - x0, 0.0), |found| {
        (found.vertical, found.tilt)
    });
    let (bounds, transform) = turned(lettering.bbox, tilt);
    let outline = typography.and_then(|found| found.outline);
    let stroke = outline.map(|outline| {
        let [red, green, blue] = outline.color.map(|channel| f32::from(channel) / 255.0);
        Rgba([red, green, blue, 1.0])
    });
    json!({
        "kind": "block",
        "role": match lettering.kind {
            Kind::Text => "text",
            Kind::Onomatopoeia => "sfx",
        },
        "source": {
            "frame": {"box": {"bounds": bounds}},
            "direction": if vertical { "vertical" } else { "horizontal" },
            "transform": transform,
            "style": {"chars": {
                "size": typography.map(|found| found.size),
                "stroke": stroke,
                "stroke_width": outline.map(|outline| outline.width),
            }},
        },
    })
}

/// The box that, turned by `tilt` degrees clockwise about its centre, `bbox`
/// bounds, and that turn; `bbox` itself, unturned, when no box does.
fn turned(bbox: [f32; 4], tilt: f32) -> (Bounds, Transform) {
    let [x0, y0, x1, y1] = bbox.map(f64::from);
    let (width, height) = (x1 - x0, y1 - y0);
    let unturned = (
        Bounds {
            x: x0,
            y: y0,
            width,
            height,
        },
        Transform::default(),
    );
    let angle = f64::from(tilt).to_radians();
    let (sin, cos) = angle.sin_cos();
    // Tilts are within ±45°, where cos² − sin² is positive.
    let turn = cos * cos - sin * sin;
    let (inner_width, inner_height) = (
        (width * cos.abs() - height * sin.abs()) / turn,
        (height * cos.abs() - width * sin.abs()) / turn,
    );
    if tilt == 0.0 || turn <= f64::EPSILON || inner_width < 1.0 || inner_height < 1.0 {
        return unturned;
    }
    let centre = Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // Clockwise on the page, whose y runs down.
    let [a, b, c, d, e, f] = Affine::rotate_about(angle, centre).as_coeffs();
    (
        Bounds {
            x: centre.x - inner_width / 2.0,
            y: centre.y - inner_height / 2.0,
            width: inner_width,
            height: inner_height,
        },
        Transform([[a, c, e], [b, d, f]]),
    )
}

#[cfg(test)]
mod tests {
    use anyhow::Context as _;
    use koharu_ml::koharu_layout_highres::{Bubble, Mask, Panel};
    use kurbo::Shape as _;

    use super::*;

    fn mask() -> Mask {
        Mask {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            pixels: Vec::new(),
        }
    }

    fn lettering(bubble: Option<usize>, panel: Option<usize>) -> Lettering {
        Lettering {
            kind: Kind::Text,
            score: 0.9,
            bbox: [0.0, 0.0, 10.0, 10.0],
            mask: mask(),
            bubble,
            panel,
            room: None,
        }
    }

    fn shown(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|node| {
                let name = match node.part {
                    Part::Panel(at) => format!("P{at}"),
                    Part::Bubble(at) => format!("B{at}"),
                    Part::Lettering(at) => format!("L{at}"),
                };
                match node.children.is_empty() {
                    true => name,
                    false => format!("{name}({})", shown(&node.children)),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn parts_nest_in_reading_order() {
        let panel = || Panel {
            score: 0.9,
            bbox: [0.0, 0.0, 10.0, 10.0],
            mask: mask(),
        };
        let bubble = |lettering, panel| Bubble {
            score: 0.9,
            bbox: [0.0, 0.0, 10.0, 10.0],
            mask: mask(),
            outline: kurbo::Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1),
            lettering,
            panel,
        };
        let layout = Layout {
            width: 100,
            height: 100,
            // Free in the second panel, in a balloon there, free on the page.
            lettering: vec![
                lettering(None, Some(1)),
                lettering(Some(0), Some(1)),
                lettering(None, None),
            ],
            // The second is empty, crossing panels.
            bubbles: vec![bubble(vec![1], Some(1)), bubble(Vec::new(), None)],
            panels: vec![panel(), panel(), panel()],
        };
        assert_eq!(shown(&Tree::new(&layout).top), "P0 P1(L0 B0(L1)) L2 P2 B1");
    }

    #[test]
    fn a_part_reaching_past_its_panel_sits_on_the_page() {
        let panel = Panel {
            score: 0.9,
            bbox: [0.0, 0.0, 100.0, 100.0],
            mask: mask(),
        };
        let mut inside = lettering(None, Some(0));
        inside.bbox = [10.0, 10.0, 40.0, 90.0];
        let mut crossing = lettering(None, Some(0));
        crossing.bbox = [-5.0, 10.0, 30.0, 90.0];
        let mut spilling = lettering(None, Some(0));
        spilling.bbox = [50.0, 10.0, 80.0, 90.0];
        spilling.mask = Mask {
            x: 60,
            y: 10,
            width: 45,
            height: 10,
            pixels: vec![255; 450],
        };
        let layout = Layout {
            width: 200,
            height: 200,
            lettering: vec![inside, crossing, spilling],
            bubbles: Vec::new(),
            panels: vec![panel],
        };
        assert_eq!(shown(&Tree::new(&layout).top), "P0(L0) L1 L2");
    }

    #[test]
    fn a_mask_grows_past_its_glyphs_within_the_page() -> anyhow::Result<()> {
        // One pixel at the page's corner, and one in the middle.
        let corner = Mask {
            x: 0,
            y: 1,
            width: 1,
            height: 1,
            pixels: vec![255],
        };
        let (rect, pixels) =
            grown(&corner, [0.0, 0.0, 100.0, 100.0], [0.0; 2], &[], 100, 100).context("a mask")?;
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (0, 0, 4, 5));
        assert!(pixels.iter().all(|&value| value == 255));
        let middle = Mask {
            x: 50,
            y: 50,
            width: 1,
            height: 1,
            pixels: vec![255],
        };
        let (rect, pixels) =
            grown(&middle, [0.0, 0.0, 100.0, 100.0], [0.0; 2], &[], 100, 100).context("a mask")?;
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (47, 47, 7, 7));
        assert_eq!(pixels.iter().filter(|&&value| value == 255).count(), 49);
        Ok(())
    }

    #[test]
    fn a_mask_reaches_a_glyph_past_its_lines_ends_only() -> anyhow::Result<()> {
        // Three pixels on a row: in the box from 0 to 10, 4 past its right
        // edge, and 15 past it.
        let spilled = Mask {
            x: 5,
            y: 5,
            width: 21,
            height: 1,
            pixels: (0..21)
                .map(|at| if matches!(at, 0 | 9 | 20) { 255 } else { 0 })
                .collect(),
        };
        // The pixels each grows to on the row, reaching `reach` with
        // `others` beside the box.
        let covered = |reach: [f32; 2], others: &[[f32; 4]]| -> anyhow::Result<Vec<i32>> {
            let (rect, pixels) = grown(&spilled, [0.0, 0.0, 10.0, 10.0], reach, others, 100, 100)
                .context("a mask")?;
            Ok((0..rect.width as i32)
                .filter(|&x| pixels[(GROW * rect.width) as usize + x as usize] == 255)
                .map(|x| rect.x + x)
                .collect())
        };
        // Horizontal lines of glyphs 5 across end at the right edge: the
        // pixel 4 past it stays, each grown 3.
        let both: Vec<i32> = (2..=8).chain(11..=17).collect();
        assert_eq!(covered([5.0, 0.0], &[])?, both);
        // Vertical lines have their side there, where a neighbour's stand.
        let own: Vec<i32> = (2..=8).collect();
        assert_eq!(covered([0.0, 5.0], &[])?, own);
        // And no mask reaches into another lettering's box.
        assert_eq!(covered([5.0, 0.0], &[[12.0, 0.0, 30.0, 10.0]])?, own);
        Ok(())
    }

    #[test]
    fn a_tilted_box_turns_back_to_the_box_it_bounds() {
        // A 100 by 20 box turned by 10° about (60, 40).
        let (sin, cos) = 10f64.to_radians().sin_cos();
        let (width, height) = (100.0 * cos + 20.0 * sin, 100.0 * sin + 20.0 * cos);
        let bbox = [
            60.0 - width / 2.0,
            40.0 - height / 2.0,
            60.0 + width / 2.0,
            40.0 + height / 2.0,
        ]
        .map(|value| value as f32);
        let (bounds, Transform([[a, b, _], [d, e, _]])) = turned(bbox, 10.0);
        assert!((bounds.width - 100.0).abs() < 0.01 && (bounds.height - 20.0).abs() < 0.01);
        assert!((a - cos).abs() < 1e-9 && (b + sin).abs() < 1e-9);
        assert!((d - sin).abs() < 1e-9 && (e - cos).abs() < 1e-9);
    }
}
