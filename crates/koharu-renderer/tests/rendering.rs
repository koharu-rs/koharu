//! Rendering a scene page: the order text fits in, where text sets, what a
//! render reuses, what exports read, and how layers composite.

use std::io::Cursor;

use koharu_renderer::{Frame, Raster, Renderer, ResolvedText, TypesettingConfig};
use koharu_scene::{Author, Change, Created, Edit, LayerId, Op, PageId, Session, StyleId};
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
                "content": "縦書き\n二行目",
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
fn path_text_anchored_at_an_edge_fits_beside_it() {
    // A balloon whose top slants, with the text at its top.
    let (_dir, session, page, change) = page(
        400,
        400,
        vec![json!({
            "op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text",
                "content": "HELLO",
                "style": {"chars": {"family": "Arial", "size": 20.0}},
                "frame": {"path": {"path": "M0 10 L300 0 L300 200 L0 200 Z", "anchor": "start"}},
            },
        })],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page).unwrap();
    let text = frame.layer(layer(&change, "text")).unwrap().text().unwrap();
    assert!(!text.overflowed && text.scale == 1.0, "{text:?}");
}

fn text<'a>(frame: &'a Frame, change: &Change, name: &str) -> &'a ResolvedText {
    frame.layer(layer(change, name)).unwrap().text().unwrap()
}

#[test]
fn a_text_style_change_refits_every_text_naming_it() {
    let dialogue = StyleId::new();
    let (_dir, mut session, page, change) = page(
        400,
        400,
        vec![
            json!({"op": "add_text_style", "id": dialogue, "style": {"name": "Dialogue", "style": {"chars": {"size": 20.0}}}}),
            json!({"op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text", "content": "Hello", "text_style": dialogue,
            }}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page).unwrap();
    assert_eq!(text(&frame, &change, "text").runs[0].size, 20.0);

    // The layer itself is unchanged, so only the style's part of what the
    // renderer keeps its layout by can tell it to lay out again.
    apply(
        &mut session,
        vec![
            json!({"op": "update_text_style", "style": dialogue, "patch": {"style": {"chars": {"size": 30.0}}}}),
        ],
    );
    let frame = renderer.render(&session, page).unwrap();
    assert_eq!(text(&frame, &change, "text").runs[0].size, 30.0);
}

#[test]
fn an_edit_lays_out_its_text_again_and_reuses_the_rest() {
    let (_dir, mut session, page, change) = page(
        400,
        400,
        vec![
            json!({"op": "add_layer", "id": "$edited", "page": "$page", "index": 0, "layer": {"kind": "text", "content": "Before"}}),
            json!({"op": "add_layer", "id": "$kept", "page": "$page", "index": 1, "layer": {"kind": "text", "content": "Kept"}}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let before = renderer.render(&session, page).unwrap();
    let edited = layer(&change, "edited");
    apply(
        &mut session,
        vec![json!({"op": "set_text", "layer": edited, "text": "After the edit"})],
    );
    let after = renderer.render(&session, page).unwrap();
    let end = text(&after, &change, "edited")
        .runs
        .iter()
        .map(|run| run.range.end)
        .max();
    assert_eq!(end, Some("After the edit".len()));
    assert!(std::ptr::eq(
        text(&before, &change, "kept"),
        text(&after, &change, "kept")
    ));
}

#[test]
fn text_runs_count_utf16_units_and_split_where_spans_do() {
    // 𠮷 is one character in two UTF-16 units, as a PSD counts it.
    let (_dir, session, page, change) = page(
        400,
        400,
        vec![
            json!({"op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text", "language": "ja",
                "content": [{"insert": "𠮷野家 ", "attributes": {"size": 28.0}}, {"insert": "OPEN", "attributes": {"size": 24.0}}],
            }}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page).unwrap();
    let mut runs: Vec<_> = text(&frame, &change, "text")
        .runs
        .iter()
        .map(|run| (run.range.clone(), run.size))
        .collect();
    runs.sort_by_key(|(range, _)| range.start);
    assert_eq!(runs.first().map(|(range, _)| range.start), Some(0));
    assert!(
        runs.windows(2).all(|pair| pair[0].0.end == pair[1].0.start),
        "{runs:?}"
    );
    assert_eq!(runs.last(), Some(&(5..9, 24.0)), "{runs:?}");
}

#[test]
fn text_a_box_cannot_hold_overflows_and_still_draws() {
    let (_dir, session, page, change) = page(
        400,
        400,
        vec![
            json!({"op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text", "content": "A narration far too long for its small box",
                "style": {"chars": {"family": "Arial", "size": 20.0}},
                "frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0}, "fit": "none"}},
            }}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    let frame = renderer.render(&session, page).unwrap();
    assert!(text(&frame, &change, "text").overflowed);
    let bounds = frame
        .layer(layer(&change, "text"))
        .unwrap()
        .bounds()
        .unwrap();
    assert!(
        bounds.y1 > 60.0,
        "drawn past the box, not clipped: {bounds:?}"
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

#[test]
fn a_hidden_layer_stays_out_of_the_page_but_exports_at_its_offset() {
    let (_dir, session, page, change) = page(
        96,
        96,
        vec![
            json!({"op": "add_layer", "id": "$draft", "page": "$page", "index": 0, "layer": {
                "kind": "raster", "visible": false, "offset": {"x": 16, "y": 8},
            }}),
            json!({"op": "write_pixels", "layer": "$draft", "rect": {"x": 0, "y": 0, "width": 16, "height": 16}, "pixels": vec![255u8; 16 * 16 * 4]}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    if let Err(error) = renderer.gpu() {
        eprintln!("skipped without a GPU: {error}");
        return;
    }
    let frame = renderer.render(&session, page).unwrap();
    let draft = layer(&change, "draft");
    assert!(!frame.layer(draft).unwrap().visible());
    let page_pixels = renderer.rasterize(&frame).unwrap();
    assert_eq!(page_pixels.image.get_pixel(20, 12)[3], 0);

    let own = renderer.rasterize(&frame.cropped(draft).unwrap()).unwrap();
    assert_eq!((own.left, own.top), (16, 8));
    assert_eq!(own.image.get_pixel(4, 4)[3], 255);
    assert_eq!(own.image.get_pixel(20, 20)[3], 0);
}

#[test]
fn an_image_draws_through_its_transform() {
    // A 2×2 red image, scaled 16 times and moved to (8, 8).
    let mut png = Vec::new();
    image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]))
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let file = koharu_scene::Hash::of(&png);
    let (_dir, session, page, change) = page(
        64,
        64,
        vec![
            json!({"op": "add_file", "media_type": "image/png", "bytes": png}),
            json!({"op": "add_layer", "id": "$photo", "page": "$page", "index": 0, "layer": {
                "kind": "image", "file": file, "transform": [[16.0, 0.0, 8.0], [0.0, 16.0, 8.0]],
            }}),
        ],
    );
    let renderer = Renderer::new(TypesettingConfig::default());
    if let Err(error) = renderer.gpu() {
        eprintln!("skipped without a GPU: {error}");
        return;
    }
    let frame = renderer.render(&session, page).unwrap();
    let bounds = frame
        .layer(layer(&change, "photo"))
        .unwrap()
        .bounds()
        .unwrap();
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (8.0, 8.0, 40.0, 40.0)
    );
    let pixels = renderer.rasterize(&frame).unwrap().image;
    assert_eq!(pixels.get_pixel(24, 24).0, [255, 0, 0, 255]);
    assert_eq!(
        (pixels.get_pixel(4, 4)[3], pixels.get_pixel(44, 44)[3]),
        (0, 0)
    );
}
