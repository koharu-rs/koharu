# koharu-bindgen

A build step that generates Rust bindings to C libraries opened at runtime. The
generated functions keep the names and signatures of linked bindings, so callers
don't change.

## Generated code

`generate` runs bindgen over a crate's `wrapper.h` and writes `bindings.rs` to
`OUT_DIR`; the crate needs `libloading` as a dependency. Each function in an `extern`
block becomes a free function that calls through a table:

```rust
pub unsafe fn foo_add(a: c_int, b: c_int) -> c_int {
    unsafe {
        let function: unsafe extern "C" fn(c_int, c_int) -> c_int =
            ::std::mem::transmute(__function(0usize));
        function(a, b)
    }
}
```

Variadic functions and foreign statics aren't bound, and a wrapper has the Rust ABI,
so it can't be passed to C as a callback.

## Loading

`load(paths: &[&Path]) -> io::Result<()>` opens each absolute path in order, then
resolves a static list of symbols in a loop, so its stack frame doesn't depend on the
number of functions. The first library that exports a symbol wins. An error names the
library or the first missing symbol, and a wrapper called before `load` panics.

| OS | libloading call | Flags | Effect |
|---|---|---|---|
| Windows | `os::windows::Library::load_with_flags` | `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR \| LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` | Dependencies come from the library's folder, the application's folder, `System32`, or `AddDllDirectory` paths, never `PATH`. One already loaded is reused |
| Unix | `os::unix::Library::open` | `RTLD_NOW \| RTLD_LOCAL` | Undefined symbols resolve at open. Exports stay out of the global scope, so libraries with the same symbols don't resolve into each other |
