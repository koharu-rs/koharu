//! What the pipeline does: models write layers and record what they did and
//! read, the user's changes survive reruns as overrides, results go stale
//! when what they read changes, and Reset brings the model's values back.
//!
//! `cargo run -p koharu-scene --example models`

use koharu_scene::{
    Action, Author, Change, Created, Edit, Error, Layer, LayerId, Op, Pointer, PointerBuf,
    Relationship, Session, Size, SoftwareAgent,
};
use serde_json::{Map, Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A model run, described as a C2PA action.
fn model(action: &str, name: &str, parameters: Value) -> Author {
    let Value::Object(parameters) = parameters else {
        unreachable!("parameters are an object")
    };
    Author::Model {
        action: Action {
            action: action.to_owned(),
            software_agent: SoftwareAgent {
                name: name.to_owned(),
                version: Some("1.0".to_owned()),
            },
            digital_source_type: Some(
                "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia".to_owned(),
            ),
            parameters,
        },
    }
}

fn user() -> Author {
    Author::User { via: None }
}

fn edit(session: &Session, author: Author, ops: Vec<Op>) -> Edit {
    Edit {
        project: session.id(),
        label: "Pipeline".to_owned(),
        author,
        reads: Vec::new(),
        ops,
    }
}

fn op(value: Value) -> Op {
    serde_json::from_value(value).expect("a valid op")
}

fn created(change: &Change) -> LayerId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, .. } => Some(*id),
            _ => None,
        })
        .expect("a layer was created")
}

