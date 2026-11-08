//! Texts, block translations, and attributes.

mod common;
use common::*;
use koharu_scene::{Content, Error, LayerId, Op, RichText, Session, Span, Text};
use serde_json::json;

fn text(session: &Session, layer: LayerId) -> Text {
    match &common::layer(session, layer).content {
        Content::Text(text) => (**text).clone(),
        Content::Block(block) => block.translation.clone().expect("a translation"),
        other => panic!("{other:?} holds no text"),
    }
}

fn set_text(session: &mut Session, layer: LayerId, text: &str) -> koharu_scene::Result<()> {
    session
        .apply(edit(vec![Op::SetText {
            layer: layer.into(),
            text: text.to_owned(),
        }]))
        .map(|_| ())
}

fn marked(text: &RichText) -> Vec<(&str, Option<u16>)> {
    text.0
        .iter()
        .map(|span| (span.insert.as_str(), span.attributes.weight))
        .collect()
}

#[test]
fn set_text_keeps_marks_on_the_characters_that_stayed() {
    let (_dir, mut session, page) = session();
    let layer = add(
        &mut session,
        page,
        None,
        json!({"kind": "text", "content": [
            {"insert": "Hello", "attributes": {"weight": 700}},
            {"insert": " world"},
        ]}),
    );
    set_text(&mut session, layer, "Hello there world").expect("a text");
    assert_eq!(
        marked(&text(&session, layer).content),
        [("Hello", Some(700)), (" there world", None)]
    );
    set_text(&mut session, layer, "Help world").expect("a text");
    assert_eq!(
        marked(&text(&session, layer).content),
        [("Hel", Some(700)), ("p world", None)]
    );

    // The rich editor's spans replace the text whole, joined as stored.
    session
        .apply(edit(vec![Op::SetRichText {
            layer: layer.into(),
            spans: vec![
                Span {
                    insert: "a".into(),
                    attributes: Default::default(),
                },
                Span {
                    insert: "".into(),
                    attributes: Default::default(),
                },
                Span {
                    insert: "b".into(),
                    attributes: Default::default(),
                },
            ],
        }]))
        .expect("spans");
    assert_eq!(marked(&text(&session, layer).content), [("ab", None)]);

    let raster = add(&mut session, page, None, json!({"kind": "raster"}));
    let error = set_text(&mut session, raster, "x").expect_err("a raster has no text");
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
}

#[test]
fn text_and_duplicates_survive_reopening() {
    let (_dir, path) = path();
    let mut session = Session::create(&path).expect("a project");
    let page = page(&mut session);
    let balloon = add(
        &mut session,
        page,
        None,
        json!({"kind": "balloon", "outline": outline()}),
    );
    let block = add(&mut session, page, Some(balloon), block(0.0));
    set_text(&mut session, block, "Hi").expect("a translation");
    let change = session
        .apply(edit(vec![Op::DuplicateLayer {
            layer: balloon.into(),
            id: None,
            index: None,
        }]))
        .expect("a duplicate");
    let (copy, copied_block) = (created_layer(&change, 0), created_layer(&change, 1));
    assert_eq!(layer(&session, copy).name, layer(&session, balloon).name);
    assert_eq!(text(&session, copied_block).content.text(), "Hi");
    set_text(&mut session, copied_block, "Bye").expect("a translation");
    assert_eq!(
        text(&session, block).content.text(),
        "Hi",
        "the copy is its own"
    );

    let before = session.project();
    drop(session);
    let session = Session::open(&path).expect("it opens");
    assert_eq!(session.project(), before);
    assert_eq!(text(&session, copied_block).content.text(), "Bye");
}

#[test]
fn a_blocks_translation_comes_and_goes_with_its_own_ops() {
    let (_dir, mut session, page) = session();
    let block = add(&mut session, page, None, block(0.0));
    set_text(&mut session, block, "Hello").expect("a translation");
    assert_eq!(
        text(&session, block),
        Text {
            content: RichText::from("Hello"),
            ..Text::default()
        }
    );
    apply(
        &mut session,
        vec![json!({"op": "update_layer", "layer": block, "patch": {"translation": null}})],
    )
    .expect("no translation");
    let Content::Block(gone) = &layer(&session, block).content else {
        panic!("not a block")
    };
    assert_eq!(gone.translation, None);
    session.undo().expect("undo").expect("a step");
    assert_eq!(text(&session, block).content.text(), "Hello");

    let balloon = add(
        &mut session,
        page,
        None,
        json!({"kind": "balloon", "outline": outline()}),
    );
    let error = set_text(&mut session, balloon, "x").expect_err("a balloon has no text");
    assert!(matches!(error, Error::Invalid { .. }), "{error}");

    // Attributes: the last writer wins, and an empty namespace goes.
    let set = |value: serde_json::Value| json!({"op": "set_attribute", "layer": block, "namespace": "c2pa", "key": "ocr", "value": value});
    apply(&mut session, vec![set(json!({"action": "c2pa.created"}))]).expect("an attribute");
    apply(&mut session, vec![set(json!({"action": "c2pa.edited"}))]).expect("an attribute");
    assert_eq!(
        layer(&session, block).attributes["c2pa"]["ocr"],
        json!({"action": "c2pa.edited"})
    );
    apply(&mut session, vec![set(serde_json::Value::Null)]).expect("no attribute");
    assert!(layer(&session, block).attributes.is_empty());

    // A lock above stops text and attribute writes alike.
    apply(
        &mut session,
        vec![json!({"op": "update_layer", "layer": block, "patch": {"locked": true}})],
    )
    .expect("a lock");
    let error = set_text(&mut session, block, "x").expect_err("locked");
    assert!(matches!(error, Error::Locked { .. }), "{error}");
    let error = apply(&mut session, vec![set(json!(1))]).expect_err("locked");
    assert!(matches!(error, Error::Locked { .. }), "{error}");
}
