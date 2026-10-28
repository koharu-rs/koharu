# llama-cpp-sys-2, Koharu's build

Koharu runs llama.cpp through the `llama-cpp-2` crate [llama-cpp-2], with this
crate standing in for its `llama-cpp-sys-2`. Upstream's `-sys` crate compiles
llama.cpp from source with CMake and links it; this one binds the same C API
to the prebuilt llama.cpp that `koharu-runtime` installs for the machine's GPU,
loaded when the runtime starts. `koharu-llama`, Koharu's fork of
`llama-cpp-2`, goes.

```toml
# the workspace manifest
[workspace.dependencies]
llama-cpp-2 = { version = "0.1.160", default-features = false, features = ["mtmd", "llguidance"] }

[patch.crates-io]
llama-cpp-sys-2 = { path = "crates/llama-cpp-sys-2" }
```

## How it stands in

| Rule | Evidence |
|---|---|
| The package is named `llama-cpp-sys-2`, at the version `llama-cpp-2` asks for, and the workspace patches crates.io with it, so `llama-cpp-2` builds from crates.io against it. Its folder is renamed to match. | a `[patch]` entry replaces a crates.io package with a local one of the same name and a compatible version [Cargo patch] |
| It declares upstream's feature names (`common`, `cuda`, `metal`, `vulkan`, `rocm`, `mtmd`, and the rest) as features that do nothing, so `llama-cpp-2`'s feature forwarding resolves. | `llama-cpp-2` forwards each of its features to `llama-cpp-sys-2` [llama-cpp-2 features] |
| Its bindings come from `bindgen` with upstream's settings (`derive_partialeq`, no enum-name prefix, the `llama_`, `ggml_`, `gguf_`, and `mtmd_` allowlists), so every name and type `llama-cpp-2` uses is the same. | upstream's build script [llama-cpp-sys-2 build] |
| Each C function is a free function with its C name, resolved from the runtime's DLLs when the runtime starts. | `koharu-bindgen`'s README |
| `common` stays off. Every call `llama-cpp-2` makes into upstream's C++ helper library (`llama_rs_*`: JSON schema to grammar, parameter fitting, speculative decoding) is behind it, so without it `llama-cpp-2` calls only what `llama.dll` and `mtmd.dll` export. Koharu constrains output with llguidance instead. | `llama-cpp-2` gates the helper behind `common` [llama-cpp-2 common] |

## Versions in step

- **One commit.** The headers, the bindings generated from them, and the DLLs
  the runtime installs all come from the llama.cpp commit `llama-cpp-sys-2`
  pins, so nothing can disagree at run time; a function `llama-cpp-2` calls
  that changed fails the build instead.
- **Today** that's `b11565` (`f9a95aeb`), the newest of Koharu's llama.cpp
  releases that `llama-cpp-2` 0.1.160 builds against: `b11642` added fields to
  `llama_perf_context_data` that 0.1.160 doesn't set. 0.1.160 pins `d8123504`
  [llama-cpp-sys-2 pin], so its calls are checked against these headers, and
  the runtime installs the `b11565` release. Every GGUF the catalog lists was
  added before then, and each is loaded once to confirm it.
- **An upgrade** bumps `llama-cpp-2`, picks the Koharu llama.cpp release whose
  headers it builds against, copies that release's headers here, and records
  its files' hashes in the runtime.

## What the engine uses

`koharu-llm`'s engine uses `llama-cpp-2` as it is: a context that can move to
the engine's thread, `synchronize`, a sampler per sequence with llguidance,
and image evaluation per sequence, which switches attention around each image
itself. Upstream fixed what Koharu's fork still has: a double free in
`MtmdBitmap::clone` [bitmap fix], chunks that outlived their list [chunks
fix], and types that couldn't move between threads [send fix].

The context's raw pointer is private, so what the engine needs and
`llama-cpp-2` lacks, such as choosing the GPU the vision projector runs on,
goes upstream as a pull request rather than around it. An abort callback isn't
needed: llama.cpp honours it "only with CPU execution" [llama.h abort], so the
engine stops between decode calls instead.

## Changes from today

- `koharu-llama` (8,036 lines) goes, with about ten ways safe code reached
  undefined behaviour or a crash: the bitmap double free, chunks used after
  free, a batch that could overflow the heap, an image size that wrapped,
  leaked token pieces, dangling overrides, stale logit indices, unchecked
  batch sizes that let llama.cpp abort, and samplers that outlived their model.
- Special tokens in user text are no longer parsed as control tokens: the
  engine tokenizes user text with `parse_special` off.
- The bindings load from the runtime's absolute paths and fail with an error
  when something's missing, instead of panicking or finding another
  `llama.dll` on the search path.

## Sources

[llama-cpp-2]: https://docs.rs/llama-cpp-2/0.1.160/llama_cpp_2/
[Cargo patch]: https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section
[llama-cpp-2 features]: https://github.com/utilityai/llama-cpp-rs/blob/0dd5814c9b061930965d144c1653cd4d05054873/llama-cpp-2/Cargo.toml#L26-L55
[llama-cpp-sys-2 build]: https://github.com/utilityai/llama-cpp-rs/blob/0dd5814c9b061930965d144c1653cd4d05054873/llama-cpp-sys-2/build.rs
[llama-cpp-2 common]: https://github.com/utilityai/llama-cpp-rs/blob/0dd5814c9b061930965d144c1653cd4d05054873/llama-cpp-2/src/lib.rs#L39-L48
[llama-cpp-sys-2 pin]: https://github.com/utilityai/llama-cpp-rs/tree/0dd5814c9b061930965d144c1653cd4d05054873/llama-cpp-sys-2
[bitmap fix]: https://github.com/utilityai/llama-cpp-rs/commit/3553de8afaa40aef06ee89b816f0fba5a06479f9
[chunks fix]: https://github.com/utilityai/llama-cpp-rs/commit/355a7a4771ec2cff4bee8f1b290a57e49ca81fd0
[send fix]: https://github.com/utilityai/llama-cpp-rs/commit/d181df6dad3022e57323bfdb852b60d50a310fb3
[llama.h abort]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/include/llama.h#L400-L404
