//! Rendering a scene page: the order text fits in, where text sets, what a
//! render reuses, what exports read, and how layers, backgrounds, panels, and
//! blocks composite.

use std::io::Cursor;

use koharu_renderer::{Frame, Raster, Renderer, RendererConfig, ResolvedText};
use koharu_scene::{Change, Created, Edit, Hash, LayerId, Op, PageId, Session};
use serde_json::{Value, json};

fn apply(session: &mut Session, ops: Vec<Value>) -> Change {
    let edit = Edit {
        label: "test".into(),
        ops: ops
            .into_iter()
            .map(|op| serde_json::from_value::<Op>(op).expect("a valid op"))
            .collect(),
    };
    session.apply(edit).expect("an edit")
}

fn layer(change: &Change, name: &str) -> LayerId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, label } if label.as_deref() == Some(name) => Some(*id),
            _ => None,
        })
        .expect("a labelled layer")
}

/// A project with one `width`×`height` page holding `layers`, each labelled.
fn page(
    width: u32,
    height: u32,
    layers: Vec<Value>,
) -> (tempfile::TempDir, Session, PageId, Change) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
    let mut ops = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": width, "height": height}}),
    ];
    ops.extend(layers);
    let change = apply(&mut session, ops);
    let page = change.pages.added[0];
    (dir, session, page, change)
}

fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba(color))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("a PNG");
    bytes
}

/// A project with one page made from a `color` scan, holding `layers`.
fn scanned(color: [u8; 4], layers: Vec<Value>) -> (tempfile::TempDir, Session, PageId, Change) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
    let scan = png(64, 64, color);
    let file = Hash::of(&scan);
    let mut ops = vec![
        json!({"op": "add_file", "bytes": scan}),
        json!({"op": "add_page", "id": "$page", "index": 0, "background": file}),
    ];
    ops.extend(layers);
    let change = apply(&mut session, ops);
    let page = change.pages.added[0];
    (dir, session, page, change)
}

fn gpu(renderer: &Renderer) -> bool {
    match renderer.gpu() {
        Ok(_) => true,
        Err(error) => {
            eprintln!("skipped without a GPU: {error}");
            false
        }
    }
}

fn text<'a>(frame: &'a Frame, change: &Change, name: &str) -> &'a ResolvedText {
    frame
        .layer(layer(change, name))
        .and_then(|layer| layer.text())
        .expect("a laid-out text")
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
    let renderer = Renderer::new(RendererConfig::default());
    let frame = renderer.render(&session, page).expect("a frame");
    text(&frame, &change, "text").clone()
}

#[test]
fn a_long_word_hyphenates_before_it_shrinks_and_breaks_last() {
    // With room, the word sets at its size.
    assert_eq!(word_in_box(4000.0, "en").scale, 1.0);

    // A box a little narrower than the word: it hyphenates by its language
    // and keeps its size.
    let hyphenated = word_in_box(300.0, "en");
    assert!(
        hyphenated.scale == 1.0 && !hyphenated.hyphens.is_empty(),
        "{hyphenated:?}"
    );
    assert!(hyphenated.broken.is_empty() && !hyphenated.overflowed);

    // Without a language there are no hyphenation points, so it shrinks and
    // stays whole.
    let shrunk = word_in_box(300.0, "und");
    assert!(shrunk.scale < 1.0 && shrunk.scale >= 0.5, "{shrunk:?}");
    assert!(shrunk.hyphens.is_empty() && shrunk.broken.is_empty() && !shrunk.overflowed);

    // Too narrow to set its parts whole: it still hyphenates, shrinking.
    let hyphenated = word_in_box(120.0, "en");
    assert!(!hyphenated.hyphens.is_empty(), "{hyphenated:?}");
    assert!(hyphenated.broken.is_empty() && !hyphenated.overflowed);

    // And without a language, too narrow even at half size: it breaks
    // between letters.
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
    let renderer = Renderer::new(RendererConfig::default());
    let frame = renderer.render(&session, page).expect("a frame");
    let bounds = frame
        .layer(layer(&change, "text"))
        .and_then(|layer| layer.bounds())
        .expect("bounds");
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
    let renderer = Renderer::new(RendererConfig::default());
    let frame = renderer.render(&session, page).expect("a frame");
    let text = text(&frame, &change, "text");
    assert!(!text.overflowed && text.scale == 1.0, "{text:?}");
}

