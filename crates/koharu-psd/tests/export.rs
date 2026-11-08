//! Exporting a page and reading the PSD back with photocraft-psd: the layer
//! tree, each layer's properties and masks, and the blocks that keep shapes
//! and text editable.

use std::io::Cursor;

use koharu_psd::{Labels, export};
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::{Edit, Hash, Op, Session};
use photocraft_psd::{
    BlendMode, LayerRecord, MaskData, PsdFile, SectionType, Version, VersionedDescriptor,
    descriptor::Value,
};
use serde_json::{Value as Json, json};

fn labels() -> Labels {
    Labels {
        background: "Background".into(),
        cleanup: "Clean-up".into(),
        outline: "Balloon outline".into(),
    }
}

fn block(name: &str, x: f64, source: &str) -> Json {
    json!({"kind": "block", "name": name, "source": {
        "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 30.0, "height": 40.0}}},
        "content": source, "direction": "vertical", "language": "ja",
    }})
}

fn translate(block: &str, text: &str, x: f64) -> [Json; 2] {
    [
        json!({"op": "set_text", "layer": block, "text": text}),
        json!({"op": "update_layer", "layer": block, "patch": {"translation": {
            "language": "en", "style": {"chars": {"size": 12.0}},
            "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 40.0, "height": 40.0}}},
        }}}),
    ]
}

