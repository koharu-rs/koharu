//! A project's life: create it, import a page, add one layer of every kind,
//! write pixels, read everything back, and reopen it. One session holds a
//! project at a time.
//!
//! `cargo run -p koharu-scene --example project`

use std::io::Cursor;

use koharu_scene::{Created, Edit, Hash, Op, Rect, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn edit(label: &str, ops: Vec<Value>) -> Result<Edit> {
    Ok(Edit {
        label: label.to_owned(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    })
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("chapter.khr");
    let mut session = Session::create(&path)?;

    // The scan is kept byte for byte, and the page takes its size.
    let mut scan = Vec::new();
    image::RgbaImage::from_pixel(256, 384, image::Rgba([240, 240, 240, 255]))
        .write_to(&mut Cursor::new(&mut scan), image::ImageFormat::Png)?;
    let file = Hash::of(&scan);
    session.apply(Edit {
        label: "Import".into(),
        ops: vec![Op::AddFile { bytes: scan }],
    })?;
    let change = session.apply(edit(
        "Import",
        vec![json!({"op": "add_page", "index": 0, "background": file})],
    )?)?;
    let page = change.pages.added[0];

    let outline = "M8 8 L248 8 L248 376 L8 376 Z";
    let change = session.apply(edit("Add layers", vec![
        json!({"op": "add_layer", "id": "$panel", "page": page, "index": 0, "layer": {"kind": "panel", "name": "Panel", "outline": outline}}),
        json!({"op": "add_layer", "id": "$balloon", "page": page, "parent": "$panel", "index": 0, "layer": {"kind": "balloon", "name": "Balloon", "outline": "M40 40 L120 40 L120 160 L40 160 Z"}}),
        json!({"op": "add_layer", "id": "$block", "page": page, "parent": "$balloon", "index": 0, "layer": {
            "kind": "block", "name": "Text",
            "source": {"frame": {"box": {"bounds": {"x": 60.0, "y": 50.0, "width": 40.0, "height": 100.0}}}, "content": "こんにちは", "direction": "vertical", "language": "ja"},
        }}),
        json!({"op": "add_layer", "id": "$group", "page": page, "index": 1, "layer": {"kind": "group", "name": "Notes"}}),
        json!({"op": "add_layer", "page": page, "parent": "$group", "index": 0, "layer": {"kind": "raster", "name": "Paint"}}),
        json!({"op": "add_layer", "page": page, "parent": "$group", "index": 1, "layer": {"kind": "shape", "name": "Star", "path": "M10 10 L20 30 L0 30 Z", "fill": [1.0, 0.8, 0.0, 1.0]}}),
        json!({"op": "add_layer", "page": page, "parent": "$group", "index": 2, "layer": {"kind": "text", "name": "Note", "content": "Hello", "language": "en"}}),
        json!({"op": "add_layer", "page": page, "parent": "$group", "index": 3, "layer": {"kind": "image", "name": "Scan copy", "file": file}}),
        json!({"op": "add_layer", "id": "$mask", "page": page, "parent": "$group", "index": 0, "layer": {"kind": "mask"}}),
        json!({"op": "write_pixels", "layer": "$mask", "rect": {"x": 8, "y": 8, "width": 240, "height": 368}, "pixels": vec![255u8; 240 * 368]}),
    ])?)?;
    // A block's mask and clean-up are layers of its own, labelled from the
    // block's label.
    let labelled = |name: &str| {
        change.created.iter().find_map(|created| match created {
            Created::Layer {
                label: Some(label),
                id,
            } if label == name => Some(*id),
            _ => None,
        })
    };
    let (mask, cleanup) = (
        labelled("block.mask").ok_or("no mask")?,
        labelled("block.cleanup").ok_or("no clean-up")?,
    );
    let rect = Rect {
        x: 60,
        y: 50,
        width: 40,
        height: 100,
    };
    session.apply(Edit {
        label: "Clean".into(),
        ops: vec![
            Op::WritePixels {
                layer: mask.into(),
                rect,
                pixels: vec![255; 40 * 100],
            },
            Op::WritePixels {
                layer: cleanup.into(),
                rect,
                pixels: [255, 255, 255, 255].repeat(40 * 100),
            },
        ],
    })?;

    let project = session.project();
    for layer in project.layers_of(page)? {
        println!("{:?} {:?}", layer.kind(), layer.name);
    }
    let cleanup = session.tiles(cleanup, None)?;
    println!(
        "clean-up: {} tiles, {:?}",
        cleanup.tiles.len(),
        cleanup.format
    );

    drop(session);
    let reopened = Session::open(&path)?;
    assert_eq!(reopened.project(), project);
    println!(
        "reopened {} layers",
        reopened.project().layers_of(page)?.len()
    );
    Ok(())
}
