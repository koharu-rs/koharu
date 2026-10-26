//! Rendering a scene page: the order text fits in, where vertical text
//! sets, and how masks limit a layer.

use koharu_renderer::{Raster, Renderer, ResolvedText, TypesettingConfig};
use koharu_scene::{Author, Change, Created, Edit, LayerId, Op, PageId, Session};
use serde_json::{Value, json};

fn op(value: Value) -> Op {
    serde_json::from_value(value).unwrap()
}

fn apply(session: &mut Session, ops: Vec<Value>) -> Change {
    let edit = Edit {
        project: session.id(),
        label: "test".into(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: ops.into_iter().map(op).collect(),
    };
    session.apply(edit).unwrap()
}

fn layer(change: &Change, name: &str) -> LayerId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, label } if label.as_deref() == Some(name) => Some(*id),
            _ => None,
        })
        .unwrap()
}

/// A project with one `width`×`height` page holding `layers`, each labelled.
fn page(
    width: u32,
    height: u32,
    layers: Vec<Value>,
) -> (tempfile::TempDir, Session, PageId, Change) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::create(dir.path().join("test.koharu")).unwrap();
    let mut ops = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": width, "height": height}}),
    ];
    ops.extend(layers);
    let change = apply(&mut session, ops);
    let page = change.pages.added[0];
    (dir, session, page, change)
}

/// The resolved text of one word in a box `width` wide, shrinking to fit.
fn word_in_box(width: f64, language: &str) -> ResolvedText {
    let (_dir, session, page, change) = page(
        400,
        400,
        vec![json!({
            "op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text",
                "content": "Incomprehensibilities",
                "style": {"chars": {"family": "Arial", "size": 40.0}},
                "language": language,
                "frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": width, "height": 400.0}}},
            },
        })],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page).unwrap();
    frame
        .layer(layer(&change, "text"))
        .unwrap()
        .text()
        .unwrap()
        .clone()
}

#[test]
fn a_long_word_shrinks_then_hyphenates_then_breaks() {
    // With room, the word sets at its size.
    assert_eq!(word_in_box(4000.0, "en").scale, 1.0);

    // A box a little narrower than the word: it shrinks and stays whole.
    let shrunk = word_in_box(300.0, "en");
    assert!(shrunk.scale < 1.0 && shrunk.scale >= 0.5, "{shrunk:?}");
    assert!(shrunk.hyphens.is_empty() && shrunk.broken.is_empty() && !shrunk.overflowed);

    // Too narrow even at half size: it hyphenates by its language.
    let hyphenated = word_in_box(120.0, "en");
    assert!(!hyphenated.hyphens.is_empty(), "{hyphenated:?}");
    assert!(hyphenated.broken.is_empty() && !hyphenated.overflowed);

    // Without a language there are no hyphenation points, so it breaks
    // between letters instead.
    let broken = word_in_box(120.0, "und");
    assert!(
        broken.hyphens.is_empty() && !broken.broken.is_empty(),
        "{broken:?}"
    );
    assert!(!broken.overflowed);
}

#[test]
fn vertical_point_text_sets_columns_leftwards_from_its_origin() {
    let (_dir, session, page, change) = page(
        400,
        400,
        vec![json!({
            "op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text",
                "content": "縦書き
        二行目",
                "style": {"chars": {"size": 40.0}},
                "direction": "vertical",
                "language": "ja",
                "transform": [[1.0, 0.0, 300.0], [0.0, 1.0, 50.0]],
            },
        })],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page).unwrap();
    let bounds = frame
        .layer(layer(&change, "text"))
        .unwrap()
        .bounds()
        .unwrap();
    // Two columns 48 px apart, the first at the origin's left.
    assert!(
        (bounds.x1 - 300.0).abs() < 1.0 && (bounds.x0 - 204.0).abs() < 1.0,
        "{bounds:?}"
    );
    assert!(
        (bounds.y0 - 50.0).abs() < 1.0 && bounds.y1 > 150.0,
        "{bounds:?}"
    );
}

#[test]
fn masks_apply_in_order_and_cropping_drops_them() {
    // A white raster limited to its left half, less its top-left quarter.
    let (_dir, session, page, change) = page(
        64,
        64,
        vec![
            json!({"op": "add_layer", "id": "$paint", "page": "$page", "index": 0, "layer": {"kind": "raster"}}),
            json!({"op": "write_pixels", "layer": "$paint", "rect": {"x": 0, "y": 0, "width": 64, "height": 64}, "pixels": vec![255u8; 64 * 64 * 4]}),
            json!({"op": "add_layer", "id": "$left", "page": "$page", "parent": "$paint", "index": 0, "layer": {
                "kind": "mask", "mode": "add", "content": {"path": {"path": "M0 0 H32 V64 H0 Z"}},
            }}),
            json!({"op": "add_layer", "id": "$corner", "page": "$page", "parent": "$paint", "index": 1, "layer": {
                "kind": "mask", "mode": "subtract", "content": {"path": {"path": "M0 0 H32 V32 H0 Z"}},
            }}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    if let Err(error) = renderer.gpu() {
        eprintln!("skipped without a GPU: {error}");
        return;
    }
    // Alpha at the bottom left, the top left, and the right.
    let alphas = |raster: Raster| {
        [(16, 48), (16, 16), (48, 48)].map(|(x, y)| raster.image.get_pixel(x, y)[3])
    };
    let frame = renderer.render(&session, page).unwrap();
    assert_eq!(alphas(renderer.rasterize(&frame).unwrap()), [255, 0, 0]);

    let paint = layer(&change, "paint");
    let cropped = renderer.rasterize(&frame.cropped(paint).unwrap()).unwrap();
    assert_eq!(
        (cropped.left, cropped.top, cropped.image.dimensions()),
        (0, 0, (64, 64))
    );
    assert_eq!(alphas(cropped), [255; 3]);

    // The mask frame draws no image, which once left the next frame's
    // images transparent in vello.
    let mask = frame.mask(paint).unwrap().unwrap();
    assert_eq!(alphas(renderer.rasterize(&mask).unwrap()), [255, 0, 0]);

    // A hidden component is left out.
    let mut session = session;
    let corner = layer(&change, "corner");
    apply(
        &mut session,
        vec![json!({"op": "update_layer", "layer": corner, "patch": {"visible": false}})],
    );
    let frame = renderer.render(&session, page).unwrap();
    assert_eq!(alphas(renderer.rasterize(&frame).unwrap()), [255, 255, 0]);
}
