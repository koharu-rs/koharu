//! The structure of a project through edits, undo, redo, and reopening, and
//! the edits that must fail without writing.

mod common;
use common::*;

#[test]
fn structure_survives_save_reopen_and_undo() {
    let (_dir, path) = path();
    let mut session = Session::create(&path).unwrap();
    let page = page(&mut session);
    let bytes = b"fake png bytes".to_vec();
    let hash = Hash::of(&bytes);
    let change = session
        .apply(edit(&session, user(), vec![
            Op::AddFile { media_type: "image/png".into(), bytes },
            op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "image", "name": "scan", "file": hash}})),
            op(json!({"op": "add_layer", "id": "$g", "page": page, "index": 1, "layer": {"kind": "group", "name": "block"}})),
            op(json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "text", "content": "Hello", "language": "en"}})),
            op(json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "mask", "content": {"path": {"path": "M0 0 L10 0 L10 10 Z"}}}})),
            op(json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "shape", "path": "M0 0 L5 5", "fill": [1.0, 0.0, 0.0, 1.0]}})),
        ]))
        .unwrap();
    assert_eq!(change.created.len(), 5);
    let view = session.page(page).unwrap();
    assert_eq!(view.layers.len(), 2);
    let group = &view.layers[1];
    assert_eq!(group.children.len(), 2, "shape then text");
    assert!(matches!(group.children[0].layer, Layer::Shape(_)));
    assert!(matches!(group.children[1].layer, Layer::Text(_)));
    assert_eq!(group.masks.len(), 1);

    session.save().unwrap();
    drop(session);
    let mut session = Session::open(&path).unwrap();
    assert_eq!(
        session.page(page).unwrap(),
        view,
        "the reopened view matches"
    );
    assert!(
        session.undo().unwrap().is_none(),
        "nothing to undo after reopening"
    );

    // Undo brings a removed layer back with its id; redo removes it again.
    let text = group.children[1].id;
    let change = session
        .apply(edit(
            &session,
            user(),
            vec![Op::RemoveLayer { layer: text.into() }],
        ))
        .unwrap();
    assert!(change.layers.contains(&text));
    assert!(session.layer(text).is_err());
    let undone = session.undo().unwrap().unwrap();
    assert!(undone.layers.contains(&text), "{undone:?}");
    assert!(undone.pages.changed.contains(&page));
    assert_eq!(session.layer(text).unwrap().view.id, text);
    assert_eq!(common::text(&session, text).content.text(), "Hello");
    let redone = session.redo().unwrap().unwrap();
    assert!(redone.layers.contains(&text));
    assert!(session.layer(text).is_err());

    // Removing the image layer frees its file, and undo restores both.
    let image = view.layers[0].id;
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::RemoveLayer {
                layer: image.into(),
            }],
        ))
        .unwrap();
    assert!(session.file(&hash).is_err());
    session.undo().unwrap().unwrap();
    assert_eq!(session.file(&hash).unwrap().bytes, b"fake png bytes");

    session.compact().unwrap();
    drop(session);
    let session = Session::open(&path).unwrap();
    fn count(layers: &[LayerView]) -> usize {
        layers
            .iter()
            .map(|layer| 1 + count(&layer.children) + count(&layer.masks))
            .sum()
    }
    let project = session.project().unwrap();
    assert_eq!(count(&project.pages[0].layers), 4);
    assert_eq!(project.pages[0], session.page(page).unwrap());
}

#[test]
fn undo_reports_page_and_layer_changes() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, user(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "a"}}))]))
        .unwrap();
    let text = created_layer(&change, 0);
    let change = session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "add_page", "index": 1, "size": {"width": 10, "height": 10}}),
            )],
        ))
        .unwrap();
    let second = change.pages.added[0];
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::MovePage {
                page: second.into(),
                index: 0,
            }],
        ))
        .unwrap();
    assert!(session.undo().unwrap().unwrap().pages.reordered);
    assert_eq!(session.undo().unwrap().unwrap().pages.removed, vec![second]);
    assert_eq!(session.redo().unwrap().unwrap().pages.added, vec![second]);

    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": text, "patch": {"name": "renamed"}}),
            )],
        ))
        .unwrap();
    let undone = session.undo().unwrap().unwrap();
    assert!(undone.layers.contains(&text), "{undone:?}");
    assert!(undone.pages.changed.contains(&page));

    // An edit that changes a layer and then removes its page undoes whole.
    session.save().unwrap();
    session
        .apply(edit(
            &session,
            user(),
            vec![
                op(json!({"op": "update_layer", "layer": text, "patch": {"name": "x"}})),
                Op::RemovePage { page: page.into() },
            ],
        ))
        .unwrap();
    session.undo().unwrap().unwrap();
    assert_eq!(name(&session.layer(text).unwrap().view.layer), "");
}