/// A 200×200 page made from a scan: paint limited by two masks, a group with
/// a mask holding a shape and paragraph text, hidden point text on
/// two lines in two sizes with a stroke, a panel holding a balloon with two
/// blocks, and a free block on the page.
fn poster() -> (tempfile::TempDir, Session, koharu_scene::PageId) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
    let mut scan = Vec::new();
    image::RgbaImage::from_pixel(200, 200, image::Rgba([240, 240, 240, 255]))
        .write_to(&mut Cursor::new(&mut scan), image::ImageFormat::Png)
        .expect("a PNG");
    let file = Hash::of(&scan);
    let white = [1.0, 1.0, 1.0, 1.0];
    let red = [200u8, 30, 30, 255].repeat(64 * 64);
    let band = vec![255u8; 200 * 64];
    let clean = vec![255u8; 30 * 40 * 4];
    let glyphs = vec![255u8; 30 * 40];
    let mut ops: Vec<Json> = vec![
        json!({"op": "add_file", "bytes": scan}),
        json!({"op": "add_page", "id": "$page", "index": 0, "background": file}),
        json!({"op": "add_layer", "id": "$paint", "page": "$page", "index": 0, "layer": {
            "kind": "raster", "name": "Paint", "opacity": 0.5, "blend": "multiply",
        }}),
        json!({"op": "write_pixels", "layer": "$paint", "rect": {"x": 0, "y": 0, "width": 64, "height": 64}, "pixels": red}),
        json!({"op": "add_layer", "id": "$keep", "page": "$page", "parent": "$paint", "index": 0, "layer": {"kind": "mask"}}),
        json!({"op": "write_pixels", "layer": "$keep", "rect": {"x": 0, "y": 0, "width": 48, "height": 64}, "pixels": vec![255u8; 48 * 64]}),
        json!({"op": "add_layer", "id": "$trim", "page": "$page", "parent": "$paint", "index": 1, "layer": {"kind": "mask", "mode": "subtract"}}),
        json!({"op": "write_pixels", "layer": "$trim", "rect": {"x": 32, "y": 0, "width": 32, "height": 64}, "pixels": vec![255u8; 32 * 64]}),
        json!({"op": "add_layer", "id": "$balloon", "page": "$page", "index": 1, "layer": {"kind": "group", "name": "Balloon"}}),
        json!({"op": "add_layer", "page": "$page", "parent": "$balloon", "index": 0, "layer": {
            "kind": "shape", "name": "Outline", "path": "M40 100 A60 40 0 1 0 160 100 A60 40 0 1 0 40 100 Z",
            "fill": white, "stroke": {"color": [0.0, 0.0, 0.0, 1.0], "width": 2.0},
        }}),
        json!({"op": "add_layer", "page": "$page", "parent": "$balloon", "index": 1, "layer": {
            "kind": "text", "name": "Line", "content": "HELLO THERE", "language": "en",
            "style": {"chars": {"size": 20.0}, "align": "center"},
            "frame": {"box": {"bounds": {"x": 60.0, "y": 80.0, "width": 80.0, "height": 40.0}}},
        }}),
        json!({"op": "add_layer", "id": "$cut", "page": "$page", "parent": "$balloon", "index": 0, "layer": {"kind": "mask"}}),
        json!({"op": "write_pixels", "layer": "$cut", "rect": {"x": 0, "y": 64, "width": 200, "height": 64}, "pixels": band}),
        json!({"op": "add_layer", "page": "$page", "index": 2, "layer": {
            "kind": "text", "name": "Sign", "visible": false, "language": "en",
            "transform": [[1.0, 0.0, 20.0], [0.0, 1.0, 160.0]],
            "style": {"chars": {"stroke": white, "stroke_width": 2.0}},
            "content": [{"insert": "BIG\r\n", "attributes": {"size": 24.0}}, {"insert": "small", "attributes": {"size": 12.0}}],
        }}),
        json!({"op": "add_layer", "id": "$panel", "page": "$page", "index": 3, "layer": {"kind": "panel", "name": "Panel", "outline": "M100 0 H200 V100 H100 Z"}}),
        json!({"op": "add_layer", "id": "$speech", "page": "$page", "parent": "$panel", "index": 0, "layer": {"kind": "balloon", "name": "Speech", "outline": "M110 10 H190 V90 H110 Z"}}),
        json!({"op": "add_layer", "id": "$first", "page": "$page", "parent": "$speech", "index": 0, "layer": block("First", 150.0, "最初")}),
        json!({"op": "add_layer", "id": "$second", "page": "$page", "parent": "$speech", "index": 1, "layer": block("Second", 115.0, "二番目")}),
        json!({"op": "add_layer", "id": "$free", "page": "$page", "index": 4, "layer": block("Free", 10.0, "自由")}),
        json!({"op": "write_pixels", "layer": "$first.cleanup", "rect": {"x": 150, "y": 20, "width": 30, "height": 40}, "pixels": clean}),
        json!({"op": "write_pixels", "layer": "$first.mask", "rect": {"x": 150, "y": 20, "width": 30, "height": 40}, "pixels": glyphs}),
    ];
    ops.extend(translate("$first", "First line", 145.0));
    ops.extend(translate("$second", "Second line", 110.0));
    ops.extend(translate("$free", "Free line", 5.0));
    // Locked last: a locked layer takes no more writes.
    ops.push(json!({"op": "update_layer", "layer": "$paint", "patch": {"locked": true}}));
    let change = session
        .apply(Edit {
            label: "test".into(),
            ops: ops
                .into_iter()
                .map(|op| serde_json::from_value::<Op>(op).expect("a valid op"))
                .collect(),
        })
        .expect("an edit");
    (dir, session, change.pages.added[0])
}

fn record<'a>(file: &'a PsdFile, name: &str) -> &'a LayerRecord {
    file.layers()
        .iter()
        .find(|record| record.name() == name)
        .expect("a record of that name")
}

/// A type layer's text descriptor: after the version, the transform, and the
/// text version.
fn type_text(record: &LayerRecord) -> VersionedDescriptor {
    let data = &record.block(b"TySh").expect("a type layer").data;
    VersionedDescriptor::parse_prefix(&data[2 + 48 + 2..])
        .expect("a descriptor")
        .0
}

