//! The brush tools, draw, eraser, and remove: a stroke's outline, which the
//! preview and the coverage both fill, and paint and erasure on a raster's
//! straight-alpha pixels.

use image::{RgbaImage, imageops};
use koharu_scene::{Content, Edit, LayerId, Op, Raster, Ref, Rgba};
use vello::kurbo::{
    Affine, BezPath, Cap, Circle, Join, Point, Shape as _, Size, Stroke as Style, StrokeOpts,
    stroke,
};
use vello_cpu::{Pixmap, RenderContext, Resources, color::palette::css::WHITE};

use super::added;
use crate::{
    Effect, Notice, Tool,
    draw::overlay::Overlay,
    gesture::{Context, Gesture, GestureLook, Press},
    view::Sample,
};

/// How closely a stroke's outline follows its round caps and joins, in page
/// pixels.
const TOLERANCE: f64 = 0.1;

/// A brush tool's press starts a stroke: the draw tool's onto the one
/// selected raster or a new one, the eraser's onto the one selected.
pub(crate) fn press(cx: &Context, sample: Sample) -> Press {
    let target = match cx.tool {
        Tool::Remove => None,
        _ => cx
            .selection
            .single()
            .filter(|&layer| cx.snapshot.writable(layer).is_some()),
    };
    if cx.tool == Tool::Eraser && target.is_none() {
        return Press::answers(vec![Effect::Notice(Notice::EraserNeedsRaster)]);
    }
    let points = vec![sample.page];
    Press::starts(Gesture::Stroke(Stroke {
        tool: cx.tool,
        outline: outline(&points, cx.brush.diameter),
        points,
        target,
    }))
}

/// A brush stroke through page points, onto `target` or, for the draw tool
/// with none, a new raster.
pub(crate) struct Stroke {
    tool: Tool,
    points: Vec<Point>,
    target: Option<LayerId>,
    /// The points' outline, expanded again only when a point arrives.
    outline: BezPath,
}

impl Stroke {
    pub fn moved(&mut self, cx: &Context, samples: &[Sample]) -> Vec<Effect> {
        let before = self.points.len();
        for sample in samples {
            if self.points.last() != Some(&sample.page) {
                self.points.push(sample.page);
            }
        }
        if self.points.len() != before {
            self.outline = outline(&self.points, cx.brush.diameter);
        }
        Vec::new()
    }

    /// The remove tool's inpaint request, or the draw tool's or the eraser's
    /// edit.
    pub fn finish(self, cx: &Context) -> anyhow::Result<Vec<Effect>> {
        let page = cx.snapshot.page().id;
        let Some((rect, coverage)) = coverage(&self.outline, cx.snapshot.size()) else {
            return Ok(Vec::new());
        };
        if self.tool == Tool::Remove {
            return Ok(vec![Effect::Inpaint {
                page,
                rect,
                mask: coverage,
            }]);
        }
        self.stroked(cx, rect, &coverage)
    }

    pub fn preview(&self, cx: &Context, overlay: &mut Overlay) {
        let tint = match self.tool {
            Tool::Draw => cx.brush.color,
            _ => {
                let Rgba([r, g, b, _]) = cx.colors.selection;
                Rgba([r, g, b, 0.4])
            }
        };
        overlay.fill(&self.outline, tint);
    }

    pub fn look(&self) -> GestureLook {
        GestureLook::Stroke {
            points: self.points.len(),
            last: self.points.last().copied().unwrap_or_default(),
        }
    }

