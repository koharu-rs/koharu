//! What the pipeline does once a page is translated: one text layer per
//! source block, set in its balloon's outline and naming one dialogue style.
//! The renderer fits each line; when the editor sets the style larger, every
//! line fits again on the next render.
//!
//! The scan and its inpainting would sit under these layers; they don't
//! change how the text fits, so the example leaves them out.
//!
//! `cargo run -p koharu-renderer --example typeset`

use koharu_renderer::{Frame, Renderer, TypesettingConfig};
use koharu_scene::{Author, Change, Created, Edit, Session, StyleId};
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
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    let dialogue = StyleId::new();
    let mut ops = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 640, "height": 640}}),
        json!({"op": "add_text_style", "id": dialogue, "style": {
            "name": "Dialogue", "style": {"chars": {"size": 22.0}, "align": "center"},
        }}),
    ];
    for (index, (outline, english)) in BALLOONS.iter().enumerate() {
        ops.push(json!({"op": "add_layer", "id": format!("$balloon{}", index + 1), "page": "$page", "index": index, "layer": {
            "kind": "text", "content": english, "text_style": dialogue, "language": "en",
            "frame": {"path": {"path": outline, "anchor": "center"}},
        }}));
    }
    let change = apply(&mut session, ops)?;
    let page = change.pages.added[0];

    let renderer = Renderer::new(TypesettingConfig::default());
    println!("the dialogue at 22 px:");
    report(&renderer.render(&session, page)?, &change);

    apply(
        &mut session,
        vec![
            json!({"op": "update_text_style", "style": dialogue, "patch": {
                "style": {"chars": {"size": 30.0}},
            }}),
        ],
    )?;
    println!("the dialogue at 30 px:");
    report(&renderer.render(&session, page)?, &change);
    Ok(())
}

/// How each text fit, as the canvas's inspector shows it.
fn report(frame: &Frame, change: &Change) {
    for created in &change.created {
        let Created::Layer {
            id,
            label: Some(label),
        } = created
        else {
            continue;
        };
        let Some(text) = frame.layer(*id).and_then(|layer| layer.text()) else {
            continue;
        };
        println!(
            "  {label}: drawn at {:.0}% of its size, {} hyphen(s){}",
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
        project: session.id(),
        label: "Typeset".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    };
    Ok(session.apply(edit)?)
}
