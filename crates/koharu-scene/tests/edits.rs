//! The structure of a project through edits, undo, redo, and reopening, and
//! the edits that must fail without writing.

mod common;
use common::*;
use koharu_scene::{Content, Direction, Error, LayerId, LayerKind, Op, PageId, Ref, Session, Text};
use serde_json::{Value, json};

fn names(session: &Session, page: PageId) -> Vec<String> {
    let project = session.project();
    project
        .page(page)
        .expect("the page")
        .layers
        .iter()
        .map(|layer| layer.name.clone())
        .collect()
}

fn update(session: &mut Session, layer: LayerId, patch: Value) -> koharu_scene::Result<()> {
    apply(
        session,
        vec![json!({"op": "update_layer", "layer": layer, "patch": patch})],
    )
    .map(|_| ())
}

fn mv(
    session: &mut Session,
    layer: LayerId,
    parent: Option<LayerId>,
    index: usize,
) -> koharu_scene::Result<()> {
    session
        .apply(edit(vec![Op::MoveLayer {
            layer: layer.into(),
            parent: parent.map(Ref::from),
            index,
        }]))
        .map(|_| ())
}

#[test]
fn structure_survives_reopening_and_undo() {
    let (_dir, path) = path();
    let mut session = Session::create(&path).expect("a project");
    let page = page(&mut session);
    let change = apply(&mut session, vec![
        json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "raster", "name": "paint"}}),
        json!({"op": "add_layer", "id": "$g", "page": page, "index": 1, "layer": {"kind": "group", "name": "block"}}),
        json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "text", "content": "Hello", "language": "en"}}),
        json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "mask", "mode": "subtract"}}),
        json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "shape", "path": "M0 0 L5 5", "fill": [1.0, 0.0, 0.0, 1.0]}}),
    ])
    .expect("an edit");
    assert_eq!(change.created.len(), 5);
    let project = session.project();
    let top = &project.page(page).expect("the page").layers;
    assert_eq!(top.len(), 2);
    let group = &top[1];
    let kinds: Vec<LayerKind> = group.children.iter().map(|child| child.kind()).collect();
    assert_eq!(kinds, [LayerKind::Shape, LayerKind::Mask, LayerKind::Text]);
    let text = group.children[2].id;

    drop(session);
    let mut session = Session::open(&path).expect("it opens");
    assert_eq!(session.project(), project, "the reopened project matches");
    assert!(
        session.undo().expect("undo").is_none(),
        "nothing to undo after reopening"
    );

    // Undo brings a removed layer back with its id; redo removes it again.
    let change = session
        .apply(edit(vec![Op::RemoveLayer { layer: text.into() }]))
        .expect("a removal");
    assert!(change.layers.contains(&text));
    assert!(session.project().layer(text).is_err());
    let undone = session.undo().expect("undo").expect("a step");
    assert!(undone.layers.contains(&text), "{undone:?}");
    assert!(undone.pages.changed.contains(&page));
    let Content::Text(restored) = &layer(&session, text).content else {
        panic!("not a text")
    };
    assert_eq!(restored.content.text(), "Hello");
    let redone = session.redo().expect("redo").expect("a step");
    assert!(redone.layers.contains(&text));
    assert!(session.project().layer(text).is_err());
}

#[test]
fn undo_reports_page_and_layer_changes() {
    let (_dir, mut session, page) = session();
    let text = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": "a"}),
    );
    let second = apply(
        &mut session,
        vec![json!({"op": "add_page", "index": 1, "size": {"width": 10, "height": 10}})],
    )
    .expect("a page")
    .pages
    .added[0];
    session
        .apply(edit(vec![Op::MovePage {
            page: second.into(),
            index: 0,
        }]))
        .expect("a move");
    assert!(
        session
            .undo()
            .expect("undo")
            .expect("a step")
            .pages
            .reordered
    );
    assert_eq!(
        session.undo().expect("undo").expect("a step").pages.removed,
        vec![second]
    );
    assert_eq!(
        session.redo().expect("redo").expect("a step").pages.added,
        vec![second]
    );

    update(&mut session, text, json!({"name": "renamed"})).expect("a rename");
    let undone = session.undo().expect("undo").expect("a step");
    assert!(undone.layers.contains(&text), "{undone:?}");
    assert!(undone.pages.changed.contains(&page));

    // An edit that changes a layer and then removes its page undoes whole.
    session
        .apply(edit(vec![
            op(json!({"op": "update_layer", "layer": text, "patch": {"name": "x"}})),
            Op::RemovePage { page: page.into() },
        ]))
        .expect("an edit");
    session.undo().expect("undo").expect("a step");
    assert_eq!(layer(&session, text).name, "");
}

