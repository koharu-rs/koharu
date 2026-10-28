# koharu-bindgen

`koharu-bindgen` is the build step behind Koharu's `-sys` crates. It runs
`bindgen` over a crate's vendored C headers at build time, or reads the Rust
declarations an upstream `-sys` crate ships, and turns each C function into a
Rust function that calls through a library loaded when the
runtime starts, so one Koharu binary runs whichever libtorch, llama.cpp, and
stable-diffusion.cpp build fits the machine's GPU.

```rust,ignore
// in a -sys crate's build.rs
koharu_bindgen::Bindings::new("wrapper.h")
    .configure(|builder| builder.allowlist_function("llama_.*"))
    .write(out_dir.join("bindings.rs"))?;

// at runtime, by koharu-runtime
llama_cpp_sys_2::load(&[&llama_dll, &mtmd_dll])?;
```

| Rule | Evidence |
|---|---|
| Bindings are generated from headers copied verbatim from the library's pinned commit; nothing generated is checked in or edited. | `AGENTS.md` |
| `load` opens each library by absolute path and resolves every function it binds before returning, and a missing library or function is an error naming it. Nothing is resolved lazily, and no library is looked up on the search path, where a different build could be found. Calling a bound function before `load` panics, as there's nothing to call. | bindgen's runtime loading can require every symbol at load (`dynamic_link_require_all`) [bindgen dynamic] |
| Each function stays a free function with the C name and signature, so a `-sys` crate built this way can stand in for one that links normally, as Koharu's llama.cpp bindings stand in for `llama-cpp-sys-2`. | `llama-cpp-sys-2`'s README in this repository |
| A crate whose upstream declares its functions in Rust rather than a header, as `torch-sys` does, passes those sources to `rewrite`, which reads the module files they declare and rewrites every `extern` block in place; a module's `#[cfg]` also decides whether `load` resolves its functions. | `torch-sys`'s README |
| Library file names come from `libloading`. | `library_filename` builds the platform's name for a library [libloading filename] |

## Changes from today

- `load` resolves everything up front and returns an error; today the first
  call to a missing function panics, and a library that fails to load is
  looked up on the search path.
- `rewrite` takes an upstream crate's Rust declarations, for `torch-sys`.
- The unused command-line tool, with its `clap` dependency, and the unused
  rewrite entry points go.

## Sources

[bindgen dynamic]: https://docs.rs/bindgen/latest/bindgen/struct.Builder.html#method.dynamic_link_require_all
[libloading filename]: https://docs.rs/libloading/0.9.0/libloading/fn.library_filename.html