#[test]
fn a_balloon_merely_full_shrinks_without_hyphens() {
    // No word is wider than its line, so a hyphen in "al-ready" would only
    // save shrinking by a little.
    let (_dir, session, page, change) = page(
        800,
        400,
        vec![json!({
            "op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text",
                "content": "I can't believe it's already over.",
                "language": "en",
                "style": {"chars": {"family": "Arial", "size": 34.0}, "align": "center"},
                "frame": {"path": {"path": "M380 200 A180 115 0 1 0 740 200 A180 115 0 1 0 380 200 Z", "anchor": "center"}},
            },
        })],
    );
    let renderer = Renderer::new(RendererConfig::default());
    let frame = renderer.render(&session, page).expect("a frame");
    let text = text(&frame, &change, "text");
    assert!(
        text.scale < 1.0 && text.hyphens.is_empty() && !text.overflowed,
        "{text:?}"
    );
}

#[test]
fn a_text_lays_out_again_only_when_its_layout_changes() {
    let (_dir, mut session, page, change) = page(
        400,
        400,
        vec![
            json!({"op": "add_layer", "id": "$edited", "page": "$page", "index": 0, "layer": {"kind": "text", "content": "Before"}}),
            json!({"op": "add_layer", "id": "$kept", "page": "$page", "index": 1, "layer": {"kind": "text", "content": "Kept"}}),
        ],
    );
    let renderer = Renderer::new(RendererConfig::default());
    let before = renderer.render(&session, page).expect("a frame");
    let edited = layer(&change, "edited");
    apply(
        &mut session,
        vec![json!({"op": "set_text", "layer": edited, "text": "After the edit"})],
    );
    let after = renderer.render(&session, page).expect("a frame");
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

    // Moving a text draws the same layout elsewhere.
    let kept = layer(&change, "kept");
    apply(
        &mut session,
        vec![
            json!({"op": "update_layer", "layer": kept, "patch": {"transform": [[1.0, 0.0, 50.0], [0.0, 1.0, 20.0]]}}),
        ],
    );
    let moved = renderer.render(&session, page).expect("a frame");
    let x0 = |frame: &Frame| {
        frame
            .layer(layer(&change, "kept"))
            .and_then(|layer| layer.bounds())
            .expect("bounds")
            .x0
    };
    assert_eq!(x0(&moved) - x0(&after), 50.0);
    assert!(std::ptr::eq(
        text(&after, &change, "kept"),
        text(&moved, &change, "kept")
    ));
}

#[test]
fn text_runs_count_utf16_units_and_split_where_spans_do() {
    // 𠮷 is one character in two UTF-16 units, as a PSD counts it.
    let (_dir, session, page, change) = page(
        400,
        400,
        vec![json!({
            "op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text", "language": "ja",
                "content": [{"insert": "𠮷野家 ", "attributes": {"size": 28.0}}, {"insert": "OPEN", "attributes": {"size": 24.0}}],
            },
        })],
    );
    let renderer = Renderer::new(RendererConfig::default());
    let frame = renderer.render(&session, page).expect("a frame");
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
        vec![json!({
            "op": "add_layer", "id": "$text", "page": "$page", "index": 0, "layer": {
                "kind": "text", "content": "A narration far too long for its small box",
                "style": {"chars": {"family": "Arial", "size": 20.0}},
                "frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0}, "fit": "none"}},
            },
        })],
    );
    let renderer = Renderer::new(RendererConfig::default());
    let frame = renderer.render(&session, page).expect("a frame");
    assert!(text(&frame, &change, "text").overflowed);
    let bounds = frame
        .layer(layer(&change, "text"))
        .and_then(|layer| layer.bounds())
        .expect("bounds");
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
            json!({"op": "add_layer", "id": "$left", "page": "$page", "parent": "$paint", "index": 0, "layer": {"kind": "mask", "mode": "add"}}),
            json!({"op": "write_pixels", "layer": "$left", "rect": {"x": 0, "y": 0, "width": 32, "height": 64}, "pixels": vec![255u8; 32 * 64]}),
            json!({"op": "add_layer", "id": "$corner", "page": "$page", "parent": "$paint", "index": 1, "layer": {"kind": "mask", "mode": "subtract"}}),
            json!({"op": "write_pixels", "layer": "$corner", "rect": {"x": 0, "y": 0, "width": 32, "height": 32}, "pixels": vec![255u8; 32 * 32]}),
        ],
    );
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    // Alpha at the bottom left, the top left, and the right, none outside
    // the raster.
    let alphas = |raster: Raster| {
        [(16, 48), (16, 16), (48, 48)].map(|(x, y): (i32, i32)| {
            let at = u32::try_from(x - raster.offset.x)
                .ok()
                .zip(u32::try_from(y - raster.offset.y).ok());
            at.and_then(|(x, y)| raster.image.get_pixel_checked(x, y))
                .map_or(0, |pixel| pixel[3])
        })
    };
    let frame = renderer.render(&session, page).expect("a frame");
    assert_eq!(
        alphas(renderer.rasterize(&frame).expect("pixels")),
        [255, 0, 0]
    );

    let paint = frame.layer(layer(&change, "paint")).expect("drawn");
    let cropped = renderer.rasterize(&paint.cropped()).expect("pixels");
    assert_eq!(
        (
            cropped.offset.x,
            cropped.offset.y,
            cropped.image.dimensions()
        ),
        (0, 0, (64, 64))
    );
    assert_eq!(alphas(cropped), [255; 3]);

    // The mask frame covers only what its adding mask does, and draws no
    // image, which once left the next frame's images transparent in vello.
    let mask = renderer
        .rasterize(&paint.mask().expect("masks"))
        .expect("pixels");
    assert_eq!(
        (mask.offset.x, mask.offset.y, mask.image.dimensions()),
        (0, 0, (32, 64))
    );
    assert_eq!(alphas(mask), [255, 0, 0]);

    // A hidden component is left out.
    let mut session = session;
    let corner = layer(&change, "corner");
    apply(
        &mut session,
        vec![json!({"op": "update_layer", "layer": corner, "patch": {"visible": false}})],
    );
    let frame = renderer.render(&session, page).expect("a frame");
    assert_eq!(
        alphas(renderer.rasterize(&frame).expect("pixels")),
        [255, 255, 0]
    );
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
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    let frame = renderer.render(&session, page).expect("a frame");
    let draft = frame.layer(layer(&change, "draft")).expect("drawn");
    assert!(!draft.visible());
    let page_pixels = renderer.rasterize(&frame).expect("pixels");
    assert_eq!(page_pixels.image.get_pixel(20, 12)[3], 0);

    // Its own pixels are what it painted, not the 64×64 tile holding them.
    let own = renderer.rasterize(&draft.cropped()).expect("pixels");
    assert_eq!(
        (own.offset.x, own.offset.y, own.image.dimensions()),
        (16, 8, (16, 16))
    );
    assert!(own.image.pixels().all(|pixel| pixel[3] == 255));
}

