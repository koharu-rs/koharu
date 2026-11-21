//! One session shared by the editor and a background model. The model reads a
//! snapshot of the project, which stays as it was while the editor goes on,
//! and holds the session only to apply its result. An edit that changes
//! nothing adds no undo step.
//!
//! `cargo run -p koharu-scene --example sharing`

use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
};

use koharu_scene::{Content, Created, Edit, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn edit(label: &str, ops: Vec<Value>) -> Result<Edit> {
    Ok(Edit {
        label: label.to_owned(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    })
}

fn lock(session: &Mutex<Session>) -> MutexGuard<'_, Session> {
    session.lock().unwrap_or_else(PoisonError::into_inner)
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let session = Arc::new(Mutex::new(Session::create(dir.path().join("shared.khr"))?));
    let change = lock(&session).apply(edit("Detect", vec![
        json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 600}}),
        json!({"op": "add_layer", "page": "$p", "index": 0, "layer": {"kind": "block", "source": {
            "frame": {"box": {"bounds": {"x": 10.0, "y": 10.0, "width": 40.0, "height": 80.0}}}, "content": "おはよう",
        }}}),
    ])?)?;
    let Some(Created::Layer { id: block, .. }) = change.created.get(1).cloned() else {
        return Err("no block".into());
    };

    // The model translates from a snapshot, without holding the session.
    let snapshot = lock(&session).project();
    let model = {
        let session = Arc::clone(&session);
        thread::spawn(move || -> Result<()> {
            let Content::Block(source) = &snapshot.layer(block)?.content else {
                return Err("not a block".into());
            };
            let text = format!("Good morning ({})", source.source.content.text());
            lock(&session).apply(edit(
                "Translate",
                vec![json!({"op": "set_text", "layer": block, "text": text})],
            )?)?;
            Ok(())
        })
    };

    // Meanwhile the editor renames the block.
    lock(&session).apply(edit(
        "Rename",
        vec![json!({"op": "update_layer", "layer": block, "patch": {"name": "Greeting"}})],
    )?)?;
    model.join().map_err(|_| "the model panicked")??;

    let mut session = lock(&session);
    let history = session.history();
    let same = session.apply(edit(
        "Rename",
        vec![json!({"op": "update_layer", "layer": block, "patch": {"name": "Greeting"}})],
    )?)?;
    assert!(same.layers.is_empty() && session.history() == history);
    let project = session.project();
    let layer = project.layer(block)?;
    let Content::Block(done) = &layer.content else {
        return Err("not a block".into());
    };
    println!(
        "{}: {}",
        layer.name,
        done.translation
            .as_ref()
            .map(|text| text.content.text())
            .unwrap_or_default()
    );
    Ok(())
}