fn words(record: &LayerRecord) -> String {
    let text = type_text(record);
    let Some(Value::Text(content)) = text.descriptor.get("Txt ") else {
        panic!("no text");
    };
    content.to_string_lossy()
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
    let renderer = Renderer::new(RendererConfig::default());
    if let Err(error) = renderer.gpu() {
        eprintln!("skipped without a GPU: {error}");
        return;
    }
    let bytes = export(&session, &renderer, page, &labels()).expect("a PSD");
    let file = PsdFile::from_bytes(&bytes).expect("it reads back");
    assert_eq!((file.header.width, file.header.height), (200, 200));
    assert_eq!(file.has_real_merged_data(), Some(true));

    // Bottom to top, each folder over its divider: the scan, the paint, the
    // group, the sign, then the panel with its balloon of two blocks, and a
    // free block. A block holds its clean-up, its source, and its translation.
    let names: Vec<String> = file.layers().iter().map(LayerRecord::name).collect();
    assert_eq!(
        names,
        [
            "Background",
            "Paint",
            "</Layer group>",
            "Outline",
            "Line",
            "Balloon",
            "Sign",
            "</Layer group>",
            "</Layer group>",
            "Balloon outline",
            "</Layer group>",
            "Clean-up",
            "最初",
            "First line",
            "First",
            "</Layer group>",
            "Clean-up",
            "二番目",
            "Second line",
            "Second",
            "Speech",
            "Panel",
            "</Layer group>",
            "Clean-up",
            "自由",
            "Free line",
            "Free",
        ]
    );
    assert!(file.layers()[0].is_visible());

    let paint = record(&file, "Paint");
    assert_eq!(
        (paint.blend_mode, paint.opacity),
        (BlendMode::Multiply, 128)
    );
    assert_eq!(
        paint.block(b"lspf").expect("a lock").data,
        0x8000_0000u32.to_be_bytes()
    );
    // Its masks, one adding and one subtracting, merge into its one user
    // mask, cropped to what shows.
    assert!(paint.block(b"vmsk").is_none());
    let MaskData::Mask(mask) = &paint.mask else {
        panic!("the paint's masks merge into its user mask");
    };
    assert_eq!(
        (
            mask.rect.left,
            mask.rect.top,
            mask.rect.right,
            mask.rect.bottom
        ),
        (0, 0, 32, 64)
    );
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
        panic!("the group's mask is its user mask");
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
    assert_eq!(words(line), "HELLO THERE");
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
    let red = sign.decode_channel(0, 8, Version::Psd).expect("pixels");
    assert!(
        red.iter().all(|&value| value < 128),
        "a stroke is in the pixels"
    );
    let data = engine_data(sign);
    let runs = data.split("/RunLengthArray").nth(2).expect("runs");
    assert!(runs.trim_start().starts_with("[ 4 6 ]"), "{runs}");
    assert!(data.contains("/ShapeType 0"));

    // A panel is a folder masked by its outline; a balloon's outline is a
    // hidden shape inside its folder.
    let panel = record(&file, "Panel");
    assert_eq!(panel.section_type(), SectionType::OpenFolder);
    assert!(
        panel.block(b"vmsk").is_some(),
        "the panel clips by its outline"
    );
    assert!(!record(&file, "Balloon outline").is_visible());
    assert_eq!(
        record(&file, "Speech").section_type(),
        SectionType::OpenFolder
    );

    // A block's clean-up shows through its mask, its source is a hidden type
    // layer, and its translation a type layer.
    let first_cleanup = file
        .layers()
        .iter()
        .position(|record| record.name() == "最初")
        .map(|at| &file.layers()[at - 1])
        .expect("the first block's clean-up");
    let MaskData::Mask(mask) = &first_cleanup.mask else {
        panic!("a clean-up's user mask is its block's mask");
    };
    assert_eq!(
        (
            mask.rect.left,
            mask.rect.top,
            mask.rect.right,
            mask.rect.bottom
        ),
        (150, 20, 180, 60)
    );
    for (source, translation) in [
        ("最初", "First line"),
        ("二番目", "Second line"),
        ("自由", "Free line"),
    ] {
        let source_record = record(&file, source);
        assert!(!source_record.is_visible(), "{source}");
        assert_eq!(words(source_record), source);
        let translation_record = record(&file, translation);
        assert!(translation_record.is_visible());
        assert_eq!(words(translation_record), translation);
    }
}
