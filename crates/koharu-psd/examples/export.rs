//! What the app's PSD export does: render a page and write it as a layered
//! PSD, then read the file back and print its layers, bottom to top. Given a
//! path, it also saves the file there, to open in Photoshop or GIMP.
//!
//! `cargo run -p koharu-psd --example export -- [page.psd]`

use koharu_psd::export_page;
use koharu_renderer::{Renderer, TypesettingConfig};
use koharu_scene::{Author, Edit, Session};
use photocraft_psd::PsdFile;
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    let tone = [180u8, 200, 230, 255].repeat(320 * 240);
    let balloon = "M60 100 A100 70 0 1 0 260 100 A100 70 0 1 0 60 100 Z";
    let ops: Vec<Value> = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 320, "height": 240}}),
        json!({"op": "add_layer", "id": "$scan", "page": "$page", "index": 0, "layer": {"kind": "raster", "name": "Scan"}}),
        json!({"op": "write_pixels", "layer": "$scan", "rect": {"x": 0, "y": 0, "width": 320, "height": 240}, "pixels": tone}),
        json!({"op": "add_layer", "id": "$group", "page": "$page", "index": 1, "layer": {"kind": "group", "name": "Balloon"}}),
        json!({"op": "add_layer", "page": "$page", "parent": "$group", "index": 0, "layer": {
            "kind": "shape", "name": "Outline", "path": balloon,
            "fill": [1.0, 1.0, 1.0, 1.0], "stroke": {"color": [0.0, 0.0, 0.0, 1.0], "width": 3.0},
        }}),
        json!({"op": "add_layer", "page": "$page", "parent": "$group", "index": 1, "layer": {
            "kind": "text", "name": "Line", "language": "en",
            "content": "Did you hear? There's a meteor shower tonight!",
            "style": {"chars": {"size": 20.0}, "align": "center"},
            "frame": {"path": {"path": balloon, "anchor": "center"}},
        }}),
        json!({"op": "add_layer", "page": "$page", "index": 2, "layer": {
            "kind": "text", "name": "Sound", "language": "en", "content": "FSSHHH",
            "style": {"chars": {"size": 28.0, "stroke": [1.0, 1.0, 1.0, 1.0], "stroke_width": 3.0}},
            "transform": [[1.0, 0.0, 200.0], [0.0, 1.0, 200.0]],
        }}),
    ];
    let change = session.apply(Edit {
        project: session.id(),
        label: "Letter".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    })?;

    let renderer = Renderer::new(TypesettingConfig::default());
    let bytes = export_page(&session, &renderer, change.pages.added[0])?;
    let file = PsdFile::from_bytes(&bytes)?;
    println!(
        "{}×{}, {} bytes",
        file.header.width,
        file.header.height,
        bytes.len()
    );
    for layer in file.layers() {
        let keys: Vec<String> = layer
            .blocks
            .iter()
            .map(|block| String::from_utf8_lossy(&block.key).into_owned())
            .collect();
        println!("  {:14} {:?} {:?}", layer.name(), layer.rect, keys);
    }
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &bytes)?;
        println!("wrote {path}");
    }
    Ok(())
}
