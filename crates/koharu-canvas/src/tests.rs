//! The canvas's behaviour, through its public API, on a real project and
//! GPU.

use std::{io::Cursor, sync::Arc};

use image::{ImageFormat, Rgb, RgbImage};
use koharu_renderer::{Gpu, Renderer};
use koharu_scene::{Background, Edit, Hash, LayerId, Op, PageId, Ref, Rgba, Session};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _, TranslateScale, Vec2};
use parking_lot::Mutex;
use serde_json::{Value, json};
use ui_events::{
    ScrollDelta,
    keyboard::{Code, Key, KeyState, KeyboardEvent, Modifiers, NamedKey},
    pointer::{
        PointerButton, PointerButtonEvent, PointerEvent, PointerGesture, PointerGestureEvent,
        PointerId, PointerInfo, PointerScrollEvent, PointerState, PointerType, PointerUpdate,
    },
};

use crate::{Brush, Canvas, CharKey, Colors, Effect, Notice, RubyEdit, Tool};

/// One renderer for every test, as the app has one, setting text in
/// installed fonts so no test downloads one.
static RENDERER: std::sync::LazyLock<Renderer> = std::sync::LazyLock::new(Renderer::new);

/// A project of one page, its renderer, and a canvas showing it.
struct Fixture {
    _dir: tempfile::TempDir,
    session: Arc<Mutex<Session>>,
    renderer: Renderer,
    canvas: Canvas,
    page: PageId,
}

impl Fixture {
    /// The page's frame again, after an edit, shown on the canvas.
    fn render(&mut self) {
        let session = self.session.lock();
        let frame = self.renderer.render(&session, self.page).expect("a frame");
        let snapshot = crate::Snapshot::new(frame, &session).expect("the page");
        self.canvas.show(snapshot);
    }
}

/// A project of one page made by `ops` after `add_page` (labelled
/// `$page`), shown on a canvas; `None` without a GPU.
fn fixture(size: (u32, u32), ops: Vec<Value>) -> Option<Fixture> {
    let dir = tempfile::tempdir().expect("a folder");
    let mut session = Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())
        .expect("a project");
    let page = json!({"op": "add_page", "id": "$page", "index": 0,
        "size": {"width": size.0, "height": size.1}});
    let ops: Vec<Op> = std::iter::once(page)
        .chain(ops)
        .map(|op| serde_json::from_value(op).expect("an op"))
        .collect();
    let change = session
        .apply(Edit {
            label: "test".into(),
            ops,
        })
        .expect("an edit");
    let page = change.pages.added[0];
    let renderer = RENDERER.clone();
    if Gpu::get().is_err() {
        eprintln!("skipped without a GPU");
        return None;
    }
    let session = Arc::new(Mutex::new(session));
    let mut fixture = Fixture {
        _dir: dir,
        session: Arc::clone(&session),
        renderer,
        canvas: Canvas::new(session, RENDERER.clone()),
        page,
    };
    fixture.render();
    Some(fixture)
}

fn red_square() -> Vec<Value> {
    vec![
        json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
            "kind": "shape", "path": "M50 50 H100 V100 H50 Z", "fill": [1.0, 0.0, 0.0, 1.0],
        }}),
    ]
}

#[test]
fn an_empty_viewport_draws_nothing() {
    let Some(mut fixture) = fixture((200, 200), red_square()) else {
        return;
    };
    fixture.canvas.set_viewport(Rect::ZERO, 1.0);
    assert!(fixture.canvas.draw().expect("a draw").is_none());
}

#[test]
fn a_page_shows_where_the_camera_puts_it() {
    let Some(mut fixture) = fixture((200, 200), red_square()) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    canvas.set_viewport(Rect::new(0.0, 0.0, 400.0, 300.0), 1.0);
    canvas.set_colors(Colors {
        workspace: Rgba([0.0, 0.0, 1.0, 1.0]),
        ..Colors::default()
    });
    canvas.set_camera(TranslateScale::default());
    assert!(canvas.draw().expect("a draw").is_some());
    let snapshot = canvas.snapshot().expect("pixels");
    let at = |x, y| snapshot.get_pixel(x, y).0;
    assert_eq!(at(75, 75), [255, 0, 0, 255]);
    assert_eq!(at(25, 25), [255; 4]);
    assert_eq!(at(350, 250), [0, 0, 255, 255]);
}

/// Showing the original draws the scan alone, and leaves the page as it was.
#[test]
fn the_original_shows_the_scan_without_the_layers() {
    let Some(mut fixture) = unscaled_of((200, 200), red_square()) else {
        return;
    };
    let mut scan = Vec::new();
    RgbImage::from_pixel(200, 200, Rgb([0, 0, 255]))
        .write_to(&mut Cursor::new(&mut scan), ImageFormat::Png)
        .expect("a PNG");
    let background = Background {
        file: Hash::of(&scan),
        visible: true,
    };
    let ops = vec![
        Op::AddFile { bytes: scan },
        Op::SetBackground {
            page: Ref::from(fixture.page),
            background: Some(background),
        },
    ];
    fixture
        .session
        .lock()
        .apply(Edit {
            label: "test".into(),
            ops,
        })
        .expect("a scan");
    fixture.render();
    for (original, square) in [
        (false, [255, 0, 0, 255]),
        (true, [0, 0, 255, 255]),
        (false, [255, 0, 0, 255]),
    ] {
        fixture.canvas.set_original(original);
        assert!(fixture.canvas.draw().expect("a draw").is_some());
        let snapshot = fixture.canvas.snapshot().expect("pixels");
        assert_eq!(snapshot.get_pixel(75, 75).0, square);
        assert_eq!(snapshot.get_pixel(25, 25).0, [0, 0, 255, 255]);
    }
}

/// A shape of `width`×`height` at `(x, y)`, drawn in order at `index`,
/// with its layer values merged in.
fn shape(index: usize, rect: (f64, f64, f64, f64), extra: Value) -> Value {
    let (x, y, width, height) = rect;
    let mut layer = json!({
        "kind": "shape", "path": format!("M{x} {y} h{width} v{height} h-{width} Z"),
        "fill": [1.0, 0.0, 0.0, 1.0],
    });
    if let (Some(layer), Some(extra)) = (layer.as_object_mut(), extra.as_object()) {
        layer.extend(extra.clone());
    }
    json!({"op": "add_layer", "page": "$page", "index": index, "layer": layer})
}

/// The page's top-level layer ids, bottom first.
fn layers(fixture: &Fixture) -> Vec<LayerId> {
    let project = fixture.session.lock().project();
    let page = project.page(fixture.page).expect("the page");
    page.layers.iter().map(|layer| layer.id).collect()
}

/// A mouse, `pointer` 0 having no id.
fn mouse(pointer: u64) -> PointerInfo {
    PointerInfo {
        pointer_id: PointerId::new(pointer),
        persistent_device_id: None,
        pointer_type: PointerType::Mouse,
    }
}

/// The pointer at `at` in window pixels.
fn state(at: (f64, f64), modifiers: Modifiers) -> PointerState {
    PointerState {
        position: at.into(),
        modifiers,
        ..PointerState::default()
    }
}

/// `pointer`'s primary button pressed at `at`.
fn press(pointer: u64, at: (f64, f64), modifiers: Modifiers) -> PointerEvent {
    PointerEvent::Down(PointerButtonEvent {
        button: Some(PointerButton::Primary),
        pointer: mouse(pointer),
        state: state(at, modifiers),
    })
}

fn down(at: (f64, f64), modifiers: Modifiers) -> PointerEvent {
    press(1, at, modifiers)
}

fn drag(pointer: u64, to: (f64, f64)) -> PointerEvent {
    PointerEvent::Move(PointerUpdate {
        pointer: mouse(pointer),
        current: state(to, Modifiers::default()),
        coalesced: Vec::new(),
        predicted: Vec::new(),
    })
}

fn up(pointer: u64, at: (f64, f64)) -> PointerEvent {
    PointerEvent::Up(PointerButtonEvent {
        button: Some(PointerButton::Primary),
        pointer: mouse(pointer),
        state: state(at, Modifiers::default()),
    })
}

/// The selections among `effects`.
fn selections(effects: &[Effect]) -> Vec<Vec<LayerId>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Selection(layers) => Some(layers.clone()),
            _ => None,
        })
        .collect()
}

/// A 400×400 page shown at its own size in a 400×400 viewport, so
/// viewport and page points match.
fn unscaled(ops: Vec<Value>) -> Option<Fixture> {
    let mut fixture = fixture((400, 400), ops)?;
    fixture
        .canvas
        .set_viewport(Rect::new(0.0, 0.0, 400.0, 400.0), 1.0);
    fixture.canvas.set_camera(TranslateScale::default());
    Some(fixture)
}

const SHIFT: Modifiers = Modifiers::SHIFT;

const CONTROL: Modifiers = Modifiers::CONTROL;

