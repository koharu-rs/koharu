#![allow(dead_code)]

use std::path::PathBuf;

use koharu_scene::{Change, Content, Created, Edit, Layer, LayerId, Op, PageId, Session};
use serde_json::{Value, json};
use tempfile::TempDir;

pub fn path() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("test.khr");
    (dir, path)
}

/// A new project holding one 64×64 page.
pub fn session() -> (TempDir, Session, PageId) {
    let (dir, path) = path();
    let mut session = Session::create(&path).expect("a new project");
    let page = page(&mut session);
    (dir, session, page)
}

pub fn edit(ops: Vec<Op>) -> Edit {
    Edit {
        label: "test".to_owned(),
        ops,
    }
}

pub fn op(value: Value) -> Op {
    serde_json::from_value(value).expect("a valid op")
}

pub fn apply(session: &mut Session, ops: Vec<Value>) -> koharu_scene::Result<Change> {
    session.apply(edit(ops.into_iter().map(op).collect()))
}

/// Adds a 64×64 page at the end.
pub fn page(session: &mut Session) -> PageId {
    let index = session.project().pages().count();
    let change = apply(
        session,
        vec![json!({"op": "add_page", "index": index, "size": {"width": 64, "height": 64}})],
    )
    .expect("a page");
    change.pages.added[0]
}

pub fn created_layer(change: &Change, index: usize) -> LayerId {
    match &change.created[index] {
        Created::Layer { id, .. } => *id,
        other => panic!("created {other:?}, not a layer"),
    }
}

/// Adds `layer` under `parent`, or at the top of `page`, at the end.
pub fn add(session: &mut Session, page: PageId, parent: Option<LayerId>, layer: Value) -> LayerId {
    let project = session.project();
    let index = match parent {
        Some(parent) => project.layer(parent).expect("the parent").children.len(),
        None => project.page(page).expect("the page").layers.len(),
    };
    let change = apply(
        session,
        vec![json!({"op": "add_layer", "page": page, "parent": parent, "index": index, "layer": layer})],
    )
    .expect("a layer");
    created_layer(&change, 0)
}

pub fn layer(session: &Session, id: LayerId) -> Layer {
    session.project().layer(id).expect("the layer").clone()
}

/// A block's mask and clean-up.
pub fn parts(session: &Session, block: LayerId) -> (LayerId, LayerId) {
    match &layer(session, block).content {
        Content::Block(block) => (block.mask.id, block.cleanup.id),
        other => panic!("{other:?} is not a block"),
    }
}

pub fn block(x: f64) -> Value {
    json!({"kind": "block", "source": {"frame": {"box": {"bounds": {"x": x, "y": 0.0, "width": 10.0, "height": 10.0}}}}})
}

pub fn outline() -> Value {
    json!("M0 0 L64 0 L64 64 L0 64 Z")
}
