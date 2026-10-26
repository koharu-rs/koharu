//! What is stored and read back: hashes stable across reopening, rich text
//! and deltas, files freed when nothing names them, tiles, and values that
//! look like Loro's own.

mod common;
use common::*;

#[test]
fn hashes_text_and_duplicates_survive_reopening() {
    let (_dir, path) = path();
    let mut session = Session::create(&path).unwrap();
    let page = page(&mut session);
    let change = session
        .apply(edit(&session, user(), vec![
            op(json!({"op": "add_layer", "id": "$g", "page": page, "index": 0, "layer": {"kind": "group", "attributes": {"koharu": {"role": "balloon", "score": 0.5}}}})),
            op(json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "text",
                "content": [{"insert": "Hello", "attributes": {"weight": 700, "fill": [0.0, 0.0, 1.0, 1.0]}}, {"insert": " world"}]}})),
            op(json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "mask", "mode": "subtract"}})),
        ]))
        .unwrap();
    let (group, layer) = (created_layer(&change, 0), created_layer(&change, 1));
    let (_, whole) = session.read(group, Pointer::root()).unwrap();
    let (_, role) = session
        .read(group, &pointer("/attributes/koharu/role"))
        .unwrap();
    let before = session.page(page).unwrap();
    session.save().unwrap();
    drop(session);

    // Loro maps iterate in hash order, so hashes must not depend on it.
    let mut session = Session::open(&path).unwrap();
    assert_eq!(
        session.read(group, Pointer::root()).unwrap().1.hash,
        whole.hash
    );
    assert_eq!(
        session
            .read(group, &pointer("/attributes/koharu/role"))
            .unwrap()
            .1
            .hash,
        role.hash
    );
    assert_eq!(session.page(page).unwrap(), before);
    let shown = text(&session, layer);
    assert_eq!(marked(&shown.content).len(), 1);
    assert_eq!(marked(&shown.content)[0].attributes.weight, Some(700));
    assert_eq!(
        serde_json::to_value(&shown.content).unwrap()[1],
        json!({"insert": " world"})
    );

    // A delta needs a read of `/content`, keeps marks on unchanged text, and
    // can't run past the end.
    let replace = delta(json!([{"retain": 6}, {"delete": 5}, {"insert": "there"}]));
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![Op::EditText {
                layer: layer.into(),
                delta: replace.clone(),
            }],
        ))
        .unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }));
    let (_, read) = session.read(layer, &pointer("/content")).unwrap();
    session
        .apply(Edit {
            reads: vec![read],
            ..edit(
                &session,
                user(),
                vec![Op::EditText {
                    layer: layer.into(),
                    delta: replace,
                }],
            )
        })
        .unwrap();
    let shown = text(&session, layer);
    assert_eq!(shown.content.text(), "Hello there");
    assert_eq!(marked(&shown.content)[0].insert, "Hello");
    let (_, read) = session.read(layer, &pointer("/content")).unwrap();
    let past = delta(json!([{"retain": 99}]));
    assert!(
        session
            .apply(Edit {
                reads: vec![read],
                ..edit(
                    &session,
                    user(),
                    vec![Op::EditText {
                        layer: layer.into(),
                        delta: past
                    }]
                )
            })
            .is_err()
    );

    // A duplicate copies children and masks with new ids, and marks.
    let change = session
        .apply(edit(
            &session,
            user(),
            vec![Op::DuplicateLayer {
                layer: group.into(),
                id: Some(label("copy")),
                index: None,
            }],
        ))
        .unwrap();
    assert_eq!(change.created.len(), 3);
    let view = session.page(page).unwrap();
    let copy = &view.layers[1];
    assert_eq!((copy.children.len(), copy.masks.len()), (1, 1));
    assert_ne!(copy.children[0].id, layer);
    let Layer::Text(copied) = &copy.children[0].layer else {
        panic!()
    };
    assert_eq!(copied.content.text(), "Hello there");
    assert_eq!(marked(&copied.content).len(), 1);
}