#[test]
fn a_marquee_selects_what_lies_wholly_inside_it() {
    let Some(mut fixture) = unscaled(vec![
        shape(0, (50.0, 50.0, 40.0, 40.0), json!({})),
        shape(1, (150.0, 50.0, 40.0, 40.0), json!({})),
    ]) else {
        return;
    };
    let [inside, crossing] = layers(&fixture)[..] else {
        panic!("two layers");
    };
    let canvas = &mut fixture.canvas;
    let mut effects = canvas
        .input(down((20.0, 20.0), Modifiers::default()))
        .expect("input");
    effects.extend(canvas.input(drag(1, (170.0, 120.0))).expect("input"));
    effects.extend(canvas.input(up(1, (170.0, 120.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![inside]]);

    canvas.select(vec![crossing]);
    let mut effects = canvas.input(down((20.0, 20.0), SHIFT)).expect("input");
    effects.extend(canvas.input(drag(1, (120.0, 120.0))).expect("input"));
    effects.extend(canvas.input(up(1, (120.0, 120.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![crossing, inside]]);
}

#[test]
fn a_click_selects_the_topmost_and_a_modifier_toggles() {
    let Some(mut fixture) = unscaled(vec![
        shape(0, (50.0, 50.0, 100.0, 100.0), json!({})),
        shape(1, (100.0, 100.0, 100.0, 100.0), json!({})),
        shape(2, (100.0, 100.0, 100.0, 100.0), json!({"visible": false})),
        shape(3, (100.0, 100.0, 100.0, 100.0), json!({"locked": true})),
    ]) else {
        return;
    };
    let top = layers(&fixture)[1];
    let canvas = &mut fixture.canvas;
    let mut click = |modifiers| {
        let mut effects = canvas
            .input(down((120.0, 120.0), modifiers))
            .expect("input");
        effects.extend(canvas.input(up(1, (120.0, 120.0))).expect("input"));
        selections(&effects)
    };
    assert_eq!(click(Modifiers::default()), [vec![top]]);
    assert_eq!(click(CONTROL), [Vec::<LayerId>::new()]);
    assert_eq!(click(CONTROL), [vec![top]]);
}

#[test]
fn a_second_pointer_is_ignored_during_a_gesture() {
    let Some(mut fixture) = unscaled(vec![
        shape(0, (50.0, 50.0, 40.0, 40.0), json!({})),
        shape(1, (250.0, 250.0, 40.0, 40.0), json!({})),
    ]) else {
        return;
    };
    let first = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    // A mouse with no id is pointer 0.
    let mut effects = canvas
        .input(press(0, (20.0, 20.0), Modifiers::default()))
        .expect("input");
    effects.extend(
        canvas
            .input(press(2, (270.0, 270.0), Modifiers::default()))
            .expect("input"),
    );
    effects.extend(canvas.input(drag(0, (120.0, 120.0))).expect("input"));
    effects.extend(canvas.input(up(2, (270.0, 270.0))).expect("input"));
    effects.extend(canvas.input(up(0, (120.0, 120.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![first]]);
}

/// A page of `size` shown at its own size in a viewport of that size.
fn unscaled_of(size: (u32, u32), ops: Vec<Value>) -> Option<Fixture> {
    let mut fixture = fixture(size, ops)?;
    let (width, height) = (f64::from(size.0), f64::from(size.1));
    fixture
        .canvas
        .set_viewport(Rect::new(0.0, 0.0, width, height), 1.0);
    fixture.canvas.set_camera(TranslateScale::default());
    Some(fixture)
}

/// Applies the edits among `effects` and shows the page again; the
/// labels of the edits applied.
fn apply(fixture: &mut Fixture, effects: Vec<Effect>) -> Vec<String> {
    let mut labels = Vec::new();
    for effect in effects {
        if let Effect::Edit(edit) = effect {
            labels.push(edit.label.clone());
            fixture
                .session
                .lock()
                .apply(edit)
                .expect("the edit applies");
            fixture.render();
        }
    }
    labels
}

/// A press at `from`, a drag to `to`, and the release there.
fn gesture(canvas: &mut Canvas, from: (f64, f64), to: (f64, f64)) -> Vec<Effect> {
    let mut effects = canvas
        .input(down(from, Modifiers::default()))
        .expect("input");
    effects.extend(canvas.input(drag(1, to)).expect("input"));
    effects.extend(canvas.input(up(1, to)).expect("input"));
    effects
}

/// The page's layer `id` as the session holds it, as JSON.
fn layer(fixture: &Fixture, id: LayerId) -> Value {
    fn find(layers: &[koharu_scene::Layer], id: LayerId) -> Option<&koharu_scene::Layer> {
        layers.iter().find_map(|layer| {
            (layer.id == id)
                .then_some(layer)
                .or_else(|| find(&layer.children, id))
        })
    }
    let project = fixture.session.lock().project();
    let page = project.page(fixture.page).expect("the page");
    serde_json::to_value(find(&page.layers, id).expect("the layer")).expect("JSON")
}

fn affine(value: &Value) -> Affine {
    let transform: koharu_scene::Transform =
        serde_json::from_value(value.clone()).expect("a transform");
    Affine::from(transform)
}

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-6
}

/// A 10×10 PNG and its hash.
fn png() -> (Vec<u8>, String) {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("a PNG");
    let hash = koharu_scene::Hash::of(&bytes).to_string();
    (bytes, hash)
}

#[test]
fn a_move_moves_each_kind_by_its_own_values() {
    let (bytes, hash) = png();
    let Some(mut fixture) = unscaled_of(
        (800, 800),
        vec![
            json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
                "kind": "text", "content": "Hello",
                "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 100, "height": 30}}},
                "transform": [[1.0, 0.0, 400.0], [0.0, 1.0, 50.0]],
            }}),
            json!({"op": "add_layer", "page": "$page", "index": 1, "layer": {
                "kind": "block", "source": {"content": "やあ"}, "translation": {
                    "content": "Hi",
                    "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 100, "height": 30}}},
                    "transform": [[1.0, 0.0, 400.0], [0.0, 1.0, 150.0]],
                },
            }}),
            json!({"op": "add_file", "bytes": bytes}),
            json!({"op": "add_layer", "page": "$page", "index": 2, "layer": {
                "kind": "image", "file": hash, "transform": [[1.0, 0.0, 600.0], [0.0, 1.0, 50.0]],
            }}),
            shape(3, (50.0, 300.0, 40.0, 40.0), json!({})),
            json!({"op": "add_layer", "id": "$raster", "page": "$page", "index": 4, "layer": {
                "kind": "raster", "offset": {"x": 3, "y": 4},
            }}),
            json!({"op": "write_pixels", "layer": "$raster",
                "rect": {"x": 0, "y": 0, "width": 20, "height": 20}, "pixels": vec![255u8; 20 * 20 * 4]}),
        ],
    ) else {
        return;
    };
    let [text, block, image, shape, raster] = layers(&fixture)[..] else {
        panic!("five layers");
    };
    let by = Vec2::new(25.4, -10.0);
    for (at, layer) in [
        ((450.0, 65.0), text),
        ((450.0, 165.0), block),
        ((605.0, 55.0), image),
        ((70.0, 320.0), shape),
        ((13.0, 14.0), raster),
    ] {
        let to = (at.0 + by.x, at.1 + by.y);
        let effects = gesture(&mut fixture.canvas, at, to);
        assert_eq!(apply(&mut fixture, effects), ["move"], "{layer}");
    }
    let moved = |value: &Value, from: (f64, f64)| {
        close(
            affine(value) * Point::ZERO,
            Point::new(from.0 + by.x, from.1 + by.y),
        )
    };
    assert!(moved(&layer(&fixture, text)["transform"], (400.0, 50.0)));
    assert!(moved(
        &layer(&fixture, block)["translation"]["transform"],
        (400.0, 150.0)
    ));
    assert!(moved(&layer(&fixture, image)["transform"], (600.0, 50.0)));
    let path = layer(&fixture, shape)["path"]
        .as_str()
        .expect("a path")
        .to_owned();
    let bounds = BezPath::from_svg(&path).expect("a path").bounding_box();
    assert!(close(
        bounds.origin(),
        Point::new(50.0 + by.x, 300.0 + by.y)
    ));
    assert_eq!(layer(&fixture, raster)["offset"], json!({"x": 28, "y": -6}));
}

#[test]
fn box_text_resizes_by_its_edges_and_rotates_about_its_centre() {
    let Some(mut fixture) = unscaled(vec![json!({"op": "add_layer", "page": "$page", "index": 0,
    "layer": {
        "kind": "text", "content": "Hello",
        "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 100, "height": 40}}},
        "transform": [[1.0, 0.0, 50.0], [0.0, 1.0, 50.0]],
    }})]) else {
        return;
    };
    let text = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    let mut effects = canvas
        .input(down((100.0, 70.0), Modifiers::default()))
        .expect("input");
    effects.extend(canvas.input(up(1, (100.0, 70.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![text]]);

    let effects = gesture(&mut fixture.canvas, (150.0, 70.0), (170.0, 70.0));
    assert_eq!(apply(&mut fixture, effects), ["resize"]);
    assert_eq!(
        layer(&fixture, text)["frame"]["box"]["bounds"],
        json!({"x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0})
    );

    // The rotate handle stands 16 pixels above the top's middle, at (110, 34).
    let effects = gesture(&mut fixture.canvas, (110.0, 34.0), (146.0, 70.0));
    assert_eq!(apply(&mut fixture, effects), ["rotate"]);
    let turned = affine(&layer(&fixture, text)["transform"]);
    assert!(close(
        turned * Point::new(60.0, 20.0),
        Point::new(110.0, 70.0)
    ));
    let across = turned * Point::new(61.0, 20.0) - turned * Point::new(60.0, 20.0);
    assert!(close(across.to_point(), Point::new(0.0, 1.0)));
}

/// A corner of a text's box resizes it both ways, as a corner of Photoshop's
/// paragraph type box does, while a side is still caught along its length.
#[test]
fn box_text_resizes_both_ways_from_a_corner() {
    let Some(mut fixture) = unscaled(vec![json!({"op": "add_layer", "page": "$page", "index": 0,
    "layer": {
        "kind": "text", "content": "Hello",
        "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 100, "height": 40}}},
        "transform": [[1.0, 0.0, 50.0], [0.0, 1.0, 50.0]],
    }})]) else {
        return;
    };
    let text = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    let mut effects = canvas
        .input(down((100.0, 70.0), Modifiers::default()))
        .expect("input");
    effects.extend(canvas.input(up(1, (100.0, 70.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![text]]);

    let effects = gesture(&mut fixture.canvas, (150.0, 90.0), (170.0, 110.0));
    assert_eq!(apply(&mut fixture, effects), ["resize"]);
    assert_eq!(
        layer(&fixture, text)["frame"]["box"]["bounds"],
        json!({"x": 0.0, "y": 0.0, "width": 120.0, "height": 60.0})
    );
}

#[test]
fn a_locked_layer_does_not_move() {
    let Some(mut fixture) = unscaled(vec![shape(
        0,
        (50.0, 50.0, 40.0, 40.0),
        json!({"locked": true}),
    )]) else {
        return;
    };
    let effects = gesture(&mut fixture.canvas, (70.0, 70.0), (120.0, 120.0));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Edit(_)))
    );
}

#[test]
fn a_change_under_a_gesture_cancels_it() {
    let Some(mut fixture) = unscaled(vec![shape(0, (50.0, 50.0, 40.0, 40.0), json!({}))]) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    canvas
        .input(down((70.0, 70.0), Modifiers::default()))
        .expect("input");
    canvas.input(drag(1, (120.0, 120.0))).expect("input");
    // Another writer edits the project, and the host shows it.
    let other =
        serde_json::from_value(shape(1, (200.0, 200.0, 10.0, 10.0), json!({}))).expect("an op");
    let mut op = other;
    if let Op::AddLayer { page, .. } = &mut op {
        *page = fixture.page.into();
    }
    fixture
        .session
        .lock()
        .apply(Edit {
            label: "other".into(),
            ops: vec![op],
        })
        .expect("an edit");
    fixture.render();
    let effects = fixture.canvas.input(up(1, (120.0, 120.0))).expect("input");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Edit(_)))
    );
}

/// A layer's pixel at `own`, in its own coordinates, as the session
/// holds it; transparent where it has none.
fn pixel(fixture: &Fixture, layer: LayerId, own: (i32, i32)) -> [u8; 4] {
    let tiles = fixture.session.lock().tiles(layer, None).expect("tiles");
    let side = koharu_scene::Tile::SIZE as i32;
    tiles
        .tiles
        .iter()
        .find(|tile| own.0.div_euclid(side) == tile.x && own.1.div_euclid(side) == tile.y)
        .map_or([0; 4], |tile| {
            let (x, y) = (own.0.rem_euclid(side), own.1.rem_euclid(side));
            let at = ((y * side + x) * 4) as usize;
            [
                tile.pixels[at],
                tile.pixels[at + 1],
                tile.pixels[at + 2],
                tile.pixels[at + 3],
            ]
        })
}

/// A raster at (10, 10) on a 100×100 page, opaque white over its own
/// (30, 30) to (50, 50).
fn offset_raster() -> Vec<Value> {
    vec![
        json!({"op": "add_layer", "id": "$raster", "page": "$page", "index": 0, "layer": {
            "kind": "raster", "offset": {"x": 10, "y": 10},
        }}),
        json!({"op": "write_pixels", "layer": "$raster",
            "rect": {"x": 30, "y": 30, "width": 20, "height": 20}, "pixels": vec![255u8; 20 * 20 * 4]}),
    ]
}

fn brush(diameter: f64, color: [f32; 4]) -> Brush {
    Brush {
        diameter,
        color: Rgba(color),
    }
}

#[test]
fn a_stroke_paints_a_new_raster_where_it_ran() {
    let Some(mut fixture) = unscaled_of((100, 100), Vec::new()) else {
        return;
    };
    fixture.canvas.set_tool(Tool::Draw);
    fixture.canvas.set_brush(brush(10.0, [1.0, 0.0, 0.0, 1.0]));
    let effects = gesture(&mut fixture.canvas, (20.0, 50.0), (80.0, 50.0));
    assert_eq!(apply(&mut fixture, effects), ["paint"]);
    let [raster] = layers(&fixture)[..] else {
        panic!("one layer");
    };
    assert_eq!(fixture.canvas.selection(), [raster]);
    assert_eq!(pixel(&fixture, raster, (50, 50)), [255, 0, 0, 255]);
    assert_eq!(pixel(&fixture, raster, (50, 70))[3], 0);
}

#[test]
fn painting_an_offset_raster_composites_onto_its_pixels() {
    let Some(mut fixture) = unscaled_of((100, 100), offset_raster()) else {
        return;
    };
    let raster = layers(&fixture)[0];
    fixture.canvas.select(vec![raster]);
    fixture.canvas.set_tool(Tool::Draw);
    fixture.canvas.set_brush(brush(6.0, [0.0, 0.0, 1.0, 1.0]));
    let effects = gesture(&mut fixture.canvas, (50.0, 50.0), (50.0, 50.0));
    assert_eq!(apply(&mut fixture, effects), ["paint"]);
    assert_eq!(layers(&fixture), [raster]);
    assert_eq!(pixel(&fixture, raster, (40, 40)), [0, 0, 255, 255]);
    assert_eq!(pixel(&fixture, raster, (40, 30)), [255; 4]);
}

#[test]
fn erasing_lowers_alpha_under_the_stroke_only() {
    let Some(mut fixture) = unscaled_of((100, 100), offset_raster()) else {
        return;
    };
    let raster = layers(&fixture)[0];
    fixture.canvas.set_tool(Tool::Eraser);
    fixture.canvas.set_brush(brush(6.0, [0.0, 0.0, 0.0, 1.0]));
    let effects = gesture(&mut fixture.canvas, (50.0, 50.0), (50.0, 50.0));
    assert!(matches!(
        effects[..],
        [Effect::Notice(Notice::EraserNeedsRaster)]
    ));

    fixture.canvas.select(vec![raster]);
    let effects = gesture(&mut fixture.canvas, (50.0, 50.0), (50.0, 50.0));
    assert_eq!(apply(&mut fixture, effects), ["erase"]);
    assert_eq!(pixel(&fixture, raster, (40, 40))[3], 0);
    assert_eq!(pixel(&fixture, raster, (40, 30))[3], 255);
}

#[test]
fn a_remove_stroke_asks_to_inpaint_what_it_covered() {
    let Some(mut fixture) = unscaled_of((100, 100), Vec::new()) else {
        return;
    };
    fixture.canvas.set_tool(Tool::Remove);
    fixture.canvas.set_brush(brush(8.0, [0.0, 0.0, 0.0, 1.0]));
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (40.0, 20.0));
    let Some(Effect::Inpaint {
        page,
        layer,
        rect,
        mask,
    }) = effects.last()
    else {
        panic!("{effects:?}");
    };
    assert_eq!(*page, fixture.page);
    assert!(rect.x <= 16 && rect.y <= 16);
    assert!(rect.x + rect.width as i32 >= 44 && rect.y + rect.height as i32 >= 24);
    let at = ((20 - rect.y) * rect.width as i32 + (30 - rect.x)) as usize;
    assert_eq!(mask[at], 255);

    // Onto a raster it adds and selects, which the next stroke goes onto.
    let layer = *layer;
    assert_eq!(apply(&mut fixture, effects), ["add_layer"]);
    assert_eq!(layers(&fixture), [layer]);
    assert_eq!(fixture.canvas.selection(), [layer]);
    let effects = gesture(&mut fixture.canvas, (60.0, 60.0), (70.0, 60.0));
    assert!(matches!(effects[..], [Effect::Inpaint { layer: next, .. }] if next == layer));
}

/// The remove tool paints a selected clean-up, and no mask.
#[test]
fn a_remove_stroke_goes_onto_the_selected_cleanup() {
    let Some((mut fixture, block, cleanup)) = painted_cleanup() else {
        return;
    };
    let mask: LayerId =
        serde_json::from_value(layer(&fixture, block)["mask"]["id"].clone()).expect("a mask");
    fixture.canvas.set_tool(Tool::Remove);
    fixture.canvas.set_brush(brush(8.0, [0.0, 0.0, 0.0, 1.0]));
    fixture.canvas.select(vec![cleanup]);
    let effects = gesture(&mut fixture.canvas, (40.0, 40.0), (60.0, 40.0));
    assert!(matches!(effects[..], [Effect::Inpaint { layer, .. }] if layer == cleanup));
    fixture.canvas.select(vec![mask]);
    let effects = gesture(&mut fixture.canvas, (40.0, 40.0), (60.0, 40.0));
    assert!(matches!(
        effects[..],
        [
            Effect::Edit(_),
            Effect::Selection(_),
            Effect::Inpaint { .. }
        ]
    ));
}

/// A box drawn with the text block tool adds a block in the balloon under it,
/// else in the panel, else on the page: its source's frame and its mask are
/// the box. A click adds nothing.
#[test]
fn a_drawn_box_adds_a_text_block_where_it_lies() {
    let Some(mut fixture) = unscaled_of(
        (200, 200),
        vec![
            json!({"op": "add_layer", "id": "$panel", "page": "$page", "index": 0, "layer": {
                "kind": "panel", "outline": "M0 0 H100 V200 H0 Z",
            }}),
            json!({"op": "add_layer", "page": "$page", "parent": "$panel", "index": 0, "layer": {
                "kind": "balloon", "outline": "M10 10 H90 V90 H10 Z",
            }}),
        ],
    ) else {
        return;
    };
    let panel = layers(&fixture)[0];
    let balloon = fixture
        .session
        .lock()
        .project()
        .layer(panel)
        .expect("a panel")
        .children[0]
        .id;
    fixture.canvas.set_tool(Tool::Block);
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (20.0, 20.0));
    assert!(effects.is_empty(), "{effects:?}");
    for (from, to, holder) in [
        ((20.0, 20.0), (40.0, 60.0), Some(balloon)),
        ((20.0, 120.0), (60.0, 140.0), Some(panel)),
        ((120.0, 20.0), (160.0, 40.0), None),
    ] {
        let effects = gesture(&mut fixture.canvas, from, to);
        assert_eq!(apply(&mut fixture, effects), ["add_text"]);
        let [id] = fixture.canvas.selection()[..] else {
            panic!("the block selected");
        };
        let session = fixture.session.lock();
        let project = session.project();
        let parent = project
            .parent(id)
            .expect("a parent")
            .1
            .map(|layer| layer.id);
        assert_eq!(parent, holder);
        let blocks = project.blocks(fixture.page).expect("blocks");
        let (_, block) = blocks
            .iter()
            .find(|(found, _)| *found == id)
            .expect("a block");
        let drawn = kurbo::Rect::from_points(from, to);
        assert_eq!(block.source.bounds(), Some(drawn));
        let rect = koharu_scene::Rect::covering(drawn, 200, 200).expect("a box");
        let mask = session.pixels(block.mask.id, rect).expect("a mask");
        assert!(mask.iter().all(|&value| value == 255));
    }
}