#[test]
fn moves_locks_and_invalid_edits() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, user(), vec![
            op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "group", "name": "a"}})),
            op(json!({"op": "add_layer", "page": page, "index": 1, "layer": {"kind": "group", "name": "b"}})),
            op(json!({"op": "add_layer", "page": page, "index": 2, "layer": {"kind": "group", "name": "c"}})),
        ]))
        .unwrap();
    let (a, b, c) = (
        created_layer(&change, 0),
        created_layer(&change, 1),
        created_layer(&change, 2),
    );
    let names = |session: &Session| -> Vec<String> {
        session
            .page(page)
            .unwrap()
            .layers
            .iter()
            .map(|view| name(&view.layer).to_owned())
            .collect()
    };

    // An index counts after the layer is taken out.
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::MoveLayer {
                layer: a.into(),
                parent: None,
                index: 2,
            }],
        ))
        .unwrap();
    assert_eq!(names(&session), ["b", "c", "a"]);
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::MoveLayer {
                layer: c.into(),
                parent: None,
                index: 0,
            }],
        ))
        .unwrap();
    assert_eq!(names(&session), ["c", "b", "a"]);
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::MoveLayer {
                layer: c.into(),
                parent: Some(b.into()),
                index: 0,
            }],
        ))
        .unwrap();
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![Op::MoveLayer {
                layer: b.into(),
                parent: Some(c.into()),
                index: 0,
            }],
        ))
        .unwrap_err();
    assert!(
        matches!(error, Error::Invalid { .. }),
        "a layer can't move inside itself: {error}"
    );

    // A locked layer refuses edits inside it, but can be unlocked.
    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": b, "patch": {"locked": true}}),
            )],
        ))
        .unwrap();
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": c, "patch": {"name": "x"}}),
            )],
        ))
        .unwrap_err();
    assert!(
        matches!(error, Error::Locked { layer, .. } if layer == b),
        "{error}"
    );
    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": b, "patch": {"locked": false}}),
            )],
        ))
        .unwrap();

    // Unknown fields and values fail; ranges are not checked.
    for patch in [
        json!({"visable": false}),
        json!({"blend": "pass_through_x"}),
        json!({"opacity": "half"}),
    ] {
        let error = session
            .apply(edit(
                &session,
                user(),
                vec![op(
                    json!({"op": "update_layer", "layer": a, "patch": patch}),
                )],
            ))
            .unwrap_err();
        assert!(matches!(error, Error::Invalid { .. }), "{error}");
    }
    assert!(
        serde_json::from_value::<Op>(
            json!({"op": "move_layer", "layer": a, "index": 0, "extra": 1})
        )
        .is_err()
    );
    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": a, "patch": {"opacity": 2.0}}),
            )],
        ))
        .unwrap();

    // A failed edit writes nothing, and names the op that failed.
    let before = session.page(page).unwrap();
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![
                op(json!({"op": "update_layer", "layer": a, "patch": {"name": "half"}})),
                op(json!({"op": "update_layer", "layer": a, "patch": {"visable": false}})),
            ],
        ))
        .unwrap_err();
    assert!(
        matches!(error, Error::Invalid { op: Some(1), .. }),
        "{error}"
    );
    assert_eq!(session.page(page).unwrap(), before);

    // A mask needs a parent; labels must exist and be unique in an edit.
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "mask"}}),
            )],
        ))
        .unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![Op::RemoveLayer {
                layer: label("nope"),
            }],
        ))
        .unwrap_err();
    assert!(matches!(error, Error::NotFound { .. }), "{error}");
    let error = session
        .apply(edit(&session, user(), vec![
            op(json!({"op": "add_layer", "id": "$x", "page": page, "index": 0, "layer": {"kind": "group"}})),
            op(json!({"op": "add_layer", "id": "$x", "page": page, "index": 0, "layer": {"kind": "group"}})),
        ]))
        .unwrap_err();
    assert!(
        matches!(error, Error::Invalid { op: Some(1), .. }),
        "{error}"
    );
    let error = session
        .apply(Edit {
            project: ProjectId::new(),
            ..edit(&session, user(), vec![])
        })
        .unwrap_err();
    assert!(matches!(error, Error::WrongProject { .. }));
}

#[test]
fn only_this_major_version_up_to_this_one_opens() {
    let (_dir, path) = path();
    let session = Session::create(&path).unwrap();
    assert_eq!(session.project().unwrap().version, "1.0.0");
    drop(session);
    for (version, opens) in [
        ("1.0.0", true),
        ("1.1.0", false),
        ("1.0.1", false),
        ("2.0.0", false),
        ("0.9.0", false),
        ("1.0.0-alpha", false),
        ("1.0.0+build", false),
        ("1.0", false),
        ("01.0.0", false),
    ] {
        let (file, doc) = koharu_storage::Document::open(&path).unwrap();
        doc.get_map("project").insert("version", version).unwrap();
        doc.commit();
        file.save(&doc.export(loro::ExportMode::Snapshot).unwrap())
            .unwrap();
        drop(file);
        match Session::open(&path) {
            Ok(_) => assert!(opens, "{version} opened"),
            Err(error) => {
                assert!(!opens, "{version}: {error}");
                let malformed = matches!(version, "1.0" | "01.0.0");
                assert_eq!(
                    matches!(error, Error::Version(_)),
                    !malformed,
                    "{version}: {error:?}"
                );
            }
        }
    }
}
