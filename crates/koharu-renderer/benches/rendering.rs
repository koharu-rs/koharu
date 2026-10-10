use std::{collections::BTreeMap, hint::black_box, io::Cursor, sync::Arc};

use criterion::{Criterion, criterion_group, criterion_main};
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_renderer::Renderer;
use koharu_scene::{
    AssetInput, AssetMetadata, AssetRole, At, Change, Geometry, PageDraft, Session, Snapshot,
};

struct Fixture {
    renderer: Renderer,
    page: koharu_scene::EntityId,
    snapshot: Snapshot,
    moved: Snapshot,
    change: Change,
    frame: koharu_renderer::Frame,
}

fn png(width: u32, height: u32, color: [u8; 4]) -> Arc<[u8]> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner().into()
}

async fn fixture() -> Fixture {
    let mut session = Session::memory().await.unwrap();
    let source = AssetRole::new("source").unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(PageDraft::new("bench", 512.0, 768.0), At::End)?;
            edit.set_asset(
                page,
                &source,
                AssetInput::new(
                    png(512, 768, [240, 240, 240, 255]),
                    "image/png",
                    AssetMetadata {
                        width: Some(512),
                        height: Some(768),
                        attributes: BTreeMap::new(),
                    },
                ),
            )?;
            let overlay = edit.add_entity(page, At::End)?;
            edit.set(overlay, &Geometry::rectangle(40.0, 60.0, 256.0, 384.0))?;
            edit.set_asset(
                overlay,
                &source,
                AssetInput::new(
                    png(256, 384, [40, 80, 120, 180]),
                    "image/png",
                    AssetMetadata {
                        width: Some(256),
                        height: Some(384),
                        attributes: BTreeMap::new(),
                    },
                ),
            )?;
            ids = Some((page, overlay));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(create).await.unwrap().snapshot;
    let (page, overlay) = ids.unwrap();
    let renderer = Renderer::new().unwrap();
    let frame = renderer.render(&snapshot, page).await.unwrap();
    let placement = snapshot
        .patch(|edit| edit.set(overlay, &Geometry::rectangle(48.0, 64.0, 256.0, 384.0)))
        .unwrap();
    let commit = session.commit(placement).await.unwrap();
    Fixture {
        renderer,
        page,
        snapshot,
        moved: commit.snapshot,
        change: commit.changes,
        frame,
    }
}

async fn page_frame(width: u32, height: u32) -> koharu_renderer::Frame {
    let mut session = Session::memory().await.unwrap();
    let source = AssetRole::new("source").unwrap();
    let mut page = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let id = edit.add_page(
                PageDraft::new("bench", f64::from(width), f64::from(height)),
                At::End,
            )?;
            edit.set_asset(
                id,
                &source,
                AssetInput::new(
                    png(width, height, [240, 240, 240, 255]),
                    "image/png",
                    AssetMetadata {
                        width: Some(width),
                        height: Some(height),
                        attributes: BTreeMap::new(),
                    },
                ),
            )?;
            page = Some(id);
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(create).await.unwrap().snapshot;
    Renderer::new()
        .unwrap()
        .render(&snapshot, page.unwrap())
        .await
        .unwrap()
}

fn rendering_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = runtime.block_on(fixture());

    c.bench_function("renderer/render_cold_512x768_2_layers", |b| {
        b.iter(|| {
            let renderer = Renderer::new().unwrap();
            black_box(runtime.block_on(renderer.render(&fixture.snapshot, fixture.page))).unwrap();
        });
    });

    c.bench_function("renderer/render_retained_512x768_2_layers", |b| {
        b.iter(|| {
            black_box(runtime.block_on(fixture.renderer.render(&fixture.snapshot, fixture.page)))
                .unwrap();
        });
    });

    let no_op = Change {
        from: fixture.snapshot.revision(),
        to: fixture.snapshot.revision(),
        entities: Vec::new(),
        hierarchy: Vec::new(),
        components: Vec::new(),
        relations: Vec::new(),
    };
    c.bench_function("renderer/update_noop_512x768_2_layers", |b| {
        b.iter(|| {
            black_box(runtime.block_on(fixture.renderer.update(
                &fixture.frame,
                &fixture.snapshot,
                &no_op,
            )))
            .unwrap();
        });
    });

    c.bench_function("renderer/update_placement_512x768_2_layers", |b| {
        b.iter(|| {
            black_box(runtime.block_on(fixture.renderer.update(
                &fixture.frame,
                &fixture.moved,
                &fixture.change,
            )))
            .unwrap();
        });
    });

    let rasterizer = Rasterizer::new();
    let raster_frame = fixture.frame.raster_frame();
    if let (Ok(rasterizer), Ok(raster_frame)) = (rasterizer, raster_frame) {
        c.bench_function("renderer/rasterize_warm_512x768_2_layers", |b| {
            b.iter(|| {
                black_box(rasterizer.rasterize(&raster_frame, RasterOptions::default())).unwrap();
            });
        });

        let mut group = c.benchmark_group("renderer");
        group.sample_size(20);
        for (width, height) in [(4096, 6144), (8192, 8192)] {
            let page = runtime
                .block_on(page_frame(width, height))
                .raster_frame()
                .unwrap();
            group.bench_function(format!("rasterize_warm_{width}x{height}_1_layer"), |b| {
                b.iter(|| {
                    black_box(rasterizer.rasterize(&page, RasterOptions::default())).unwrap();
                });
            });
        }
        group.finish();
    }

    let mut group = c.benchmark_group("rasterizer");
    group.sample_size(10);
    group.bench_function("new", |b| {
        b.iter(|| black_box(Rasterizer::new()).unwrap());
    });
    group.finish();
}

criterion_group!(benches, rendering_benchmark);
criterion_main!(benches);