#[test]
fn the_marquee_marks_an_area_shift_adds_to_alt_takes_from_and_a_click_drops() {
    let Some(mut fixture) = unscaled_of((100, 100), Vec::new()) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    canvas.set_tool(Tool::Marquee);
    let mark = |canvas: &mut Canvas, from, to, modifiers| {
        let mut effects = canvas.input(down(from, modifiers)).expect("input");
        effects.extend(canvas.input(drag(1, to)).expect("input"));
        effects.extend(canvas.input(up(1, to)).expect("input"));
        assert!(effects.is_empty(), "{effects:?}");
    };
    mark(canvas, (10.0, 10.0), (50.0, 50.0), Modifiers::default());
    assert_eq!(
        canvas.area_bounds(),
        Some(Rect::new(10.0, 10.0, 50.0, 50.0))
    );
    mark(canvas, (60.0, 60.0), (80.0, 80.0), SHIFT);
    assert_eq!(
        canvas.area_bounds(),
        Some(Rect::new(10.0, 10.0, 80.0, 80.0))
    );
    mark(canvas, (40.0, 40.0), (90.0, 90.0), Modifiers::ALT);
    assert_eq!(
        canvas.area_bounds(),
        Some(Rect::new(10.0, 10.0, 50.0, 50.0))
    );

    let (page, rect, mask) = canvas.area().expect("the marked area");
    assert_eq!(page, fixture.page);
    let at = |x: i32, y: i32| mask[((y - rect.y) * rect.width as i32 + (x - rect.x)) as usize];
    assert_eq!((at(20, 20), at(45, 45)), (255, 0));

    mark(canvas, (5.0, 5.0), (5.0, 5.0), Modifiers::default());
    assert_eq!(canvas.area_bounds(), None);
    assert!(canvas.area().is_none());
}

