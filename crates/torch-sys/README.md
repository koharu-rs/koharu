# torch-sys, Koharu's build

Koharu runs libtorch through the `tch` crate [tch], with this crate standing
in for its `torch-sys`. Upstream's `torch-sys` compiles its C++ shim and links
libtorch at build time, with no option to load it at run time [torch-sys
build]; this one binds the same declarations to the koharu-rs/torch shim that
`koharu-runtime` installs for the machine's GPU, loaded when the runtime
starts. `koharu-torch` and `koharu-torch-sys`, Koharu's fork of `tch`, go.

```toml
# the workspace manifest
[workspace.dependencies]
tch = { version = "=0.26.0", default-features = false }

[patch.crates-io]
torch-sys = { path = "crates/torch-sys" }
```

## How it stands in

| Rule | Evidence |
|---|---|
| The package is named `torch-sys`, at the version `tch` asks for, and the workspace patches crates.io with it, so `tch` builds from crates.io against it. | a `[patch]` entry replaces a crates.io package with a local one of the same name and a compatible version [Cargo patch] |
| It declares upstream's features (`doc-only`, `download-libtorch`, `python-extension`) as features that do nothing, so `tch`'s feature forwarding resolves. `python-extension` stays off: the shim doesn't export libtorch's Python functions, so `load` would fail. | `tch` forwards its features to `torch-sys` [tch features] |
| Its declarations are upstream's own Rust sources, copied unchanged into `upstream/`, so every name and type `tch` calls is the same; `bindgen` over the shim's C headers would give the hand-written functions other types. `koharu-bindgen` rewrites their `extern` blocks into functions resolved from the shim when the runtime starts. | upstream's sources [torch-sys src]; `koharu-bindgen`'s README |
| One file diverges. Upstream's shim is linked in and calls `io.rs`'s stream callbacks by name, which a library loaded at run time can't, so the koharu-rs/torch shim takes them through `at_set_stream_callbacks`; `io.rs` adds `register_stream_callbacks`, which the runtime calls after `load`. | the shim's header [shim streams] |
| `tch`'s own build script passes `-ltorch` to the linker, but Cargo passes a package's link arguments only to that package's own targets, so Koharu's binaries don't link libtorch. | Cargo's link arguments [Cargo link args] |

## Versions in step

- **One commit.** `tch` 0.26.0, the `torch-sys` sources copied here, and the
  koharu-rs/torch release the runtime installs, v2.13.0.7, are all built from
  tch-rs commit `4227b89` for libtorch 2.13.0 [tch vcs] [shim release]. The
  shim's headers differ from upstream's only in `const`, `int8_t` for
  upstream's `uint8_t` null flags, and one typedef's name, none of which
  changes how a function is called, and the shim exports every function
  these declarations name.
- **An upgrade** waits for a `tch` release and a shim release built for the
  same libtorch, then bumps `tch`, copies that `torch-sys`'s sources here,
  and records the shim's hashes in the runtime. A shim that lacks a function
  fails `load`, which resolves every one before any runs.

## Errors and memory

The shim catches each `std::exception` and keeps its message, with libtorch's
C++ backtrace, until `get_and_reset_last_err` takes it [shim protect]. `tch`
returns it as `TchError::Torch` from each `f_` op and panics with it from the
others [tch fallible]; Koharu calls the `f_` forms (`koharu-ml`'s README), so
nothing reports running out of memory as an error of its own.

The release binds no way to empty libtorch's caching allocator, so the memory
a dropped model held stays reserved for libtorch's next tensors rather than
going back to the driver for llama.cpp or stable-diffusion.cpp [PyTorch
empty_cache].

## Changes from today

- `koharu-torch` (79,323 lines of Rust) and `koharu-torch-sys` (351 lines,
  over 2,987 lines of copied headers) go. `tch` brings every op the shim
  exports, each in a panicking and an `f_` form.
- Inference layers and checkpoint loading move to `koharu-ml`, as `tch`'s
  `nn` builds for training: weights start random and track gradients, its
  layers panic, and loading ignores weights the model doesn't declare.
- `tch` brings `zip` 0.6, `safetensors` 0.3, and `thiserror` 1 beside the
  workspace's newer majors, and `ndarray`, `rand`, and `lazy_static`, which
  the workspace doesn't otherwise use.

## Sources

[tch]: https://docs.rs/tch/0.26.0/tch/
[torch-sys build]: https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/torch-sys/build.rs#L379-L458
[Cargo patch]: https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section
[tch features]: https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/Cargo.toml#L47-L51
[torch-sys src]: https://github.com/LaurentMazare/tch-rs/tree/4227b89b72059b0651ff83a38637693574e0a2d6/torch-sys/src
[shim streams]: https://github.com/koharu-rs/torch/blob/565302e8e0912582dd610d4fd15534c36fdab31f/shim/torch_api.h#L99-L113
[Cargo link args]: https://github.com/rust-lang/cargo/blob/797e8a9bca276c1c9f9f738d2a20f484fa4eea9d/src/cargo/core/compiler/mod.rs#L585-L596
[tch vcs]: https://docs.rs/crate/tch/0.26.0/source/.cargo_vcs_info.json
[shim release]: https://github.com/koharu-rs/torch/releases/tag/v2.13.0.7
[shim protect]: https://github.com/koharu-rs/torch/blob/565302e8e0912582dd610d4fd15534c36fdab31f/shim/torch_api.h#L19-L24
[tch fallible]: https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/gen/gen.ml#L786
[PyTorch empty_cache]: https://docs.pytorch.org/docs/2.13/generated/torch.cuda.memory.empty_cache.html