#[test]
fn marks_and_text_reads() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, user(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "abc"}}))]))
        .unwrap();
    let layer = created_layer(&change, 0);
    let apply = |session: &mut Session, value: Value| {
        let (_, read) = session.read(layer, &pointer("/content")).unwrap();
        session.apply(Edit {
            reads: vec![read],
            ..edit(
                session,
                user(),
                vec![Op::EditText {
                    layer: layer.into(),
                    delta: delta(value),
                }],
            )
        })
    };

    // Marks are not range-checked, but unknown ones fail, even to remove.
    apply(
        &mut session,
        json!([{"retain": 1, "attributes": {"size": -1.0}}]),
    )
    .unwrap();
    for attributes in [json!({"bold": true}), json!({"bold": null})] {
        let error = apply(
            &mut session,
            json!([{"retain": 1, "attributes": attributes}]),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Invalid { .. }), "{error:?}");
    }
    assert!(
        serde_json::from_value::<Op>(
            json!({"op": "add_layer", "page": page, "index": 0, "layer": {
                "kind": "text", "content": [{"insert": "ab", "attributes": {"bold": true}}],
            }})
        )
        .is_err()
    );
    apply(
        &mut session,
        json!([{"retain": 2, "attributes": {"size": 12.0}}]),
    )
    .unwrap();
    let shown = text(&session, layer);
    assert_eq!(marked(&shown.content).len(), 1);
    assert_eq!(
        (
            marked(&shown.content)[0].insert.as_str(),
            marked(&shown.content)[0].attributes.size
        ),
        ("ab", Some(12.0))
    );

    // A text's `/content` is read whole; a mask's is an ordinary property.
    assert!(session.read(layer, &pointer("/content/x")).is_err());
    let change = session
        .apply(edit(&session, user(), vec![
            op(json!({"op": "add_layer", "id": "$g", "page": page, "index": 0, "layer": {"kind": "group"}})),
            op(json!({"op": "add_layer", "page": page, "parent": "$g", "index": 0, "layer": {"kind": "mask", "content": {"path": {"path": "M0 0 L1 1 Z"}}}})),
        ]))
        .unwrap();
    let mask = created_layer(&change, 1);
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": mask, "patch": {"content": {"pixels": {"offset": {"x": 1, "y": 2}}, "path": null}}}))]))
        .unwrap();
    assert_eq!(
        session
            .read(mask, &pointer("/content/pixels/offset/y"))
            .unwrap()
            .0,
        json!(2)
    );
    assert!(session.tiles(mask, None).unwrap().tiles.is_empty());
}

#[test]
fn files_go_when_nothing_names_them() {
    let (_dir, mut session, page) = session();
    let (a, b) = (b"a".to_vec(), b"b".to_vec());
    let (ha, hb) = (Hash::of(&a), Hash::of(&b));
    let change = session
        .apply(edit(&session, user(), vec![
            Op::AddFile { media_type: "image/png".into(), bytes: a },
            Op::AddFile { media_type: "image/png".into(), bytes: b },
            op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "image", "file": ha}})),
        ]))
        .unwrap();
    let image = created_layer(&change, 0);
    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": image, "patch": {"file": hb}}),
            )],
        ))
        .unwrap();
    assert!(session.file(&ha).is_err(), "the replaced file is freed");
    assert!(session.file(&hb).is_ok());

    let change = session
        .apply(edit(
            &session,
            user(),
            vec![Op::RemovePage { page: page.into() }],
        ))
        .unwrap();
    assert_eq!(change.pages.removed, vec![page]);
    assert!(session.file(&hb).is_err() && session.page(page).is_err());
    session.undo().unwrap().unwrap();
    assert!(session.file(&hb).is_ok());
    assert_eq!(session.page(page).unwrap().layers.len(), 1);
}