#[test]
fn a_click_types_point_text_and_a_drag_box_text_each_added_on_commit() {
    let Some(mut fixture) = unscaled(Vec::new()) else {
        return;
    };
    fixture.canvas.set_tool(Tool::Text);
    // A click starts typing; the text is added only once it's committed.
    let effects = gesture(&mut fixture.canvas, (30.0, 40.0), (30.0, 40.0));
    assert!(apply(&mut fixture, effects).is_empty());
    type_text(&mut fixture.canvas, "Hi");
    let effects = press_key(&mut fixture.canvas, NamedKey::Enter, Code::Enter, CONTROL);
    assert_eq!(apply(&mut fixture, effects), ["add_text"]);
    let [point] = layers(&fixture)[..] else {
        panic!("one layer");
    };
    assert_eq!(layer(&fixture, point)["content"], json!([{"insert": "Hi"}]));
    assert_eq!(fixture.canvas.selection(), [point]);
    let value = layer(&fixture, point);
    assert_eq!(value["frame"], json!("point"));
    assert_eq!(
        value["transform"],
        json!([[1.0, 0.0, 30.0], [0.0, 1.0, 40.0]])
    );

    let effects = gesture(&mut fixture.canvas, (10.0, 10.0), (110.0, 60.0));
    assert!(apply(&mut fixture, effects).is_empty());
    type_text(&mut fixture.canvas, "Box");
    let effects = press_key(
        &mut fixture.canvas,
        NamedKey::Enter,
        Code::NumpadEnter,
        Modifiers::default(),
    );
    assert_eq!(apply(&mut fixture, effects), ["add_text"]);
    let [_, boxed] = layers(&fixture)[..] else {
        panic!("two layers");
    };
    assert_eq!(fixture.canvas.selection(), [boxed]);
    let value = layer(&fixture, boxed);
    assert_eq!(
        value["frame"]["box"]["bounds"],
        json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 50.0})
    );
    assert_eq!(
        value["transform"],
        json!([[1.0, 0.0, 10.0], [0.0, 1.0, 10.0]])
    );
}

#[test]
fn the_colour_picker_takes_the_page_colour_under_the_pointer() {
    let Some(mut fixture) = unscaled(red_square()) else {
        return;
    };
    let square = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    // A selection's outline drawn over the square isn't picked.
    canvas.select(vec![square]);
    canvas.set_tool(Tool::ColorPicker);
    let picked = |effects: Vec<Effect>| {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Color(color) => Some(color),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let effects = canvas
        .input(down((50.0, 75.0), Modifiers::default()))
        .expect("input");
    assert_eq!(picked(effects), [Rgba([1.0, 0.0, 0.0, 1.0])]);
    let effects = canvas
        .input(down((250.0, 75.0), Modifiers::default()))
        .expect("input");
    assert_eq!(picked(effects), [Rgba([1.0, 1.0, 1.0, 1.0])]);
    canvas.set_camera(TranslateScale::translate((150.0, 0.0)));
    let effects = canvas
        .input(down((20.0, 75.0), Modifiers::default()))
        .expect("input");
    assert!(picked(effects).is_empty());
}

/// The canvas starts its view on a whole viewport pixel, so the
/// renderer's tiles meet on screen pixels at any camera.
#[test]
fn a_page_shows_no_seam_between_its_tiles() {
    let blue: Vec<u8> = [0, 0, 255, 255].repeat(1100 * 100);
    let Some(mut fixture) = fixture(
        (1100, 100),
        vec![
            json!({"op": "add_layer", "id": "$raster", "page": "$page", "index": 0,
                "layer": {"kind": "raster"}}),
            json!({"op": "write_pixels", "layer": "$raster",
                "rect": {"x": 0, "y": 0, "width": 1100, "height": 100},
                "pixels": blue}),
        ],
    ) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    canvas.set_viewport(Rect::new(0.0, 0.0, 800.0, 200.0), 1.0);
    canvas.set_camera(TranslateScale::new(Vec2::new(10.3, 5.7), 0.6));
    canvas.draw().expect("a draw");
    let snapshot = canvas.snapshot().expect("pixels");
    for x in 12..668 {
        assert_eq!(snapshot.get_pixel(x, 40).0, [0, 0, 255, 255], "x {x}");
    }
}

#[test]
fn a_rotated_text_is_hit_inside_its_frame_only() {
    let turn = std::f64::consts::FRAC_PI_4;
    let (sin, cos) = turn.sin_cos();
    let Some(mut fixture) = unscaled_of(
        (800, 800),
        vec![
            json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
                "kind": "text", "content": "Hello",
                "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 200, "height": 40}}},
                "transform": [[cos, -sin, 300.0], [sin, cos, 300.0]],
            }}),
        ],
    ) else {
        return;
    };
    let text = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    let mut click = |at: Point| {
        let mut effects = canvas
            .input(down((at.x, at.y), Modifiers::default()))
            .expect("input");
        effects.extend(canvas.input(up(1, (at.x, at.y))).expect("input"));
        selections(&effects)
    };
    let rotated = Affine::translate((300.0, 300.0)) * Affine::rotate(turn);
    assert_eq!(click(rotated * Point::new(100.0, 20.0)), [vec![text]]);
    // Inside the rotated box's axis-aligned bounds, outside the box.
    assert_eq!(click(Point::new(420.0, 310.0)), [Vec::<LayerId>::new()]);
}

