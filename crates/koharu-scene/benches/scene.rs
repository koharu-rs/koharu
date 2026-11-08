//! Pixel writes and reads, where each 64×64 tile is compressed on its own,
//! and opening a project, which checks every layer.
//!
//! A page is 2048×2896 RGBA, about a scanned manga page, filled with gradients
//! and low noise so zlib does the work a scan would give it.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use koharu_scene::{Created, Edit, LayerId, Op, Rect, Session};
use serde_json::{Value, json};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 2896;

fn edit(ops: Vec<Value>) -> Edit {
    Edit {
        label: "bench".to_owned(),
        ops: ops
            .into_iter()
            .map(|op| serde_json::from_value(op).expect("a valid op"))
            .collect(),
    }
}

fn write(layer: LayerId, pixels: Vec<u8>) -> Edit {
    Edit {
        label: "bench".to_owned(),
        ops: vec![Op::WritePixels {
            layer: layer.into(),
            rect: Rect {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
            pixels,
        }],
    }
}

/// A page of RGBA pixels: smooth gradients with low noise, as in a scan.
fn page_pixels(seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state & 7) as u8;
            pixels.extend([
                (x / 8) as u8 ^ noise,
                (y / 12) as u8 ^ noise,
                ((x + y) / 16) as u8,
                255,
            ]);
        }
    }
    pixels
}

/// A project in a fresh directory with one page holding one empty raster.
fn raster() -> (TempDir, Session, LayerId) {
    let dir = TempDir::new().expect("a temporary directory");
    let mut session = Session::create(dir.path().join("bench.khr")).expect("a project");
    let change = session
        .apply(edit(vec![
            json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": WIDTH, "height": HEIGHT}}),
            json!({"op": "add_layer", "page": "$p", "index": 0, "layer": {"kind": "raster"}}),
        ]))
        .expect("an edit");
    let layer = change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, .. } => Some(*id),
            Created::Page { .. } => None,
        })
        .expect("a raster");
    (dir, session, layer)
}

fn pixels(c: &mut Criterion) {
    let (first, second) = (page_pixels(1), page_pixels(2));
    let mut group = c.benchmark_group("pixels");
    group.sample_size(10);
    group.bench_function("write a page into an empty raster", |b| {
        b.iter_batched(
            || (raster(), first.clone()),
            |((dir, mut session, layer), pixels)| {
                session.apply(write(layer, pixels)).expect("a paint");
                (dir, session)
            },
            BatchSize::PerIteration,
        );
    });
    group.bench_function("write a page over a full raster", |b| {
        b.iter_batched(
            || {
                let (dir, mut session, layer) = raster();
                session.apply(write(layer, first.clone())).expect("a paint");
                ((dir, session, layer), second.clone())
            },
            |((dir, mut session, layer), pixels)| {
                session.apply(write(layer, pixels)).expect("a paint");
                (dir, session)
            },
            BatchSize::PerIteration,
        );
    });
    let (_dir, mut session, layer) = raster();
    session.apply(write(layer, first.clone())).expect("a paint");
    group.bench_function("decode a full raster", |b| {
        b.iter(|| black_box(session.tiles(layer, None).expect("tiles")));
    });
    group.finish();
}

/// Opening checks every layer of every page.
fn open(c: &mut Criterion) {
    const PAGES: usize = 20;
    const TEXTS: usize = 30;
    let dir = TempDir::new().expect("a temporary directory");
    let path = dir.path().join("bench.khr");
    let mut session = Session::create(&path).expect("a project");
    for index in 0..PAGES {
        let mut ops = vec![
            json!({"op": "add_page", "id": "$p", "index": index, "size": {"width": WIDTH, "height": HEIGHT}}),
        ];
        ops.extend((0..TEXTS).map(|text| {
            json!({
                "op": "add_layer", "page": "$p", "index": text,
                "layer": {
                    "kind": "text",
                    "content": [{"insert": "吹き出しの", "attributes": {"weight": 700}}, {"insert": "テキスト"}],
                    "frame": {"box": {"bounds": {"x": 100.0, "y": 100.0, "width": 300.0, "height": 400.0}}},
                    "language": "ja",
                },
            })
        }));
        session.apply(edit(ops)).expect("a page");
    }
    drop(session);
    c.bench_function("open a project of 20 pages with 30 texts each", |b| {
        b.iter(|| black_box(Session::open(&path).expect("it opens")));
    });
}

criterion_group!(benches, pixels, open);
criterion_main!(benches);