fn shown(session: &Session, layer: LayerId) -> Result<String> {
    match session.layer(layer)?.view.layer {
        Layer::Text(text) => Ok(text.content.text()),
        _ => Err("not a text layer".into()),
    }
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    let change = session.apply(edit(
        &session,
        user(),
        vec![Op::AddPage {
            id: None,
            index: 0,
            size: Size {
                width: 1200,
                height: 1700,
            },
        }],
    ))?;
    let page = change.pages.added[0];
    let content = Pointer::from_static("/content");

    // OCR creates the source text. Its origin records the action and the
    // paths it wrote: `""`, the whole layer.
    let ocr = model("c2pa.created", "ocr", json!({}));
    let change = session.apply(edit(
        &session,
        ocr.clone(),
        vec![op(json!({
            "op": "add_layer", "page": page, "index": 0,
            "layer": {"kind": "text", "content": "こんばんは", "language": "ja", "visible": false},
        }))],
    ))?;
    let source = created(&change);
    let origin = session.layer(source)?.view.origin;
    assert_eq!(origin[0].writes, vec![PointerBuf::new()]);

    // The translator reads the source and names it as its `parentOf`
    // ingredient, by hash, as C2PA does.
    let translator = || {
        model(
            "c2pa.translated",
            "translator",
            json!({"sourceLanguage": "ja", "targetLanguage": "en"}),
        )
    };
    let (_, read) = session.read(source, content)?;
    let change = session.apply(edit(
        &session,
        translator(),
        vec![Op::AddLayer {
            id: None,
            page: page.into(),
            parent: None,
            index: 1,
            layer: serde_json::from_value(json!({
                "kind": "text", "content": "Good evening", "language": "en",
                "style": {"chars": {"size": 24.0}},
            }))?,
            ingredients: vec![read.parent_of()],
        }],
    ))?;
    let translation = created(&change);
    let view = session.layer(translation)?.view;
    assert_eq!(
        view.origin[0].ingredients[0].relationship,
        Relationship::ParentOf
    );
    assert!(view.overrides.is_empty() && view.stale.is_empty());
    println!(
        "{:?} was translated from {:?}",
        shown(&session, translation)?,
        shown(&session, source)?
    );

    // The user changes what the model wrote. The model's values are kept
    // under overrides.
    session.apply(edit(&session, user(), vec![
        Op::SetText {
            layer: translation.into(),
            text: "Evening!".to_owned(),
            ingredients: Vec::new(),
        },
        op(json!({"op": "update_layer", "layer": translation, "patch": {"style": {"chars": {"size": 30.0}}}})),
    ]))?;
    let overrides = session.layer(translation)?.view.overrides;
    assert_eq!(overrides.len(), 2, "{overrides:?}");

    // A rerun writes its new values under the overrides, so the user's keep
    // showing.
    let (_, read) = session.read(source, content)?;
    session.apply(edit(&session, translator(), vec![
        Op::SetText {
            layer: translation.into(),
            text: "Good evening.".to_owned(),
            ingredients: vec![read.parent_of()],
        },
        op(json!({"op": "update_layer", "layer": translation, "patch": {"style": {"chars": {"size": 22.0}}}})),
    ]))?;
    assert_eq!(shown(&session, translation)?, "Evening!");
    println!(
        "after a rerun the user's {:?} still shows",
        shown(&session, translation)?
    );

    // Changing the source makes the translation stale. A result computed from
    // the old source is rejected before anything is written.
    let (_, old) = session.read(source, content)?;
    session.apply(edit(
        &session,
        user(),
        vec![Op::SetText {
            layer: source.into(),
            text: "おはよう".to_owned(),
            ingredients: Vec::new(),
        }],
    ))?;
    assert!(!session.layer(translation)?.view.stale.is_empty());
    let error = session
        .apply(edit(
            &session,
            translator(),
            vec![Op::SetText {
                layer: translation.into(),
                text: "Good evening.".to_owned(),
                ingredients: vec![old.parent_of()],
            }],
        ))
        .unwrap_err();
    assert!(matches!(error, Error::Stale { .. }));
    println!("rejected: {error}");
    let (_, fresh) = session.read(source, content)?;
    session.apply(edit(
        &session,
        translator(),
        vec![Op::SetText {
            layer: translation.into(),
            text: "Good morning.".to_owned(),
            ingredients: vec![fresh.parent_of()],
        }],
    ))?;
    // The rerun takes over everything the translator wrote, so the
    // translation is current again, with one translator action.
    let view = session.layer(translation)?.view;
    assert!(view.stale.is_empty());
    assert_eq!(view.origin.len(), 1);
    assert_eq!(view.origin[0].writes, vec![PointerBuf::new()]);
    println!("a fresh rerun made the translation current again");

    // Reset shows the model's latest values again.
    session.apply(edit(
        &session,
        user(),
        vec![Op::Reset {
            layer: translation.into(),
            paths: Some(vec![content.to_buf()]),
        }],
    ))?;
    assert_eq!(shown(&session, translation)?, "Good morning.");
    assert_eq!(session.layer(translation)?.view.overrides.len(), 1);
    println!("reset the text to {:?}", shown(&session, translation)?);

    // A model may write into a layer the user made, taking over the paths it
    // writes, but it can only remove layers it created.
    let change = session.apply(edit(&session, user(), vec![op(json!({
        "op": "add_layer", "page": page, "index": 2, "layer": {"kind": "text", "content": "SFX"},
    }))]))?;
    let mine = created(&change);
    session.apply(edit(
        &session,
        translator(),
        vec![op(json!({
            "op": "update_layer", "layer": mine, "patch": {"language": "en"},
        }))],
    ))?;
    assert_eq!(
        session.layer(mine)?.view.origin[0].writes,
        vec![PointerBuf::parse("/language")?]
    );
    let error = session
        .apply(edit(
            &session,
            translator(),
            vec![op(json!({"op": "remove_layer", "layer": mine}))],
        ))
        .unwrap_err();
    println!("rejected: {error}");

    // When a rerun no longer produces a layer, the layer goes, unless the
    // user changed it.
    let change = session.apply(edit(
        &session,
        translator(),
        vec![op(json!({"op": "remove_layer", "layer": translation}))],
    ))?;
    assert_eq!(change.kept, vec![translation]);
    let change = session.apply(edit(&session, ocr.clone(), vec![op(json!({
        "op": "add_layer", "page": page, "index": 0, "layer": {"kind": "text", "content": "ドン"},
    }))]))?;
    let stray = created(&change);
    let change = session.apply(edit(
        &session,
        ocr,
        vec![op(json!({"op": "remove_layer", "layer": stray}))],
    ))?;
    assert!(change.kept.is_empty() && session.layer(stray).is_err());
    println!("kept the translation the user changed and removed an untouched OCR layer");

    // Pixels are not a property: a model's pixel write records nothing, and
    // the user's painting replaces it with no override.
    let inpaint = model("c2pa.created", "inpaint", Value::Object(Map::new()));
    let change = session.apply(edit(
        &session,
        inpaint.clone(),
        vec![op(json!({
            "op": "add_layer", "page": page, "index": 0, "layer": {"kind": "raster"},
        }))],
    ))?;
    let raster = created(&change);
    session.apply(edit(&session, inpaint, vec![op(json!({
        "op": "write_pixels", "layer": raster,
        "rect": {"x": 0, "y": 0, "width": 2, "height": 1}, "pixels": [1, 2, 3, 255, 4, 5, 6, 255],
    }))]))?;
    let view = session.layer(raster)?.view;
    assert_eq!(view.origin.len(), 1);
    assert_eq!(view.origin[0].writes, vec![PointerBuf::new()]);
    println!("the inpaint layer's origin records only its creation");
    Ok(())
}
