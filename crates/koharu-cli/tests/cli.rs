use std::{
    io::{BufRead as _, BufReader, Read as _},
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use koharu_runtime::AcceleratorLock;
use koharu_scene::{Edit, Hash, Meta, Op, Session};
use serde_json::{Value, json};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_koharu-cli")
}

fn isolated_env(cmd: &mut Command, home: &Path, xdg_cache: &Path) {
    cmd.env("HOME", home)
        .env("XDG_CACHE_HOME", xdg_cache)
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .env_remove("DEEPSEEK_API_KEY");
}

fn make_blank_khr(path: &Path) {
    let mut session = Session::create(path, Meta::default()).unwrap();
    session
        .apply(Edit {
            label: "test".to_owned(),
            ops: vec![
                serde_json::from_value(json!({
                    "op": "add_page",
                    "index": 0,
                    "size": { "width": 64, "height": 64 },
                }))
                .unwrap(),
            ],
        })
        .unwrap();
}

fn make_image_khr(path: &Path) {
    let mut png = Vec::new();
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let hash = Hash::of(&png);
    let mut session = Session::create(path, Meta::default()).unwrap();
    session
        .apply(Edit {
            label: "test".to_owned(),
            ops: vec![
                Op::AddFile { bytes: png },
                Op::AddPage {
                    id: None,
                    index: 0,
                    size: None,
                    background: Some(hash),
                },
            ],
        })
        .unwrap();
}

#[test]
fn help_exits_zero_and_invalid_args_exit_one() {
    let help = Command::new(bin()).arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("koharu-cli"));

    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("page.png");
    fs_err::write(&img, b"stub").unwrap();
    let img_arg = img.to_str().unwrap();

    for args in [
        vec!["--quality", "0", img_arg],
        vec!["--quality", "101", img_arg],
        vec!["--stages", "", img_arg],
        vec!["--stages", "invalid_stage", img_arg],
        vec!["--export-format", "bmp", "--export-out", "out.bmp", img_arg],
        vec!["--cbz-codec", "avif", img_arg],
        vec!["--target", "zz", img_arg],
        vec!["--export-format", "cbz", img_arg],
        vec!["/nonexistent/path/definitely/missing.png"],
    ] {
        let out = Command::new(bin()).args(&args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "expected exit 1 for {args:?}, got {:?}, stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !out.stderr.is_empty(),
            "expected error message on stderr for {args:?}"
        );
    }
}

