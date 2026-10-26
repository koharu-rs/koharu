//! What models record: ingredients, staleness, a rerun taking over its
//! model's paths, and which layers a model may remove.

mod common;
use common::*;

#[test]
fn results_go_stale_and_stale_results_are_rejected() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, model("c2pa.created", "ocr"), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "こんにちは", "language": "ja"}}))]))
        .unwrap();
    let source = created_layer(&change, 0);
    let (value, read) = session.read(source, &pointer("/content")).unwrap();
    assert_eq!(value, json!("こんにちは"));
    let change = session
        .apply(edit(
            &session,
            model("c2pa.translated", "llm"),
            vec![Op::AddLayer {
                id: None,
                page: page.into(),
                parent: None,
                index: 1,
                layer: Box::new(
                    serde_json::from_value(
                        json!({"kind": "text", "content": "Hello", "language": "en"}),
                    )
                    .unwrap(),
                ),
                ingredients: vec![read.clone().parent_of()],
            }],
        ))
        .unwrap();
    let translation = created_layer(&change, 0);
    assert!(session.layer(translation).unwrap().view.stale.is_empty());

    // Changing what a result read makes it stale.
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::SetText {
                layer: source.into(),
                text: "さようなら".into(),
                ingredients: vec![],
            }],
        ))
        .unwrap();
    assert!(!session.layer(translation).unwrap().view.stale.is_empty());

    // An edit carrying the old read is rejected before anything is written.
    let error = session
        .apply(Edit {
            reads: vec![read],
            ..edit(
                &session,
                user(),
                vec![Op::SetText {
                    layer: translation.into(),
                    text: "x".into(),
                    ingredients: vec![],
                }],
            )
        })
        .unwrap_err();
    assert!(matches!(error, Error::Stale { .. }), "{error}");

    // An action derives from at most one parent.
    let (_, again) = session.read(source, &pointer("/content")).unwrap();
    let (_, other) = session.read(translation, &pointer("/content")).unwrap();
    let error = session
        .apply(edit(
            &session,
            model("c2pa.translated", "llm"),
            vec![Op::SetText {
                layer: translation.into(),
                text: "y".into(),
                ingredients: vec![again.parent_of(), other.parent_of()],
            }],
        ))
        .unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
}

#[test]
fn a_rerun_takes_over_every_path_its_model_wrote() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, user(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "src"}}))]))
        .unwrap();
    let source = created_layer(&change, 0);
    let translator = model("c2pa.translated", "translator");
    let (_, read) = session.read(source, &pointer("/content")).unwrap();
    let change = session
        .apply(edit(
            &session,
            translator.clone(),
            vec![Op::AddLayer {
                id: None,
                page: page.into(),
                parent: None,
                index: 1,
                layer: Box::new(
                    serde_json::from_value(json!({"kind": "text", "content": "t"})).unwrap(),
                ),
                ingredients: vec![read.parent_of()],
            }],
        ))
        .unwrap();
    let layer = created_layer(&change, 0);
    // Another model writes one path with its own ingredient.
    let (_, styled) = session.read(source, &pointer("")).unwrap();
    session
        .apply(edit(
            &session,
            model("c2pa.edited", "typeset"),
            vec![Op::UpdateLayer {
                layer: layer.into(),
                patch: json!({"name": "styled"}),
                ingredients: vec![styled],
            }],
        ))
        .unwrap();

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
    assert!(!session.layer(layer).unwrap().view.stale.is_empty());

    // Rerunning only the text takes over everything the translator wrote,
    // so only the other model's path stays stale.
    let (_, fresh) = session.read(source, &pointer("/content")).unwrap();
    session
        .apply(edit(
            &session,
            translator.clone(),
            vec![Op::SetText {
                layer: layer.into(),
                text: "t2".into(),
                ingredients: vec![fresh.parent_of()],
            }],
        ))
        .unwrap();
    let view = session.layer(layer).unwrap().view;
    assert_eq!(view.stale, vec![pointer("/name")]);
    let translators: Vec<_> = view
        .origin
        .iter()
        .filter(|record| record.action.software_agent.name == "translator")
        .collect();
    assert_eq!(translators.len(), 1);
    assert_eq!(translators[0].writes, vec![pointer("")]);

    // The same values again, with fresh ingredients, still take over.
    let (_, fresh) = session.read(source, &pointer("")).unwrap();
    session
        .apply(edit(
            &session,
            model("c2pa.edited", "typeset"),
            vec![Op::UpdateLayer {
                layer: layer.into(),
                patch: json!({"name": "styled"}),
                ingredients: vec![fresh],
            }],
        ))
        .unwrap();
    assert!(session.layer(layer).unwrap().view.stale.is_empty());
}

#[test]
fn a_model_removes_only_what_it_created_and_the_user_left_alone() {
    let (_dir, mut session, page) = session();
    let detect = model("c2pa.created", "detect");
    let change = session
        .apply(edit(&session, detect.clone(), vec![
            op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "group", "name": "a"}})),
            op(json!({"op": "add_layer", "page": page, "index": 1, "layer": {"kind": "group", "name": "c"}})),
        ]))
        .unwrap();
    let (a, c) = (created_layer(&change, 0), created_layer(&change, 1));
    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "update_layer", "layer": a, "patch": {"name": "renamed"}}),
            )],
        ))
        .unwrap();
    let change = session
        .apply(edit(
            &session,
            detect.clone(),
            vec![Op::RemoveLayer { layer: a.into() }],
        ))
        .unwrap();
    assert_eq!(change.kept, vec![a], "the user changed it");
    let error = session
        .apply(edit(
            &session,
            model("c2pa.created", "other"),
            vec![Op::RemoveLayer { layer: c.into() }],
        ))
        .unwrap_err();
    assert!(
        matches!(error, Error::Invalid { .. }),
        "another model made it"
    );
    let change = session
        .apply(edit(
            &session,
            detect.clone(),
            vec![Op::RemoveLayer { layer: c.into() }],
        ))
        .unwrap();
    assert!(change.kept.is_empty() && session.layer(c).is_err());

    // A layer made earlier in the same edit, and one made by any action.
    session
        .apply(edit(&session, detect, vec![
            op(json!({"op": "add_layer", "id": "$x", "page": page, "index": 0, "layer": {"kind": "group"}})),
            Op::RemoveLayer { layer: label("x") },
        ]))
        .unwrap();
    assert_eq!(session.page(page).unwrap().layers.len(), 1);
    let translator = model("c2pa.translated", "translator");
    let change = session
        .apply(edit(&session, translator.clone(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "t"}}))]))
        .unwrap();
    let translation = created_layer(&change, 0);
    session
        .apply(edit(
            &session,
            translator,
            vec![Op::RemoveLayer {
                layer: translation.into(),
            }],
        ))
        .unwrap();
    assert!(session.layer(translation).is_err());
}
