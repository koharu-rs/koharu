//! Helpers the integration tests share; each test file uses a few of them.
#![allow(dead_code)]

pub use koharu_scene::*;
pub use serde_json::{Value, json};
pub use tempfile::TempDir;

/// A path for a new project in a directory removed when the guard drops.
pub fn path() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.koharu");
    (dir, path)
}

/// A new project with one 640×960 page.
pub fn session() -> (TempDir, Session, PageId) {
    let (dir, path) = path();
    let mut session = Session::create(path).unwrap();
    let page = page(&mut session);
    (dir, session, page)
}

pub fn user() -> Author {
    Author::User { via: None }
}

pub fn model(action: &str, name: &str) -> Author {
    Author::Model {
        action: Action {
            action: action.into(),
            software_agent: SoftwareAgent {
                name: name.into(),
                version: None,
            },
            digital_source_type: None,
            parameters: Default::default(),
        },
    }
}

pub fn edit(session: &Session, author: Author, ops: Vec<Op>) -> Edit {
    Edit {
        project: session.id(),
        label: "test".into(),
        author,
        reads: vec![],
        ops,
    }
}

pub fn op(value: Value) -> Op {
    serde_json::from_value(value).unwrap()
}

pub fn label(name: &str) -> Ref {
    Ref::try_from(format!("${name}")).unwrap()
}

pub fn pointer(path: &str) -> PointerBuf {
    PointerBuf::parse(path).unwrap()
}

pub fn page(session: &mut Session) -> PageId {
    let change = session
        .apply(edit(
            session,
            user(),
            vec![op(
                json!({"op": "add_page", "index": 0, "size": {"width": 640, "height": 960}}),
            )],
        ))
        .unwrap();
    change.pages.added[0]
}

pub fn created_layer(change: &Change, index: usize) -> LayerId {
    match &change.created[index] {
        Created::Layer { id, .. } => *id,
        other => panic!("{other:?}"),
    }
}

pub fn name(layer: &Layer) -> &str {
    match layer {
        Layer::Group(layer) => &layer.name,
        Layer::Image(layer) => &layer.name,
        Layer::Raster(layer) => &layer.name,
        Layer::Shape(layer) => &layer.name,
        Layer::Text(layer) => &layer.name,
        Layer::Mask(layer) => &layer.name,
    }
}

pub fn text(session: &Session, layer: LayerId) -> TextLayer {
    match session.layer(layer).unwrap().view.layer {
        Layer::Text(text) => *text,
        other => panic!("{other:?}"),
    }
}

/// Spans that carry marks.
pub fn marked(text: &RichText) -> Vec<&Span> {
    text.0
        .iter()
        .filter(|span| span.attributes != CharStyle::default())
        .collect()
}

pub fn delta(value: Value) -> Vec<TextDelta> {
    serde_json::from_value(value).unwrap()
}
