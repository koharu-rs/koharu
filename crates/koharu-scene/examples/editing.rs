//! What the editor does: patch properties, lock, edit text, paint, duplicate,
//! move, and remove layers and pages, undo and redo, and handle the errors
//! an edit can fail with.
//!
//! `cargo run -p koharu-scene --example editing`

use koharu_scene::{
    Author, Change, Created, Edit, Error, Layer, LayerId, Op, PageId, Pointer, Rect, Ref, Session,
    Size, TextDelta,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn label(name: &str) -> Ref {
    Ref::try_from(format!("${name}")).expect("a valid label")
}

/// An edit by the user, as the editor's commands send it.
fn edit(session: &Session, label: &str, ops: Vec<Op>) -> Edit {
    Edit {
        project: session.id(),
        label: label.to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops,
    }
}

/// An op in its JSON form, which is what the editor's commands send.
fn op(value: Value) -> Op {
    serde_json::from_value(value).expect("a valid op")
}

fn created(change: &Change, name: &str) -> LayerId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, label } if label.as_deref() == Some(name) => Some(*id),
            _ => None,
        })
        .expect("the layer was created")
}

fn text_layer(session: &Session, layer: LayerId) -> Result<koharu_scene::TextLayer> {
    match session.layer(layer)?.view.layer {
        Layer::Text(text) => Ok(text),
        _ => Err("not a text layer".into()),
    }
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    let change = session.apply(edit(&session, "Set up", vec![
        Op::AddPage {
            id: Some(label("page")),
            index: 0,
            size: Size { width: 1200, height: 1700 },
        },
        op(json!({"op": "add_layer", "id": "$inpaint", "page": "$page", "index": 0, "layer": {"kind": "raster"}})),
        op(json!({"op": "add_layer", "id": "$balloon", "page": "$page", "index": 1, "layer": {"kind": "group", "name": "Balloon"}})),
        op(json!({"op": "add_layer", "id": "$outline", "page": "$page", "parent": "$balloon", "index": 0, "layer": {
            "kind": "shape", "path": "M100 100 H400 V300 H100 Z",
            "stroke": {"color": [0.0, 0.0, 0.0, 1.0], "width": 3.0},
        }})),
        op(json!({"op": "add_layer", "id": "$line", "page": "$page", "parent": "$balloon", "index": 1, "layer": {
            "kind": "text",
            "content": [{"insert": "Hello, ", "attributes": {"weight": 700}}, {"insert": "world"}],
        }})),
    ]))?;
    let page: PageId = match &change.created[0] {
        Created::Page { id, .. } => *id,
        Created::Layer { .. } => unreachable!("the page comes first"),
    };
    let (inpaint, balloon, outline, line) = (
        created(&change, "inpaint"),
        created(&change, "balloon"),
        created(&change, "outline"),
        created(&change, "line"),
    );

    // Properties change with a JSON Merge Patch; `null` clears an optional
    // property.
    session.apply(edit(
        &session,
        "Restyle",
        vec![
            op(json!({"op": "update_layer", "layer": line, "patch": {
                "name": "Line 1",
                "style": {"chars": {"size": 28.0}, "align": "center"},
                "transform": [[1.0, 0.0, 40.0], [0.0, 1.0, 12.0]],
            }})),
            op(json!({"op": "update_layer", "layer": outline, "patch": {"stroke": null}})),
        ],
    ))?;
    let text = text_layer(&session, line)?;
    assert_eq!(
        (text.name.as_str(), text.style.chars.size),
        ("Line 1", Some(28.0))
    );
    let Layer::Shape(shape) = session.layer(outline)?.view.layer else {
        unreachable!("the outline is a shape")
    };
    assert!(shape.stroke.is_none());
    println!("patched the text and cleared the outline's stroke");

    // A locked layer, or one inside it, can't change; unlocking is the one
    // change a locked layer takes.
    session.apply(edit(
        &session,
        "Lock",
        vec![op(
            json!({"op": "update_layer", "layer": balloon, "patch": {"locked": true}}),
        )],
    ))?;
    let error = session
        .apply(edit(
            &session,
            "Rename",
            vec![op(
                json!({"op": "update_layer", "layer": line, "patch": {"name": "x"}}),
            )],
        ))
        .unwrap_err();
    assert!(matches!(error, Error::Locked { layer, .. } if layer == balloon));
    println!("rejected: {error}");
    session.apply(edit(
        &session,
        "Unlock",
        vec![op(
            json!({"op": "update_layer", "layer": balloon, "patch": {"locked": false}}),
        )],
    ))?;

    // Replacing the text keeps the marks on the part that didn't change.
    session.apply(edit(
        &session,
        "Edit text",
        vec![Op::SetText {
            layer: line.into(),
            text: "Hello, there".to_owned(),
            ingredients: Vec::new(),
        }],
    ))?;
    let text = text_layer(&session, line)?;
    assert_eq!(text.content.text(), "Hello, there");
    assert_eq!(text.content.0[0].attributes.weight, Some(700));

    // The rich editor sends Loro deltas. A delta's positions count the plain
    // text it was made from, so the edit carries a read of `/content`, the
    // plain text, and fails as stale if that text changed since.
    let content = Pointer::from_static("/content");
    let (_, read) = session.read(line, content)?;
    let italic: Vec<TextDelta> = serde_json::from_value(
        json!([{"retain": 7}, {"retain": 5, "attributes": {"italic": true}}]),
    )?;
    session.apply(Edit {
        reads: vec![read.clone()],
        ..edit(
            &session,
            "Italic",
            vec![Op::EditText {
                layer: line.into(),
                delta: italic.clone(),
            }],
        )
    })?;
    let text = text_layer(&session, line)?;
    assert_eq!(
        text.content.0.last().map(|span| span.attributes.italic),
        Some(Some(true))
    );
    session.apply(edit(
        &session,
        "Edit text",
        vec![Op::SetText {
            layer: line.into(),
            text: "Hello, there!".to_owned(),
            ingredients: Vec::new(),
        }],
    ))?;
    let error = session
        .apply(Edit {
            reads: vec![read],
            ..edit(
                &session,
                "Italic",
                vec![Op::EditText {
                    layer: line.into(),
                    delta: italic,
                }],
            )
        })
        .unwrap_err();
    assert!(matches!(error, Error::Stale { .. }));
    println!("rejected: {error}");

    // Painting replaces pixels; painting transparent ones drops their tiles.
    let rect = Rect {
        x: 10,
        y: 10,
        width: 32,
        height: 32,
    };
    session.apply(edit(
        &session,
        "Brush",
        vec![Op::WritePixels {
            layer: inpaint.into(),
            rect,
            pixels: [255, 255, 255, 255].repeat(32 * 32),
        }],
    ))?;
    assert_eq!(session.tiles(inpaint, None)?.tiles.len(), 1);
    session.apply(edit(
        &session,
        "Erase",
        vec![Op::WritePixels {
            layer: inpaint.into(),
            rect,
            pixels: vec![0; 32 * 32 * 4],
        }],
    ))?;
    assert!(session.tiles(inpaint, None)?.tiles.is_empty());
    println!("painted and erased");

    // A duplicate copies the layer with everything inside it, just above the
    // original. The app names the copy in the same edit, since the word
    // "copy" is translated.
    let change = session.apply(edit(
        &session,
        "Duplicate",
        vec![
            op(json!({"op": "duplicate_layer", "layer": balloon, "id": "$copy"})),
            op(json!({"op": "update_layer", "layer": "$copy", "patch": {"name": "Balloon copy"}})),
        ],
    ))?;
    let copy = created(&change, "copy");
    assert_eq!(session.page(page)?.layers[2].id, copy);
    session.apply(edit(
        &session,
        "Move",
        vec![op(json!({"op": "move_layer", "layer": copy, "index": 0}))],
    ))?;
    assert_eq!(session.page(page)?.layers[0].id, copy);
    session.apply(edit(
        &session,
        "Delete",
        vec![op(json!({"op": "remove_layer", "layer": copy}))],
    ))?;
    assert_eq!(session.page(page)?.layers.len(), 2);
    println!("duplicated, moved, and removed a group");

    // Pages move and go the same way.
    let change = session.apply(edit(
        &session,
        "Add page",
        vec![Op::AddPage {
            id: None,
            index: 1,
            size: Size {
                width: 1200,
                height: 1700,
            },
        }],
    ))?;
    let second = change.pages.added[0];
    session.apply(edit(
        &session,
        "Move page",
        vec![op(json!({"op": "move_page", "page": second, "index": 0}))],
    ))?;
    assert_eq!(session.project()?.pages[0].id, second);
    session.apply(edit(
        &session,
        "Delete page",
        vec![op(json!({"op": "remove_page", "page": second}))],
    ))?;
    assert_eq!(session.project()?.pages.len(), 1);

    // Every edit is one undo step. Undo and redo report what changed, and the
    // history names the steps for the menu.
    assert_eq!(session.history().undo.as_deref(), Some("Delete page"));
    let undone = session.undo()?.expect("there is a step to undo");
    assert_eq!(undone.pages.added, vec![second]);
    assert_eq!(session.history().redo.as_deref(), Some("Delete page"));
    let redone = session.redo()?.expect("there is a step to redo");
    assert_eq!(redone.pages.removed, vec![second]);
    println!("undid and redid {:?}", session.history().undo);

    // Errors reach the UI as JSON with a stable code.
    let error = session
        .apply(edit(
            &session,
            "Rename",
            vec![op(
                json!({"op": "update_layer", "layer": LayerId::new(), "patch": {"name": "x"}}),
            )],
        ))
        .unwrap_err();
    let json = serde_json::to_value(&error)?;
    assert_eq!(json["error"], "not_found");
    println!("{json}");
    Ok(())
}
