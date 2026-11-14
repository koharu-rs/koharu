# koharu-torch

libtorch for Koharu's vision models.

Rust bindings to libtorch, forked from tch-rs [tch-rs], over the C functions of
`koharu-torch-sys`. The links are under [Sources](#sources).

## Goal

A model port runs the operators its upstream runs in PyTorch, with the same
arguments, and a failure inside libtorch is an error the caller handles, never
a crash: a model meets input it hasn't seen, and a panic would end the
pipeline's run that met it rather than report it.

## 1. Operators

| Part | Model | Cite |
|---|---|---|
| where they come from | generated when the crate builds, from the shim's header of ATen operators, so they always match the libtorch the runtime loads; none are checked in | tch-rs generates its operators from PyTorch's operator declarations, but checks the result in [tch-rs operators] |
| shape | a tensor method for each operator, or a free function when it takes no tensor, named as in PyTorch | |
| errors | every operator returns a `Result`, holding libtorch's exception message | tch-rs offers `f_` methods that return one, beside methods that panic [tch-rs operators] |
| arguments | tensors, lists, optional values, kinds, devices, and tensor options, as Rust types | |
| left out | an operator that takes a fixed-length list of bools, since the shim reads the length its schema says and a shorter list would be read past its end; a parameter of a type the generator doesn't know stops the build | |

## 2. The rest

- **Tensors** own their handle and free it when dropped. A tensor can move
  between threads, but not be shared between them.
- **Indexing** reads as NumPy's does [NumPy indexing].
- **Conversion** to and from flat vectors, and from a one-element tensor to a
  number.
- **Inference mode** is libtorch's, on the calling thread: a tensor made in it
  can't be changed in place outside it.
- **CUDA**: memory in use, emptying the cache, synchronizing, and cuDNN's
  settings.

## Sources

Read in November 2026.

- [tch-rs], [tch-rs operators], [NumPy indexing]

[tch-rs]: https://github.com/LaurentMazare/tch-rs
[tch-rs operators]: https://github.com/LaurentMazare/tch-rs/blob/main/src/wrappers/tensor_fallible_generated.rs
[NumPy indexing]: https://numpy.org/doc/stable/user/basics.indexing.html