#[test]
fn moves_locks_and_invalid_edits() {
    let (_dir, mut session, page) = session();
    let a = add(
        &mut session,
        page,
        None,
        json!({"kind": "group", "name": "a"}),
    );
    let b = add(
        &mut session,
        page,
        None,
        json!({"kind": "group", "name": "b"}),
    );
    let c = add(
        &mut session,
        page,
        None,
        json!({"kind": "group", "name": "c"}),
    );

    // An index counts after the layer is taken out.
    mv(&mut session, a, None, 2).expect("a move");
    assert_eq!(names(&session, page), ["b", "c", "a"]);
    mv(&mut session, c, None, 0).expect("a move");
    assert_eq!(names(&session, page), ["c", "b", "a"]);
    mv(&mut session, c, Some(b), 0).expect("a move");
    let error = mv(&mut session, b, Some(c), 0).expect_err("a layer can't move inside itself");
    assert!(matches!(error, Error::Invalid { .. }), "{error}");

    // A locked layer refuses edits inside it, but can be unlocked.
    update(&mut session, b, json!({"locked": true})).expect("a lock");
    let error = update(&mut session, c, json!({"name": "x"})).expect_err("locked");
    assert!(
        matches!(error, Error::Locked { layer, .. } if layer == b),
        "{error}"
    );
    let error = update(&mut session, c, json!({"locked": true})).expect_err("locked above");
    assert!(
        matches!(error, Error::Locked { layer, .. } if layer == b),
        "{error}"
    );
    update(&mut session, b, json!({"locked": false})).expect("an unlock");

    // Unknown fields, unknown values, and values out of range fail.
    for patch in [
        json!({"visable": false}),
        json!({"blend": "pass_through_x"}),
        json!({"opacity": "half"}),
        json!({"opacity": 2.0}),
    ] {
        let error = update(&mut session, a, patch).expect_err("invalid");
        assert!(matches!(error, Error::Invalid { .. }), "{error}");
    }
    assert!(
        serde_json::from_value::<Op>(
            json!({"op": "move_layer", "layer": a, "index": 0, "extra": 1})
        )
        .is_err()
    );

    // A failed edit writes nothing, and names the op that failed.
    let before = session.project();
    let error = session
        .apply(edit(vec![
            op(json!({"op": "update_layer", "layer": a, "patch": {"name": "half"}})),
            op(json!({"op": "update_layer", "layer": a, "patch": {"visable": false}})),
        ]))
        .expect_err("invalid");
    assert!(matches!(error, Error::Invalid { op: 1, .. }), "{error}");
    assert_eq!(session.project(), before);

    // A mask needs a parent; labels must exist and be unique in an edit.
    let error = apply(
        &mut session,
        vec![json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "mask"}})],
    )
    .expect_err("a mask on the page");
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
    let error = session
        .apply(edit(vec![Op::RemoveLayer {
            layer: serde_json::from_value(json!("$nope")).expect("a label"),
        }]))
        .expect_err("no such label");
    assert!(matches!(error, Error::NotFound(_)), "{error}");
    let error = apply(&mut session, vec![
        json!({"op": "add_layer", "id": "$x", "page": page, "index": 0, "layer": {"kind": "group"}}),
        json!({"op": "add_layer", "id": "$x", "page": page, "index": 0, "layer": {"kind": "group"}}),
    ])
    .expect_err("a label used twice");
    assert!(matches!(error, Error::Invalid { op: 1, .. }), "{error}");
    let error = apply(&mut session, vec![json!({"op": "add_layer", "id": a, "page": page, "index": 0, "layer": {"kind": "group"}})])
        .expect_err("an id in use");
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
    assert_eq!(session.project(), before);
}

