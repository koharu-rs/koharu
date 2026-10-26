//! Renders a page to a PNG: a scanned page under a balloon with path text,
//! vertical text, a stroked caption, justified box text, and a masked
//! multiply group, then prints what each text resolved.
//!
//! `cargo run -p koharu-renderer --example page -- page.png`

use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};
use koharu_renderer::{Renderer, TypesettingConfig};
use koharu_scene::{Author, Created, Edit, LayerKind, Op, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const BALLOON: &str = "M150 260 A230 150 0 1 0 610 260 A230 150 0 1 0 150 260 Z";

fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "page.png".to_owned());
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("page.koharu"))?;

    // A page of screentone dots, standing in for a scan.
    let scan = RgbImage::from_fn(800, 1100, |x, y| {
        let dot = ((x % 12) as i32 - 6).pow(2) + ((y % 12) as i32 - 6).pow(2) < 9;
        if dot && y > 520 {
            Rgb([150, 150, 150])
        } else {
            Rgb([250, 248, 240])
        }
    });
    let mut png = Vec::new();
    scan.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
    let file = koharu_scene::Hash::of(&png);

    let ops: Vec<Value> = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 800, "height": 1100}}),
        json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {"kind": "image", "file": file}}),
        json!({"op": "add_layer", "page": "$page", "index": 1, "layer": {
            "kind": "shape", "path": BALLOON, "fill": [1.0, 1.0, 1.0, 1.0],
            "stroke": {"color": [0.0, 0.0, 0.0, 1.0], "width": 4.0},
        }}),
        json!({"op": "add_layer", "id": "$balloon", "page": "$page", "index": 2, "layer": {
            "kind": "text", "name": "balloon",
            "content": "I never thought the old lighthouse would still be standing after all these years!",
            "style": {"chars": {"size": 34.0}, "align": "center"},
            "language": "en",
            "frame": {"path": {"path": BALLOON, "anchor": "center"}},
        }}),
        json!({"op": "add_layer", "id": "$vertical", "page": "$page", "index": 3, "layer": {
            "kind": "text", "name": "vertical",
            "content": "ドキドキする!!\nまさか本当に?",
            "style": {"chars": {"size": 40.0}},
            "direction": "vertical", "language": "ja",
            "transform": [[1.0, 0.0, 740.0], [0.0, 1.0, 460.0]],
        }}),
        json!({"op": "add_layer", "id": "$caption", "page": "$page", "index": 4, "layer": {
            "kind": "text", "name": "caption",
            "content": [{"insert": "THOOM", "attributes": {"weight": 700, "size": 96.0}}, {"insert": "\nthe door slams"}],
            "style": {"chars": {"size": 30.0, "fill": [0.1, 0.1, 0.1, 1.0], "stroke": [1.0, 1.0, 1.0, 1.0], "stroke_width": 4.0}},
            "language": "en",
            "transform": [[1.0, 0.0, 60.0], [0.0, 1.0, 600.0]],
        }}),
        json!({"op": "add_layer", "id": "$narration", "page": "$page", "index": 5, "layer": {
            "kind": "text", "name": "narration",
            "content": "Narration boxes set their text justified, stretching every line but the last to both edges of the box.",
            "style": {"chars": {"size": 24.0}, "align": "justify"},
            "language": "en",
            "frame": {"box": {"bounds": {"x": 60.0, "y": 880.0, "width": 420.0, "height": 160.0}, "anchor": "start"}},
        }}),
        json!({"op": "add_layer", "id": "$tint", "page": "$page", "index": 6, "layer": {
            "kind": "group", "blend": "multiply", "opacity": 0.8,
        }}),
        json!({"op": "add_layer", "page": "$page", "parent": "$tint", "index": 0, "layer": {
            "kind": "shape", "path": "M500 820 H760 V1060 H500 Z", "fill": [0.95, 0.35, 0.3, 1.0],
        }}),
        json!({"op": "add_layer", "page": "$page", "parent": "$tint", "index": 0, "layer": {
            "kind": "mask", "content": {"path": {"path": "M630 820 L760 940 L630 1060 L500 940 Z"}},
        }}),
    ];
    let mut all = vec![Op::AddFile {
        media_type: "image/png".to_owned(),
        bytes: png,
    }];
    all.extend(
        ops.into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<Op>, _>>()?,
    );
    let change = session.apply(Edit {
        project: session.id(),
        label: "Set up".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: all,
    })?;
    let page = change.pages.added[0];

    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page)?;
    let raster = renderer.rasterize(&frame)?;
    raster.image.save(&out)?;
    println!(
        "wrote {out}, {}×{}",
        raster.image.width(),
        raster.image.height()
    );

    for created in &change.created {
        let Created::Layer {
            id,
            label: Some(label),
        } = created
        else {
            continue;
        };
        let Some(layer) = frame
            .layer(*id)
            .filter(|layer| layer.kind() == LayerKind::Text)
        else {
            continue;
        };
        let text = layer.text().expect("a text layer resolves its text");
        let faces: Vec<&str> = text
            .runs
            .iter()
            .map(|run| run.post_script_name.as_str())
            .collect();
        println!(
            "{label}: scale {:.2}, {} hyphens, {} breaks in words, overflowed {}, faces {faces:?}",
            text.scale,
            text.hyphens.len(),
            text.broken.len(),
            text.overflowed,
        );
    }
    if frame.needs_fonts() {
        println!("some text drew with a fallback until its bundled font downloads");
    }
    Ok(())
}