#[test]
fn the_selection_bounds_hold_a_point_texts_laid_out_lines() {
    let Some(mut fixture) = unscaled(vec![
        json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
            "kind": "text", "content": "Hello",
            "style": {"chars": {"size": 40}},
            "transform": [[1.0, 0.0, 100.0], [0.0, 1.0, 200.0]],
        }}),
    ]) else {
        return;
    };
    let text = layers(&fixture)[0];
    assert_eq!(fixture.canvas.selection_bounds(), None);
    fixture.canvas.select(vec![text]);
    // A point text's frame is its origin; its bounds are where its lines lie.
    let bounds = fixture.canvas.selection_bounds().expect("bounds");
    assert!(
        bounds.width() > 40.0 && bounds.height() > 20.0,
        "{bounds:?}"
    );
    assert!(
        bounds.contains(Point::new(110.0, bounds.center().y)),
        "{bounds:?}"
    );
}

#[test]
fn the_selection_bounds_hold_the_handles_drawn_on_it() {
    let Some(mut fixture) = unscaled(vec![
        json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
            "kind": "text", "content": "Hello",
            "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 100, "height": 30}}},
            "transform": [[1.0, 0.0, 100.0], [0.0, 1.0, 200.0]],
        }}),
    ]) else {
        return;
    };
    let text = layers(&fixture)[0];
    fixture.canvas.select(vec![text]);
    let bounds = fixture.canvas.selection_bounds().expect("bounds");
    // The rotate handle stands 16 pixels above the top, reached 7 around,
    // so the host's own controls keep off it.
    assert!(bounds.y0 <= 200.0 - 16.0 - 7.0 + 0.01, "{bounds:?}");
}

/// The wheel at `at`, held with `modifiers`.
fn scroll(at: (f64, f64), delta: ScrollDelta, modifiers: Modifiers) -> PointerEvent {
    PointerEvent::Scroll(PointerScrollEvent {
        pointer: mouse(1),
        delta,
        state: state(at, modifiers),
    })
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_wheel_pans_by_lines_or_pixels_and_ctrl_zooms_about_the_pointer() {
    let Some(mut fixture) = unscaled(Vec::new()) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    canvas.set_viewport(Rect::new(0.0, 0.0, 400.0, 400.0), 1.5);
    canvas.set_camera(TranslateScale::default());
    let none = Modifiers::default();
    canvas
        .input(scroll(
            (200.0, 200.0),
            ScrollDelta::LineDelta(0.0, 1.0),
            none,
        ))
        .expect("input");
    assert!(near(canvas.camera().translation.y, -150.0));
    canvas
        .input(scroll(
            (200.0, 200.0),
            ScrollDelta::PixelDelta((30.0, 0.0).into()),
            none,
        ))
        .expect("input");
    assert!(near(canvas.camera().translation.x, -30.0));

    let before = canvas.camera();
    let at = Point::new(100.0, 100.0);
    canvas
        .input(scroll(
            (at.x, at.y),
            ScrollDelta::LineDelta(0.0, -1.0),
            CONTROL,
        ))
        .expect("input");
    let after = canvas.camera();
    assert!(near(after.scale, before.scale * 0.15f64.exp()));
    assert!((after.inverse() * at).distance(before.inverse() * at) < 1e-9);
}

#[test]
fn a_pinch_zooms_about_where_it_is() {
    let Some(mut fixture) = unscaled(Vec::new()) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    let before = canvas.camera();
    let at = Point::new(200.0, 120.0);
    canvas
        .input(PointerEvent::Gesture(PointerGestureEvent {
            pointer: mouse(1),
            gesture: PointerGesture::Pinch(0.5),
            state: state((at.x, at.y), Modifiers::default()),
        }))
        .expect("input");
    let after = canvas.camera();
    assert!(near(after.scale, before.scale * 1.5));
    assert!((after.inverse() * at).distance(before.inverse() * at) < 1e-9);
}

#[test]
fn a_viewport_away_from_the_window_corner_hits_what_is_under_the_press() {
    let Some(mut fixture) = unscaled(vec![shape(0, (50.0, 50.0, 40.0, 40.0), json!({}))]) else {
        return;
    };
    let square = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    canvas.set_viewport(Rect::new(200.0, 100.0, 600.0, 500.0), 1.0);
    canvas.set_camera(TranslateScale::default());
    let mut click = |at| {
        let mut effects = canvas.input(down(at, Modifiers::default())).expect("input");
        effects.extend(canvas.input(up(1, at)).expect("input"));
        selections(&effects)
    };
    assert_eq!(click((270.0, 170.0)), [vec![square]]);
    assert_eq!(click((70.0, 70.0)), [Vec::<LayerId>::new()]);
}

/// Runs `ops` on the fixture's project as another writer would, and shows
/// the page again.
fn other_writer(fixture: &mut Fixture, ops: Vec<Value>) {
    let ops = ops
        .into_iter()
        .map(|op| serde_json::from_value(op).expect("an op"))
        .collect();
    fixture
        .session
        .lock()
        .apply(Edit {
            label: "other".into(),
            ops,
        })
        .expect("the edit applies");
    fixture.render();
}

/// A translated block far from the page's corner, its clean-up and mask
/// painted over (10, 10) to (30, 30): the fixture, the block, and the
/// clean-up.
fn painted_cleanup() -> Option<(Fixture, LayerId, LayerId)> {
    let mut fixture = unscaled(vec![
        json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
            "kind": "block", "source": {"content": "やあ"}, "translation": {
                "content": "Hi",
                "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 60, "height": 30}}},
                "transform": [[1.0, 0.0, 300.0], [0.0, 1.0, 300.0]],
            },
        }}),
    ])?;
    let block = layers(&fixture)[0];
    let value = layer(&fixture, block);
    let id = |part: &str| {
        serde_json::from_value::<LayerId>(value[part]["id"].clone()).expect("a part's id")
    };
    let (cleanup, mask) = (id("cleanup"), id("mask"));
    let rect = json!({"x": 10, "y": 10, "width": 20, "height": 20});
    other_writer(
        &mut fixture,
        vec![
            json!({"op": "write_pixels", "layer": cleanup, "rect": rect,
                "pixels": ([0u8, 128, 255, 255].repeat(20 * 20))}),
            json!({"op": "write_pixels", "layer": mask, "rect": rect,
                "pixels": vec![255u8; 20 * 20]}),
        ],
    );
    Some((fixture, block, cleanup))
}

/// A block's clean-up is picked only by its button in Layers: a click or a
/// marquee on the canvas passes over it.
#[test]
fn the_canvas_never_picks_a_cleanup() {
    let Some((mut fixture, _, cleanup)) = painted_cleanup() else {
        return;
    };
    for (from, to) in [((20.0, 20.0), (20.0, 20.0)), ((5.0, 5.0), (40.0, 40.0))] {
        let effects = gesture(&mut fixture.canvas, from, to);
        assert!(
            !selections(&effects).concat().contains(&cleanup),
            "{from:?} to {to:?}"
        );
    }
}

/// A selected mask shows as Photoshop's rubylith, red over what it hides,
/// and a selected clean-up against the page greyed out; what each draws
/// stays as it was.
#[test]
fn a_selected_part_shows_on_its_own() {
    let Some((mut fixture, block, cleanup)) = painted_cleanup() else {
        return;
    };
    let mask: LayerId =
        serde_json::from_value(layer(&fixture, block)["mask"]["id"].clone()).expect("a mask");
    let canvas = &mut fixture.canvas;
    let mut seen = |selection: Vec<LayerId>| {
        canvas.select(selection);
        canvas.draw().expect("a draw");
        canvas.snapshot().expect("pixels")
    };
    let plain = seen(Vec::new());
    assert_eq!(plain.get_pixel(60, 60).0, [255; 4]);
    // Half red over white, and grey 128 at 60% over white.
    for (part, pixel) in [(mask, [255, 128, 128]), (cleanup, [179, 179, 179])] {
        let shown = seen(vec![part]);
        assert_eq!(shown.get_pixel(20, 20), plain.get_pixel(20, 20), "{part}");
        let [r, g, b, _] = shown.get_pixel(60, 60).0;
        let near = |value: u8, want: u8| value.abs_diff(want) <= 2;
        assert!(
            near(r, pixel[0]) && near(g, pixel[1]) && near(b, pixel[2]),
            "{part}: {:?}",
            [r, g, b]
        );
    }
}

#[test]
fn a_selected_cleanup_moves_by_its_offset() {
    let Some((mut fixture, block, cleanup)) = painted_cleanup() else {
        return;
    };
    fixture.canvas.select(vec![cleanup]);
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (45.4, 30.0));
    assert_eq!(apply(&mut fixture, effects), ["move"]);
    assert_eq!(
        layer(&fixture, block)["cleanup"]["offset"],
        json!({"x": 25, "y": 10})
    );
}

