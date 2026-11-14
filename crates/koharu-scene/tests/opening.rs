//! What opening refuses, which versions open, and the blobs it sweeps.

mod common;
use std::{io::Cursor, path::Path};

use common::*;
use koharu_scene::{Error, Hash, History, Op, Rect, Session};
use serde_json::json;

/// Reopens the project at `path` after `change` alters its file by hand.
fn reopen_after(path: &Path, change: &str) -> koharu_scene::Result<Session> {
    let conn = rusqlite::Connection::open(path).expect("the file opens");
    conn.execute_batch(change).expect("the change applies");
    drop(conn);
    Session::open(path)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::new(width, height)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("a PNG");
    bytes
}

#[test]
fn a_file_that_is_not_a_project_fails_and_stays_unchanged() {
    let (_dir, path) = path();
    fs_err::write(&path, vec![7; 8192]).expect("a file");
    let junk = fs_err::read(&path).expect("its bytes");
    assert!(matches!(
        Session::open(&path).err().expect("refused"),
        Error::Corrupt(_)
    ));
    assert_eq!(fs_err::read(&path).expect("its bytes"), junk);

    fs_err::remove_file(&path).expect("removed");
    let conn = rusqlite::Connection::open(&path).expect("a database");
    conn.execute_batch("PRAGMA application_id=7; CREATE TABLE other(a);")
        .expect("a table");
    drop(conn);
    let other = fs_err::read(&path).expect("its bytes");
    assert!(matches!(
        Session::open(&path).err().expect("refused"),
        Error::Corrupt(_)
    ));
    assert_eq!(fs_err::read(&path).expect("its bytes"), other);

    assert!(Session::open(path.with_extension("missing")).is_err());
    assert!(!path.with_extension("missing").exists());
}

#[test]
fn a_create_that_fails_leaves_no_file() {
    let (_dir, path) = path();
    // Directories where SQLite's journals go make its first write fail.
    let journals = ["-journal", "-wal"].map(|suffix| {
        let mut journal = path.clone().into_os_string();
        journal.push(suffix);
        std::path::PathBuf::from(journal)
    });
    for journal in &journals {
        fs_err::create_dir(journal).expect("a directory");
    }
    assert!(Session::create(&path).is_err());
    assert!(
        !path.exists(),
        "a half-made project would refuse both a create and an open"
    );
    for journal in &journals {
        fs_err::remove_dir(journal).expect("removed");
    }
    Session::create(&path).expect("a project");
}

#[test]
fn only_compatible_versions_open() {
    let (_dir, path) = path();
    drop(Session::create(&path).expect("a project"));
    let bytes = fs_err::read(&path).expect("its bytes");
    for version in ["1.1.0", "2.0.0", "0.9.0", "1.0.0-alpha", "x"] {
        let copy = path.with_extension(format!("v{version}"));
        fs_err::copy(&path, &copy).expect("a copy");
        let conn = rusqlite::Connection::open(&copy).expect("the file opens");
        conn.execute("UPDATE project SET version = ?1", [version])
            .expect("a version");
        drop(conn);
        let before = fs_err::read(&copy).expect("its bytes");
        let error = Session::open(&copy).err().expect("refused");
        assert!(
            matches!(error, Error::Version(ref got) if got == version),
            "{version}: {error:?}"
        );
        assert_eq!(fs_err::read(&copy).expect("its bytes"), before, "{version}");
    }
    assert_eq!(fs_err::read(&path).expect("its bytes"), bytes);
    Session::open(&path).expect("1.0.0 opens");
}

#[test]
fn corrupt_tile_coordinates_and_oversized_tiles_fail_closed() {
    let (dir, mut session, page) = session();
    let path = dir.path().join("test.khr");
    let raster = add(&mut session, page, None, json!({"kind": "raster"}));
    let paint = |rect: Rect, pixels: Vec<u8>| Op::WritePixels {
        layer: raster.into(),
        rect,
        pixels,
    };
    session
        .apply(edit(vec![paint(
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            vec![1, 1, 1, 255],
        )]))
        .expect("a paint");

    // A rectangle can't reach past what a tile's position can say.
    let error = session
        .apply(edit(vec![paint(
            Rect {
                x: i32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
            vec![1; 8],
        )]))
        .expect_err("out of range");
    assert!(matches!(error, Error::Invalid { .. }), "{error}");
    drop(session);

    let error = reopen_after(&path, "UPDATE tiles SET x = 1 << 40;")
        .err()
        .expect("refused");
    assert!(matches!(error, Error::Corrupt(_)), "{error:?}");
    let error = reopen_after(&path, "UPDATE tiles SET x = 0, y = 1 << 26;")
        .err()
        .expect("refused");
    assert!(matches!(error, Error::Corrupt(_)), "{error:?}");
    reopen_after(&path, "UPDATE tiles SET y = 0;").expect("it opens");

    // A tile that inflates past a tile's size stops being read there.
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &vec![0; 1 << 24]).expect("written");
    let bomb = encoder.finish().expect("finished");
    let hex: String = bomb.iter().map(|byte| format!("{byte:02x}")).collect();
    let session =
        reopen_after(&path, &format!("UPDATE blobs SET bytes = x'{hex}';")).expect("it opens");
    let error = session.tiles(raster, None).expect_err("a bomb");
    assert!(matches!(error, Error::Corrupt(_)), "{error:?}");
}

