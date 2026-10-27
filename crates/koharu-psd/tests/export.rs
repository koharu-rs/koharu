//! Exporting a page and reading the PSD back with photocraft-psd: the layer
//! tree, each layer's properties and masks, and the blocks that keep shapes
//! and text editable.

use koharu_psd::export_page;
use koharu_renderer::{Renderer, TypesettingConfig};
use koharu_scene::{Author, Edit, Op, Session, StyleId};
use photocraft_psd::{
    BlendMode, LayerRecord, MaskData, PsdFile, SectionType, Version, VersionedDescriptor,
    descriptor::Value, path::VectorMaskBlock,
};
use serde_json::{Value as Json, json};

/// A 200×200 page: paint limited by two path masks, a balloon group with a
/// pixel mask holding a shape and paragraph text, and hidden point text on two
/// lines in two sizes with a stroke.
fn poster() -> (tempfile::TempDir, Session, koharu_scene::PageId) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::create(dir.path().join("test.koharu")).unwrap();
    let dialogue = StyleId::new();
    let white = [1.0, 1.0, 1.0, 1.0];
    let red = [200u8, 30, 30, 255].repeat(64 * 64);
    let band = vec![255u8; 200 * 64];
    let ops: Vec<Json> = vec![
        json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 200, "height": 200}}),
        json!({"op": "add_text_style", "id": dialogue, "style": {"name": "Dialogue", "style": {"chars": {"size": 20.0}, "align": "center"}}}),
        json!({"op": "add_layer", "id": "$paint", "page": "$page", "index": 0, "layer": {
            "kind": "raster", "name": "Paint", "opacity": 0.5, "blend": "multiply",
        }}),
        json!({"op": "write_pixels", "layer": "$paint", "rect": {"x": 0, "y": 0, "width": 64, "height": 64}, "pixels": red}),
        json!({"op": "add_layer", "page": "$page", "parent": "$paint", "index": 0, "layer": {
            "kind": "mask", "content": {"path": {"path": "M0 0 H48 V64 H0 Z"}},
        }}),
        json!({"op": "add_layer", "page": "$page", "parent": "$paint", "index": 1, "layer": {
            "kind": "mask", "mode": "subtract", "content": {"path": {"path": "M0 0 H16 V16 H0 Z"}},
        }}),
        json!({"op": "add_layer", "id": "$balloon", "page": "$page", "index": 1, "layer": {"kind": "group", "name": "Balloon"}}),
        json!({"op": "add_layer", "page": "$page", "parent": "$balloon", "index": 0, "layer": {
            "kind": "shape", "name": "Outline", "path": "M40 100 A60 40 0 1 0 160 100 A60 40 0 1 0 40 100 Z",
            "fill": white, "stroke": {"color": [0.0, 0.0, 0.0, 1.0], "width": 2.0},
        }}),
        json!({"op": "add_layer", "page": "$page", "parent": "$balloon", "index": 1, "layer": {
            "kind": "text", "name": "Line", "content": "HELLO THERE", "text_style": dialogue, "language": "en",
            "frame": {"box": {"bounds": {"x": 60.0, "y": 80.0, "width": 80.0, "height": 40.0}}},
        }}),
        json!({"op": "add_layer", "id": "$cut", "page": "$page", "parent": "$balloon", "index": 0, "layer": {
            "kind": "mask", "content": {"pixels": {"offset": {"x": 0, "y": 0}}},
        }}),
        json!({"op": "write_pixels", "layer": "$cut", "rect": {"x": 0, "y": 64, "width": 200, "height": 64}, "pixels": band}),
        json!({"op": "add_layer", "page": "$page", "index": 2, "layer": {
            "kind": "text", "name": "Sign", "visible": false, "language": "en",
            "transform": [[1.0, 0.0, 20.0], [0.0, 1.0, 160.0]],
            "style": {"chars": {"stroke": white, "stroke_width": 2.0}},
            "content": [{"insert": "BIG\r\n", "attributes": {"size": 24.0}}, {"insert": "small", "attributes": {"size": 12.0}}],
        }}),
        // Locked last: a locked layer takes no more writes.
        json!({"op": "update_layer", "layer": "$paint", "patch": {"locked": true}}),
    ];
    let change = session
        .apply(Edit {
            project: session.id(),
            label: "test".into(),
            author: Author::User { via: None },
            reads: Vec::new(),
            ops: ops
                .into_iter()
                .map(|op| serde_json::from_value::<Op>(op).unwrap())
                .collect(),
        })
        .unwrap();
    (dir, session, change.pages.added[0])
}