#[test]
fn an_image_draws_through_its_transform() {
    // A 2×2 red image, scaled 16 times and moved to (8, 8).
    let red = png(2, 2, [255, 0, 0, 255]);
    let file = Hash::of(&red);
    let (_dir, session, page, change) = page(
        64,
        64,
        vec![
            json!({"op": "add_file", "bytes": red}),
            json!({"op": "add_layer", "id": "$photo", "page": "$page", "index": 0, "layer": {
                "kind": "image", "file": file, "transform": [[16.0, 0.0, 8.0], [0.0, 16.0, 8.0]],
            }}),
        ],
    );
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    let frame = renderer.render(&session, page).expect("a frame");
    let bounds = frame
        .layer(layer(&change, "photo"))
        .and_then(|layer| layer.bounds())
        .expect("bounds");
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (8.0, 8.0, 40.0, 40.0)
    );
    let pixels = renderer.rasterize(&frame).expect("pixels").image;
    assert_eq!(pixels.get_pixel(24, 24).0, [255, 0, 0, 255]);
    assert_eq!(
        (pixels.get_pixel(4, 4)[3], pixels.get_pixel(44, 44)[3]),
        (0, 0)
    );
}

#[test]
fn the_background_draws_under_every_layer() {
    let red = [255u8, 0, 0, 255].repeat(32 * 64);
    let (_dir, session, page, _) = scanned(
        [0, 0, 255, 255],
        vec![
            json!({"op": "add_layer", "id": "$paint", "page": "$page", "index": 0, "layer": {"kind": "raster"}}),
            json!({"op": "write_pixels", "layer": "$paint", "rect": {"x": 0, "y": 0, "width": 32, "height": 64}, "pixels": red}),
        ],
    );
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    let frame = renderer.render(&session, page).expect("a frame");
    assert!(frame.background().expect("drawn").visible());
    let pixels = renderer.rasterize(&frame).expect("pixels").image;
    assert_eq!(
        pixels.get_pixel(16, 32).0,
        [255, 0, 0, 255],
        "the raster is on top"
    );
    assert_eq!(
        pixels.get_pixel(48, 32).0,
        [0, 0, 255, 255],
        "the scan shows where nothing covers it"
    );
}