#[test]
fn a_project_whose_values_dont_read_fails_to_open() {
    let (dir, mut session, page) = session();
    let path = dir.path().join("test.khr");
    let first = add(
        &mut session,
        page,
        None,
        json!({"kind": "balloon", "outline": outline()}),
    );
    // Top level: the balloon, the raster, and the image; the text sits in the
    // balloon.
    add(
        &mut session,
        page,
        Some(first),
        json!({"kind": "text", "name": "t"}),
    );
    let raster = add(&mut session, page, None, json!({"kind": "raster"}));
    let image_file = png(2, 2);
    let file = Hash::of(&image_file);
    session
        .apply(edit(vec![Op::AddFile { bytes: image_file }]))
        .expect("a file");
    add(
        &mut session,
        page,
        None,
        json!({"kind": "image", "file": file}),
    );
    session
        .apply(edit(vec![Op::WritePixels {
            layer: raster.into(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            pixels: vec![1, 1, 1, 255],
        }]))
        .expect("a paint");
    drop(session);
    let missing = Hash::of(b"missing");
    let set = |path: &str, value: &str| {
        format!("UPDATE pages SET page = json_set(page, '{path}', {value});")
    };
    for change in [
        // A field no version wrote, a kind that doesn't exist, and a value out of range.
        set("$.layers[0].children[0].nope", "1"),
        set("$.layers[0].children[0].kind", "'nope'"),
        set("$.layers[0].children[0].opacity", "9"),
        // A common value missing.
        "UPDATE pages SET page = json_remove(page, '$.layers[0].children[0].visible');".to_owned(),
        // Tiles naming a missing blob, and an image naming a file that isn't there.
        format!("UPDATE tiles SET hash = '{missing}';"),
        set("$.layers[2].file", &format!("'{missing}'")),
    ] {
        let copy = dir.path().join("broken.khr");
        fs_err::copy(&path, &copy).expect("a copy");
        let error = reopen_after(&copy, &change).err().expect("refused");
        assert!(matches!(error, Error::Corrupt(_)), "{change}: {error:?}");
        fs_err::remove_file(&copy).expect("removed");
    }
}

#[test]
fn files_go_at_reopening_when_nothing_names_them() {
    let (dir, mut session, page) = session();
    let (a, b, scan) = (b"a".to_vec(), b"b".to_vec(), png(4, 4));
    let (ha, hb, hscan) = (Hash::of(&a), Hash::of(&b), Hash::of(&scan));
    session
        .apply(edit(vec![
            Op::AddFile { bytes: a },
            Op::AddFile { bytes: b },
            Op::AddFile { bytes: scan },
        ]))
        .expect("files");
    apply(
        &mut session,
        vec![json!({"op": "add_page", "index": 1, "background": hscan})],
    )
    .expect("a page");
    let image = add(
        &mut session,
        page,
        None,
        json!({"kind": "image", "file": ha}),
    );
    apply(
        &mut session,
        vec![json!({"op": "update_layer", "layer": image, "patch": {"file": hb}})],
    )
    .expect("a new file");
    assert!(session.file(&ha).is_ok(), "undo can still need it");
    session
        .apply(edit(vec![Op::RemovePage { page: page.into() }]))
        .expect("a removal");
    session.undo().expect("undo").expect("a step");
    assert_eq!(
        session.project().page(page).expect("the page").layers.len(),
        1
    );

    drop(session);
    let session = Session::open(dir.path().join("test.khr")).expect("it opens");
    assert!(
        matches!(session.file(&ha), Err(Error::NotFound(_))),
        "nothing names it any more"
    );
    assert_eq!(session.file(&hb).expect("the image's file"), b"b");
    assert!(session.file(&hscan).is_ok(), "a background names its file");
    assert_eq!(session.history(), History::default());
}