#[test]
fn a_second_open_is_locked_at_once() {
    let (_dir, path) = path();
    let session = Session::create(&path).expect("a project");
    let start = std::time::Instant::now();
    let error = Session::open(&path).err().expect("locked");
    assert!(matches!(error, Error::ProjectLocked(_)), "{error:?}");
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    assert!(matches!(
        Session::create(&path).err().expect("it exists"),
        Error::Io(_)
    ));
    drop(session);
    Session::open(&path).expect("it opens");
}

#[test]
fn undo_and_redo_survive_cascaded_removal() {
    let (_dir, mut session, page) = session();
    apply(&mut session, vec![
        json!({"op": "add_layer", "id": "$g", "page": page, "index": 0, "layer": {"kind": "group"}}),
        json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "raster"}}),
        json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "mask"}}),
        json!({"op": "add_layer", "page": page, "index": 1, "layer": {"kind": "group", "name": "above"}}),
    ])
    .expect("an edit");
    let group = session.project().page(page).expect("the page").layers[0].id;
    let before = session.project();
    for remove in [
        Op::RemoveLayer {
            layer: group.into(),
        },
        Op::RemovePage { page: page.into() },
    ] {
        session.apply(edit(vec![remove])).expect("a removal");
        let gone = session.project();
        for _ in 0..3 {
            session.undo().expect("undo").expect("a step");
            assert_eq!(session.project(), before);
            session.redo().expect("redo").expect("a step");
            assert_eq!(session.project(), gone);
        }
        session.undo().expect("undo").expect("a step");
    }
}

#[test]
fn a_failed_edit_keeps_undo_and_redo() {
    let (_dir, mut session, page) = session();
    let text = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": "a"}),
    );
    update(&mut session, text, json!({"name": "one"})).expect("a rename");
    update(&mut session, text, json!({"name": "two"})).expect("a rename");
    session.undo().expect("undo").expect("a step");
    let (revision, history) = (session.revision(), session.history());
    assert_eq!(history.redo(), Some("test"));

    // A delete that rolls back leaves both stacks working.
    let error = session
        .apply(edit(vec![
            Op::RemoveLayer { layer: text.into() },
            Op::RemoveLayer { layer: text.into() },
        ]))
        .expect_err("removed twice");
    assert!(matches!(error, Error::NotFound(_)), "{error}");
    assert_eq!(layer(&session, text).name, "one");
    assert_eq!((session.revision(), session.history()), (revision, history));
    session.redo().expect("redo").expect("a step");
    assert_eq!(layer(&session, text).name, "two");
    session.undo().expect("undo").expect("a step");
    session.undo().expect("undo").expect("a step");
    assert_eq!(layer(&session, text).name, "");
}

#[test]
fn going_to_a_step_moves_through_every_step_between() {
    let (_dir, mut session, page) = session();
    let group = add(
        &mut session,
        page,
        None,
        json!({"kind": "group", "name": "zero"}),
    );
    let start = session.history();
    for name in ["one", "two", "three"] {
        update(&mut session, group, json!({"name": name})).expect("a rename");
    }
    let history = session.history();
    assert_eq!(history.steps.len(), start.steps.len() + 3);
    assert_eq!(history.current, history.steps.len());

    // Back three steps at once, then forward two; the steps stay, to redo.
    let change = session
        .go_to(start.current)
        .expect("a jump")
        .expect("a change");
    assert_eq!(layer(&session, group).name, "zero");
    assert!(change.layers.contains(&group));
    assert_eq!(change.history.current, start.current);
    assert_eq!(change.history.steps, history.steps);
    session.go_to(start.current + 2).expect("a jump");
    assert_eq!(layer(&session, group).name, "two");
    assert_eq!(session.history().redo(), Some("test"));
    assert!(session.go_to(start.current + 2).expect("no jump").is_none());
    assert!(session.go_to(history.steps.len() + 1).is_err());

    // A new edit drops the steps after the current one.
    update(&mut session, group, json!({"name": "four"})).expect("a rename");
    assert_eq!(session.history().steps.len(), start.current + 3);
    assert_eq!(session.history().redo(), None);
}

