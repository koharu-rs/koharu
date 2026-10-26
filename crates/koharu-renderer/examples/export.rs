//! What exports take from a rendered page. PNG and CBZ rasterize the whole
//! frame. PSD export rasterizes each layer's own content, without its
//! opacity, blend mode, or masks, which the PSD records as properties; each
//! masked layer's masks, merged into one; and each text's runs, so its type
//! layer names every font and size it drew with.
//!
//! `cargo run -p koharu-renderer --example export`

use std::collections::HashMap;

use koharu_renderer::{Renderer, TypesettingConfig};
use koharu_scene::{Author, Created, Edit, LayerKind, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("poster.koharu"))?;
    let red: Vec<u8> = [220, 40, 60, 255].repeat(64 * 64);
    let ops: Vec<Value> = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 320, "height": 240}}),
        json!({"op": "add_layer", "id": "$paint", "page": "$page", "index": 0, "layer": {
            "kind": "raster", "opacity": 0.9, "offset": {"x": 40, "y": 30},
        }}),
        json!({"op": "write_pixels", "layer": "$paint", "rect": {"x": 0, "y": 0, "width": 64, "height": 64}, "pixels": red}),
        json!({"op": "add_layer", "id": "$stripe", "page": "$page", "parent": "$paint", "index": 0, "layer": {
            "kind": "mask", "mode": "subtract", "content": {"path": {"path": "M40 60 H104 V70 H40 Z"}},
        }}),
        json!({"op": "add_layer", "id": "$evening", "page": "$page", "index": 1, "layer": {
            "kind": "group", "blend": "multiply", "opacity": 0.6,
        }}),
        json!({"op": "add_layer", "id": "$shade", "page": "$page", "parent": "$evening", "index": 0, "layer": {
            "kind": "shape", "path": "M0 160 H320 V240 H0 Z", "fill": [1.0, 0.75, 0.4, 1.0],
        }}),
        json!({"op": "add_layer", "id": "$draft", "page": "$page", "index": 2, "layer": {
            "kind": "shape", "visible": false, "path": "M200 20 H300 V80 H200 Z", "fill": [0.2, 0.6, 0.2, 1.0],
        }}),
        // 𠮷 is one character but two UTF-16 units, which is what a PSD counts.
        json!({"op": "add_layer", "id": "$sign", "page": "$page", "index": 3, "layer": {
            "kind": "text", "language": "ja", "transform": [[1.0, 0.0, 120.0], [0.0, 1.0, 180.0]],
            "content": [
                {"insert": "𠮷野家 ", "attributes": {"size": 28.0}},
                {"insert": "OPEN", "attributes": {"size": 24.0, "weight": 700, "stroke": [1.0, 1.0, 1.0, 1.0], "stroke_width": 2.0}},
            ],
        }}),
    ];
    let change = session.apply(Edit {
        project: session.id(),
        label: "Make the poster".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    })?;
    let labels: HashMap<_, _> = change
        .created
        .iter()
        .filter_map(|created| match created {
            Created::Layer {
                id,
                label: Some(label),
            } => Some((*id, label.as_str())),
            _ => None,
        })
        .collect();

    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, change.pages.added[0])?;
    let composite = renderer.rasterize(&frame)?;
    println!(
        "composite: {}×{}",
        composite.image.width(),
        composite.image.height()
    );

    for layer in frame.layers() {
        let label = labels[&layer.id()];
        let hidden = if layer.visible() { "" } else { ", hidden" };
        match layer.kind() {
            // A mask is merged into its layer's mask.
            LayerKind::Mask => continue,
            LayerKind::Group => println!("{label}: a folder{hidden}"),
            _ => {
                let own = renderer.rasterize(&frame.cropped(layer.id())?)?;
                let (width, height) = own.image.dimensions();
                println!(
                    "{label}: {width}×{height} at ({}, {}){hidden}",
                    own.left, own.top
                );
            }
        }
        if let Some(mask) = frame.mask(layer.id())? {
            let coverage = renderer.rasterize(&mask)?;
            println!(
                "  and a mask over the page, {} pixels covered",
                coverage.image.pixels().filter(|pixel| pixel[3] > 0).count()
            );
        }
        for run in layer.text().map_or(&[][..], |text| &text.runs) {
            let stroke = run
                .stroke
                .map_or(String::new(), |(_, width)| format!(", stroked {width} px"));
            println!(
                "  UTF-16 {:?}: {} at {} px{stroke}",
                run.range, run.post_script_name, run.size
            );
        }
    }
    Ok(())
}