fn record<'a>(file: &'a PsdFile, name: &str) -> &'a LayerRecord {
    file.layers()
        .iter()
        .find(|record| record.name() == name)
        .unwrap()
}

/// A type layer's text descriptor: after the version, the transform, and the
/// text version.
fn type_text(record: &LayerRecord) -> VersionedDescriptor {
    let data = &record.block(b"TySh").unwrap().data;
    VersionedDescriptor::parse_prefix(&data[2 + 48 + 2..])
        .unwrap()
        .0
}

fn engine_data(record: &LayerRecord) -> String {
    let text = type_text(record);
    let Some(Value::RawData(bytes)) = text.descriptor.get("EngineData") else {
        panic!("no EngineData");
    };
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_page_exports_as_layers_photoshop_can_edit() {
    let (_dir, session, page) = poster();
    let renderer = Renderer::new(TypesettingConfig::default());
    if let Err(error) = renderer.gpu() {
        eprintln!("skipped without a GPU: {error}");
        return;
    }
    let bytes = export_page(&session, &renderer, page).unwrap();
    let file = PsdFile::from_bytes(&bytes).unwrap();
    assert_eq!((file.header.width, file.header.height), (200, 200));
    assert_eq!(file.has_real_merged_data(), Some(true));

    // Bottom to top: the paint, the balloon folder over its divider, and the
    // sign; the balloon holds the outline and the line.
    let names: Vec<String> = file.layers().iter().map(LayerRecord::name).collect();
    assert_eq!(
        names,
        [
            "Paint",
            "</Layer group>",
            "Outline",
            "Line",
            "Balloon",
            "Sign"
        ]
    );

    let paint = record(&file, "Paint");
    assert_eq!(
        (paint.blend_mode, paint.opacity),
        (BlendMode::Multiply, 128)
    );
    assert_eq!(
        paint.block(b"lspf").unwrap().data,
        0x8000_0000u32.to_be_bytes()
    );
    let vector = VectorMaskBlock::from_bytes(&paint.block(b"vmsk").unwrap().data).unwrap();
    let operations: Vec<i16> = vector
        .path
        .subpaths()
        .iter()
        .map(|subpath| subpath.operation)
        .collect();
    assert_eq!(operations, [1, 2]);
    assert!(!vector.path.initial_fill());
    // Its own pixels are what it painted, at its place.
    assert_eq!(
        (
            paint.rect.left,
            paint.rect.top,
            paint.rect.right,
            paint.rect.bottom
        ),
        (0, 0, 64, 64)
    );

    let balloon = record(&file, "Balloon");
    assert_eq!(balloon.section_type(), SectionType::OpenFolder);
    assert_eq!(balloon.blend_mode, BlendMode::PassThrough);
    let MaskData::Mask(mask) = &balloon.mask else {
        panic!("the balloon's pixel mask is its user mask");
    };
    assert_eq!(
        (
            mask.rect.top,
            mask.rect.bottom,
            mask.rect.left,
            mask.rect.right
        ),
        (64, 128, 0, 200)
    );

    let outline = record(&file, "Outline");
    for key in [b"vmsk", b"vscg", b"vstk"] {
        assert!(
            outline.block(key).is_some(),
            "{}",
            String::from_utf8_lossy(key)
        );
    }

    let line = record(&file, "Line");
    let text = type_text(line);
    let Some(Value::Text(content)) = text.descriptor.get("Txt ") else {
        panic!("no text");
    };
    assert_eq!(content.to_string_lossy(), "HELLO THERE");
    let data = engine_data(line);
    assert!(
        data.contains("/ShapeType 1") && data.contains("/Justification 2"),
        "{data}"
    );

    // Two sizes make two style runs, which reach the closing paragraph mark;
    // the \r\n is one paragraph mark.
    let sign = record(&file, "Sign");
    assert!(!sign.is_visible());
    assert!(sign.block(b"lfx2").is_some());
    // Its pixels are the black glyphs alone, which the effect strokes white.
    let red = sign.decode_channel(0, 8, Version::Psd).unwrap();
    assert!(
        red.iter().all(|&value| value < 128),
        "a stroke is in the pixels"
    );
    let data = engine_data(sign);
    let runs = data.split("/RunLengthArray").nth(2).unwrap();
    assert!(runs.trim_start().starts_with("[ 4 6 ]"), "{runs}");
    assert!(data.contains("/ShapeType 0"));
}
