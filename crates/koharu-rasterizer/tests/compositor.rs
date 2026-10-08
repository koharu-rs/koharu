#![cfg(feature = "native")]

use std::{collections::HashMap, sync::Arc};

use koharu_rasterizer::{
    Bounds, LayerId, LayerKind, Point, PreparedContent, PreparedFrame, PreparedFrameBundle,
    PreparedLayer, PreparedRaster, PreparedRasterTile, PreparedResource, Presentation,
    RasterOptions, Rasterizer, Revision,
};
use vello::Scene;

#[test]
fn linear_filtering_does_not_add_borders_to_any_raster_layer() {
    let background =
        PreparedResource::encoded_raster(3, 1, "image/png", Arc::from(&b"background"[..])).unwrap();
    let background_id = background.id();
    let resource =
        PreparedResource::encoded_raster(2, 1, "image/png", Arc::from(&b"foreground"[..])).unwrap();
    let resource_id = resource.id();
    let frame = PreparedFrameBundle {
        frame: PreparedFrame {
            revision: Revision::new(1),
            page: LayerId::from_bytes([1; 16]),
            width: 3,
            height: 1,
            origin: (0, 0),
            normalization: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            layers: vec![
                PreparedLayer {
                    id: LayerId::from_bytes([2; 16]),
                    geometry: vec![
                        Point { x: 0.0, y: 0.0 },
                        Point { x: 3.0, y: 0.0 },
                        Point { x: 3.0, y: 1.0 },
                        Point { x: 0.0, y: 1.0 },
                    ],
                    bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 3.0,
                        height: 1.0,
                    },
                    local_bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 3.0,
                        height: 1.0,
                    },
                    presentation: Presentation {
                        visible: true,
                        opacity: 1.0,
                    },
                    kind: LayerKind::Raster,
                    placement: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    content: PreparedContent::Raster(PreparedRaster {
                        source: background_id,
                        width: 3,
                        height: 1,
                        tiles: vec![PreparedRasterTile {
                            x: 0,
                            y: 0,
                            width: 3,
                            height: 1,
                            gutter: [0; 4],
                        }],
                    }),
                    element_frame: None,
                },
                PreparedLayer {
                    id: LayerId::from_bytes([3; 16]),
                    geometry: vec![
                        Point { x: 0.5, y: 0.0 },
                        Point { x: 2.5, y: 0.0 },
                        Point { x: 2.5, y: 1.0 },
                        Point { x: 0.5, y: 1.0 },
                    ],
                    bounds: Bounds {
                        x: 0.5,
                        y: 0.0,
                        width: 2.0,
                        height: 1.0,
                    },
                    local_bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 2.0,
                        height: 1.0,
                    },
                    presentation: Presentation {
                        visible: true,
                        opacity: 1.0,
                    },
                    kind: LayerKind::Raster,
                    placement: [1.0, 0.0, 0.0, 1.0, 0.5, 0.0],
                    content: PreparedContent::Raster(PreparedRaster {
                        source: resource_id,
                        width: 2,
                        height: 1,
                        tiles: vec![PreparedRasterTile {
                            x: 0,
                            y: 0,
                            width: 2,
                            height: 1,
                            gutter: [0; 4],
                        }],
                    }),
                    element_frame: None,
                },
            ],
        },
        resources: vec![background, resource],
    }
    .into_frame_with_raster_sources(&HashMap::from([
        (background_id, Arc::from(&[255; 12][..])),
        (resource_id, Arc::from(&[255, 0, 0, 255, 0, 0, 0, 0][..])),
    ]))
    .unwrap();

    let raster = Rasterizer::new()
        .unwrap()
        .rasterize(&frame, RasterOptions::default())
        .unwrap();

    let edge = raster.image.get_pixel(1, 0).0;
    assert_eq!(edge[0], u8::MAX);
    assert!((126..=129).contains(&edge[1]));
    assert!((126..=129).contains(&edge[2]));
    assert_eq!(edge[3], u8::MAX);
}

