//! Text styles: texts take their unset values from the style they name,
//! styles keep the user's changes under a model's reruns, and a named style
//! can't be removed. Also the frames' fit and anchor.

mod common;
use common::*;

fn style(session: &Session, id: StyleId) -> StyleView {
    session.style(id).unwrap()
}

#[test]
fn texts_name_styles_that_stay_while_named() {
    let (_dir, mut session, page) = session();
    let dialogue = StyleId::new();
    let change = session
        .apply(edit(
            &session,
            user(),
            vec![
                op(json!({"op": "add_text_style", "id": dialogue, "style": {
                    "name": "Dialogue", "style": {"chars": {"size": 18.0}, "line_height": 1.1},
                }})),
                op(
                    json!({"op": "add_layer", "page": page, "index": 0, "layer": {
                        "kind": "text", "content": "a", "text_style": dialogue,
                    }}),
                ),
            ],
        ))
        .unwrap();
    assert_eq!(change.styles, [dialogue].into());
    let text = created_layer(&change, 1);
    assert!(session.layer(text).unwrap().view.style_overrides.is_empty());

    // The text's own value wins over the style's, and the view says so.
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": text, "patch": {"style": {"chars": {"size": 24.0}}}}))]))
        .unwrap();
    assert_eq!(
        session.layer(text).unwrap().view.style_overrides,
        [pointer("/style/chars/size")]
    );
    let layer = common::text(&session, text);
    let shown = layer.style.or(&style(&session, dialogue).style.style);
    assert_eq!(
        (shown.chars.size, shown.line_height),
        (Some(24.0), Some(1.1))
    );

    // A style a text names can't go, and a text can't name a missing one.
    let remove = Op::RemoveTextStyle {
        style: dialogue.into(),
    };
    let error = session
        .apply(edit(&session, user(), vec![remove.clone()]))
        .unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
    let error = session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": text, "patch": {"text_style": StyleId::new()}}))]))
        .unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }), "{error}");

    // Unnamed, it goes; undo brings it back, named again.
    session.save().unwrap();
    session
        .apply(edit(
            &session,
            user(),
            vec![
                op(json!({"op": "update_layer", "layer": text, "patch": {"text_style": null}})),
                remove,
            ],
        ))
        .unwrap();
    assert!(session.style(dialogue).is_err());
    let undone = session.undo().unwrap().unwrap();
    assert!(undone.styles.contains(&dialogue), "{undone:?}");
    assert_eq!(style(&session, dialogue).style.name, "Dialogue");
    assert_eq!(common::text(&session, text).text_style, Some(dialogue));
    assert_eq!(session.project().unwrap().styles.len(), 1);
}

#[test]
fn a_rerun_keeps_the_users_change_to_a_style() {
    let (dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, user(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "原文"}}))]))
        .unwrap();
    let source = created_layer(&change, 0);
    let (_, read) = session.read(source, &pointer("/content")).unwrap();
    let typeset = model("c2pa.created", "typeset");
    let change = session
        .apply(edit(
            &session,
            typeset.clone(),
            vec![Op::AddTextStyle {
                id: None,
                style: serde_json::from_value(
                    json!({"name": "Dialogue", "style": {"chars": {"size": 14.0}}}),
                )
                .unwrap(),
                ingredients: vec![read],
            }],
        ))
        .unwrap();
    let Created::Style { id: dialogue, .. } = change.created[0] else {
        panic!("{change:?}");
    };

    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_text_style", "style": dialogue, "patch": {"style": {"chars": {"size": 20.0}}}}))]))
        .unwrap();
    let view = style(&session, dialogue);
    assert_eq!(view.overrides, [pointer("/style/chars/size")]);

    // A rerun's size goes under the user's; its other change lands.
    let (_, fresh) = session.read(source, &pointer("/content")).unwrap();
    session
        .apply(edit(
            &session,
            typeset,
            vec![Op::UpdateTextStyle {
                style: dialogue.into(),
                patch: json!({"style": {"chars": {"size": 16.0}, "line_height": 1.0}}),
                ingredients: vec![fresh],
            }],
        ))
        .unwrap();
    let view = style(&session, dialogue).style.style;
    assert_eq!((view.chars.size, view.line_height), (Some(20.0), Some(1.0)));

    // The style goes stale with its ingredients, and Reset shows the rerun's.
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
    assert!(!style(&session, dialogue).stale.is_empty());
    session
        .apply(edit(
            &session,
            user(),
            vec![Op::ResetTextStyle {
                style: dialogue.into(),
                paths: None,
            }],
        ))
        .unwrap();
    session.save().unwrap();
    drop(session);
    let session = Session::open(dir.path().join("test.koharu")).unwrap();
    let view = style(&session, dialogue);
    assert_eq!(view.style.style.chars.size, Some(16.0));
    assert!(view.overrides.is_empty());
}

#[test]
fn frames_fit_and_anchor() {
    let (_dir, mut session, page) = session();
    let change = session
        .apply(edit(&session, user(), vec![op(json!({"op": "add_layer", "page": page, "index": 0, "layer": {
            "kind": "text", "content": "a", "frame": {"path": {"path": "M0 0 L10 0 L10 10 Z"}},
        }}))]))
        .unwrap();
    let layer = created_layer(&change, 0);
    assert!(matches!(
        common::text(&session, layer).frame,
        Frame::Path {
            fit: PathFit::Shrink,
            anchor: Anchor::Start,
            ..
        }
    ));

    // A path never grows; a box can.
    let error = session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"frame": {"path": {"fit": "grow"}}}}))]))
        .unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
    session
        .apply(edit(&session, user(), vec![op(json!({"op": "update_layer", "layer": layer, "patch": {"frame": {
            "box": {"bounds": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0}, "fit": "grow", "anchor": "center"},
            "path": null,
        }}}))]))
        .unwrap();
    assert!(matches!(
        common::text(&session, layer).frame,
        Frame::Box {
            fit: Fit::Grow,
            anchor: Anchor::Center,
            ..
        }
    ));
}
