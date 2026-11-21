//! What the editor does: patch, lock, edit text, paint a raster and a block's
//! mask, duplicate, move, and remove, refresh from a change, undo and redo,
//! and the errors an edit can fail with.
//!
//! `cargo run -p koharu-scene --example editing`

use koharu_scene::{Change, Created, Edit, Error, LayerId, Op, Rect, Session};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn apply(session: &mut Session, label: &str, ops: Vec<Value>) -> koharu_scene::Result<Change> {
    let ops = ops
        .into_iter()
        .map(|op| serde_json::from_value(op).map_err(|error| Error::Rejected(error.to_string())))
        .collect::<koharu_scene::Result<_>>()?;
    session.apply(Edit {
        label: label.to_owned(),
        ops,
    })
}

fn created(change: &Change) -> Vec<LayerId> {
    change
        .created
        .iter()
        .filter_map(|created| match created {
            Created::Layer { id, .. } => Some(*id),
            Created::Page { .. } => None,
        })
        .collect()
}

/// What a canvas redraws after a change: only what it names.
fn refresh(change: &Change) {
    println!(
        "redraw pages {:?} and layers {:?}; undo {:?}",
        change.pages.changed,
        change.layers,
        change.history.undo()
    );
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("edit.khr"))?;
    let change = apply(
        &mut session,
        "New page",
        vec![json!({"op": "add_page", "index": 0, "size": {"width": 512, "height": 512}})],
    )?;
    let page = change.pages.added[0];
    let change = apply(
        &mut session,
        "Add",
        vec![
            json!({"op": "add_layer", "id": "$b", "page": page, "index": 0, "layer": {"kind": "balloon", "name": "Balloon", "outline": "M10 10 L200 10 L200 200 L10 200 Z"}}),
            json!({"op": "add_layer", "page": page, "parent": "$b", "index": 0, "layer": {"kind": "block", "name": "Text", "source": {"frame": {"box": {"bounds": {"x": 20.0, "y": 20.0, "width": 100.0, "height": 40.0}}}}}}),
            json!({"op": "add_layer", "page": page, "index": 1, "layer": {"kind": "raster", "name": "Paint"}}),
        ],
    )?;
    // A block comes with its mask and clean-up.
    let [balloon, block, mask, _cleanup, raster] = created(&change)[..] else {
        return Err("five layers".into());
    };
    refresh(&change);

    // Patch values; a lock stops edits inside, and its own lock comes off.
    refresh(&apply(
        &mut session,
        "Fade",
        vec![
            json!({"op": "update_layer", "layer": raster, "patch": {"opacity": 0.5, "blend": "multiply"}}),
        ],
    )?);
    apply(
        &mut session,
        "Lock",
        vec![json!({"op": "update_layer", "layer": balloon, "patch": {"locked": true}})],
    )?;
    let error = apply(
        &mut session,
        "Edit",
        vec![json!({"op": "set_text", "layer": block, "text": "Hi"})],
    )
    .expect_err("locked");
    println!("refused: {error}");
    apply(
        &mut session,
        "Unlock",
        vec![json!({"op": "update_layer", "layer": balloon, "patch": {"locked": false}})],
    )?;

    // Edit text: a block's translation is added on its first text.
    refresh(&apply(
        &mut session,
        "Letter",
        vec![json!({"op": "set_text", "layer": block, "text": "Hi there"})],
    )?);
    apply(
        &mut session,
        "Style",
        vec![
            json!({"op": "update_layer", "layer": block, "patch": {"translation": {"style": {"chars": {"size": 18.0}}}}}),
        ],
    )?;

    // Paint a raster and a block's mask.
    session.apply(Edit {
        label: "Paint".into(),
        ops: vec![
            Op::WritePixels {
                layer: raster.into(),
                rect: Rect {
                    x: -8,
                    y: 0,
                    width: 16,
                    height: 16,
                },
                pixels: [200, 0, 0, 255].repeat(256),
            },
            Op::WritePixels {
                layer: mask.into(),
                rect: Rect {
                    x: 20,
                    y: 20,
                    width: 100,
                    height: 40,
                },
                pixels: vec![255; 4000],
            },
        ],
    })?;
    println!("raster tiles: {}", session.tiles(raster, None)?.tiles.len());

    // Duplicate, move a text free of its balloon, and remove the copy.
    let copy = created(&apply(
        &mut session,
        "Duplicate",
        vec![json!({"op": "duplicate_layer", "layer": balloon})],
    )?)[0];
    refresh(&apply(
        &mut session,
        "Move",
        vec![json!({"op": "move_layer", "layer": block, "index": 0})],
    )?);
    refresh(&apply(
        &mut session,
        "Delete",
        vec![json!({"op": "remove_layer", "layer": copy})],
    )?);

    // Undo and redo restore whole projects, pixels included.
    let undone = session.undo()?.ok_or("nothing to undo")?;
    println!("undid; redo {:?}", undone.history.redo());
    session.redo()?;

    // Errors fail the whole edit and change nothing.
    let before = session.project();
    for (ops, what) in [
        (
            vec![json!({"op": "update_layer", "layer": raster, "patch": {"opacty": 1.0}})],
            "a typo",
        ),
        (
            vec![json!({"op": "move_layer", "layer": balloon, "parent": balloon, "index": 0})],
            "a layer inside itself",
        ),
        (
            vec![
                json!({"op": "add_layer", "page": page, "parent": block, "index": 0, "layer": {"kind": "group"}}),
            ],
            "a block holds nothing",
        ),
        (
            vec![
                json!({"op": "update_layer", "layer": block, "patch": {"translation": {"content": "x"}}}),
            ],
            "content outside set_text",
        ),
    ] {
        let error = apply(&mut session, "Fail", ops).expect_err(what);
        println!("{what}: {error}");
    }
    assert_eq!(session.project(), before);
    Ok(())
}