#[test]
fn history_keeps_the_most_recent_steps() {
    let (_dir, mut session, page) = session();
    let group = add(&mut session, page, None, json!({"kind": "group"}));
    for index in 0..30 {
        update(&mut session, group, json!({"name": index.to_string()})).expect("a rename");
    }
    let history = session.history();
    assert_eq!((history.steps.len(), history.current), (20, 20));
    session.go_to(0).expect("a jump");
    assert_eq!(
        layer(&session, group).name,
        "9",
        "the oldest kept step renamed it to 10"
    );
}

#[test]
fn an_edit_that_changes_nothing_adds_no_step() {
    let (_dir, mut session, page) = session();
    let a = add(
        &mut session,
        page,
        None,
        json!({"kind": "group", "name": "a"}),
    );
    add(
        &mut session,
        page,
        None,
        json!({"kind": "group", "name": "b"}),
    );
    let (revision, history) = (session.revision(), session.history());
    for ops in [
        vec![op(
            json!({"op": "update_layer", "layer": a, "patch": {"name": "a"}}),
        )],
        vec![Op::MoveLayer {
            layer: a.into(),
            parent: None,
            index: 0,
        }],
        vec![op(json!({"op": "update_layer", "layer": a, "patch": {}}))],
    ] {
        let change = session.apply(edit(ops)).expect("an edit");
        assert_eq!(change.revision, revision);
        assert!(change.layers.is_empty() && change.pages.changed.is_empty());
        assert_eq!(session.history(), history);
    }
}

#[test]
fn committed_edits_survive_a_process_that_never_closed() {
    let (dir, mut session, page) = session();
    let text = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": "kept"}),
    );

    // While it's open, the main file alone may not hold the edit: its `-wal`
    // does, and a copy of both is what a crash leaves behind.
    let copy = dir.path().join("copy.khr");
    let original = dir.path().join("test.khr");
    fs_err::copy(&original, &copy).expect("a copy");
    fs_err::copy(
        original.with_extension("khr-wal"),
        copy.with_extension("khr-wal"),
    )
    .expect("a copy");
    let copied = Session::open(&copy).expect("it opens");
    let Content::Text(kept) = &layer(&copied, text).content else {
        panic!("not a text")
    };
    assert_eq!(kept.content.text(), "kept");
    drop(session);
}

#[test]
fn undo_keeps_what_moved_in_or_out_of_a_group_it_removes() {
    let (_dir, mut session, page) = session();
    let ops = (0..30)
        .map(|index| op(json!({"op": "add_layer", "page": page, "index": index, "layer": {"kind": "group", "name": index.to_string()}})))
        .collect();
    session.apply(edit(ops)).expect("an edit");
    let layers = session
        .project()
        .page(page)
        .expect("the page")
        .layers
        .clone();
    let before = session.project();

    // Undoing this removes the new group while the moved layers still sit in
    // it, so taking its contents with it would lose them.
    let mut ops = vec![op(
        json!({"op": "add_layer", "id": "$g", "page": page, "index": 0, "layer": {"kind": "group"}}),
    )];
    ops.extend(layers.iter().map(|layer| Op::MoveLayer {
        layer: layer.id.into(),
        parent: Some(serde_json::from_value(json!("$g")).expect("a label")),
        index: 0,
    }));
    session.apply(edit(ops)).expect("an edit");
    session.undo().expect("undo").expect("a step");
    assert_eq!(session.project(), before);
    session.redo().expect("redo").expect("a step");
    session.undo().expect("undo").expect("a step");
    assert_eq!(session.project(), before);
}

