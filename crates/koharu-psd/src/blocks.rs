//! The tagged blocks that keep shapes and text editable in Photoshop: vector
//! masks, a shape's fill and stroke, type layers, and the stroke effect.
//! Layouts follow Adobe's "Photoshop File Formats Specification" and, where
//! it is silent, ag-psd (`src/additionalInfo.ts`, `src/descriptor.ts`).

use koharu_renderer::ResolvedText;
use koharu_scene::{
    Align, Color, Direction, FillRule, Frame as TextFrame, MaskMode, ShapeLayer, TextLayer,
};
use kurbo::{Affine, BezPath, PathEl, Point};
use photocraft_psd::{
    TaggedBlock, VersionedDescriptor,
    descriptor::{Descriptor, Id, UnicodeString, Value},
    path::{PathData, PsdKnot, PsdSubpath, VectorMaskBlock},
};

use crate::{
    Result,
    engine_data::{self, Run, Shape},
    parse,
};

/// One component of a vector mask: a path, how it combines with the
/// components before it, and the rule it fills by.
pub(crate) struct Component {
    pub path: BezPath,
    pub mode: MaskMode,
    pub fill_rule: FillRule,
}

/// A `vmsk` block for `components` in order, starting full unless the first
/// one adds. A component's first subpath carries its operation, after
/// psd-tools 1 adding, 2 subtracting, and 3 intersecting, and the rest join
/// it, as ag-psd's `writeVectorMask` writes them; bytes 4–5 of each length
/// record hold the subpath's fill rule, 1 for even-odd and 2 for non-zero.
pub(crate) fn vector_mask(components: &[Component], size: (u32, u32)) -> TaggedBlock {
    let mut subpaths = Vec::new();
    let mut rules = Vec::new();
    for component in components {
        for (index, mut subpath) in knots(&component.path, size).into_iter().enumerate() {
            subpath.operation = match (index, component.mode) {
                (0, MaskMode::Add) => 1,
                (0, MaskMode::Subtract) => 2,
                (0, MaskMode::Intersect) => 3,
                _ => -1,
            };
            subpaths.push(subpath);
            rules.push(match component.fill_rule {
                FillRule::EvenOdd => 1u16,
                FillRule::NonZero => 2,
            });
        }
    }
    let full = components
        .first()
        .is_some_and(|component| component.mode != MaskMode::Add);
    let mut path = PathData::from_subpaths(&subpaths, full);
    for (record, rule) in path
        .records
        .iter_mut()
        .filter(|record| record.is_length())
        .zip(rules)
    {
        record.data[4..6].copy_from_slice(&rule.to_be_bytes());
    }
    let mask = VectorMaskBlock {
        version: 3,
        flags: 0,
        path,
    };
    block(*b"vmsk", mask.to_bytes())
}

/// `path`'s subpaths as Bézier knots in fractions of the page. Lines and
/// quadratic curves become cubic segments, and a closed subpath that ends on
/// its start point closes onto its first knot.
fn knots(path: &BezPath, (width, height): (u32, u32)) -> Vec<PsdSubpath> {
    let fraction = |point: Point| (point.x / f64::from(width), point.y / f64::from(height));
    let mut subpaths = Vec::new();
    let mut points: Vec<[Point; 3]> = Vec::new();
    let mut finish = |points: &mut Vec<[Point; 3]>, closed: bool| {
        if closed
            && points.len() > 1
            && let Some([pre, anchor, _]) = points.last().copied()
            && anchor.distance(points[0][1]) < 1e-6
        {
            points[0][0] = pre;
            points.pop();
        }
        if !points.is_empty() {
            subpaths.push(PsdSubpath {
                closed,
                operation: -1,
                knots: points
                    .drain(..)
                    .map(|[pre, anchor, post]| PsdKnot {
                        linked: false,
                        pre: fraction(pre),
                        anchor: fraction(anchor),
                        post: fraction(post),
                    })
                    .collect(),
            });
        }
    };
    for element in path.elements() {
        let last = points.last().map(|[_, anchor, _]| *anchor);
        match *element {
            PathEl::MoveTo(point) => {
                finish(&mut points, false);
                points.push([point; 3]);
            }
            PathEl::LineTo(point) => points.push([point; 3]),
            PathEl::QuadTo(control, point) => {
                let start = last.unwrap_or(point);
                if let Some(knot) = points.last_mut() {
                    knot[2] = start + (control - start) * (2.0 / 3.0);
                }
                points.push([point + (control - point) * (2.0 / 3.0), point, point]);
            }
            PathEl::CurveTo(first, second, point) => {
                if let Some(knot) = points.last_mut() {
                    knot[2] = first;
                }
                points.push([second, point, point]);
            }
            PathEl::ClosePath => finish(&mut points, true),
        }
    }
    finish(&mut points, false);
    subpaths
}

