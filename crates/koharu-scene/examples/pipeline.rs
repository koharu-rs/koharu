//! The translation workflow writing a page as the README's section 4 lays it
//! out: detection adds panels, balloons, and blocks in reading order, OCR fills
//! the sources, inpainting the clean-ups, and translation the translations
//! with their frames. Each step writes its own fields, and a rerun replaces
//! what it wrote, a user's change included.
//!
//! `cargo run -p koharu-scene --example pipeline`

use koharu_scene::{Block, Content, Edit, Frame, LayerId, Op, Rect, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn apply(session: &mut Session, label: &str, ops: Vec<Value>) -> Result<()> {
    let ops = ops
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()?;
    session.apply(Edit {
        label: label.to_owned(),
        ops,
    })?;
    Ok(())
}

fn bounds(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y, "width": 40.0, "height": 90.0})
}

fn block(x: f64, y: f64, role: &str) -> Value {
    json!({"kind": "block", "role": role, "source": {"frame": {"box": {"bounds": bounds(x, y)}}, "direction": "vertical"}})
}

/// The blocks of the only page, in reading order.
fn blocks(session: &Session) -> Result<Vec<(LayerId, Block)>> {
    let project = session.project();
    let page = project.pages().next().ok_or("no page")?.id;
    Ok(project
        .layers_of(page)?
        .into_iter()
        .filter_map(|layer| match &layer.content {
            Content::Block(block) => Some((layer.id, (**block).clone())),
            _ => None,
        })
        .collect())
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("pipeline.khr"))?;
    apply(
        &mut session,
        "Import",
        vec![
            json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 600, "height": 900}}),
        ],
    )?;
    let page = session.project().pages().next().ok_or("no page")?.id;

    // Detection: panel > balloon > two texts, a sound effect and narration free
    // in the panel, a balloon that crosses panels on the page, the next panel,
    // and a text outside every panel.
    let panel = "M0 0 L600 0 L600 440 L0 440 Z";
    apply(
        &mut session,
        "Detect",
        vec![
            json!({"op": "add_layer", "id": "$panel", "page": page, "index": 0, "layer": {"kind": "panel", "name": "Panel", "outline": panel}}),
            json!({"op": "add_layer", "id": "$balloon", "page": page, "parent": "$panel", "index": 0, "layer": {"kind": "balloon", "name": "Balloon", "outline": "M300 20 L560 20 L560 200 L300 200 Z"}}),
            json!({"op": "add_layer", "page": page, "parent": "$balloon", "index": 0, "layer": block(500.0, 40.0, "text")}),
            json!({"op": "add_layer", "page": page, "parent": "$balloon", "index": 1, "layer": block(420.0, 40.0, "text")}),
            json!({"op": "add_layer", "page": page, "parent": "$panel", "index": 1, "layer": block(100.0, 250.0, "sfx")}),
            json!({"op": "add_layer", "page": page, "parent": "$panel", "index": 2, "layer": block(30.0, 30.0, "text")}),
            json!({"op": "add_layer", "id": "$crossing", "page": page, "index": 1, "layer": {"kind": "balloon", "outline": "M200 380 L400 380 L400 520 L200 520 Z"}}),
            json!({"op": "add_layer", "page": page, "parent": "$crossing", "index": 0, "layer": block(280.0, 400.0, "text")}),
            json!({"op": "add_layer", "page": page, "index": 2, "layer": {"kind": "panel", "outline": "M0 460 L600 460 L600 900 L0 900 Z"}}),
            json!({"op": "add_layer", "page": page, "index": 3, "layer": block(550.0, 800.0, "text")}),
        ],
    )?;
    let found = blocks(&session)?;
    println!("{} blocks in reading order", found.len());

    // OCR fills each source; inpainting writes each mask and clean-up, only
    // in the tiles it changes.
    let mut ops = Vec::new();
    for (index, (id, _)) in found.iter().enumerate() {
        ops.push(json!({"op": "update_layer", "layer": id, "patch": {"source": {"content": format!("テキスト{index}"), "language": "ja"}}}));
        ops.push(json!({"op": "set_attribute", "layer": id, "namespace": "c2pa", "key": "ocr", "value": {"action": "c2pa.created"}}));
    }
    apply(&mut session, "OCR", ops)?;
    let mut ops = Vec::new();
    for (_, block) in &found {
        let Frame::Box { bounds, .. } = block.source.frame else {
            continue;
        };
        let rect = Rect {
            x: bounds.x as i32,
            y: bounds.y as i32,
            width: bounds.width as u32,
            height: bounds.height as u32,
        };
        let area = (rect.width * rect.height) as usize;
        ops.push(Op::WritePixels {
            layer: block.mask.id.into(),
            rect,
            pixels: vec![255; area],
        });
        ops.push(Op::WritePixels {
            layer: block.cleanup.id.into(),
            rect,
            pixels: [255, 255, 255, 255].repeat(area),
        });
    }
    session.apply(Edit {
        label: "Inpaint".into(),
        ops,
    })?;

    // Translation reads the page's sources in reading order, and places each
    // translation in its source's frame, filling it at every render.
    let translate = |found: &[(LayerId, Block)], suffix: &str| -> Vec<Value> {
        found
            .iter()
            .flat_map(|(id, block)| {
                let mut frame = json!(block.source.frame);
                if let Some(placed) = frame.get_mut("box") {
                    placed["fit"] = json!("best");
                    placed["anchor"] = json!("center");
                }
                [
                    json!({"op": "set_text", "layer": id, "text": format!("{}{suffix}", block.source.content.text())}),
                    json!({"op": "update_layer", "layer": id, "patch": {"translation": {"language": "en", "frame": frame}}}),
                ]
            })
            .collect()
    };
    let found = blocks(&session)?;
    apply(&mut session, "Translate", translate(&found, " (en)"))?;

    // The user fixes one translation; a rerun replaces it.
    let (first, _) = &found[0];
    apply(
        &mut session,
        "Edit",
        vec![json!({"op": "set_text", "layer": first, "text": "A better line"})],
    )?;
    apply(
        &mut session,
        "Translate",
        translate(&found[..1], " (rerun)"),
    )?;
    for (id, block) in blocks(&session)? {
        let text = block
            .translation
            .map(|text| text.content.text())
            .unwrap_or_default();
        println!(
            "{id} {:?}: {} -> {text}",
            block.role,
            block.source.content.text()
        );
    }
    Ok(())
}
