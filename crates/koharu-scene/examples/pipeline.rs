//! The translation pipeline as data: detection finds a balloon and paints a
//! text mask, OCR reads the scan, typesetting sets the page's dialogue style,
//! translation fills the balloon, and inpainting cleans under the mask.
//! Workflow data goes in `attributes`, and every model names what it read.
//!
//! `cargo run -p koharu-scene --example pipeline`

use koharu_scene::{
    Action, Author, Change, Created, Edit, Hash, Layer, LayerId, LayerView, Op, Pointer, Read,
    Rect, Ref, Relationship, Session, Size, SoftwareAgent, StyleId,
};
use serde_json::{Map, Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SIZE: u32 = 256;
const BALLOON: &str = "M128 64 C200 64 200 160 128 160 C56 160 56 64 128 64 Z";

fn model(action: &str, name: &str) -> Author {
    Author::Model {
        action: Action {
            action: action.to_owned(),
            software_agent: SoftwareAgent {
                name: name.to_owned(),
                version: None,
            },
            digital_source_type: None,
            parameters: Map::new(),
        },
    }
}

fn user() -> Author {
    Author::User { via: None }
}

fn edit(session: &Session, author: Author, label: &str, ops: Vec<Op>) -> Edit {
    Edit {
        project: session.id(),
        label: label.to_owned(),
        author,
        reads: Vec::new(),
        ops,
    }
}

fn label(name: &str) -> Ref {
    Ref::try_from(format!("${name}")).expect("a valid label")
}

/// Adds a layer named `$id` that read `ingredients`.
fn add(
    page: Ref,
    parent: Option<&str>,
    index: usize,
    id: &str,
    layer: Value,
    ingredients: &[Read],
) -> Op {
    Op::AddLayer {
        id: Some(label(id)),
        page,
        parent: parent.map(label),
        index,
        layer: serde_json::from_value(layer).expect("a valid layer"),
        ingredients: ingredients.to_vec(),
    }
}

fn op(value: Value) -> Op {
    serde_json::from_value(value).expect("a valid op")
}

fn created(change: &Change, name: &str) -> LayerId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, label } if label.as_deref() == Some(name) => Some(*id),
            _ => None,
        })
        .expect("the layer was created")
}

/// Every layer on a page, parents first.
fn walk(layers: &[LayerView]) -> Vec<&LayerView> {
    layers
        .iter()
        .flat_map(|view| {
            std::iter::once(view)
                .chain(walk(&view.children))
                .chain(walk(&view.masks))
        })
        .collect()
}

/// The layers a model created: its action wrote all of the layer, `""`.
fn made_by<'a>(layers: &[&'a LayerView], agent: &str) -> Vec<&'a LayerView> {
    layers
        .iter()
        .filter(|view| {
            view.origin.iter().any(|record| {
                record.action.software_agent.name == agent
                    && record.writes.iter().any(|path| path.is_root())
            })
        })
        .copied()
        .collect()
}