#[test]
fn which_kind_holds_which() {
    let (_dir, mut session, page) = session();
    let panel = add(
        &mut session,
        page,
        None,
        json!({"kind": "panel", "outline": outline()}),
    );
    let balloon = add(
        &mut session,
        page,
        Some(panel),
        json!({"kind": "balloon", "outline": outline()}),
    );
    let block = add(&mut session, page, Some(balloon), block(0.0));
    add(
        &mut session,
        page,
        Some(balloon),
        json!({"kind": "text", "content": "lettered"}),
    );
    let free = add(&mut session, page, Some(panel), common::block(20.0));
    add(
        &mut session,
        page,
        None,
        json!({"kind": "balloon", "outline": outline()}),
    );
    add(&mut session, page, None, common::block(40.0));
    let group = add(&mut session, page, None, json!({"kind": "group"}));
    let raster = add(&mut session, page, Some(group), json!({"kind": "raster"}));

    // A block leaves its balloon for the page and comes back.
    mv(&mut session, block, None, 0).expect("free on the page");
    mv(&mut session, block, Some(balloon), 0).expect("back in its balloon");
    mv(&mut session, free, Some(group), 0).expect("a block in a group");

    let before = session.project();
    let refused: Vec<Vec<Value>> = vec![
        vec![json!({"op": "move_layer", "layer": balloon, "parent": balloon, "index": 0})],
        vec![
            json!({"op": "add_layer", "page": page, "parent": balloon, "index": 0, "layer": {"kind": "balloon", "outline": outline()}}),
        ],
        vec![
            json!({"op": "add_layer", "page": page, "parent": block, "index": 0, "layer": common::block(0.0)}),
        ],
        vec![json!({"op": "move_layer", "layer": raster, "parent": balloon, "index": 0})],
        vec![
            json!({"op": "add_layer", "page": page, "parent": panel, "index": 0, "layer": {"kind": "mask"}}),
        ],
        vec![
            json!({"op": "add_layer", "page": page, "parent": balloon, "index": 0, "layer": {"kind": "mask"}}),
        ],
        vec![
            json!({"op": "add_layer", "page": page, "parent": block, "index": 0, "layer": {"kind": "mask"}}),
        ],
        vec![
            json!({"op": "add_layer", "page": page, "parent": raster, "index": 0, "layer": {"kind": "group"}}),
        ],
    ];
    for ops in refused {
        let error = apply(&mut session, ops.clone()).expect_err("refused");
        assert!(matches!(error, Error::Invalid { .. }), "{ops:?}: {error}");
        assert_eq!(session.project(), before, "{ops:?}");
    }

    // Masks go on groups and drawn layers.
    for parent in [group, raster] {
        apply(&mut session, vec![json!({"op": "add_layer", "page": page, "parent": parent, "index": 0, "layer": {"kind": "mask"}})])
            .expect("a mask");
    }
}

#[test]
fn update_layer_merges_and_fails_closed() {
    let (_dir, mut session, page) = session();
    let raster = add(&mut session, page, None, json!({"kind": "raster"}));
    let text = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": "x"}),
    );
    let shape = add(
        &mut session,
        page,
        None,
        json!({"kind": "shape", "path": "M0 0 L1 1", "fill": [0.0, 0.0, 0.0, 1.0]}),
    );
    let block = add(&mut session, page, None, block(0.0));

    update(&mut session, raster, json!({"opacity": 0.5})).expect("an opacity");
    assert_eq!(f32::from(layer(&session, raster).opacity), 0.5);

    let before = session.project();
    for (layer, patch) in [
        (raster, json!({"opacty": 0.5})),
        (raster, json!({"direction": "vertical"})),
        (raster, json!({"kind": "text"})),
        (raster, json!({"id": raster})),
        (raster, json!({"attributes": {}})),
        (raster, json!({"children": []})),
        (raster, json!([])),
        (text, json!({"content": "y"})),
        (block, json!({"translation": {"content": "y"}})),
    ] {
        let error = update(&mut session, layer, patch.clone()).expect_err("refused");
        assert!(matches!(error, Error::Invalid { .. }), "{patch}: {error}");
        assert_eq!(session.project(), before, "{patch}");
    }

    // A patch on a block reaches its parts.
    update(
        &mut session,
        block,
        json!({"cleanup": {"opacity": 0.4}, "mask": {"visible": false}}),
    )
    .expect("parts");
    let Content::Block(patched) = &layer(&session, block).content else {
        panic!("not a block")
    };
    assert_eq!(f32::from(patched.cleanup.opacity), 0.4);
    assert!(!patched.mask.visible);

    // Merging into a missing translation starts from an empty text.
    update(
        &mut session,
        block,
        json!({"translation": {"direction": "vertical"}}),
    )
    .expect("a translation");
    let Content::Block(patched) = &layer(&session, block).content else {
        panic!("not a block")
    };
    assert_eq!(
        patched.translation,
        Some(Text {
            direction: Direction::Vertical,
            ..Text::default()
        })
    );
    update(&mut session, block, json!({"translation": null})).expect("no translation");
    let Content::Block(patched) = &layer(&session, block).content else {
        panic!("not a block")
    };
    assert_eq!(patched.translation, None);
    update(&mut session, block, json!({"source": {"content": "原文"}})).expect("a source");

    update(&mut session, shape, json!({"fill": null})).expect("no fill");
    let Content::Shape(cleared) = &layer(&session, shape).content else {
        panic!("not a shape")
    };
    assert_eq!(cleared.fill, None);
}

