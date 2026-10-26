//! The user's changes to what a model wrote: overrides, reruns under them,
//! Reset, and duplicates that keep them.

mod common;
use common::*;

#[test]
fn user_changes_survive_reruns_until_reset() {
    let (_dir, mut session, page) = session();
    let ocr = model("c2pa.created", "ocr");
    let change = session
        .apply(edit(&session, ocr.clone(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {
            "kind": "text", "content": "first", "style": {"chars": {"size": 12.0}},
            "frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0}}},
        }}))]))
        .unwrap();
    let layer = created_layer(&change, 0);

    // The user's size is an override at the leaf path.
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"style": {"chars": {"size": 20.0}}}}))]))
        .unwrap();
    assert_eq!(
        session.layer(layer).unwrap().view.overrides,
        vec![pointer("/style/chars/size")]
    );

    // A rerun's size goes under the override; its other changes land.
    session
        .apply(edit(&session, ocr.clone(), vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"style": {"chars": {"size": 14.0}}, "name": "ocr"}}))]))
        .unwrap();
    let shown = text(&session, layer);
    assert_eq!(
        (shown.style.chars.size, shown.name.as_str()),
        (Some(20.0), "ocr")
    );

    // Switching the frame's variant overrides the whole frame.
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"frame": {"path": {"path": "M0 0 L1 1 Z"}, "box": null}}}))]))
        .unwrap();
    assert!(matches!(text(&session, layer).frame, Frame::Path { .. }));
    assert!(
        session
            .layer(layer)
            .unwrap()
            .view
            .overrides
            .contains(&pointer("/frame"))
    );

    // The user's text keeps the model's, and a rerun writes to the kept one.
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::SetText {
                layer: layer.into(),
                text: "mine".into(),
                ingredients: vec![],
            }],
        ))
        .unwrap();
    session
        .apply(edit(
            &session,
            ocr,
            vec![Op::SetText {
                layer: layer.into(),
                text: "second".into(),
                ingredients: vec![],
            }],
        ))
        .unwrap();
    assert_eq!(text(&session, layer).content.text(), "mine");

    // Reset shows the model's latest values.
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::Reset {
                layer: layer.into(),
                paths: None,
            }],
        ))
        .unwrap();
    let shown = text(&session, layer);
    assert_eq!(shown.content.text(), "second");
    assert_eq!(shown.style.chars.size, Some(14.0));
    assert!(matches!(shown.frame, Frame::Box { .. }));
    assert!(session.layer(layer).unwrap().view.overrides.is_empty());
    let error = session
        .apply(edit(
            &session,
            user(),
            vec![Op::Reset {
                layer: layer.into(),
                paths: Some(vec![pointer("/name")]),
            }],
        ))
        .unwrap_err();
    assert!(
        matches!(error, Error::Invalid { .. }),
        "nothing is overridden there: {error}"
    );

    // A model may write into a layer the user made.
    let change = session
        .apply(edit(&session, user(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "user text"}}))]))
        .unwrap();
    let mine = created_layer(&change, 0);
    session
        .apply(edit(
            &session,
            model("c2pa.translated", "llm"),
            vec![Op::SetText {
                layer: mine.into(),
                text: "model text".into(),
                ingredients: vec![],
            }],
        ))
        .unwrap();
    assert_eq!(text(&session, mine).content.text(), "model text");
    assert_eq!(session.layer(mine).unwrap().view.origin.len(), 1);
}