    /// A paint or erase stroke's coverage over the page pixels `rect` as one
    /// write to its raster, read and composited in the raster's own
    /// coordinates; painting with no raster adds one at the top of the page,
    /// selected.
    fn stroked(
        &self,
        cx: &Context,
        rect: koharu_scene::Rect,
        coverage: &[u8],
    ) -> anyhow::Result<Vec<Effect>> {
        let label = match self.tool {
            Tool::Draw => "paint",
            _ => "erase",
        };
        let target = self
            .target
            .and_then(|layer| Some((layer, cx.snapshot.writable(layer)?)));
        let (ops, created) = match target {
            Some((layer, offset)) => {
                let own = koharu_scene::Rect {
                    x: rect.x - offset.x,
                    y: rect.y - offset.y,
                    ..rect
                };
                let mut pixels = cx.session.lock().pixels(layer, own)?;
                match self.tool {
                    Tool::Draw => {
                        let mut under = RgbaImage::from_raw(own.width, own.height, pixels)
                            .ok_or_else(|| anyhow::anyhow!("a raster's pixels are RGBA"))?;
                        imageops::overlay(&mut under, &paint(rect, coverage, cx.brush.color), 0, 0);
                        pixels = under.into_raw();
                    }
                    _ => erased(&mut pixels, coverage),
                }
                let write = Op::WritePixels {
                    layer: Ref::from(layer),
                    rect: own,
                    pixels,
                };
                (vec![write], None)
            }
            None if self.tool == Tool::Draw => {
                let pixels = paint(rect, coverage, cx.brush.color).into_raw();
                let (id, add) = added(cx.snapshot.page(), Content::Raster(Raster::default()));
                let write = Op::WritePixels {
                    layer: Ref::from(id),
                    rect,
                    pixels,
                };
                (vec![add, write], Some(id))
            }
            None => return Ok(Vec::new()),
        };
        let edit = Effect::Edit(Edit {
            label: label.to_owned(),
            ops,
        });
        Ok(std::iter::once(edit)
            .chain(created.map(|id| Effect::Selection(vec![id])))
            .collect())
    }
}

/// A round stroke through `points`, `diameter` wide, as the outline it
/// fills; a single point is a dot.
fn outline(points: &[Point], diameter: f64) -> BezPath {
    match points {
        [] => BezPath::new(),
        [point] => Circle::new(*point, diameter * 0.5).to_path(TOLERANCE),
        [first, rest @ ..] => {
            let mut line = BezPath::new();
            line.move_to(*first);
            for point in rest {
                line.line_to(*point);
            }
            let style = Style::new(diameter)
                .with_caps(Cap::Round)
                .with_join(Join::Round);
            stroke(line.iter(), &style, &StrokeOpts::default(), TOLERANCE)
        }
    }
}

/// `outline`'s antialiased 8-bit coverage over the page pixels it touches.
fn coverage(outline: &BezPath, page: Size) -> Option<(koharu_scene::Rect, Vec<u8>)> {
    let rect = koharu_scene::Rect::covering(
        outline.bounding_box(),
        page.width as u32,
        page.height as u32,
    )?;
    let (width, height) = (
        u16::try_from(rect.width).ok()?,
        u16::try_from(rect.height).ok()?,
    );
    let mut context = RenderContext::new(width, height);
    context.set_transform(Affine::translate((-f64::from(rect.x), -f64::from(rect.y))));
    context.set_paint(WHITE);
    context.fill_path(outline);
    context.flush();
    let mut pixmap = Pixmap::new(width, height);
    context.render(&mut pixmap, &mut Resources::new());
    Some((rect, pixmap.data().iter().map(|pixel| pixel.a).collect()))
}

/// The brush's `color` over `rect`, as opaque as `coverage` says of it.
fn paint(rect: koharu_scene::Rect, coverage: &[u8], Rgba([r, g, b, a]): Rgba) -> RgbaImage {
    let [r, g, b] = [r, g, b].map(|channel| (channel * 255.0).round() as u8);
    let pixels = coverage
        .iter()
        .flat_map(|&covered| [r, g, b, (f32::from(covered) * a).round() as u8])
        .collect();
    RgbaImage::from_raw(rect.width, rect.height, pixels).expect("coverage covers `rect`")
}

/// Straight-alpha RGBA `pixels` with their alpha taken away where
/// `coverage` says.
fn erased(pixels: &mut [u8], coverage: &[u8]) {
    for (pixel, &covered) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
        let kept = 1.0 - f32::from(covered) / 255.0;
        pixel[3] = (f32::from(pixel[3]) * kept).round() as u8;
    }
}