/// A block whose length counts its padding: the spec rounds a block's length
/// up to an even byte count, and readers such as psd-tools skip exactly that
/// many bytes, so the zero that evens it out goes inside the data.
fn block(key: [u8; 4], mut data: Vec<u8>) -> TaggedBlock {
    if data.len() % 2 == 1 {
        data.push(0);
    }
    TaggedBlock::new(key, data)
}

fn versioned(descriptor: Descriptor) -> Vec<u8> {
    VersionedDescriptor::new(descriptor).to_bytes()
}

fn enumerated(type_id: &str, value: &str) -> Value {
    Value::Enumerated {
        type_id: Id::new(type_id),
        value: Id::new(value),
    }
}

fn unit(unit: &[u8; 4], value: f64) -> Value {
    Value::UnitFloat { unit: *unit, value }
}

/// A colour's alpha as a percentage, which a descriptor's colour leaves out.
fn opacity(color: Color) -> Value {
    unit(b"#Prc", f64::from(color.components[3]) * 100.0)
}

/// A colour as a descriptor holds it: red, green, and blue from 0 to 255.
fn rgb(color: Color) -> Value {
    let [red, green, blue, _] = color.components.map(|c| f64::from(c) * 255.0);
    Value::Descriptor(
        Descriptor::new("RGBC")
            .with("Rd  ", Value::Double(red))
            .with("Grn ", Value::Double(green))
            .with("Bl  ", Value::Double(blue)),
    )
}

/// A shape layer's blocks: its path as the vector mask, its fill as a solid
/// colour fill (`SoCo`), and its stroke as vector stroke data (`vstk`). With a
/// stroke, the fill moves to `vscg`, which names its kind first, as ag-psd
/// writes it; a stroke without a fill turns the fill off. A shape with
/// neither draws nothing and gets no blocks.
pub(crate) fn shape(layer: &ShapeLayer, size: (u32, u32)) -> Result<Vec<TaggedBlock>> {
    if layer.fill.is_none() && layer.stroke.is_none() {
        return Ok(Vec::new());
    }
    let mut blocks = vec![vector_mask(
        &[Component {
            path: parse(&layer.path)?,
            mode: MaskMode::Add,
            fill_rule: layer.fill_rule,
        }],
        size,
    )];
    let content = |color: Color| Descriptor::new("null").with("Clr ", rgb(color));
    let Some(stroke) = &layer.stroke else {
        blocks.extend(
            layer
                .fill
                .map(|fill| block(*b"SoCo", versioned(content(fill)))),
        );
        return Ok(blocks);
    };
    let mut vscg = b"SoCo".to_vec();
    vscg.extend(versioned(content(layer.fill.unwrap_or(stroke.color))));
    let style = Descriptor::new("strokeStyle")
        .with("strokeStyleVersion", Value::Integer(2))
        .with("strokeEnabled", Value::Boolean(true))
        .with("fillEnabled", Value::Boolean(layer.fill.is_some()))
        .with("strokeStyleLineWidth", unit(b"#Pxl", stroke.width))
        .with("strokeStyleLineDashOffset", unit(b"#Pxl", 0.0))
        // kurbo's default miter limit, which the renderer strokes with.
        .with("strokeStyleMiterLimit", Value::Double(4.0))
        .with(
            "strokeStyleLineCapType",
            enumerated("strokeStyleLineCapType", "strokeStyleButtCap"),
        )
        .with(
            "strokeStyleLineJoinType",
            enumerated("strokeStyleLineJoinType", "strokeStyleMiterJoin"),
        )
        .with(
            "strokeStyleLineAlignment",
            enumerated("strokeStyleLineAlignment", "strokeStyleAlignCenter"),
        )
        .with("strokeStyleScaleLock", Value::Boolean(false))
        .with("strokeStyleStrokeAdjust", Value::Boolean(false))
        .with("strokeStyleLineDashSet", Value::List(Vec::new()))
        .with("strokeStyleBlendMode", enumerated("BlnM", "Nrml"))
        .with("strokeStyleOpacity", opacity(stroke.color))
        .with(
            "strokeStyleContent",
            Value::Descriptor(Descriptor::new("solidColorLayer").with("Clr ", rgb(stroke.color))),
        )
        .with("strokeStyleResolution", Value::Double(72.0));
    blocks.extend([block(*b"vscg", vscg), block(*b"vstk", versioned(style))]);
    Ok(blocks)
}