/// The layer a result was derived from: its `parentOf` ingredient.
fn parent_of(view: &LayerView) -> Option<LayerId> {
    view.origin
        .iter()
        .flat_map(|record| &record.ingredients)
        .find(|read| read.relationship == Relationship::ParentOf)
        .map(|read| read.layer)
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    let scan = b"scan bytes".to_vec();
    let change = session.apply(edit(
        &session,
        user(),
        "Import",
        vec![
            Op::AddFile {
                media_type: "image/png".to_owned(),
                bytes: scan.clone(),
            },
            Op::AddPage {
                id: Some(label("page")),
                index: 0,
                size: Size {
                    width: SIZE,
                    height: SIZE,
                },
            },
            add(
                label("page"),
                None,
                0,
                "scan",
                json!({"kind": "image", "name": "Scan", "file": Hash::of(&scan)}),
                &[],
            ),
        ],
    ))?;
    let (page, scan) = (change.pages.added[0], created(&change, "scan"));

    // Detection reads the scan: the file its image layer names.
    let (file, from_scan) = session.read(scan, Pointer::from_static("/file"))?;
    let bytes = session.file(&serde_json::from_value::<Hash>(file)?)?.bytes;
    assert!(!bytes.is_empty(), "a detector would decode these");
    let from_scan = [from_scan.parent_of()];
    let detector = model("c2pa.created", "detector");
    let change = session.apply(edit(&session, detector.clone(), "Detect", vec![
        // The balloon it found. What only the workflow needs goes in
        // attributes, which the renderer never reads.
        add(page.into(), None, 1, "balloon", json!({
            "kind": "shape", "name": "Balloon", "path": BALLOON, "fill": [1.0, 1.0, 1.0, 1.0],
            "attributes": {"koharu": {"region": "bubble", "confidence": 0.93}},
        }), &from_scan),
        // A raster for the cleanup, under the balloon, limited by a pixel
        // mask of the text.
        add(page.into(), None, 1, "cleanup", json!({"kind": "raster", "name": "Cleanup"}), &from_scan),
        add(page.into(), Some("cleanup"), 0, "mask", json!({
            "kind": "mask", "name": "Text mask", "attributes": {"koharu": {"mask": "text"}},
        }), &from_scan),
    ]))?;
    let (balloon, cleanup, mask) = (
        created(&change, "balloon"),
        created(&change, "cleanup"),
        created(&change, "mask"),
    );
    // A pixel mask is 8-bit grey: 255 where the text is.
    let (x, y, width, height) = (96, 96, 64, 32);
    session.apply(edit(
        &session,
        detector.clone(),
        "Detect",
        vec![Op::WritePixels {
            layer: mask.into(),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            pixels: vec![255; (width * height) as usize],
        }],
    ))?;
    println!("detection found a balloon and painted a {width}×{height} text mask");

    // OCR reads the scan, and the balloon as context, and writes the source
    // text hidden: only the translation shows.
    let (_, from_file) = session.read(scan, Pointer::from_static("/file"))?;
    let (_, outline) = session.read(balloon, Pointer::from_static("/path"))?;
    let change = session.apply(edit(&session, model("c2pa.created", "ocr"), "Read text", vec![add(
        page.into(), None, 3, "source",
        json!({
            "kind": "text", "content": "こんにちは", "language": "ja", "direction": "vertical", "visible": false,
            "style": {"chars": {"size": 14.0}},
            "attributes": {"koharu": {"role": "dialogue"}},
        }),
        &[from_file.parent_of(), outline],
    )]))?;

    // Typesetting takes the page's dialogue size from the source text, not
    // from each balloon, and puts it in one style that every translation
    // names, so one edit resizes them all.
    let (_, measured) = session.read(created(&change, "source"), Pointer::from_static("/style"))?;
    let dialogue = StyleId::new();
    session.apply(edit(&session, model("c2pa.created", "typesetter"), "Typeset", vec![Op::AddTextStyle {
        id: Some(dialogue.into()),
        style: serde_json::from_value(json!({
            "name": "Dialogue", "style": {"chars": {"size": 14.0}, "align": "center", "line_height": 1.05},
        }))?,
        ingredients: vec![measured.parent_of()],
    }]))?;

    // Translation finds, from the page alone, each source with no translation
    // yet: a translation names its source as its `parentOf`.
    let view = session.page(page)?;
    let layers = walk(&view.layers);
    let translated: Vec<LayerId> = layers.iter().filter_map(|view| parent_of(view)).collect();
    let pending: Vec<LayerId> = made_by(&layers, "ocr")
        .iter()
        .map(|view| view.id)
        .filter(|source| !translated.contains(source))
        .collect();
    assert_eq!(pending.len(), 1);
    for source in pending {
        let (_, text) = session.read(source, Pointer::from_static("/content"))?;
        let (path, outline) = session.read(balloon, Pointer::from_static("/path"))?;
        // The translation fills the balloon: its frame is the balloon's path,
        // centred, and it shrinks only if it would overflow.
        session.apply(edit(
            &session,
            model("c2pa.translated", "translator"),
            "Translate",
            vec![add(
                page.into(),
                None,
                4,
                "translation",
                json!({
                    "kind": "text", "content": "Hello", "language": "en", "text_style": dialogue,
                    "frame": {"path": {"path": path, "anchor": "center"}},
                }),
                &[text.parent_of(), outline],
            )],
        ))?;
    }
    let view = session.page(page)?;
    let layers = walk(&view.layers);
    let pairs: Vec<(LayerId, LayerId)> = made_by(&layers, "translator")
        .iter()
        .filter_map(|view| Some((view.id, parent_of(view)?)))
        .collect();
    assert_eq!(pairs.len(), 1);
    println!("layer {} translates layer {}", pairs[0].0, pairs[0].1);

    // The user sets dialogue larger once; the change stays under the
    // typesetter's reruns.
    session.apply(edit(&session, user(), "Resize dialogue", vec![op(json!({
        "op": "update_text_style", "style": dialogue, "patch": {"style": {"chars": {"size": 16.0}}},
    }))]))?;
    let style = session.style(dialogue)?;
    assert_eq!(style.overrides.len(), 1);
    println!(
        "every text naming {:?} is now set at {:?} px",
        style.style.name, style.style.style.chars.size
    );

    // Inpainting reads the mask's pixels and paints the cleanup under them.
    // Pixels are not a property, so it can't name the mask as an ingredient.
    let tiles = session.tiles(mask, None)?;
    let mut pixels = vec![0; (SIZE * SIZE * 4) as usize];
    for tile in &tiles.tiles {
        for (index, &value) in tile.pixels.iter().enumerate() {
            let (px, py) = (
                tile.x * 64 + index as i32 % 64,
                tile.y * 64 + index as i32 / 64,
            );
            if value > 0 && (0..SIZE as i32).contains(&px) && (0..SIZE as i32).contains(&py) {
                let at = (py as usize * SIZE as usize + px as usize) * 4;
                pixels[at..at + 4].copy_from_slice(&[250, 250, 250, 255]);
            }
        }
    }
    session.apply(edit(
        &session,
        model("c2pa.edited", "inpainter"),
        "Inpaint",
        vec![Op::WritePixels {
            layer: cleanup.into(),
            rect: Rect {
                x: 0,
                y: 0,
                width: SIZE,
                height: SIZE,
            },
            pixels,
        }],
    ))?;
    println!(
        "inpainting painted {} tiles under the mask",
        session.tiles(cleanup, None)?.tiles.len()
    );

    // The user restyles the balloon. Detection then reruns, finds what it
    // made before from the page, and replaces it; what the user changed stays.
    session.apply(edit(
        &session,
        user(),
        "Restyle",
        vec![op(json!({
            "op": "update_layer", "layer": balloon, "patch": {"fill": [1.0, 0.95, 0.8, 1.0]},
        }))],
    ))?;
    let view = session.page(page)?;
    // A mask goes with the layer it limits, so only that layer is removed.
    let ops = made_by(&walk(&view.layers), "detector")
        .iter()
        .filter(|view| !matches!(view.layer, Layer::Mask(_)))
        .map(|view| op(json!({"op": "remove_layer", "layer": view.id})))
        .collect();
    let change = session.apply(edit(&session, detector, "Detect", ops))?;
    assert_eq!(change.kept, vec![balloon]);
    assert!(session.layer(cleanup).is_err() && session.layer(mask).is_err());
    println!("the rerun removed the cleanup and its mask, and kept the balloon the user restyled");
    Ok(())
}
