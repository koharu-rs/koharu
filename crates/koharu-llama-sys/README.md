# koharu-llama-sys

llama.cpp's C API.

The declarations `koharu-llama` wraps: llama.cpp, ggml, and the multimodal
library from llama.cpp's own headers [llama.cpp], and a small C wrapper over
llama.cpp's C++ common library. The libraries come from the runtime's
download, and their functions resolve when they're opened. The links are under
[Sources](#sources).

## Goal

The headers and the libraries always come from the same llama.cpp release, so
a function never has a signature the library doesn't, and a struct is never a
different size on either side.

## 1. Model

| Part | Model | Cite |
|---|---|---|
| crate | declarations only, for a safe crate to wrap | Cargo's `-sys` convention [Cargo sys] |
| headers | llama.cpp's, ggml's, and the multimodal library's, copied unchanged from the release | |
| the wrapper | C functions over llama.cpp's common library, which is C++: fitting a model's parameters to free memory, turning a JSON Schema into a grammar, grammar samplers, and multi-token prediction | the C wrapper llama-cpp-rs builds for the same library [llama-cpp-rs wrapper] |
| release | the koharu-rs/llama release the runtime downloads, named after the llama.cpp release it's built from, with the wrapper built in | llama.cpp's releases [llama.cpp releases] |
| left out | a function the headers declare that no build exports, or opening would always fail | |

The wrapper's C signatures stay as llama-cpp-rs wrote them. Where llama.cpp
has moved on, as when its fit started taking a model's files as a list, the
wrapper adapts inside and keeps its signature.

## Sources

Read in November 2026.

- [llama.cpp], [llama.cpp releases], [llama-cpp-rs wrapper], [Cargo sys]

[llama.cpp]: https://github.com/ggml-org/llama.cpp
[llama.cpp releases]: https://github.com/ggml-org/llama.cpp/releases
[llama-cpp-rs wrapper]: https://github.com/utilityai/llama-cpp-rs/tree/0364f5d/llama-cpp-sys-2
[Cargo sys]: https://doc.rust-lang.org/cargo/reference/build-scripts.html#-sys-packages