/// A black scan holding one block whose clean-up is white everywhere and
/// whose mask covers the left half.
fn cleaned() -> (tempfile::TempDir, Session, PageId, Change) {
    scanned(
        [0, 0, 0, 255],
        vec![
            json!({"op": "add_layer", "id": "$block", "page": "$page", "index": 0, "layer": {
                "kind": "block", "source": {"frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": 64.0, "height": 64.0}}}},
            }}),
            json!({"op": "write_pixels", "layer": "$block.cleanup", "rect": {"x": 0, "y": 0, "width": 64, "height": 64}, "pixels": vec![255u8; 64 * 64 * 4]}),
            json!({"op": "write_pixels", "layer": "$block.mask", "rect": {"x": 0, "y": 0, "width": 32, "height": 64}, "pixels": vec![255u8; 32 * 64]}),
        ],
    )
}

#[test]
fn a_block_shows_its_cleanup_only_inside_its_mask_under_its_translation() {
    let (_dir, mut session, page, change) = cleaned();
    let block = layer(&change, "block");
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    let pixels = renderer
        .rasterize(&renderer.render(&session, page).expect("a frame"))
        .expect("pixels")
        .image;
    assert_eq!(
        pixels.get_pixel(16, 32).0,
        [255; 4],
        "clean inside the mask"
    );
    assert_eq!(
        pixels.get_pixel(48, 32).0,
        [0, 0, 0, 255],
        "the scan outside it"
    );

    apply(
        &mut session,
        vec![
            json!({"op": "set_text", "layer": block, "text": "WWW"}),
            json!({"op": "update_layer", "layer": block, "patch": {"translation": {
                "style": {"chars": {"family": "Arial", "size": 16.0, "fill": [1.0, 0.0, 0.0, 1.0]}},
                "transform": [[1.0, 0.0, 2.0], [0.0, 1.0, 20.0]],
            }}}),
        ],
    );
    let frame = renderer.render(&session, page).expect("a frame");
    let bounds = frame
        .translation(block)
        .and_then(|layer| layer.bounds())
        .expect("a translation");
    assert!(frame.layer(layer(&change, "block.cleanup")).is_some());
    let pixels = renderer.rasterize(&frame).expect("pixels").image;
    let inked = (bounds.x0.max(0.0) as u32..(bounds.x1.min(32.0) as u32))
        .flat_map(|x| (bounds.y0.max(0.0) as u32..bounds.y1.min(64.0) as u32).map(move |y| (x, y)))
        .any(|(x, y)| {
            let [r, g, b, _] = pixels.get_pixel(x, y).0;
            r > 128 && g < 128 && b < 128
        });
    assert!(
        inked,
        "the translation draws over the clean-up, in {bounds:?}"
    );
}

#[test]
fn hiding_a_block_shows_the_background_again() {
    let (_dir, mut session, page, change) = cleaned();
    let block = layer(&change, "block");
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    apply(
        &mut session,
        vec![json!({"op": "update_layer", "layer": block, "patch": {"visible": false}})],
    );
    let pixels = renderer
        .rasterize(&renderer.render(&session, page).expect("a frame"))
        .expect("pixels")
        .image;
    assert!(
        pixels.pixels().all(|pixel| pixel.0 == [0, 0, 0, 255]),
        "only the scan shows"
    );
}

#[test]
fn a_panel_clips_its_children_and_a_layer_on_the_page_crosses_it() {
    let (_dir, mut session, page, _) = page(
        64,
        64,
        vec![
            json!({"op": "add_layer", "id": "$panel", "page": "$page", "index": 0, "layer": {"kind": "panel", "outline": "M0 0 H32 V64 H0 Z"}}),
            json!({"op": "add_layer", "id": "$inside", "page": "$page", "parent": "$panel", "index": 0, "layer": {"kind": "raster"}}),
            json!({"op": "write_pixels", "layer": "$inside", "rect": {"x": 0, "y": 0, "width": 64, "height": 64}, "pixels": vec![255u8; 64 * 64 * 4]}),
        ],
    );
    let renderer = Renderer::new(RendererConfig::default());
    if !gpu(&renderer) {
        return;
    }
    let pixels = renderer
        .rasterize(&renderer.render(&session, page).expect("a frame"))
        .expect("pixels")
        .image;
    assert_eq!(
        (pixels.get_pixel(16, 32)[3], pixels.get_pixel(48, 32)[3]),
        (255, 0)
    );

    apply(
        &mut session,
        vec![
            json!({"op": "add_layer", "id": "$crossing", "page": page, "index": 1, "layer": {"kind": "raster"}}),
            json!({"op": "write_pixels", "layer": "$crossing", "rect": {"x": 24, "y": 0, "width": 32, "height": 8}, "pixels": vec![255u8; 32 * 8 * 4]}),
        ],
    );
    let pixels = renderer
        .rasterize(&renderer.render(&session, page).expect("a frame"))
        .expect("pixels")
        .image;
    assert_eq!(
        pixels.get_pixel(48, 4)[3],
        255,
        "a layer on the page draws across the panel's edge"
    );
}
