//! What the desktop canvas does with the renderer: render the page when the
//! session changes, draw what the window shows at its zoom, find the layer
//! under the pointer, and mark text that overflows. Rendering after an edit
//! reuses every layer the edit didn't touch.
//!
//! `cargo run -p koharu-renderer --example canvas`

use std::time::Instant;

use koharu_renderer::{Frame, Renderer, TypesettingConfig, View};
use koharu_scene::{Author, Created, Edit, LayerId, LayerKind, Op, Session};
use serde_json::{Value, json};
use vello::{
    kurbo::{Point, Rect},
    wgpu,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const WINDOW: (f64, f64) = (1280.0, 800.0);

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("episode.koharu"))?;

    // A webtoon episode: balloons down a tall page, and a narration box set
    // too small for its text.
    let lines = [
        "It's so hot today!",
        "Let's get ice cream.",
        "Is that a shooting star?!",
    ];
    let mut ops: Vec<Value> = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 1080, "height": 4000}}),
    ];
    for (index, line) in lines.iter().enumerate() {
        let y = 400.0 + 1200.0 * index as f64;
        ops.push(json!({"op": "add_layer", "id": format!("$line{index}"), "page": "$page", "index": index, "layer": {
            "kind": "text", "content": line, "language": "en", "style": {"chars": {"size": 40.0}, "align": "center"},
            "frame": {"path": {"path": format!("M320 {y} A220 130 0 1 0 760 {y} A220 130 0 1 0 320 {y} Z"), "anchor": "center"}},
        }}));
    }
    ops.push(json!({"op": "add_layer", "id": "$narration", "page": "$page", "index": lines.len(), "layer": {
        "kind": "text", "language": "en", "style": {"chars": {"size": 36.0}},
        "content": "Years later, she would still remember the night the sky fell.",
        "frame": {"box": {"bounds": {"x": 140.0, "y": 3700.0, "width": 400.0, "height": 60.0}, "fit": "none"}},
    }}));
    let change = session.apply(edit(&session, ops)?)?;
    let page = change.pages.added[0];
    let line = |index: usize| {
        change.created.iter().find_map(|created| match created {
            Created::Layer {
                id,
                label: Some(label),
            } if *label == format!("line{index}") => Some(*id),
            _ => None,
        })
    };

    let renderer = Renderer::new(TypesettingConfig::default());
    let gpu = renderer.gpu()?;
    let started = Instant::now();
    let frame = renderer.render(&session, page)?;
    println!("rendered in {:?}", started.elapsed());

    // The window, zoomed to 200% on the second balloon. Each strip renders
    // into a texture of its own, which the canvas places at `strip.rect`.
    let balloon = frame
        .layer(line(1).expect("the line"))
        .and_then(|layer| layer.bounds())
        .expect("it draws");
    let view = View {
        rect: Rect::from_center_size(balloon.center(), (WINDOW.0 / 2.0, WINDOW.1 / 2.0)),
        scale: 2.0,
    };
    let strips = frame.draw(view);
    for strip in &strips {
        let (width, height) = (strip.rect.width() as u32, strip.rect.height() as u32);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas strip"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        gpu.render(
            &strip.scene,
            &texture.create_view(&Default::default()),
            width,
            height,
        )?;
    }
    println!("drew the 200% view in {} strip(s)", strips.len());

    if let Some(layer) = topmost(&frame, view.rect.center()) {
        println!(
            "under the pointer: the {:?} layer {layer}",
            frame.layer(layer).expect("drawn").kind()
        );
    }
    for layer in frame
        .layers()
        .filter(|layer| layer.text().is_some_and(|text| text.overflowed))
    {
        println!("overflow mark on {} over {:?}", layer.id(), layer.bounds());
    }

    let retype = Op::SetText {
        layer: line(1).expect("the line").into(),
        text: "Let's get ice cream after school.".to_owned(),
        ingredients: Vec::new(),
    };
    session.apply(Edit {
        ops: vec![retype],
        ..edit(&session, Vec::new())?
    })?;
    let started = Instant::now();
    renderer.render(&session, page)?;
    println!("rendered after the edit in {:?}", started.elapsed());
    Ok(())
}

/// The topmost layer drawing at `point`: the last in the page's order, as
/// layers draw first to last. Groups and masks draw nothing of their own.
fn topmost(frame: &Frame, point: Point) -> Option<LayerId> {
    frame
        .layers()
        .filter(|layer| {
            layer.visible() && !matches!(layer.kind(), LayerKind::Group | LayerKind::Mask)
        })
        .filter(|layer| layer.bounds().is_some_and(|bounds| bounds.contains(point)))
        .last()
        .map(|layer| layer.id())
}

fn edit(session: &Session, ops: Vec<Value>) -> Result<Edit> {
    Ok(Edit {
        project: session.id(),
        label: "Letter".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: ops
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    })
}
