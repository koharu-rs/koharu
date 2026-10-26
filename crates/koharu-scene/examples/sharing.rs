//! One session shared by the editor and a background model. The model reads
//! under the lock, works without it, and applies its result naming what it
//! read; if the user changed that meanwhile, the result is stale and nothing
//! is written. An edit that changes nothing adds no undo step.
//!
//! `cargo run -p koharu-scene --example sharing`

use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
};

use koharu_scene::{
    Action, Author, Edit, Error, LayerId, Op, Pointer, Session, Size, SoftwareAgent,
};
use serde_json::{Map, Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn user_edit(session: &Session, label: &str, ops: Vec<Op>) -> Edit {
    Edit {
        project: session.id(),
        label: label.to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops,
    }
}

fn op(value: Value) -> Op {
    serde_json::from_value(value).expect("a valid op")
}

/// A model run in the background: read under the lock, compute without it,
/// and apply under it. `between` runs after the read, standing in for
/// whatever the user does while the model works.
fn run_model(
    session: &Arc<Mutex<Session>>,
    layer: LayerId,
    between: impl FnOnce(),
) -> koharu_scene::Result<()> {
    let content = Pointer::from_static("/content");
    let (text, read) = session.lock().expect("not poisoned").read(layer, content)?;
    between();
    let result = text.as_str().unwrap_or_default().to_uppercase();
    let mut session = session.lock().expect("not poisoned");
    let edit = Edit {
        project: session.id(),
        label: "Shout".to_owned(),
        author: Author::Model {
            action: Action {
                action: "c2pa.edited".to_owned(),
                software_agent: SoftwareAgent {
                    name: "shouter".to_owned(),
                    version: None,
                },
                digital_source_type: None,
                parameters: Map::new(),
            },
        },
        reads: Vec::new(),
        ops: vec![Op::SetText {
            layer: layer.into(),
            text: result,
            ingredients: vec![read.parent_of()],
        }],
    };
    session.apply(edit).map(|_| ())
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    let change = session.apply(user_edit(&session, "Set up", vec![
        Op::AddPage {
            id: Some("$page".to_owned().try_into()?),
            index: 0,
            size: Size { width: 800, height: 1200 },
        },
        op(json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {"kind": "text", "content": "hello"}})),
    ]))?;
    let text = match &change.created[1] {
        koharu_scene::Created::Layer { id, .. } => *id,
        koharu_scene::Created::Page { .. } => unreachable!("the layer comes second"),
    };
    // `Session` is `Send` and `Sync`, so it can sit behind a mutex that the
    // UI and background work share.
    let session = Arc::new(Mutex::new(session));

    // The model works on another thread; the user edits the same text before
    // it finishes, so its result is stale and rejected.
    let (read_done, after_read) = mpsc::channel();
    let (edited, after_edit) = mpsc::channel::<()>();
    let worker = thread::spawn({
        let session = session.clone();
        move || {
            run_model(&session, text, || {
                read_done.send(()).expect("the main thread waits");
                after_edit.recv().expect("the main thread edits");
            })
        }
    });
    after_read.recv()?;
    {
        let mut session = session.lock().expect("not poisoned");
        let edit = user_edit(
            &session,
            "Edit text",
            vec![Op::SetText {
                layer: text.into(),
                text: "hello there".to_owned(),
                ingredients: Vec::new(),
            }],
        );
        session.apply(edit)?;
    }
    edited.send(())?;
    let result = worker.join().expect("the worker finished");
    assert!(matches!(result, Err(Error::Stale { .. })));
    println!("the model's result was stale: {}", result.unwrap_err());

    // Run again with nothing in between, and it lands.
    let worker = thread::spawn({
        let session = session.clone();
        move || run_model(&session, text, || {})
    });
    worker.join().expect("the worker finished")?;
    let mut session = Arc::into_inner(session)
        .expect("the workers are done")
        .into_inner()?;
    let shown = session.read(text, Pointer::from_static("/content"))?.0;
    assert_eq!(shown, "HELLO THERE");
    println!("the rerun wrote {shown}");

    // An edit that changes nothing writes nothing and adds no undo step.
    let before = session.history();
    let change = session.apply(user_edit(
        &session,
        "Rename",
        vec![op(json!({
            "op": "update_layer", "layer": text, "patch": {"name": ""},
        }))],
    ))?;
    assert!(change.layers.is_empty() && change.pages.changed.is_empty());
    assert_eq!(session.history(), before);
    println!("a no-op edit left the history at {:?}", before.undo);
    Ok(())
}
