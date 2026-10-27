//! A type layer's EngineData: the PDF-like dictionary Photoshop's text engine
//! reads. The keys, their order, and the resource sets follow ag-psd's
//! encoder (`src/text.rs`, `encodeEngineData`); Photoshop fills in what isn't
//! written from its own defaults.

use koharu_scene::Align;

/// How the text sits: from its anchor, or wrapped in a box of this size.
pub(crate) enum Shape {
    Point,
    Box(f64, f64),
}

/// A style run: its length in UTF-16 units, its index in the font set, its
/// size, and its fill as straight RGBA from 0 to 1.
pub(crate) struct Run {
    pub length: usize,
    pub font: usize,
    pub size: f64,
    pub fill: [f32; 4],
}

pub(crate) struct Type<'a> {
    /// Paragraphs ending with `\r`, the last one included.
    pub text: &'a str,
    pub runs: &'a [Run],
    /// PostScript names; Photoshop's first font is always AdobeInvisFont.
    pub fonts: &'a [String],
    pub align: Align,
    /// Whether the text runs right to left, which puts the start of its lines
    /// on the right.
    pub rtl: bool,
    pub leading: f64,
    pub vertical: bool,
    pub shape: Shape,
}

enum Value {
    Int(i32),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Dict(Vec<(&'static str, Value)>),
}

use Value::{Array, Bool, Dict, Float, Int};

fn string(text: &str) -> Value {
    Value::String(text.to_owned())
}

fn floats(values: &[f64]) -> Value {
    Array(values.iter().copied().map(Float).collect())
}

fn color([r, g, b, a]: [f32; 4]) -> Value {
    Dict(vec![
        ("Type", Int(1)),
        ("Values", floats(&[a, r, g, b].map(f64::from))),
    ])
}

pub(crate) fn encode(spec: &Type) -> Vec<u8> {
    let (direction, procession) = if spec.vertical { (2, 1) } else { (0, 0) };
    let paragraph_run = || {
        Dict(vec![
            (
                "ParagraphSheet",
                Dict(vec![
                    ("DefaultStyleSheet", Int(0)),
                    ("Properties", paragraph(spec)),
                ]),
            ),
            (
                "Adjustments",
                Dict(vec![
                    ("Axis", floats(&[1.0, 0.0, 1.0])),
                    ("XY", floats(&[0.0, 0.0])),
                ]),
            ),
        ])
    };
    let paragraphs: Vec<i32> = spec.text.split_inclusive('\r').map(utf16).collect();
    let (shape, cookie) = match spec.shape {
        Shape::Point => (0, ("PointBase", floats(&[0.0, 0.0]))),
        Shape::Box(width, height) => (1, ("BoxBounds", floats(&[0.0, 0.0, width, height]))),
    };
    let root = Dict(vec![
        (
            "EngineDict",
            Dict(vec![
                ("Editor", Dict(vec![("Text", string(spec.text))])),
                (
                    "ParagraphRun",
                    Dict(vec![
                        ("DefaultRunData", paragraph_run()),
                        (
                            "RunArray",
                            Array(paragraphs.iter().map(|_| paragraph_run()).collect()),
                        ),
                        (
                            "RunLengthArray",
                            Array(paragraphs.into_iter().map(Int).collect()),
                        ),
                        ("IsJoinable", Int(1)),
                    ]),
                ),
                (
                    "StyleRun",
                    Dict(vec![
                        (
                            "DefaultRunData",
                            Dict(vec![(
                                "StyleSheet",
                                Dict(vec![("StyleSheetData", Dict(Vec::new()))]),
                            )]),
                        ),
                        (
                            "RunArray",
                            Array(
                                spec.runs
                                    .iter()
                                    .map(|run| {
                                        Dict(vec![(
                                            "StyleSheet",
                                            Dict(vec![("StyleSheetData", style(run))]),
                                        )])
                                    })
                                    .collect(),
                            ),
                        ),
                        (
                            "RunLengthArray",
                            Array(spec.runs.iter().map(|run| Int(run.length as i32)).collect()),
                        ),
                        ("IsJoinable", Int(2)),
                    ]),
                ),
                (
                    "GridInfo",
                    Dict(vec![
                        ("GridIsOn", Bool(false)),
                        ("ShowGrid", Bool(false)),
                        ("GridSize", Float(18.0)),
                        ("GridLeading", Float(22.0)),
                        ("GridColor", color([0.0, 0.0, 1.0, 1.0])),
                        ("GridLeadingFillColor", color([0.0, 0.0, 1.0, 1.0])),
                        ("AlignLineHeightToGridFlags", Bool(false)),
                    ]),
                ),
                ("AntiAlias", Int(4)),
                ("UseFractionalGlyphWidths", Bool(true)),
                (
                    "Rendered",
                    Dict(vec![
                        ("Version", Int(1)),
                        (
                            "Shapes",
                            Dict(vec![
                                ("WritingDirection", Int(direction)),
                                (
                                    "Children",
                                    Array(vec![Dict(vec![
                                        ("ShapeType", Int(shape)),
                                        ("Procession", Int(procession)),
                                        (
                                            "Lines",
                                            Dict(vec![
                                                ("WritingDirection", Int(direction)),
                                                ("Children", Array(Vec::new())),
                                            ]),
                                        ),
                                        (
                                            "Cookie",
                                            Dict(vec![(
                                                "Photoshop",
                                                Dict(vec![
                                                    ("ShapeType", Int(shape)),
                                                    cookie,
                                                    (
                                                        "Base",
                                                        Dict(vec![
                                                            ("ShapeType", Int(shape)),
                                                            (
                                                                "TransformPoint0",
                                                                floats(&[1.0, 0.0]),
                                                            ),
                                                            (
                                                                "TransformPoint1",
                                                                floats(&[0.0, 1.0]),
                                                            ),
                                                            (
                                                                "TransformPoint2",
                                                                floats(&[0.0, 0.0]),
                                                            ),
                                                        ]),
                                                    ),
                                                ]),
                                            )]),
                                        ),
                                    ])]),
                                ),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
        ("ResourceDict", resources(spec.fonts)),
        ("DocumentResources", resources(spec.fonts)),
    ]);
    let mut out = Vec::new();
    write(&mut out, &root, 0, false);
    out.push(b'\n');
    out
}

fn utf16(text: &str) -> i32 {
    text.encode_utf16().count() as i32
}

fn paragraph(spec: &Type) -> Value {
    // Photoshop's justification: 0 left, 1 right, 2 centre, and 3 and 4
    // justify with the last line left or right.
    let justification = match (spec.align, spec.rtl) {
        (Align::Start, false) | (Align::End, true) => 0,
        (Align::End, false) | (Align::Start, true) => 1,
        (Align::Center, _) => 2,
        (Align::Justify, false) => 3,
        (Align::Justify, true) => 4,
    };
    Dict(vec![
        ("Justification", Int(justification)),
        ("FirstLineIndent", Float(0.0)),
        ("StartIndent", Float(0.0)),
        ("EndIndent", Float(0.0)),
        ("SpaceBefore", Float(0.0)),
        ("SpaceAfter", Float(0.0)),
        ("AutoHyphenate", Bool(true)),
        ("HyphenatedWordSize", Int(6)),
        ("PreHyphen", Int(2)),
        ("PostHyphen", Int(2)),
        ("ConsecutiveHyphens", Int(2)),
        ("Zone", Float(36.0)),
        ("WordSpacing", floats(&[0.8, 1.0, 1.33])),
        ("LetterSpacing", floats(&[0.0, 0.0, 0.0])),
        ("GlyphSpacing", floats(&[1.0, 1.0, 1.0])),
        ("AutoLeading", Float(spec.leading)),
        ("LeadingType", Int(0)),
        ("Hanging", Bool(false)),
        ("Burasagari", Bool(false)),
        ("KinsokuOrder", Int(0)),
        ("EveryLineComposer", Bool(false)),
    ])
}

fn style(run: &Run) -> Value {
    Dict(vec![
        ("Font", Int(run.font as i32)),
        ("FontSize", Float(run.size)),
        ("FauxBold", Bool(false)),
        ("FauxItalic", Bool(false)),
        ("AutoLeading", Bool(true)),
        ("AutoKerning", Bool(true)),
        ("Kerning", Int(0)),
        ("FillColor", color(run.fill)),
    ])
}

fn resources(fonts: &[String]) -> Value {
    let font = |name: &str| {
        Dict(vec![
            ("Name", string(name)),
            ("Script", Int(0)),
            ("FontType", Int(0)),
            ("Synthetic", Int(0)),
        ])
    };
    Dict(vec![
        (
            "KinsokuSet",
            Array(vec![
                kinsoku("PhotoshopKinsokuHard", HARD_NO_START, HARD_NO_END),
                kinsoku("PhotoshopKinsokuSoft", SOFT_NO_START, SOFT_NO_END),
            ]),
        ),
        (
            "MojiKumiSet",
            Array(
                (1..=4)
                    .map(|set| {
                        Dict(vec![(
                            "InternalName",
                            string(&format!("Photoshop6MojiKumiSet{set}")),
                        )])
                    })
                    .collect(),
            ),
        ),
        ("TheNormalStyleSheet", Int(0)),
        ("TheNormalParagraphSheet", Int(0)),
        (
            "ParagraphSheetSet",
            Array(vec![Dict(vec![
                ("Name", string("Normal RGB")),
                ("DefaultStyleSheet", Int(0)),
                (
                    "Properties",
                    Dict(vec![("Justification", Int(0)), ("AutoLeading", Float(1.2))]),
                ),
            ])]),
        ),
        (
            "StyleSheetSet",
            Array(vec![Dict(vec![
                ("Name", string("Normal RGB")),
                (
                    "StyleSheetData",
                    Dict(vec![
                        ("Font", Int(i32::from(fonts.len() > 1))),
                        ("FontSize", Float(12.0)),
                        ("AutoLeading", Bool(true)),
                        ("FillColor", color([0.0, 0.0, 0.0, 1.0])),
                    ]),
                ),
            ])]),
        ),
        (
            "FontSet",
            Array(fonts.iter().map(|name| font(name)).collect()),
        ),
        ("SuperscriptSize", Float(0.583)),
        ("SuperscriptPosition", Float(0.333)),
        ("SubscriptSize", Float(0.583)),
        ("SubscriptPosition", Float(0.333)),
        ("SmallCapSize", Float(0.7)),
    ])
}

/// Photoshop's line-breaking sets for Japanese, as ag-psd writes them.
const HARD_NO_START: &str = "\u{3001}\u{3002}\u{ff0c}\u{ff0e}\u{30fb}\u{ff1a}\u{ff1b}\u{ff1f}\u{ff01}\u{30fc}\u{2015}\u{2019}\u{201d}\u{ff09}\u{3015}\u{ff3d}\u{ff5d}\u{3009}\u{300b}\u{300d}\u{300f}\u{3011}\u{30fd}\u{30fe}\u{309d}\u{309e}\u{3005}\u{3041}\u{3043}\u{3045}\u{3047}\u{3049}\u{3063}\u{3083}\u{3085}\u{3087}\u{308e}\u{30a1}\u{30a3}\u{30a5}\u{30a7}\u{30a9}\u{30c3}\u{30e3}\u{30e5}\u{30e7}\u{30ee}\u{30f5}\u{30f6}\u{309b}\u{309c}?!)]},.:;\u{2103}\u{2109}\u{00a2}\u{ff05}\u{2030}";
const HARD_NO_END: &str = "\u{2018}\u{201c}\u{ff08}\u{3014}\u{ff3b}\u{ff5b}\u{3008}\u{300a}\u{300c}\u{300e}\u{3010}([{\u{ffe5}\u{ff04}\u{00a3}\u{ff20}\u{00a7}\u{3012}\u{ff03}";
const SOFT_NO_START: &str = "\u{3001}\u{3002}\u{ff0c}\u{ff0e}\u{30fb}\u{ff1a}\u{ff1b}\u{ff1f}\u{ff01}\u{2019}\u{201d}\u{ff09}\u{3015}\u{ff3d}\u{ff5d}\u{3009}\u{300b}\u{300d}\u{300f}\u{3011}\u{30fd}\u{30fe}\u{309d}\u{309e}\u{3005}";
const SOFT_NO_END: &str =
    "\u{2018}\u{201c}\u{ff08}\u{3014}\u{ff3b}\u{ff5b}\u{3008}\u{300a}\u{300c}\u{300e}\u{3010}";

fn kinsoku(name: &str, no_start: &str, no_end: &str) -> Value {
    Dict(vec![
        ("Name", string(name)),
        ("NoStart", string(no_start)),
        ("NoEnd", string(no_end)),
        ("Keep", string("\u{2015}\u{2025}")),
        ("Hanging", string("\u{3001}\u{3002}.,")),
    ])
}

/// Writes `value` as Photoshop does: tab-indented dictionaries, inline arrays
/// of scalars, real numbers always with a decimal point, and strings as
/// UTF-16 big-endian with a byte-order mark.
fn write(out: &mut Vec<u8>, value: &Value, indent: usize, in_property: bool) {
    let prefix = |out: &mut Vec<u8>| {
        if in_property {
            out.push(b' ');
        } else {
            out.extend(std::iter::repeat_n(b'\t', indent));
        }
    };
    match value {
        Int(number) => {
            prefix(out);
            out.extend_from_slice(number.to_string().as_bytes());
        }
        Float(number) => {
            prefix(out);
            let mut text = format!("{number:.5}");
            while text.ends_with('0') && !text.ends_with(".0") {
                text.pop();
            }
            out.extend_from_slice(text.as_bytes());
        }
        Bool(flag) => {
            prefix(out);
            out.extend_from_slice(if *flag { b"true" } else { b"false" });
        }
        Value::String(text) => {
            prefix(out);
            out.extend_from_slice(b"(\xFE\xFF");
            for byte in text.encode_utf16().flat_map(u16::to_be_bytes) {
                if matches!(byte, b'(' | b')' | b'\\') {
                    out.push(b'\\');
                }
                out.push(byte);
            }
            out.push(b')');
        }
        Array(items)
            if items
                .iter()
                .all(|item| matches!(item, Int(_) | Float(_) | Bool(_))) =>
        {
            prefix(out);
            out.push(b'[');
            for item in items {
                write(out, item, 0, true);
            }
            out.extend_from_slice(b" ]");
        }
        Array(items) => {
            prefix(out);
            out.extend_from_slice(b"[\n");
            for item in items {
                write(out, item, indent + 1, false);
                out.push(b'\n');
            }
            out.extend(std::iter::repeat_n(b'\t', indent));
            out.push(b']');
        }
        Dict(entries) => {
            if in_property {
                out.push(b'\n');
            }
            out.extend(std::iter::repeat_n(b'\t', indent));
            out.extend_from_slice(b"<<\n");
            for (entry, value) in entries {
                out.extend(std::iter::repeat_n(b'\t', indent + 1));
                out.push(b'/');
                out.extend_from_slice(entry.as_bytes());
                write(out, value, indent + 1, true);
                out.push(b'\n');
            }
            out.extend(std::iter::repeat_n(b'\t', indent));
            out.extend_from_slice(b">>");
        }
    }
}
