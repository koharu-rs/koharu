//! Texts, block translations, and attributes.

mod common;
use common::*;
use koharu_scene::{CharStyle, Content, Error, LayerId, Op, RichText, Session, Span, Text};
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
                    ..Span::default()
                },
                Span::default(),
                Span {
                    insert: "b".into(),
                    ..Span::default()
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
fn typing_splices_and_marks_rich_text_keeping_the_other_characters_marks() {
    let bold = CharStyle {
        weight: Some(700),
        ..CharStyle::default()
    };
    let text = RichText(vec![
        Span {
            insert: "Hello".to_owned(),
            attributes: bold.clone(),
            ruby: None,
        },
        Span {
            insert: " world".to_owned(),
            ..Span::default()
        },
    ]);
    // A character typed at the bold word's end takes its marks.
    let typed = text.spliced(5..5, "!", &text.marks_at(5));
    assert_eq!(marked(&typed), [("Hello!", Some(700)), (" world", None)]);
    // Typing over a selection replaces it with the given marks.
    let over = text.spliced(3..8, "p wo", &CharStyle::default());
    assert_eq!(marked(&over), [("Hel", Some(700)), ("p world", None)]);
    // Marking a range splits its spans, and alike neighbours join again.
    let light = CharStyle {
        weight: Some(300),
        ..CharStyle::default()
    };
    let lightened = text.marked(3..8, &light);
    assert_eq!(
        marked(&lightened),
        [("Hel", Some(700)), ("lo wo", Some(300)), ("rld", None)]
    );
    assert_eq!(
        marked(&lightened.marked(3..5, &bold)),
        [("Hello", Some(700)), (" wo", Some(300)), ("rld", None)]
    );
    // At the start a character takes the first one's marks.
    assert_eq!(text.marks_at(0), bold);
    assert_eq!(text.marks_at(6), CharStyle::default());
    assert_eq!(RichText::default().marks_at(0), CharStyle::default());
}

/// Each span's characters and its ruby.
fn annotated(text: &RichText) -> Vec<(&str, Option<&str>)> {
    text.0
        .iter()
        .map(|span| (span.insert.as_str(), span.ruby.as_deref()))
        .collect()
}

#[test]
fn a_base_keeps_its_ruby_until_it_is_reworded() {
    let text = RichText::from("the Holy Sword rises").annotated(4..14, Some("Excalibur"));
    let ruby = Some("Excalibur");
    assert_eq!(
        annotated(&text),
        [("the ", None), ("Holy Sword", ruby), (" rises", None)]
    );
    // What's typed inside a base is part of it, and at its edges isn't.
    let plain = CharStyle::default();
    assert_eq!(
        text.spliced(8..8, "y", &plain).ruby(),
        [(4..15, "Excalibur")]
    );
    assert_eq!(
        text.spliced(4..4, "x", &plain).ruby(),
        [(5..15, "Excalibur")]
    );
    assert_eq!(
        text.spliced(14..14, "s", &plain).ruby(),
        [(4..14, "Excalibur")]
    );
    // Deleting from its middle leaves one base, and styling it two spans of one.
    assert_eq!(
        text.spliced(8..9, "", &plain).ruby(),
        [(4..13, "Excalibur")]
    );
    let bold = CharStyle {
        weight: Some(700),
        ..CharStyle::default()
    };
    let styled = text.marked(9..14, &bold);
    assert_eq!(styled.0.len(), 4);
    assert_eq!(styled.ruby(), [(4..14, "Excalibur")]);

    // A new wording keeps the ruby of a base it leaves alone or changes a part
    // of, its last letter too, and drops that of one it runs past or rewords.
    let replaced = |new: &str| text.replaced(new);
    assert_eq!(
        annotated(&replaced("and the Holy Swords rose")),
        [("and the ", None), ("Holy Sword", ruby), ("s rose", None)]
    );
    assert_eq!(
        annotated(&replaced("the Holy Sworn rises")),
        [("the ", None), ("Holy Sworn", ruby), (" rises", None)]
    );
    for new in [
        "the 8 rises",
        "the Holy Grail sets",
        "She left home yesterday.",
    ] {
        assert_eq!(annotated(&replaced(new)), [(new, None)]);
    }
    // Nor does a whole text's stay for the letters two wordings share.
    let whole = RichText::from("Holy Sword").annotated(0..10, Some("Excalibur"));
    assert_eq!(whole.replaced("High Steward").ruby(), []);
}

#[test]
fn ruby_is_never_over_the_white_space_at_its_base_s_ends() {
    let text = RichText::from(
        "you shall
 fall now",
    );
    assert_eq!(
        text.annotated(9..16, Some("ruby")).ruby(),
        [(11..15, "ruby")]
    );
    assert_eq!(text.annotated(9..11, Some("ruby")), text);
}

#[test]
fn a_ruby_on_part_of_a_base_takes_the_rest_s_off() {
    let text = RichText::from("東京都").annotated(0..9, Some("とうきょうと"));
    assert_eq!(
        annotated(&text.annotated(0..6, Some("とうきょう"))),
        [("東京", Some("とうきょう")), ("都", None)]
    );
    assert_eq!(annotated(&text.annotated(3..6, None)), [("東京都", None)]);
}

#[test]
fn text_and_duplicates_survive_reopening() {
    let (_dir, path) = path();
    let mut session = Session::create(&path, koharu_scene::Meta::default()).expect("a project");
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
    let session = Session::open(&path, koharu_scene::Meta::default()).expect("it opens");
    assert_eq!(session.project(), before);
    assert_eq!(text(&session, copied_block).content.text(), "Bye");
}

#[test]
fn a_blocks_translation_comes_and_goes_with_its_own_ops() {
    let (_dir, mut session, page) = session();
    let block = add(&mut session, page, None, block(24.0));
    set_text(&mut session, block, "Hello").expect("a translation");
    // It starts in its source's place.
    let Content::Block(added) = &layer(&session, block).content else {
        panic!("not a block")
    };
    assert_eq!(
        added.translation,
        Some(Text {
            content: RichText::from("Hello"),
            ..added.source.clone()
        })
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
