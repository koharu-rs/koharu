//! What the app's PSD export does: render a page and write it as a layered
//! PSD, then read the file back and print its layers, bottom to top. Given a
//! path, it also saves the file there, to open in Photoshop or GIMP.
//!
//! `cargo run -p koharu-psd --example export -- [page.psd]`

use std::io::Cursor;

use koharu_psd::{Labels, export};
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::{Edit, Hash, Session};
use photocraft_psd::PsdFile;
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.khr"))?;
    let mut scan = Vec::new();
    image::RgbaImage::from_pixel(320, 240, image::Rgba([180, 200, 230, 255]))
        .write_to(&mut Cursor::new(&mut scan), image::ImageFormat::Png)?;
    let file = Hash::of(&scan);
    let balloon = "M60 100 A100 70 0 1 0 260 100 A100 70 0 1 0 60 100 Z";
    let clean = [255u8, 255, 255, 255].repeat(120 * 60);
    let ops: Vec<Value> = vec![
        json!({"op": "add_file", "bytes": scan}),
        json!({"op": "add_page", "id": "$page", "index": 0, "background": file}),
        json!({"op": "add_layer", "id": "$panel", "page": "$page", "index": 0, "layer": {"kind": "panel", "name": "Panel", "outline": "M10 10 H310 V180 H10 Z"}}),
        json!({"op": "add_layer", "id": "$balloon", "page": "$page", "parent": "$panel", "index": 0, "layer": {"kind": "balloon", "name": "Balloon", "outline": balloon}}),
        json!({"op": "add_layer", "id": "$line", "page": "$page", "parent": "$balloon", "index": 0, "layer": {"kind": "block", "name": "Line", "source": {
            "frame": {"path": {"path": balloon}}, "content": "今夜は流星群だって！", "direction": "vertical", "language": "ja",
        }}}),
        json!({"op": "write_pixels", "layer": "$line.cleanup", "rect": {"x": 100, "y": 70, "width": 120, "height": 60}, "pixels": clean}),
        json!({"op": "write_pixels", "layer": "$line.mask", "rect": {"x": 100, "y": 70, "width": 120, "height": 60}, "pixels": vec![255u8; 120 * 60]}),
        json!({"op": "set_text", "layer": "$line", "text": "Did you hear? There's a meteor shower tonight!"}),
        json!({"op": "update_layer", "layer": "$line", "patch": {"translation": {
            "language": "en", "style": {"chars": {"size": 20.0}, "align": "center"},
            "frame": {"path": {"path": balloon, "anchor": "center"}},
        }}}),
        json!({"op": "add_layer", "id": "$sound", "page": "$page", "index": 1, "layer": {"kind": "block", "name": "Sound", "role": "sfx", "source": {
            "frame": {"box": {"bounds": {"x": 200.0, "y": 190.0, "width": 100.0, "height": 40.0}}}, "content": "ザアア",
        }}}),
        json!({"op": "set_text", "layer": "$sound", "text": "FSSHHH"}),
        json!({"op": "update_layer", "layer": "$sound", "patch": {"translation": {
            "language": "en", "style": {"chars": {"size": 28.0, "stroke": [1.0, 1.0, 1.0, 1.0], "stroke_width": 3.0}},
            "transform": [[1.0, 0.0, 200.0], [0.0, 1.0, 200.0]],
        }}}),
    ];
    let change = session.apply(Edit {
        label: "Letter".to_owned(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    })?;

    let renderer = Renderer::new(RendererConfig::default());
    let labels = Labels {
        background: "Background".into(),
        cleanup: "Clean-up".into(),
        outline: "Balloon outline".into(),
    };
    let bytes = export(&session, &renderer, change.pages.added[0], &labels)?;
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
        println!("  {:16} {:?} {:?}", layer.name(), layer.rect, keys);
    }
    if let Some(path) = std::env::args().nth(1) {
        fs_err::write(&path, &bytes)?;
        println!("wrote {path}");
    }
    Ok(())
}