#[test]
fn reopening_matches_memory() {
    let (_dir, path) = path();
    let mut session = Session::create(&path).expect("a project");
    let page = page(&mut session);
    let second = common::page(&mut session);
    let panel = add(
        &mut session,
        page,
        None,
        json!({"kind": "panel", "outline": outline()}),
    );
    let balloon = add(
        &mut session,
        page,
        Some(panel),
        json!({"kind": "balloon", "outline": outline()}),
    );
    let block = add(&mut session, page, Some(balloon), block(0.5));
    add(
        &mut session,
        second,
        None,
        json!({"kind": "text", "content": [{"insert": "bold", "attributes": {"weight": 700}}], "frame": {"box": {"bounds": {"x": 0.1, "y": 0.2, "width": 30.3, "height": 40.7}}}}),
    );
    update(&mut session, block, json!({"translation": {"frame": {"box": {"bounds": {"x": 0.1, "y": 1.0, "width": 2.0, "height": 3.0}}}}})).expect("a frame");
    session
        .apply(edit(vec![Op::DuplicateLayer {
            layer: panel.into(),
            id: None,
            index: None,
        }]))
        .expect("a duplicate");
    mv(&mut session, block, None, 0).expect("a move");
    session
        .apply(edit(vec![Op::RemoveLayer {
            layer: balloon.into(),
        }]))
        .expect("a removal");
    session.undo().expect("undo").expect("a step");
    session.undo().expect("undo").expect("a step");
    session.redo().expect("redo").expect("a step");
    session
        .apply(edit(vec![Op::MovePage {
            page: second.into(),
            index: 0,
        }]))
        .expect("a page move");

    let before = session.project();
    drop(session);
    assert_eq!(Session::open(&path).expect("it opens").project(), before);
}

/// The page bounds of a path a layer's value holds at `key`.
fn path_origin(value: &Value, key: &str) -> (f64, f64) {
    use kurbo::Shape as _;
    let path = value[key].as_str().expect("a path");
    let bounds = kurbo::BezPath::from_svg(path)
        .expect("a path")
        .bounding_box();
    (bounds.x0, bounds.y0)
}

fn transform_layer(
    session: &mut Session,
    layer: LayerId,
    transform: Value,
) -> koharu_scene::Result<()> {
    apply(
        session,
        vec![json!({"op": "transform_layer", "layer": layer, "transform": transform})],
    )
    .map(|_| ())
}

