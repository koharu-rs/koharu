//! What the pipeline does once a page is translated: one block per text, in
//! its balloon, its translation set in the balloon's outline at the page's
//! dialogue size. The renderer fits each line; when typesetting sets every
//! translation larger, each one fits again on the next render.
//!
//! The scan and the clean-ups would sit under these texts; they don't change
//! how the text fits, so the example leaves them out.
//!
//! `cargo run -p koharu-renderer --example typeset`

use koharu_renderer::{Frame, NodeId, Renderer, TypesettingConfig};
use koharu_scene::{Change, Created, Edit, LayerId, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Each balloon's outline and its translation.
const BALLOONS: [(&str, &str); 3] = [
    (
        "M80 200 A140 110 0 1 0 360 200 A140 110 0 1 0 80 200 Z",
        "Hey, did you hear? There's a meteor shower tonight!",
    ),
    (
        "M440 220 A70 160 0 1 0 580 220 A70 160 0 1 0 440 220 Z",
        "But the forecast says it's going to rain...",
    ),
    (
        "M120 520 A200 80 0 1 0 520 520 A200 80 0 1 0 120 520 Z",
        "Then why don't we wait for it on the roof?",
    ),
];

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.khr"))?;
    let mut ops = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 640, "height": 640}}),
    ];
    for (index, (outline, _)) in BALLOONS.iter().enumerate() {
        let balloon = format!("$balloon{index}");
        ops.push(json!({"op": "add_layer", "id": balloon, "page": "$page", "index": index, "layer": {"kind": "balloon", "outline": outline}}));
        ops.push(json!({"op": "add_layer", "id": format!("$text{index}"), "page": "$page", "parent": balloon, "index": 0, "layer": {
            "kind": "block", "source": {"frame": {"path": {"path": outline}}, "language": "ja"},
        }}));
    }
    let change = apply(&mut session, ops)?;
    let page = change.pages.added[0];
    let blocks = texts(&change);

    let mut ops = Vec::new();
    for (block, (outline, english)) in blocks.iter().zip(BALLOONS) {
        ops.push(json!({"op": "set_text", "layer": block, "text": english}));
        ops.push(
            json!({"op": "update_layer", "layer": block, "patch": {"translation": {
                "language": "en",
                "style": {"chars": {"size": 22.0}, "align": "center"},
                "frame": {"path": {"path": outline, "anchor": "center"}},
            }}}),
        );
    }
    apply(&mut session, ops)?;

    let renderer = Renderer::new(TypesettingConfig::default());
    println!("the dialogue at 22 px:");
    report(&renderer.render(&session, page)?, &blocks);

    let ops = blocks
        .iter()
        .map(|block| json!({"op": "update_layer", "layer": block, "patch": {"translation": {"style": {"chars": {"size": 30.0}}}}}))
        .collect();
    apply(&mut session, ops)?;
    println!("the dialogue at 30 px:");
    report(&renderer.render(&session, page)?, &blocks);
    Ok(())
}

/// The blocks, in the order the balloons were added.
fn texts(change: &Change) -> Vec<LayerId> {
    change
        .created
        .iter()
        .filter_map(|created| match created {
            Created::Layer {
                id,
                label: Some(label),
            } if label.starts_with("text") => Some(*id),
            _ => None,
        })
        .collect()
}

/// How each translation fit, as the canvas's inspector shows it.
fn report(frame: &Frame, blocks: &[LayerId]) {
    for (index, block) in blocks.iter().enumerate() {
        let Some(text) = frame
            .layer(NodeId::Translation(*block))
            .and_then(|layer| layer.text())
        else {
            continue;
        };
        println!(
            "  balloon {}: drawn at {:.0}% of its size, {} hyphen(s){}",
            index + 1,
            text.scale * 100.0,
            text.hyphens.len(),
            if text.overflowed { ", overflowing" } else { "" },
        );
    }
    if frame.needs_fonts() {
        println!("  (a fallback drew some text; Renderer::load_fonts downloads its font)");
    }
}

fn apply(session: &mut Session, ops: Vec<Value>) -> Result<Change> {
    let edit = Edit {
        label: "Typeset".to_owned(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    };
    Ok(session.apply(edit)?)
}