#[test]
fn rasterizes_surfaces_beyond_the_default_wgpu_texture_limit() {
    // WGPU's default limits cap textures at 8192 px, far below what desktop
    // GPUs support, and tall scans or spreads exceed it.
    let image = Rasterizer::new()
        .unwrap()
        .rasterize_scene(
            &Scene::new(),
            4,
            9000,
            [10, 20, 30, 255],
            RasterOptions::default(),
        )
        .unwrap();

    assert_eq!(image.dimensions(), (4, 9000));
    assert_eq!(image.get_pixel(3, 8999).0, [10, 20, 30, 255]);
}

fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

fn tiled_layer(
    id: u8,
    source: koharu_rasterizer::ResourceId,
    width: u32,
    height: u32,
) -> PreparedLayer {
    let dimension = koharu_rasterizer::PREPARED_RASTER_TILE_DIMENSION;
    let mut tiles = Vec::new();
    for y in (0..height).step_by(dimension as usize) {
        for x in (0..width).step_by(dimension as usize) {
            let tile_width = (width - x).min(dimension);
            let tile_height = (height - y).min(dimension);
            tiles.push(PreparedRasterTile {
                x,
                y,
                width: tile_width,
                height: tile_height,
                gutter: [
                    u32::from(x > 0),
                    u32::from(y > 0),
                    u32::from(x + tile_width < width),
                    u32::from(y + tile_height < height),
                ],
            });
        }
    }
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: width as f32,
        height: height as f32,
    };
    PreparedLayer {
        id: LayerId::from_bytes([id; 16]),
        geometry: vec![
            Point { x: 0.0, y: 0.0 },
            Point {
                x: f64::from(width),
                y: 0.0,
            },
            Point {
                x: f64::from(width),
                y: f64::from(height),
            },
            Point {
                x: 0.0,
                y: f64::from(height),
            },
        ],
        bounds,
        local_bounds: bounds,
        presentation: Presentation {
            visible: true,
            opacity: 1.0,
        },
        kind: LayerKind::Raster,
        placement: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        content: PreparedContent::Raster(PreparedRaster {
            source,
            width,
            height,
            tiles,
        }),
        element_frame: None,
    }
}

#[test]
fn tiled_layers_composite_like_premultiplied_source_over() {
    // Two rows and columns of tiles per layer, so the frame crosses every kind of tile seam.
    let (width, height) = (1_300, 1_100);
    let len = (width * height * 4) as usize;
    let mut below = noise(1, len);
    for pixel in below.as_chunks_mut::<4>().0 {
        pixel[3] = u8::MAX;
    }
    let above = noise(2, len);
    let below_resource =
        PreparedResource::encoded_raster(width, height, "image/png", Arc::from(&b"below"[..]))
            .unwrap();
    let above_resource =
        PreparedResource::encoded_raster(width, height, "image/png", Arc::from(&b"above"[..]))
            .unwrap();
    let (below_id, above_id) = (below_resource.id(), above_resource.id());
    let frame = PreparedFrameBundle {
        frame: PreparedFrame {
            revision: Revision::new(1),
            page: LayerId::from_bytes([1; 16]),
            width,
            height,
            origin: (0, 0),
            normalization: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            layers: vec![
                tiled_layer(2, below_id, width, height),
                tiled_layer(3, above_id, width, height),
            ],
        },
        resources: vec![below_resource, above_resource],
    }
    .into_frame_with_raster_sources(&HashMap::from([
        (below_id, Arc::from(&below[..])),
        (above_id, Arc::from(&above[..])),
    ]))
    .unwrap();

    let raster = Rasterizer::new()
        .unwrap()
        .rasterize(&frame, RasterOptions::default())
        .unwrap();

    let output = raster.image.as_raw();
    assert_eq!(output.len(), len);
    for (index, ((output, below), above)) in output
        .as_chunks::<4>()
        .0
        .iter()
        .zip(below.as_chunks::<4>().0)
        .zip(above.as_chunks::<4>().0)
        .enumerate()
    {
        let alpha = u32::from(above[3]);
        for channel in 0..3 {
            let premultiplied = (u32::from(above[channel]) * alpha + 127) / 255;
            let expected = premultiplied + u32::from(below[channel]) * (255 - alpha) / 255;
            assert!(
                u32::from(output[channel]).abs_diff(expected) <= 1,
                "pixel {index} channel {channel}: {} != {expected}",
                output[channel]
            );
        }
        assert_eq!(output[3], u8::MAX);
    }
}