/// The frame under a drag leaves out a block's clean-up when the clean-up is
/// dragged along with its block's translation.
#[test]
fn a_block_dragged_with_its_cleanup_leaves_neither_behind() {
    let Some((mut fixture, block, cleanup)) = painted_cleanup() else {
        return;
    };
    let canvas = &mut fixture.canvas;
    canvas.select(vec![block, cleanup]);
    canvas
        .input(down((20.0, 20.0), Modifiers::default()))
        .expect("input");
    canvas.input(drag(1, (120.0, 20.0))).expect("input");
    canvas.draw().expect("a draw");
    let snapshot = canvas.snapshot().expect("pixels");
    assert_eq!(snapshot.get_pixel(20, 20).0, [255; 4]);
    assert_eq!(snapshot.get_pixel(120, 20).0, [0, 128, 255, 255]);
}

#[test]
fn a_locked_cleanup_neither_drags_nor_takes_the_eraser() {
    let Some((mut fixture, block, cleanup)) = painted_cleanup() else {
        return;
    };
    other_writer(
        &mut fixture,
        vec![json!({"op": "update_layer", "layer": block,
            "patch": {"cleanup": {"locked": true}}})],
    );
    fixture.canvas.select(vec![cleanup]);
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (45.0, 30.0));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Edit(_)))
    );
    fixture.canvas.set_tool(Tool::Eraser);
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (20.0, 20.0));
    assert!(matches!(
        effects[..],
        [Effect::Notice(Notice::EraserNeedsRaster)]
    ));
}

