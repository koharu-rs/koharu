//! Models and the user write the same values, and the last writer wins. A
//! model's run records what it did as a C2PA action in the layer's `c2pa`
//! namespace, keyed by its step, in the same edit as its results, so a rerun
//! replaces it. The scene never reads it.
//!
//! `cargo run -p koharu-scene --example models`

use koharu_scene::{Content, Created, Edit, LayerId, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn apply(session: &mut Session, label: &str, ops: Vec<Value>) -> Result<Vec<Created>> {
    let ops = ops
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()?;
    Ok(session
        .apply(Edit {
            label: label.to_owned(),
            ops,
        })?
        .created)
}

/// A translation step's results and its action, in one edit.
fn translate(session: &mut Session, block: LayerId, model: &str, text: &str) -> Result<()> {
    apply(
        session,
        "Translate",
        vec![
            json!({"op": "set_text", "layer": block, "text": text}),
            json!({"op": "update_layer", "layer": block, "patch": {"translation": {"language": "en"}}}),
            json!({"op": "set_attribute", "layer": block, "namespace": "c2pa", "key": "translation", "value": {
                "action": "c2pa.translated",
                "softwareAgent": {"name": model, "version": "1"},
                "digitalSourceType": "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia",
                "parameters": {"sourceLanguage": "ja", "targetLanguage": "en"},
            }}),
        ],
    )?;
    Ok(())
}

fn translation(session: &Session, block: LayerId) -> Result<String> {
    let project = session.project();
    let Content::Block(block) = &project.layer(block)?.content else {
        return Err("not a block".into());
    };
    Ok(block
        .translation
        .as_ref()
        .map(|text| text.content.text())
        .unwrap_or_default())
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("models.khr"))?;
    let created = apply(
        &mut session,
        "Detect",
        vec![
            json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 600}}),
            json!({"op": "add_layer", "page": "$p", "index": 0, "layer": {"kind": "block", "source": {
                "frame": {"box": {"bounds": {"x": 10.0, "y": 10.0, "width": 50.0, "height": 120.0}}},
                "content": "やあ", "direction": "vertical", "language": "ja",
            }}}),
        ],
    )?;
    let Some(Created::Layer { id: block, .. }) = created.get(1).cloned() else {
        return Err("no block".into());
    };

    translate(&mut session, block, "gemma", "Hey")?;
    println!("model: {}", translation(&session, block)?);

    // The user's change replaces the model's.
    apply(
        &mut session,
        "Edit",
        vec![json!({"op": "set_text", "layer": block, "text": "Hey there"})],
    )?;
    println!("user: {}", translation(&session, block)?);

    // A rerun replaces the user's change, and its action replaces the last one.
    translate(&mut session, block, "lfm", "Yo")?;
    let project = session.project();
    let action = &project.layer(block)?.attributes["c2pa"]["translation"];
    println!(
        "rerun: {} by {}",
        translation(&session, block)?,
        action["softwareAgent"]["name"]
    );

    // Undo is the way back.
    session.undo()?;
    println!("undone: {}", translation(&session, block)?);
    Ok(())
}
