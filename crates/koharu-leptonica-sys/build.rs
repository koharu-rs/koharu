use std::{env, fs, path::PathBuf};

fn main() {
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets it"));
    let sources = crate_dir.join("leptonica");
    let headers = crate_dir.join("include");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets it"));

    // Leptonica's CMake build writes this header from `endianness.h.in`.
    let endian = match env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() {
        Ok("big") => "L_BIG_ENDIAN",
        _ => "L_LITTLE_ENDIAN",
    };
    fs::write(out.join("endianness.h"), format!("#define {endian}\n"))
        .expect("couldn't write endianness.h");

    let mut files: Vec<PathBuf> = fs::read_dir(&sources)
        .expect("couldn't read leptonica/")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();
    println!("cargo:rerun-if-changed={}", sources.display());
    println!("cargo:rerun-if-changed={}", headers.display());
    cc::Build::new()
        .files(files)
        .include(&headers)
        .include(&out)
        // Pixels come in and out as memory, so no image-format library is
        // linked; Leptonica builds stubs for reading and writing files.
        .define("HAVE_LIBJPEG", "0")
        .define("HAVE_LIBTIFF", "0")
        .define("HAVE_LIBPNG", "0")
        .define("HAVE_LIBZ", "0")
        .warnings(false)
        .compile("leptonica");

    // All of Leptonica's API, as the whole library is linked anyway; nothing
    // from the system headers it includes.
    koharu_bindgen::generate_linked(|bindings| {
        bindings
            .clang_arg(format!("-I{}", out.display()))
            .allowlist_file(r".*[/\\]koharu-leptonica-sys[/\\]include[/\\].*")
    });
}