#[test]
fn a_models_variant_switch_widens_the_users_override() {
    let (_dir, mut session, page) = session();
    let typeset = model("c2pa.created", "typeset");
    let change = session
        .apply(edit(&session, typeset.clone(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {
            "kind": "text", "content": "a", "frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0}}},
        }}))]))
        .unwrap();
    let layer = created_layer(&change, 0);
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"frame": {"box": {"bounds": {"x": 5.0}}}}}))]))
        .unwrap();
    assert_eq!(
        session.layer(layer).unwrap().view.overrides,
        vec![pointer("/frame/box/bounds/x")]
    );

    session
        .apply(edit(&session, typeset, vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"frame": {"path": {"path": "M0 0 L1 1 Z"}, "box": null}}}))]))
        .unwrap();
    assert_eq!(
        session.layer(layer).unwrap().view.overrides,
        vec![pointer("/frame")]
    );
    assert!(
        matches!(text(&session, layer).frame, Frame::Box { bounds, .. } if bounds.x == 5.0),
        "the user's frame still shows"
    );
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::Reset {
                layer: layer.into(),
                paths: None,
            }],
        ))
        .unwrap();
    assert!(matches!(text(&session, layer).frame, Frame::Path { .. }));
}

#[test]
fn duplicates_keep_origin_and_overrides() {
    let (_dir, path) = path();
    let mut session = Session::create(&path).unwrap();
    let page = page(&mut session);
    let change = session
        .apply(edit(&session, model("c2pa.created", "ocr"), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "source"}}))]))
        .unwrap();
    let source = created_layer(&change, 0);
    let typeset = model("c2pa.created", "typeset");
    let change = session
        .apply(edit(&session, typeset.clone(), vec![op(json!({"op": "add_layer", "page": page, "index": 1, "layer": {
            "kind": "text", "name": "model name",
            "content": [{"insert": "model", "attributes": {"weight": 700}}, {"insert": " text"}],
        }}))]))
        .unwrap();
    let layer = created_layer(&change, 0);
    let (_, read) = session.read(source, &pointer("/content")).unwrap();
    session
        .apply(edit(
            &session,
            typeset,
            vec![Op::UpdateLayer {
                layer: layer.into(),
                patch: json!({"opacity": 0.5}),
                ingredients: vec![read.parent_of()],
            }],
        ))
        .unwrap();
    session
        .apply(edit(
            &session,
            user(),
            vec![
                op(json!({"op": "update_layer", "layer": layer, "patch": {"name": "mine"}})),
                Op::SetText {
                    layer: layer.into(),
                    text: "my text".into(),
                    ingredients: vec![],
                },
            ],
        ))
        .unwrap();
    let original = session.layer(layer).unwrap().view;
    assert_eq!(original.overrides.len(), 2, "{:?}", original.overrides);

    let change = session
        .apply(edit(
            &session,
            user(),
            vec![
                Op::DuplicateLayer {
                    layer: layer.into(),
                    id: Some(label("copy")),
                    index: None,
                },
                op(json!({"op": "update_layer", "layer": "$copy", "patch": {"name": "mine copy"}})),
            ],
        ))
        .unwrap();
    let copy = created_layer(&change, 0);
    let view = session.layer(copy).unwrap().view;
    assert_eq!(
        (view.origin.clone(), view.overrides.clone()),
        (original.origin.clone(), original.overrides.clone())
    );
    assert_eq!(text(&session, copy).content.text(), "my text");
    assert_eq!(name(&view.layer), "mine copy");

    // The copy goes stale with the original's ingredients.
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::SetText {
                layer: source.into(),
                text: "changed".into(),
                ingredients: vec![],
            }],
        ))
        .unwrap();
    assert!(!session.layer(copy).unwrap().view.stale.is_empty());
    assert_eq!(
        session.layer(copy).unwrap().view.stale,
        session.layer(layer).unwrap().view.stale
    );

    // It survives reopening, and resets to the model's values, marks
    // included, leaving the original alone.
    session.save().unwrap();
    drop(session);
    let mut session = Session::open(&path).unwrap();
    assert_eq!(session.layer(copy).unwrap().view.origin, original.origin);
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::Reset {
                layer: copy.into(),
                paths: None,
            }],
        ))
        .unwrap();
    let reset = text(&session, copy);
    assert_eq!(
        (reset.content.text(), reset.name.as_str()),
        ("model text".to_owned(), "model name")
    );
    assert_eq!(marked(&reset.content).len(), 1);
    assert_eq!(text(&session, layer).content.text(), "my text");
}
