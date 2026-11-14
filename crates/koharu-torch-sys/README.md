# koharu-torch-sys

libtorch's C API and Koharu's shim over it.

The declarations `koharu-torch` wraps: a C function for each of libtorch's
operators and what else its C++ API offers, from Koharu's shim, and libtorch's
own C functions that make, inspect, and free tensors. The libraries come from
the runtime's download, and their functions resolve when they're opened. The
links are under [Sources](#sources).

## Goal

libtorch's API is C++, which no Rust binding can call directly. The shim
gives every operator a C function, so the Rust side is generated from headers
alone, and the headers always come from the same release as the libraries.

## 1. Model

| Part | Model | Cite |
|---|---|---|
| crate | declarations only, for a safe crate to wrap | Cargo's `-sys` convention [Cargo sys] |
| libtorch's C API | the functions libtorch already exports to make, inspect, and delete tensors and to report the last error | PyTorch's AOTInductor C shim [PyTorch C shim] |
| the shim | a C function for each ATen operator, about 3,100, and what else the C++ API offers that libtorch's C functions lack; its C types are named so a binding tells a dtype from a flag | |
| left out | libtorch's per-backend functions, which its headers declare in every build but only some builds export: Metal's in a Metal build, the CPU's with MKL and oneDNN | |
| headers | copied unchanged from the shim release's headers, libtorch's C headers they include among them | |
| releases | the shim and libtorch release apart, each with its own build counter, and the shim names the libtorch it was built against | the koharu-rs/torch releases [koharu-rs/torch] |

The shim's operator header is also the input `koharu-torch` generates its
tensor methods from, so a new release's operators reach Rust with no
hand-written change.

## Sources

Read in November 2026.

- [koharu-rs/torch], [PyTorch C shim], [Cargo sys]

[koharu-rs/torch]: https://github.com/koharu-rs/torch/releases
[PyTorch C shim]: https://github.com/pytorch/pytorch/blob/main/torch/csrc/inductor/aoti_torch/c/shim.h
[Cargo sys]: https://doc.rust-lang.org/cargo/reference/build-scripts.html#-sys-packages
