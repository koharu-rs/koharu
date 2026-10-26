//! A project's lifecycle: create it, import a page, add one layer of every
//! kind, write pixels, read everything back, save, reopen, and compact.
//!
//! `cargo run -p koharu-scene --example project`

use koharu_scene::{
    Answer, Author, Change, Created, Edit, Hash, Layer, LayerId, Op, PageId, Query, Rect, Ref,
    Session, Size,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `$name`: a page or layer created earlier in the same edit.
fn label(name: &str) -> Ref {
    Ref::try_from(format!("${name}")).expect("a valid label")
}

/// A layer in its JSON form, which is what commands and tools send.
fn layer(value: Value) -> Box<Layer> {
    Box::new(serde_json::from_value(value).expect("a valid layer"))
}

fn add(page: Ref, parent: Option<Ref>, index: usize, id: &str, value: Value) -> Op {
    Op::AddLayer {
        id: Some(label(id)),
        page,
        parent,
        index,
        layer: layer(value),
        ingredients: Vec::new(),
    }
}

fn layer_id(change: &Change, name: &str) -> LayerId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Layer { id, label } if label.as_deref() == Some(name) => Some(*id),
            _ => None,
        })
        .expect("the layer was created")
}

fn page_id(change: &Change, name: &str) -> PageId {
    change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Page { id, label } if label.as_deref() == Some(name) => Some(*id),
            _ => None,
        })
        .expect("the page was created")
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("chapter.koharu");
    let mut session = Session::create(&path)?;

    // An imported file is kept byte for byte and named by the hash of its
    // bytes; an image layer names it by that hash.
    let scan = b"any image bytes; the project never decodes them".to_vec();
    let file = Hash::of(&scan);

    // One edit, one undo step. Later ops name what earlier ones created by
    // the `$label` they gave as `id`.
    let page = label("page");
    let change = session.apply(Edit {
        project: session.id(),
        label: "Import page".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: vec![
            Op::AddFile {
                media_type: "image/png".to_owned(),
                bytes: scan.clone(),
            },
            Op::AddPage {
                id: Some(page.clone()),
                index: 0,
                size: Size {
                    width: 1200,
                    height: 1700,
                },
            },
            // Children are drawn first to last, so the scan goes first.
            add(page.clone(), None, 0, "scan", json!({"kind": "image", "name": "Scan", "file": file})),
            add(page.clone(), None, 1, "inpaint", json!({"kind": "raster", "name": "Inpaint"})),
            add(page.clone(), None, 2, "balloon", json!({"kind": "group", "name": "Balloon"})),
            add(page.clone(), Some(label("balloon")), 0, "outline", json!({
                "kind": "shape",
                "name": "Outline",
                "path": "M100 100 H400 V300 H100 Z",
                "fill": [1.0, 1.0, 1.0, 1.0],
                "stroke": {"color": [0.0, 0.0, 0.0, 1.0], "width": 3.0},
            })),
            add(page.clone(), Some(label("balloon")), 1, "line", json!({
                "kind": "text",
                "name": "Line",
                "content": [{"insert": "Hello, ", "attributes": {"weight": 700}}, {"insert": "world"}],
                "style": {"chars": {"family": "Anime Ace", "size": 24.0}, "align": "center"},
                "frame": {"box": {"bounds": {"x": 110.0, "y": 110.0, "width": 280.0, "height": 180.0}}},
                "language": "en",
            })),
            // A mask limits its parent and is never drawn; its index counts
            // among the parent's masks.
            add(page.clone(), Some(label("balloon")), 0, "mask", json!({
                "kind": "mask",
                "content": {"path": {"path": "M100 100 H400 V300 H100 Z"}},
            })),
        ],
    })?;
    let (page, inpaint, balloon) = (
        page_id(&change, "page"),
        layer_id(&change, "inpaint"),
        layer_id(&change, "balloon"),
    );
    println!("created {} pages and layers", change.created.len());

    // Pixels are tiled, 64×64 each; a 128×96 write touches four tiles.
    let (width, height) = (128, 96);
    let pixels = [200, 40, 40, 255].repeat((width * height) as usize);
    session.apply(Edit {
        project: session.id(),
        label: "Paint".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: vec![Op::WritePixels {
            layer: inpaint.into(),
            rect: Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            pixels,
        }],
    })?;
    let tiles = session.tiles(inpaint, None)?;
    assert_eq!(tiles.tiles.len(), 4);
    println!(
        "the raster holds {} tiles of {:?}",
        tiles.tiles.len(),
        tiles.format
    );

    // Reads return owned views: the project, a page's tree, one layer, or a
    // file's bytes. `query` answers the same questions as data.
    let project = session.project()?;
    assert_eq!(project.pages.len(), 1);
    let view = session.page(page)?;
    assert_eq!(view.layers.len(), 3);
    let group = session.layer(balloon)?;
    assert_eq!((group.view.children.len(), group.view.masks.len()), (2, 1));
    assert_eq!(session.file(&file)?.bytes, scan);
    let Answer::History(history) = session.query(Query::GetHistory)? else {
        unreachable!("history answers with history")
    };
    println!("undo would revert {:?}", history.undo);

    // Saving writes the whole document with its history; reopening checks
    // every layer, so unknown core data fails to open.
    session.save()?;
    // A project is open in one session at a time: opening it again fails
    // until this session is dropped, so switching projects drops the old
    // session first.
    let error = Session::open(&path)
        .err()
        .expect("the project is already open");
    assert_eq!(serde_json::to_value(&error)?["error"], "storage");
    println!("a second open failed: {error}");
    drop(session);
    let mut session = Session::open(&path)?;
    assert_eq!(session.page(page)?, view);
    println!("reopened {} with the same page", path.display());

    // Compacting drops history, and with it the files and tiles nothing
    // names any more.
    session.compact()?;
    drop(session);
    assert_eq!(Session::open(&path)?.page(page)?, view);
    println!("compacted");
    Ok(())
}