#[test]
fn tiles_cover_what_was_painted() {
    let (_dir, mut session, page) = session();
    let inpaint = model("c2pa.created", "inpaint");
    let change = session
        .apply(edit(
            &session,
            inpaint.clone(),
            vec![op(
                json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "raster"}}),
            )],
        ))
        .unwrap();
    let raster = created_layer(&change, 0);
    let paint = |session: &mut Session, author: Author, rect: Rect, pixels: Vec<u8>| {
        session.apply(edit(
            session,
            author,
            vec![Op::WritePixels {
                layer: raster.into(),
                rect,
                pixels,
            }],
        ))
    };
    paint(
        &mut session,
        inpaint.clone(),
        Rect {
            x: 30,
            y: -10,
            width: 100,
            height: 70,
        },
        [255, 0, 0, 255].repeat(100 * 70),
    )
    .unwrap();
    let positions: Vec<(i32, i32)> = session
        .tiles(raster, None)
        .unwrap()
        .tiles
        .iter()
        .map(|tile| (tile.x, tile.y))
        .collect();
    assert_eq!(
        positions,
        [(0, -1), (0, 0), (1, -1), (1, 0), (2, -1), (2, 0)]
    );
    let origin = session.layer(raster).unwrap().view.origin;
    assert_eq!(
        (origin.len(), origin[0].writes.clone()),
        (1, vec![pointer("")]),
        "pixel writes record nothing"
    );

    // The user's painting replaces pixels with no override; a rerun paints
    // over it, and undo brings it back.
    paint(
        &mut session,
        user(),
        Rect {
            x: 40,
            y: 0,
            width: 2,
            height: 2,
        },
        [0, 0, 255, 255].repeat(4),
    )
    .unwrap();
    let painted = session.tiles(raster, None).unwrap();
    assert!(session.layer(raster).unwrap().view.overrides.is_empty());
    paint(
        &mut session,
        inpaint,
        Rect {
            x: 40,
            y: 0,
            width: 2,
            height: 2,
        },
        [0, 255, 0, 255].repeat(4),
    )
    .unwrap();
    let tiles = session
        .tiles(
            raster,
            Some(Rect {
                x: 40,
                y: 0,
                width: 1,
                height: 1,
            }),
        )
        .unwrap();
    assert_eq!(&tiles.tiles[0].pixels[40 * 4..41 * 4], &[0, 255, 0, 255]);
    session.undo().unwrap();
    assert_eq!(session.tiles(raster, None).unwrap(), painted);

    // Transparent pixels drop their tiles; a duplicate copies tiles.
    paint(
        &mut session,
        user(),
        Rect {
            x: 0,
            y: -64,
            width: 192,
            height: 128,
        },
        vec![0; 192 * 128 * 4],
    )
    .unwrap();
    assert!(session.tiles(raster, None).unwrap().tiles.is_empty());
    paint(
        &mut session,
        user(),
        Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        vec![9, 9, 9, 255],
    )
    .unwrap();
    let change = session
        .apply(edit(
            &session,
            user(),
            vec![Op::DuplicateLayer {
                layer: raster.into(),
                id: None,
                index: None,
            }],
        ))
        .unwrap();
    let copy = created_layer(&change, 0);
    assert_eq!(session.tiles(copy, None).unwrap().tiles.len(), 1);
    assert_eq!(
        session.page(page).unwrap().layers[1].id,
        copy,
        "the copy sits above"
    );

    let error = paint(
        &mut session,
        user(),
        Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        },
        vec![0; 3],
    )
    .unwrap_err();
    assert!(
        matches!(error, Error::Invalid { .. }),
        "the length must match"
    );
}

#[test]
fn strings_that_look_like_loro_ids_stay_strings() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "group"}}),
            )],
        ))
        .unwrap();
    let group = created_layer(&change, 0);
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": group, "patch": {"name": "🦜:x", "attributes": {"ns": {"hash": "keep"}}}}))]))
        .unwrap();
    let view = session.layer(group).unwrap().view;
    assert_eq!(name(&view.layer), "🦜:x");
    let Layer::Group(stored) = view.layer else {
        panic!()
    };
    assert_eq!(stored.attributes["ns"]["hash"], json!("keep"));
}
