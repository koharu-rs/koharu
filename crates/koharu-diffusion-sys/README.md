# koharu-diffusion-sys

stable-diffusion.cpp's C API.

The declarations of stable-diffusion.cpp's one public header [sd.cpp], for
`koharu-diffusion` to wrap. The library itself comes from the runtime's
download, and its functions resolve when it's opened. The links are under
[Sources](#sources).

## Goal

The header and the library always come from the same commit, so a function
never has a signature the library doesn't.

## 1. Model

| Part | Model | Cite |
|---|---|---|
| crate | declarations only, for a safe crate to wrap | Cargo's `-sys` convention [Cargo sys] |
| header | `stable-diffusion.h`, copied unchanged from the commit the release is built from; everything it declares is bound, but not the C library functions of the system headers it includes | |
| release | the koharu-rs/diffusion release the runtime downloads, named after sd.cpp's own release and built from it | sd.cpp's releases are named `master-<number>-<commit>` [sd.cpp releases] |
| bindings | generated when the crate builds, by `koharu-bindgen`, and resolved when the runtime opens the library | |

Moving to a new sd.cpp release changes the header, the release, and the commit
together, and whatever the safe crate wraps follows.

## Sources

Read in November 2026.

- [sd.cpp], [sd.cpp releases], [Cargo sys]

[sd.cpp]: https://github.com/leejet/stable-diffusion.cpp
[sd.cpp releases]: https://github.com/leejet/stable-diffusion.cpp/releases
[Cargo sys]: https://doc.rust-lang.org/cargo/reference/build-scripts.html#-sys-packages