#[test]
fn locked_khr_project_exits_one_with_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let khr = dir.path().join("book.khr");
    make_blank_khr(&khr);

    // Hold the SQLite exclusive lock open in this process.
    let _held = Session::open(&khr, Meta::default()).unwrap();

    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    let out = cmd
        .args(["--stages", "detect", khr.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected = format!("Project {} is currently open in Koharu", khr.display());
    assert!(
        stderr.contains(&expected),
        "expected {expected:?} in stderr, got: {stderr}"
    );
}

#[test]
fn contended_accelerator_lock_exits_one_with_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let khr = dir.path().join("book.khr");
    make_blank_khr(&khr);

    // `koharu_runtime::store::root()` uses `dirs::cache_dir()/koharu/store` -> `$XDG_CACHE_HOME/koharu/store`.
    let _lock = AcceleratorLock::acquire_in(xdg.join("koharu").join("store")).unwrap();

    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    let out = cmd
        .args(["--stages", "detect", khr.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Another Koharu process is currently using the GPU accelerator"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn hosted_translator_without_credentials_or_keyring_fails_closed_with_exit_one() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    fs_err::create_dir_all(home.join(".koharu")).unwrap();
    let khr = dir.path().join("book.khr");
    make_image_khr(&khr);

    // Case 1: No secrets.age -> hosted provider has no API key -> fails before GPU work.
    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    let out = cmd
        .args([
            "--stages",
            "detect,translate",
            "--translator",
            "openai/gpt-4o",
            khr.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("has no API key"),
        "expected missing API key error, got: {stderr}"
    );

    // Case 2: secrets.age exists, but no keyring bus is available -> fail closed on credential store.
    fs_err::write(home.join(".koharu").join("secrets.age"), b"not-valid-age").unwrap();
    let mut cmd2 = Command::new(bin());
    isolated_env(&mut cmd2, &home, &xdg);
    cmd2.env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/dbus");
    let out2 = cmd2
        .args([
            "--stages",
            "detect,translate",
            "--translator",
            "openai/gpt-4o",
            khr.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out2.status.code(), Some(1));
    assert!(!out2.stderr.is_empty());
}

#[test]
fn blank_khr_json_stdout_stderr_separation_and_cbz_export() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let khr = dir.path().join("book.khr");
    let cbz = dir.path().join("nested").join("out.cbz");
    make_blank_khr(&khr);

    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    cmd.env("RUST_LOG", "info");
    let out = cmd
        .args([
            "--stages",
            "detect",
            "--json",
            "--export-format",
            "cbz",
            "--export-out",
            cbz.to_str().unwrap(),
            khr.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .unwrap_or_else(|err| panic!("invalid JSONL line {line:?}: {err}"))
        })
        .collect();

    assert!(
        events.iter().any(|ev| ev["kind"] == "queued"),
        "missing queued event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|ev| ev["kind"] == "export" && ev["format"] == "cbz" && ev["pages"] == 1),
        "missing export event: {events:?}"
    );
    let summary = events
        .iter()
        .find(|ev| ev["kind"] == "summary")
        .expect("missing summary event");
    assert_eq!(summary["pages_total"], 1);
    assert_eq!(summary["pages_succeeded"], 1);
    assert_eq!(summary["pages_failed"], 0);
    assert_eq!(summary["exported_path"], cbz.to_str().unwrap());

    assert!(cbz.is_file());
    let file = fs_err::File::open(&cbz).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert!(archive.by_name("001.png").is_ok());
    let mut comic_xml = String::new();
    archive
        .by_name("ComicInfo.xml")
        .unwrap()
        .read_to_string(&mut comic_xml)
        .unwrap();
    assert!(comic_xml.contains("<PageCount>1</PageCount>"));
}

#[cfg(unix)]
#[test]
fn sigint_mid_run_cancels_cleanly_with_exit_130() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let khr = dir.path().join("book.khr");
    make_image_khr(&khr);

    // Local TCP listener that accepts the HF request and holds it open without answering,
    // keeping `koharu-cli` inside `WaitingForDownload` until SIGINT arrives.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop_server = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop_server);
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut sockets = Vec::new();
        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => sockets.push(stream),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    cmd.env("HF_ENDPOINT", format!("http://{addr}"))
        .args(["--stages", "detect", "--json", khr.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).unwrap();
    let first_event: Value = serde_json::from_str(&first_line).unwrap();
    assert_eq!(first_event["kind"], "queued");

    // Send SIGINT to the running child.
    let pid = child.id().to_string();
    let kill_status = Command::new("kill").args(["-INT", &pid]).status().unwrap();
    assert!(kill_status.success());

    let mut rest = String::new();
    reader.read_to_string(&mut rest).unwrap();
    let status = child.wait().unwrap();
    stop_server.store(true, Ordering::Relaxed);
    let _ = server.join();

    assert_eq!(status.code(), Some(130), "rest stdout={rest}");
    let all_lines = format!("{first_line}{rest}");
    let events: Vec<Value> = all_lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect();
    assert_eq!(
        events.last().and_then(|v| v["kind"].as_str()),
        Some("summary"),
        "expected summary event after SIGINT: {events:?}"
    );
}

#[test]
fn yotsubato_testing_import_and_cbz_export_read_only() {
    let corpus = Path::new("/home/finn/よつばとtesting");
    if !corpus.is_dir() {
        return;
    }

    let before_entries: Vec<(String, u64)> = {
        let mut entries: Vec<_> = fs_err::read_dir(corpus)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let meta = entry.metadata().unwrap();
                (entry.file_name().to_string_lossy().into_owned(), meta.len())
            })
            .collect();
        entries.sort();
        entries
    };
    assert_eq!(before_entries.len(), 9);

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let project = dir.path().join("yotsuba.khr");
    let cbz = dir.path().join("yotsuba.cbz");

    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    let out = cmd
        .args([
            "--project",
            project.to_str().unwrap(),
            "--stages",
            "recognize",
            "--json",
            "--export-format",
            "cbz",
            "--export-out",
            cbz.to_str().unwrap(),
            corpus.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect();
    let summary = events
        .iter()
        .find(|ev| ev["kind"] == "summary")
        .expect("missing summary event");
    assert_eq!(summary["pages_total"], 9);
    assert_eq!(summary["pages_succeeded"], 9);
    assert_eq!(summary["pages_failed"], 0);

    // Verify imported .khr project has 9 pages including the spread (1971x1400).
    let session = Session::open(&project, Meta::default()).unwrap();
    let pages: Vec<_> = session.project().pages().cloned().collect();
    assert_eq!(pages.len(), 9);
    assert_eq!((pages[1].size.width, pages[1].size.height), (1971, 1400));
    drop(session);

    // Verify exported CBZ has 9 images + ComicInfo.xml with <PageCount>9</PageCount>.
    let mut archive = zip::ZipArchive::new(fs_err::File::open(&cbz).unwrap()).unwrap();
    let image_entries = (0..archive.len())
        .filter(|&i| {
            archive
                .by_index(i)
                .unwrap()
                .name()
                .unwrap()
                .ends_with(".png")
        })
        .count();
    assert_eq!(image_entries, 9);
    let mut comic_xml = String::new();
    archive
        .by_name("ComicInfo.xml")
        .unwrap()
        .read_to_string(&mut comic_xml)
        .unwrap();
    assert!(comic_xml.contains("<PageCount>9</PageCount>"));

    // Verify `/home/finn/よつばとtesting` remained strictly unmodified.
    let after_entries: Vec<(String, u64)> = {
        let mut entries: Vec<_> = fs_err::read_dir(corpus)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let meta = entry.metadata().unwrap();
                (entry.file_name().to_string_lossy().into_owned(), meta.len())
            })
            .collect();
        entries.sort();
        entries
    };
    assert_eq!(before_entries, after_entries);
}

#[test]
fn failed_import_cleans_up_newly_created_project_file() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let project = dir.path().join("should_be_removed.khr");
    let unsupported = dir.path().join("notes.txt");
    fs_err::write(&unsupported, b"not an image").unwrap();

    let mut cmd = Command::new(bin());
    isolated_env(&mut cmd, &home, &xdg);
    let out = cmd
        .args([
            "--project",
            project.to_str().unwrap(),
            "--stages",
            "recognize",
            unsupported.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));
    assert!(
        !project.exists(),
        "newly created .khr should be removed if import_paths fails"
    );
}
