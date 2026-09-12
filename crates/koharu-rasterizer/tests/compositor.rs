#![cfg(feature = "native")]

use std::{collections::HashMap, sync::Arc};

use koharu_rasterizer::{
    Bounds, LayerId, LayerKind, Point, PreparedContent, PreparedFrame, PreparedFrameBundle,
    PreparedLayer, PreparedRaster, PreparedRasterTile, PreparedResource, Presentation,
    RasterOptions, Rasterizer, Revision,
};

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
fn reports_selected_export_adapter() {
    let rasterizer = Rasterizer::new().unwrap();
    let info = rasterizer.adapter_info();
    println!(
        "Export GPU: {} ({:?}, {:?})",
        info.name, info.device_type, info.backend
    );
    assert!(!info.name.is_empty());
}