#[test]
fn a_transform_moves_each_kind_by_its_own_values() {
    let (_dir, mut session, page) = session();
    let square = "M5 30 h4 v4 h-4 Z";
    let text = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": "Hi",
        "transform": [[1.0, 0.0, 40.0], [0.0, 1.0, 5.0]]}),
    );
    let translated = add(
        &mut session,
        page,
        None,
        json!({"kind": "block",
        "source": {"content": "やあ"},
        "translation": {"content": "Hi", "transform": [[1.0, 0.0, 40.0], [0.0, 1.0, 15.0]]}}),
    );
    let untranslated = add(
        &mut session,
        page,
        None,
        json!({"kind": "block",
        "source": {"content": "やあ", "transform": [[1.0, 0.0, 10.0], [0.0, 1.0, 10.0]]}}),
    );
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("a PNG");
    let hash = koharu_scene::Hash::of(&png).to_string();
    apply(&mut session, vec![json!({"op": "add_file", "bytes": png})]).expect("a file");
    let picture = add(
        &mut session,
        page,
        None,
        json!({"kind": "image", "file": hash,
        "transform": [[1.0, 0.0, 60.0], [0.0, 1.0, 5.0]]}),
    );
    let shape = add(
        &mut session,
        page,
        None,
        json!({"kind": "shape", "path": square}),
    );
    let panel = add(
        &mut session,
        page,
        None,
        json!({"kind": "panel", "outline": square}),
    );
    let balloon = add(
        &mut session,
        page,
        Some(panel),
        json!({"kind": "balloon", "outline": square}),
    );
    let raster = add(
        &mut session,
        page,
        None,
        json!({"kind": "raster", "offset": {"x": 3, "y": 4}}),
    );
    let (_, cleanup) = parts(&session, translated);
    let by = json!([[1.0, 0.0, 25.0], [0.0, 1.0, -10.0]]);
    let ops = [
        text,
        translated,
        untranslated,
        picture,
        shape,
        panel,
        balloon,
        raster,
        cleanup,
    ]
    .into_iter()
    .map(|layer| json!({"op": "transform_layer", "layer": layer, "transform": by}))
    .collect();
    apply(&mut session, ops).expect("the layers move");

    let value = |id| serde_json::to_value(layer(&session, id)).expect("JSON");
    assert_eq!(
        value(text)["transform"],
        json!([[1.0, 0.0, 65.0], [0.0, 1.0, -5.0]])
    );
    assert_eq!(
        value(translated)["translation"]["transform"],
        json!([[1.0, 0.0, 65.0], [0.0, 1.0, 5.0]])
    );
    assert_eq!(
        value(untranslated)["source"]["transform"],
        json!([[1.0, 0.0, 35.0], [0.0, 1.0, 0.0]])
    );
    assert_eq!(
        value(picture)["transform"],
        json!([[1.0, 0.0, 85.0], [0.0, 1.0, -5.0]])
    );
    assert_eq!(path_origin(&value(shape), "path"), (30.0, 20.0));
    assert_eq!(path_origin(&value(panel), "outline"), (30.0, 20.0));
    assert_eq!(path_origin(&value(balloon), "outline"), (30.0, 20.0));
    assert_eq!(value(raster)["offset"], json!({"x": 28, "y": -6}));
    assert_eq!(
        value(translated)["cleanup"]["offset"],
        json!({"x": 25, "y": -10})
    );
}

#[test]
fn a_transform_a_layer_cannot_take_changes_nothing() {
    let (_dir, mut session, page) = session();
    let raster = add(&mut session, page, None, json!({"kind": "raster"}));
    let group = add(&mut session, page, None, json!({"kind": "group"}));
    add(
        &mut session,
        page,
        Some(group),
        json!({"kind": "shape", "path": "M0 0 h4 v4 h-4 Z"}),
    );
    let before = session.revision();
    let (sin, cos) = 0.1f64.sin_cos();
    for (layer, transform) in [
        (raster, json!([[1.0, 0.0, 0.5], [0.0, 1.0, 0.0]])),
        (raster, json!([[cos, -sin, 0.0], [sin, cos, 0.0]])),
        (group, json!([[1.0, 0.0, 1.0], [0.0, 1.0, 0.0]])),
    ] {
        assert!(transform_layer(&mut session, layer, transform).is_err());
        assert_eq!(session.revision(), before);
    }
}

/// The desktop shows every writer's change, the pipeline's and the agent's
/// included, by following the session alone.
#[test]
fn every_change_reaches_a_subscriber() {
    let (_dir, mut session, page) = session();
    let mut changes = session.subscribe();

    let layer = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": "Hi"}),
    );
    let change = changes.try_recv().expect("the edit's change");
    assert!(change.layers.contains(&layer), "{change:?}");

    // An edit that changes nothing tells nothing.
    session.apply(edit(Vec::new())).expect("an empty edit");
    assert!(changes.try_recv().is_err());

    session.undo().expect("undo").expect("a step");
    let change = changes.try_recv().expect("the undo's change");
    assert!(change.layers.contains(&layer), "{change:?}");
    session.redo().expect("redo").expect("a step");
    assert!(changes.try_recv().is_ok());
}
