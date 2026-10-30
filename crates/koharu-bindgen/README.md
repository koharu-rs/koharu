# koharu-bindgen

The build step behind Koharu's `-sys` crates. It runs bindgen over a crate's
headers and turns each C function into a free function with the same name and
signature that calls through a library loaded at run time, so one Koharu
binary runs whichever libtorch, llama.cpp, and stable-diffusion.cpp build fits
the GPU.

```rust,ignore
// build.rs: what the crate binds, and nothing else
fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            .allowlist_function("^(ggml|gguf|llama|mtmd)_.*")
            .allowlist_type("^(ggml|gguf|llama|mtmd)_.*")
            .derive_partialeq(true)
    });
}

// at run time, by koharu-runtime
koharu_llama_sys::load(&[&llama, &mtmd])?;
```

## Every `-sys` crate

```text
koharu-<library>-sys/
├── Cargo.toml   libloading, and koharu-bindgen as the only build dependency
├── build.rs     one koharu_bindgen::generate call
├── wrapper.h    #includes the headers it binds
├── include/     the library's headers, copied verbatim from its pinned commit
└── src/lib.rs   the pinned commit, include!(bindings.rs), then what's written by hand
```

- **One shape for every crate.** `generate` does what each build script used
  to repeat: it reads `wrapper.h`, puts `include/` on the include path, reruns
  when any header it read changes, and writes `bindings.rs`. Two `-sys` crates
  then differ only in what they bind, which is what a reviewer needs to see.
- **Headers stay verbatim,** so updating a library is copying its headers at
  the new commit, and nothing generated is checked in. A function is bound
  as its header declares it; no call is written by hand.
- **Everything resolves at `load`.** It opens each library by absolute path
  and resolves every function before returning, so a missing library or
  function is one error naming it, not a crash partway through a page, and no
  other build on the search path can be picked up.
- **A build script just fails.** A header that doesn't parse stops the build
  with bindgen's message; there's nothing for a build script to recover.