const CR: u16 = b'\r' as u16;
const LF: u16 = b'\n' as u16;

/// A text layer's blocks as the renderer laid it out, which drew at least one
/// run: a type layer (`TySh`) holding its transform, a text descriptor with
/// the EngineData, and a warp descriptor; and, when every run shares one
/// stroke, an Outside stroke effect (`lfx2` with `FrFX`), how Photoshop
/// strokes type outside its glyphs as the renderer does. Strokes that differ
/// by span stay in the pixels only.
pub(crate) fn type_layer(layer: &TextLayer, text: &ResolvedText, index: i32) -> Vec<TaggedBlock> {
    let vertical = layer.direction == Direction::Vertical;
    let frame = text.frame;
    // Point type hangs from where its first line stands, by its alignment;
    // paragraph type fills a box from its corner, and a path frame is written
    // as paragraph type in the path's bounds.
    let (shape, anchor) = match layer.frame {
        TextFrame::Point => {
            // A right-to-left paragraph's lines start on the right.
            let along = |start: f64, end: f64| match text.align {
                Align::Start | Align::Justify => start,
                Align::Center => (start + end) / 2.0,
                Align::End => end,
            };
            let baseline = f64::from(text.baseline);
            let anchor = if vertical {
                Point::new(baseline, along(frame.y0, frame.y1))
            } else if text.rtl {
                Point::new(along(frame.x1, frame.x0), baseline)
            } else {
                Point::new(along(frame.x0, frame.x1), baseline)
            };
            (Shape::Point, anchor.to_vec2())
        }
        TextFrame::Box { .. } | TextFrame::Path { .. } => (
            Shape::Box(frame.width(), frame.height()),
            frame.origin().to_vec2(),
        ),
    };

    // Photoshop ends every paragraph with \r. A \r\n pair, one break to the
    // renderer, becomes one \r, which moves the offsets after it back by one.
    let source = layer.content.text();
    let units: Vec<u16> = source.encode_utf16().collect();
    let pairs: Vec<usize> = (1..units.len())
        .filter(|&at| units[at - 1] == CR && units[at] == LF)
        .collect();
    let offset = |at: usize| at - pairs.partition_point(|&pair| pair < at);
    let paragraphs = format!("{}\r", source.replace("\r\n", "\r").replace('\n', "\r"));
    let length = offset(units.len()) + 1;

    let mut drawn: Vec<_> = text.runs.iter().collect();
    drawn.sort_by_key(|run| run.range.start);
    let mut fonts = vec!["AdobeInvisFont".to_owned()];
    let mut runs = Vec::new();
    for (index, run) in drawn.iter().enumerate() {
        // Each run reaches the next one's start, so white space the renderer
        // didn't draw keeps the style before it, and the last run reaches the
        // final paragraph mark.
        let start = if index == 0 {
            0
        } else {
            offset(run.range.start)
        };
        let end = drawn
            .get(index + 1)
            .map_or(length, |next| offset(next.range.start));
        if end == start {
            continue;
        }
        let font = fonts
            .iter()
            .position(|name| *name == run.post_script_name)
            .unwrap_or_else(|| {
                fonts.push(run.post_script_name.clone());
                fonts.len() - 1
            });
        runs.push(Run {
            length: end - start,
            font,
            size: f64::from(run.size),
            fill: run.fill.components,
        });
    }
    let engine_data = engine_data::encode(&engine_data::Type {
        text: &paragraphs,
        runs: &runs,
        fonts: &fonts,
        align: text.align,
        rtl: text.rtl,
        leading: f64::from(text.line_height),
        vertical,
        shape,
    });

    let orientation = if vertical { "Vrtc" } else { "Hrzn" };
    let bounds = |class: &str| {
        let rect = frame - anchor;
        Value::Descriptor(
            Descriptor::new(class)
                .with("Left", unit(b"#Pnt", rect.x0))
                .with("Top ", unit(b"#Pnt", rect.y0))
                .with("Rght", unit(b"#Pnt", rect.x1))
                .with("Btom", unit(b"#Pnt", rect.y1)),
        )
    };
    let descriptor = Descriptor::new("TxLr")
        .with(
            "Txt ",
            Value::Text(UnicodeString::new_nul(&paragraphs[..paragraphs.len() - 1])),
        )
        .with("textGridding", enumerated("textGridding", "None"))
        .with("Ornt", enumerated("Ornt", orientation))
        .with("AntA", enumerated("Annt", "antiAliasSharp"))
        .with("bounds", bounds("bounds"))
        .with("boundingBox", bounds("boundingBox"))
        .with("TextIndex", Value::Integer(index))
        .with("EngineData", Value::RawData(engine_data));
    let warp = Descriptor::new("warp")
        .with("warpStyle", enumerated("warpStyle", "warpNone"))
        .with("warpValue", Value::Double(0.0))
        .with("warpPerspective", Value::Double(0.0))
        .with("warpPerspectiveOther", Value::Double(0.0))
        .with("warpRotate", enumerated("Ornt", orientation));
    // From the type's own coordinates, where its anchor is the origin, to
    // the page.
    let transform = Affine::from(layer.transform) * Affine::translate(anchor);
    let mut data = 1i16.to_be_bytes().to_vec();
    for coefficient in transform.as_coeffs() {
        data.extend(coefficient.to_be_bytes());
    }
    data.extend(50i16.to_be_bytes());
    data.extend(versioned(descriptor));
    data.extend(1i16.to_be_bytes());
    data.extend(versioned(warp));
    // Warped type keeps its warp's bounds here; Photoshop writes zeros for
    // type without a warp, as in ag-psd's test files.
    data.extend([0; 16]);

    let mut blocks = vec![block(*b"TySh", data)];
    let stroke = text.runs[0].stroke;
    if let Some((color, width)) = stroke
        && text.runs.iter().all(|run| run.stroke == stroke)
    {
        // An effect's size is in page pixels, where the renderer's stroke
        // grows with the layer's transform.
        let scale = transform.determinant().abs().sqrt();
        blocks.push(stroke_effect(color, f64::from(width) * scale));
    }
    blocks
}

fn stroke_effect(color: Color, width: f64) -> TaggedBlock {
    let stroke = Descriptor::new("FrFX")
        .with("enab", Value::Boolean(true))
        .with("present", Value::Boolean(true))
        .with("showInDialog", Value::Boolean(true))
        .with("Styl", enumerated("FStl", "OutF"))
        .with("PntT", enumerated("FrFl", "SClr"))
        .with("Md  ", enumerated("BlnM", "Nrml"))
        .with("Opct", opacity(color))
        .with("Sz  ", unit(b"#Pxl", width))
        .with("Clr ", rgb(color));
    let effects = Descriptor::new("null")
        .with("masterFXSwitch", Value::Boolean(true))
        .with("Scl ", unit(b"#Prc", 100.0))
        .with("FrFX", Value::Descriptor(stroke));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(versioned(effects));
    block(*b"lfx2", data)
}
