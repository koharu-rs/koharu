# koharu-bindgen

Bindings to C libraries the app opens at runtime.

A build step for Koharu's `-sys` crates: it turns a C header into Rust
functions with the same names and signatures a linked binding has, which call
through a table filled when the libraries are opened. Where it follows an
established convention, it cites it; the links are under [Sources](#sources).

## Goal

The native libraries Koharu runs on are downloaded after the app starts, one
build for each kind of GPU, so nothing can link against them when Koharu
builds. A `-sys` crate still looks like one that links, so the safe crates
over it are written as they would be against any `-sys` crate [Cargo sys].

## 1. Bindings

| Part | Model | Cite |
|---|---|---|
| functions | every function the header declares becomes a free function with its own name and signature, which calls through the table | a `-sys` crate declares a native library's functions for others to wrap [Cargo sys] |
| left out | variadic functions and foreign statics, and any function a crate names because some builds don't export it, or opening would always fail | |
| callbacks | a generated function has Rust's calling convention, so C can't call it back; a callback is written in Rust | |
| before opening | a call panics, since there's nothing to call | |

## 2. Opening

| Part | Model | Cite |
|---|---|---|
| order | the libraries open in the order given, and the first one exporting a symbol wins | |
| failure | names the library that didn't open, or the first symbol none exports, and binds nothing, so a missing function fails at once rather than when it's first called | |
| Windows | a library's dependencies come from its own folder, the app's folder, System32, or a folder the app added, never from `PATH`, where another program's copy of a library is a different build | `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR` and `LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` [LoadLibraryEx] |
| Unix | every symbol resolves when the library opens, and its exports stay out of the global scope, so two libraries with the same symbols don't resolve into each other | `RTLD_NOW` and `RTLD_LOCAL` [dlopen] |

Symbols resolve in a loop over a list, so the cost of opening doesn't grow the
stack with the number of functions, which in libtorch's case is thousands.

## Sources

Read in November 2026.

- Cargo: [Cargo sys]
- Operating systems: [LoadLibraryEx], [dlopen]

[Cargo sys]: https://doc.rust-lang.org/cargo/reference/build-scripts.html#-sys-packages
[LoadLibraryEx]: https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryexw
[dlopen]: https://man7.org/linux/man-pages/man3/dlopen.3.html
