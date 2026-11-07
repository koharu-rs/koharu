# koharu-torch

Rust bindings to libtorch, forked from tch-rs. Every call goes through functions
`koharu-torch-sys` resolves from libraries opened at runtime, so its `load` must
have succeeded first. Unlike tch-rs, operators always return `Result`, and they are
generated at build time instead of being checked in.

## Generated operators

`build.rs` reads the bindings and `koharu_torch_aten.h`, and writes a `Tensor` method
for each operator in that header, or an associated function if it takes no `self`:

```rust
pub fn argmax(&self, dim: impl Into<Option<i64>>, keepdim: bool) -> Result<Tensor>
```

A method's name is the C name in lower case without `aten_`, with `internal_` for
leading underscores. An operator that takes a fixed-length bool list is skipped: the
shim reads the schema's length, so a shorter slice would be read past its end. A
parameter of a type `build.rs` doesn't know stops the build. These are the main
mappings:

| C parameter | Rust parameter |
|---|---|
| `AtenTensorHandle` named `self` | `&self` |
| `AtenTensorHandle` | `&Tensor` |
| `*mut AtenTensorHandle` | `Option<&Tensor>` |
| `*const AtenTensorHandle` and its length | `&[impl Borrow<Tensor>]` |
| `*const i64` and its length | `impl IntList` |
| `*mut *const i64` and its length | `impl IntListOption` |
| `AtenBool`, `AtenDtype`, `AtenLayout`, `AtenMemoryFormat` | `bool`, `Kind`, `Layout`, `MemoryFormat` |
| `*mut i64`, `*mut f64`, `*mut AtenBool`, `*mut AtenDtype` | `impl Into<Option<_>>` of `i64`, `f64`, `bool`, `Kind` |
| `AtenDeviceType` and its index | `Device` |
| `dtype`, `layout`, `device`, `device_index_`, `pin_memory` | `options: impl Into<Option<(Kind, Device)>>` |
| `ret0`, `ret1`, … | The return value: one, or a tuple of several |

## Hand-written API

| Item | What it does |
|---|---|
| `Tensor` | Owns a handle, deleted on drop. It is `Send` but not `Sync` |
| `IndexOp` | NumPy-style indexing through `i` |
| `TryFrom` | Converts a tensor to and from a flat `Vec<T>`, and a one-element tensor to a number |
| `inference_mode`, `InferenceMode` | `c10::InferenceMode` on the calling thread. A tensor made in it can't be changed in place outside it |
| `Cuda` | `memory`, `empty_cache`, `synchronize`, and the cuDNN settings |

## Errors

A libtorch failure becomes `Error::Torch`, holding the message of the exception it
caught on the calling thread. The other `Error` variants come from this crate's own
checks of shapes, kinds, and conversions.