#[test]
fn a_selected_layer_removed_by_another_writer_draws_nothing_and_clicks_still_work() {
    let Some(mut fixture) = unscaled(vec![
        shape(0, (50.0, 50.0, 40.0, 40.0), json!({})),
        shape(1, (250.0, 250.0, 40.0, 40.0), json!({})),
    ]) else {
        return;
    };
    let [first, second] = layers(&fixture)[..] else {
        panic!("two layers");
    };
    fixture.canvas.select(vec![first]);
    other_writer(
        &mut fixture,
        vec![json!({"op": "remove_layer", "layer": first})],
    );
    fixture.canvas.draw().expect("a draw");
    let mut effects = fixture
        .canvas
        .input(down((270.0, 270.0), Modifiers::default()))
        .expect("input");
    effects.extend(fixture.canvas.input(up(1, (270.0, 270.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![second]]);
}

#[test]
fn a_touch_contact_with_no_button_selects() {
    let Some(mut fixture) = unscaled(vec![shape(0, (50.0, 50.0, 40.0, 40.0), json!({}))]) else {
        return;
    };
    let square = layers(&fixture)[0];
    let touch = PointerInfo {
        pointer_id: PointerId::new(3),
        persistent_device_id: None,
        pointer_type: PointerType::Touch,
    };
    let contact = |down: bool| {
        let event = PointerButtonEvent {
            button: None,
            pointer: touch,
            state: state((70.0, 70.0), Modifiers::default()),
        };
        match down {
            true => PointerEvent::Down(event),
            false => PointerEvent::Up(event),
        }
    };
    let canvas = &mut fixture.canvas;
    let mut effects = canvas.input(contact(true)).expect("input");
    effects.extend(canvas.input(contact(false)).expect("input"));
    assert_eq!(selections(&effects), [vec![square]]);
}

#[test]
fn a_press_from_a_pointer_whose_release_was_lost_starts_afresh() {
    let Some(mut fixture) = unscaled(vec![shape(0, (50.0, 50.0, 40.0, 40.0), json!({}))]) else {
        return;
    };
    let square = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    canvas
        .input(press(1, (20.0, 20.0), Modifiers::default()))
        .expect("input");
    canvas.input(drag(1, (30.0, 30.0))).expect("input");
    // Its release went to another window.
    let mut effects = canvas
        .input(press(1, (70.0, 70.0), Modifiers::default()))
        .expect("input");
    effects.extend(canvas.input(up(1, (70.0, 70.0))).expect("input"));
    assert_eq!(selections(&effects), [vec![square]]);
}

#[test]
fn a_zoom_during_a_move_keeps_the_drag_in_page_space() {
    let Some(mut fixture) = unscaled(vec![shape(0, (50.0, 50.0, 40.0, 40.0), json!({}))]) else {
        return;
    };
    let square = layers(&fixture)[0];
    let mut effects = fixture
        .canvas
        .input(down((70.0, 70.0), Modifiers::default()))
        .expect("input");
    effects.extend(
        fixture
            .canvas
            .input(scroll(
                (70.0, 70.0),
                ScrollDelta::LineDelta(0.0, -1.0),
                CONTROL,
            ))
            .expect("input"),
    );
    let to = fixture.canvas.camera() * Point::new(95.0, 80.0);
    effects.extend(fixture.canvas.input(drag(1, (to.x, to.y))).expect("input"));
    effects.extend(fixture.canvas.input(up(1, (to.x, to.y))).expect("input"));
    assert_eq!(apply(&mut fixture, effects), ["move"]);
    let path = layer(&fixture, square)["path"]
        .as_str()
        .expect("a path")
        .to_owned();
    let bounds = BezPath::from_svg(&path).expect("a path").bounding_box();
    assert!(close(bounds.origin(), Point::new(75.0, 60.0)));
}

#[test]
fn new_text_answers_its_edit_then_its_selection() {
    let Some(mut fixture) = unscaled(Vec::new()) else {
        return;
    };
    fixture.canvas.set_tool(Tool::Text);
    gesture(&mut fixture.canvas, (30.0, 40.0), (30.0, 40.0));
    type_text(&mut fixture.canvas, "x");
    let effects = press_key(&mut fixture.canvas, NamedKey::Enter, Code::Enter, CONTROL);
    let [Effect::Edit(edit), Effect::Selection(ids)] = &effects[..] else {
        panic!("{effects:?}");
    };
    let [id] = ids[..] else {
        panic!("{ids:?}");
    };
    assert!(matches!(
        &edit.ops[..],
        [Op::AddLayer { id: Some(added), .. }] if *added == Ref::from(id)
    ));
    apply(&mut fixture, effects);
    assert_eq!(layers(&fixture), [id]);
    assert_eq!(fixture.canvas.selection(), [id]);
}

#[test]
fn a_draw_with_nothing_changed_returns_none() {
    let Some(mut fixture) = fixture((200, 200), red_square()) else {
        return;
    };
    let square = layers(&fixture)[0];
    let canvas = &mut fixture.canvas;
    canvas.set_viewport(Rect::new(0.0, 0.0, 400.0, 300.0), 1.0);
    assert!(canvas.draw().expect("a draw").is_some());
    assert!(canvas.draw().expect("a draw").is_none());
    // Over the workspace, off the page: nothing to hover.
    canvas.input(drag(1, (390.0, 290.0))).expect("input");
    assert!(canvas.draw().expect("a draw").is_none());
    // Another monitor's scale, at the same size.
    canvas.set_viewport(Rect::new(0.0, 0.0, 400.0, 300.0), 2.0);
    assert!(canvas.draw().expect("a draw").is_some());
    canvas.select(vec![square]);
    assert!(canvas.draw().expect("a draw").is_some());
}

#[test]
fn a_frame_of_the_same_revision_redraws_the_page() {
    let Some(mut fixture) = fixture((200, 200), red_square()) else {
        return;
    };
    fixture
        .canvas
        .set_viewport(Rect::new(0.0, 0.0, 400.0, 300.0), 1.0);
    assert!(fixture.canvas.draw().expect("a draw").is_some());
    // As after `[renderer]` changes: the same revision, drawn again.
    fixture.render();
    assert!(fixture.canvas.draw().expect("a draw").is_some());
}

#[test]
fn another_tool_drops_the_hover() {
    let Some(mut fixture) = unscaled(vec![shape(0, (50.0, 50.0, 40.0, 40.0), json!({}))]) else {
        return;
    };
    let canvas = &mut fixture.canvas;
    let drawn = |canvas: &mut Canvas| {
        canvas.draw().expect("a draw");
        canvas.snapshot().expect("pixels")
    };
    canvas.set_tool(Tool::Text);
    let plain = drawn(canvas);
    canvas.set_tool(Tool::Select);
    canvas.input(drag(1, (70.0, 70.0))).expect("input");
    assert!(drawn(canvas) != plain, "the hovered square is outlined");
    canvas.set_tool(Tool::Text);
    canvas.input(drag(1, (72.0, 72.0))).expect("input");
    assert!(drawn(canvas) == plain, "the outline is gone");
}

/// A mask's grey at `own`, in its own coordinates, as the session holds it.
fn grey(fixture: &Fixture, mask: LayerId, own: (i32, i32)) -> u8 {
    let tiles = fixture.session.lock().tiles(mask, None).expect("tiles");
    let side = koharu_scene::Tile::SIZE as i32;
    tiles
        .tiles
        .iter()
        .find(|tile| own.0.div_euclid(side) == tile.x && own.1.div_euclid(side) == tile.y)
        .map_or(0, |tile| {
            let (x, y) = (own.0.rem_euclid(side), own.1.rem_euclid(side));
            tile.pixels[(y * side + x) as usize]
        })
}

#[test]
fn the_eraser_hides_a_clean_up_through_its_mask_and_any_colour_uncovers_it() {
    let Some((mut fixture, block, _)) = painted_cleanup() else {
        return;
    };
    let mask: LayerId =
        serde_json::from_value(layer(&fixture, block)["mask"]["id"].clone()).expect("an id");
    fixture.canvas.select(vec![mask]);
    fixture.canvas.set_tool(Tool::Eraser);
    fixture.canvas.set_brush(brush(6.0, [0.0, 0.0, 0.0, 1.0]));
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (20.0, 20.0));
    assert_eq!(apply(&mut fixture, effects), ["erase"]);
    assert_eq!(grey(&fixture, mask, (20, 20)), 0);
    assert_eq!(grey(&fixture, mask, (12, 12)), 255);

    // Black uncovers as white would: a mask takes coverage, not colour.
    fixture.canvas.set_tool(Tool::Draw);
    let effects = gesture(&mut fixture.canvas, (20.0, 20.0), (20.0, 20.0));
    assert_eq!(apply(&mut fixture, effects), ["paint"]);
    assert_eq!(grey(&fixture, mask, (20, 20)), 255);
}

/// A block translated into the square path (100, 100) to (200, 200),
/// selected under the direct selection tool: the fixture and the block.
fn squared() -> Option<(Fixture, LayerId)> {
    let mut fixture = unscaled(vec![json!({"op": "add_layer", "page": "$page", "index": 0,
    "layer": {"kind": "block", "source": {"content": "やあ"}, "translation": {
        "content": "Hi",
        "frame": {"path": {"path": "M100 100L200 100L200 200L100 200Z", "fit": "best",
            "anchor": "center", "inset": 0.5}},
    }}})])?;
    let block = layers(&fixture)[0];
    fixture.canvas.set_tool(Tool::DirectSelect);
    fixture.canvas.select(vec![block]);
    Some((fixture, block))
}

/// The block's translation's frame, and its anchors in order.
fn frame(fixture: &Fixture, block: LayerId) -> (Value, Vec<Point>) {
    let frame = layer(fixture, block)["translation"]["frame"].clone();
    let path = BezPath::from_svg(frame["path"]["path"].as_str().expect("a path")).expect("SVG");
    let anchors = path
        .elements()
        .iter()
        .filter_map(|element| match *element {
            kurbo::PathEl::MoveTo(point) | kurbo::PathEl::LineTo(point) => Some(point),
            _ => None,
        })
        .collect();
    (frame, anchors)
}

#[test]
fn a_dragged_anchor_reshapes_the_frame_and_keeps_its_fit() {
    let Some((mut fixture, block)) = squared() else {
        return;
    };
    let effects = gesture(&mut fixture.canvas, (200.0, 100.0), (240.0, 80.0));
    assert_eq!(apply(&mut fixture, effects), ["edit_frame"]);
    let (value, anchors) = frame(&fixture, block);
    assert_eq!(
        anchors,
        [
            (100.0, 100.0),
            (240.0, 80.0),
            (200.0, 200.0),
            (100.0, 200.0)
        ]
        .map(Point::from)
    );
    assert_eq!(value["path"]["fit"], "best");
    assert_eq!(value["path"]["inset"], 0.5);

    // A segment drags both its anchors.
    let effects = gesture(&mut fixture.canvas, (150.0, 200.0), (150.0, 220.0));
    assert_eq!(apply(&mut fixture, effects), ["edit_frame"]);
    let (_, anchors) = frame(&fixture, block);
    assert_eq!(
        anchors[2..],
        [(200.0, 220.0), (100.0, 220.0)].map(Point::from)
    );
}

/// Delete pressed, held, or released.
fn delete(state: KeyState, repeat: bool) -> KeyboardEvent {
    KeyboardEvent {
        state,
        key: Key::Named(NamedKey::Delete),
        code: Code::Delete,
        repeat,
        ..KeyboardEvent::default()
    }
}

#[test]
fn deleting_anchors_joins_their_neighbours_and_never_opens_a_frame() {
    let Some((mut fixture, block)) = squared() else {
        return;
    };
    // A marquee around the top right corner picks it.
    let effects = gesture(&mut fixture.canvas, (180.0, 80.0), (220.0, 120.0));
    assert!(apply(&mut fixture, effects).is_empty());
    let effects = fixture
        .canvas
        .key(&delete(KeyState::Down, false))
        .expect("a key")
        .expect("an anchor is picked");
    assert_eq!(apply(&mut fixture, effects), ["edit_frame"]);
    let (_, anchors) = frame(&fixture, block);
    assert_eq!(
        anchors,
        [(100.0, 100.0), (200.0, 200.0), (100.0, 200.0)].map(Point::from)
    );
    // Held on with no anchor left, the key stays the canvas's until it's
    // released, so it never goes on to delete the text.
    let held = fixture
        .canvas
        .key(&delete(KeyState::Down, true))
        .expect("a key");
    assert!(held.is_some_and(|effects| effects.is_empty()));
    let released = fixture
        .canvas
        .key(&delete(KeyState::Up, false))
        .expect("a key");
    assert!(released.is_some_and(|effects| effects.is_empty()));

    // Three anchors are the fewest a frame keeps.
    let effects = gesture(&mut fixture.canvas, (90.0, 90.0), (210.0, 210.0));
    apply(&mut fixture, effects);
    let effects = fixture
        .canvas
        .key(&delete(KeyState::Down, false))
        .expect("a key")
        .expect("anchors are picked");
    assert!(apply(&mut fixture, effects).is_empty());
    assert_eq!(frame(&fixture, block).1.len(), 3);
    fixture
        .canvas
        .key(&delete(KeyState::Up, false))
        .expect("a key");
    // With no anchor picked, Delete is the host's.
    assert!(
        fixture
            .canvas
            .key(&delete(KeyState::Down, false))
            .expect("a key")
            .is_none()
    );
}

#[test]
fn a_box_frame_becomes_a_path_through_its_corners_at_its_first_edit() {
    let Some((mut fixture, block, _)) = painted_cleanup() else {
        return;
    };
    fixture.canvas.set_tool(Tool::DirectSelect);
    fixture.canvas.select(vec![block]);
    let effects = gesture(&mut fixture.canvas, (360.0, 330.0), (380.0, 350.0));
    assert_eq!(apply(&mut fixture, effects), ["edit_frame"]);
    let (value, anchors) = frame(&fixture, block);
    assert!(value.get("box").is_none(), "{value}");
    assert_eq!(value["path"]["inset"], 0.0);
    assert_eq!(
        anchors,
        [(0.0, 0.0), (60.0, 0.0), (80.0, 50.0), (0.0, 30.0)].map(Point::from)
    );
}

/// A key pressed with `modifiers`.
fn key_down(key: Key, code: Code, modifiers: Modifiers) -> KeyboardEvent {
    KeyboardEvent {
        state: KeyState::Down,
        key,
        code,
        modifiers,
        ..KeyboardEvent::default()
    }
}

/// `named` pressed with `modifiers` and released; what typing answered.
fn press_key(
    canvas: &mut Canvas,
    named: NamedKey,
    code: Code,
    modifiers: Modifiers,
) -> Vec<Effect> {
    let pressed = key_down(Key::Named(named), code, modifiers);
    let effects = canvas
        .key(&pressed)
        .expect("a key")
        .expect("typing takes the key");
    canvas
        .key(&KeyboardEvent {
            state: KeyState::Up,
            ..pressed
        })
        .expect("a key");
    effects
}

/// Types `text` a character at a time, as key presses do.
fn type_text(canvas: &mut Canvas, text: &str) {
    for character in text.chars() {
        let pressed = key_down(
            Key::Character(character.to_string()),
            Code::Unidentified,
            Modifiers::default(),
        );
        canvas
            .key(&pressed)
            .expect("a key")
            .expect("typing takes the key");
        canvas
            .key(&KeyboardEvent {
                state: KeyState::Up,
                ..pressed
            })
            .expect("a key");
    }
}

/// A 40-pixel point text of `content` with its origin at (100, 200).
fn typed_text(content: Value) -> Vec<Value> {
    vec![
        json!({"op": "add_layer", "page": "$page", "index": 0, "layer": {
            "kind": "text", "content": content,
            "style": {"chars": {"size": 40}},
            "transform": [[1.0, 0.0, 100.0], [0.0, 1.0, 200.0]],
        }}),
    ]
}

#[test]
fn escape_or_an_empty_commit_adds_no_text() {
    let Some(mut fixture) = unscaled(Vec::new()) else {
        return;
    };
    fixture.canvas.set_tool(Tool::Text);
    gesture(&mut fixture.canvas, (30.0, 40.0), (30.0, 40.0));
    type_text(&mut fixture.canvas, "Gone");
    let escaped = press_key(
        &mut fixture.canvas,
        NamedKey::Escape,
        Code::Escape,
        Modifiers::default(),
    );
    assert!(escaped.is_empty());
    assert!(fixture.canvas.typing().is_none());
    gesture(&mut fixture.canvas, (30.0, 40.0), (30.0, 40.0));
    let effects = press_key(&mut fixture.canvas, NamedKey::Enter, Code::Enter, CONTROL);
    assert!(apply(&mut fixture, effects).is_empty());
    assert!(layers(&fixture).is_empty());
}

#[test]
fn typing_into_a_text_edits_it_at_the_click_in_one_step() {
    let Some(mut fixture) = unscaled(typed_text(json!("Hello"))) else {
        return;
    };
    let text = layers(&fixture)[0];
    fixture.canvas.set_tool(Tool::Text);
    // A click on the text types into it, the caret where it landed.
    let effects = gesture(&mut fixture.canvas, (101.0, 220.0), (101.0, 220.0));
    assert_eq!(selections(&effects), [vec![text]]);
    assert_eq!(
        fixture.canvas.typing().map(|typing| typing.layer),
        Some(Some(text))
    );
    press_key(
        &mut fixture.canvas,
        NamedKey::End,
        Code::End,
        Modifiers::default(),
    );
    type_text(&mut fixture.canvas, "!?");
    press_key(
        &mut fixture.canvas,
        NamedKey::Backspace,
        Code::Backspace,
        Modifiers::default(),
    );
    press_key(
        &mut fixture.canvas,
        NamedKey::Home,
        Code::Home,
        Modifiers::default(),
    );
    type_text(&mut fixture.canvas, "Oh ");
    // A click away commits it as one step, and starts the next text.
    let effects = gesture(&mut fixture.canvas, (300.0, 380.0), (300.0, 380.0));
    assert_eq!(apply(&mut fixture, effects), ["set_text"]);
    assert_eq!(
        layer(&fixture, text)["content"],
        json!([{"insert": "Oh Hello!"}])
    );
    assert!(
        fixture
            .canvas
            .typing()
            .is_some_and(|typing| typing.layer.is_none())
    );
}

#[test]
fn styling_a_typed_selection_marks_those_characters_and_a_caret_holds_it() {
    let Some(mut fixture) = unscaled(typed_text(json!("Hello world"))) else {
        return;
    };
    let text = layers(&fixture)[0];
    fixture.canvas.set_tool(Tool::Text);
    gesture(&mut fixture.canvas, (101.0, 220.0), (101.0, 220.0));
    press_key(
        &mut fixture.canvas,
        NamedKey::Home,
        Code::Home,
        Modifiers::default(),
    );
    for _ in 0..5 {
        press_key(
            &mut fixture.canvas,
            NamedKey::ArrowRight,
            Code::ArrowRight,
            SHIFT,
        );
    }
    let bold = koharu_scene::CharStyle {
        weight: Some(700),
        ..koharu_scene::CharStyle::default()
    };
    fixture.canvas.style_typing(&bold).expect("a style");
    let typing = fixture.canvas.typing().expect("typing");
    assert_eq!(
        (typing.chars.weight, typing.chars.size),
        (Some(700), Some(40.0))
    );
    // All of it differs in weight, and shares its size.
    fixture
        .canvas
        .key(&key_down(
            Key::Character("a".to_owned()),
            Code::KeyA,
            CONTROL,
        ))
        .expect("a key");
    let typing = fixture.canvas.typing().expect("typing");
    assert!(typing.mixed.contains(&CharKey::Weight));
    assert_eq!(typing.chars.size, Some(40.0));
    // At the caret, a style waits for what's typed next.
    press_key(
        &mut fixture.canvas,
        NamedKey::End,
        Code::End,
        Modifiers::default(),
    );
    fixture
        .canvas
        .style_typing(&koharu_scene::CharStyle {
            size: Some(30.0),
            ..koharu_scene::CharStyle::default()
        })
        .expect("a style");
    type_text(&mut fixture.canvas, "!");
    let effects = press_key(&mut fixture.canvas, NamedKey::Enter, Code::Enter, CONTROL);
    assert_eq!(apply(&mut fixture, effects), ["set_text"]);
    let content: koharu_scene::RichText =
        serde_json::from_value(layer(&fixture, text)["content"].clone()).expect("spans");
    let spans: Vec<_> = content
        .0
        .iter()
        .map(|span| {
            let marks = &span.attributes;
            (span.insert.as_str(), marks.weight, marks.size)
        })
        .collect();
    assert_eq!(
        spans,
        [
            ("Hello", Some(700), None),
            (" world", None, None),
            ("!", None, Some(30.0))
        ]
    );
}

#[test]
fn ruby_goes_over_the_base_its_field_opened_on_until_anything_else_changes() {
    let Some(mut fixture) = unscaled(typed_text(json!([
        {"insert": "Hel"}, {"insert": "lo", "ruby": "o"}, {"insert": " world"},
    ]))) else {
        return;
    };
    let text = layers(&fixture)[0];
    fixture.canvas.set_tool(Tool::Text);
    gesture(&mut fixture.canvas, (101.0, 220.0), (101.0, 220.0));
    let key = |fixture: &mut Fixture, key, code, modifiers| {
        press_key(&mut fixture.canvas, key, code, modifiers);
    };
    let ruby = |fixture: &mut Fixture, edit: RubyEdit| {
        fixture.canvas.annotate_typing(&edit).expect("a ruby");
    };
    let set = |ruby: &str| RubyEdit::Set {
        ruby: (!ruby.is_empty()).then(|| ruby.to_owned()),
    };
    let base = |fixture: &Fixture| {
        let typing = fixture.canvas.typing().expect("typing");
        let base = typing.base?;
        Some((base.text, base.ruby, typing.annotating))
    };
    let based = |text: &str, ruby: Option<&str>, open: bool| {
        Some((text.to_owned(), ruby.map(str::to_owned), open))
    };
    let none = Modifiers::default();
    key(&mut fixture, NamedKey::Home, Code::Home, none);
    assert_eq!(base(&fixture), None);
    for _ in 0..3 {
        key(&mut fixture, NamedKey::ArrowRight, Code::ArrowRight, SHIFT);
    }
    ruby(&mut fixture, RubyEdit::Open);
    assert_eq!(base(&fixture), based("Hel", None, true));
    // Passing through the neighbour's ruby joins nothing, and one taken off
    // leaves the field on its base.
    for typed in ["o", "oh", ""] {
        ruby(&mut fixture, set(typed));
    }
    assert_eq!(base(&fixture), based("Hel", None, true));
    ruby(&mut fixture, set("he"));
    ruby(&mut fixture, RubyEdit::Keep);
    assert_eq!(base(&fixture), based("Hel", Some("he"), false));
    // Closed, it sets nothing.
    ruby(&mut fixture, set("late"));
    assert_eq!(base(&fixture), based("Hel", Some("he"), false));
    // Over parts of two bases, cancelling puts both back.
    key(&mut fixture, NamedKey::Home, Code::Home, none);
    key(&mut fixture, NamedKey::ArrowRight, Code::ArrowRight, none);
    for _ in 0..3 {
        key(&mut fixture, NamedKey::ArrowRight, Code::ArrowRight, SHIFT);
    }
    ruby(&mut fixture, RubyEdit::Open);
    ruby(&mut fixture, set("x"));
    assert_eq!(base(&fixture), based("ell", Some("x"), true));
    ruby(&mut fixture, RubyEdit::Cancel);
    // A caret in a base, at its end included, opens on it; the caret moving on
    // closes the field.
    key(&mut fixture, NamedKey::Home, Code::Home, none);
    for _ in 0..5 {
        key(&mut fixture, NamedKey::ArrowRight, Code::ArrowRight, none);
    }
    ruby(&mut fixture, RubyEdit::Open);
    assert_eq!(base(&fixture), based("lo", Some("o"), true));
    key(&mut fixture, NamedKey::ArrowRight, Code::ArrowRight, none);
    assert_eq!(base(&fixture), None);
    ruby(&mut fixture, set("late"));
    let effects = press_key(&mut fixture.canvas, NamedKey::Enter, Code::Enter, CONTROL);
    assert_eq!(apply(&mut fixture, effects), ["set_text"]);
    assert_eq!(
        layer(&fixture, text)["content"],
        json!([
            {"insert": "Hel", "ruby": "he"}, {"insert": "lo", "ruby": "o"}, {"insert": " world"},
        ])
    );
}

#[test]
fn an_input_methods_composition_shows_until_it_commits() {
    let Some(mut fixture) = unscaled(Vec::new()) else {
        return;
    };
    fixture.canvas.set_tool(Tool::Text);
    gesture(&mut fixture.canvas, (30.0, 40.0), (30.0, 40.0));
    fixture
        .canvas
        .compose("ni", Some((2, 2)))
        .expect("a composition");
    // Composing types nothing yet; the commit types what it chose.
    let escaped = press_key(
        &mut fixture.canvas,
        NamedKey::Escape,
        Code::Escape,
        Modifiers::default(),
    );
    assert!(escaped.is_empty());
    gesture(&mut fixture.canvas, (30.0, 40.0), (30.0, 40.0));
    fixture
        .canvas
        .compose("ni", Some((2, 2)))
        .expect("a composition");
    fixture.canvas.compose("", None).expect("a composition");
    fixture.canvas.insert("你").expect("a commit");
    let effects = press_key(&mut fixture.canvas, NamedKey::Enter, Code::Enter, CONTROL);
    assert_eq!(apply(&mut fixture, effects), ["add_text"]);
    let [added] = layers(&fixture)[..] else {
        panic!("one layer");
    };
    assert_eq!(layer(&fixture, added)["content"], json!([{"insert": "你"}]));
}
