# koharu-runtime

Gets a machine ready to run models: it picks the best GPU, downloads and opens the
native libraries that run on it, and fetches model files. Everything is pinned to a
version and kept in one store, so after the first download nothing goes over the
network.

## API

| Item | What it does |
|---|---|
| `device().await` | The `&'static Device` models run on. Every caller shares one start, and a failed one is tried again on the next call |
| `Device::memory()` | The device's free and total bytes now, as llama.cpp reads them from the driver |
| `Repository::model(id, commit)`, `dataset(id, commit)` | A Hugging Face repository at a commit, which must be a full 40-digit hash at compile time |
| `Repository::resolve(names).await` | The files' paths, fetching any the store doesn't have, one after another |
| `Repository::exists(name)` | Whether the store has a file, without using the network |
| `transfers()` | Every transfer's state from now on. A receiver that falls behind skips the oldest |

## Starting

`device` takes the best GPU, and only that one: if it can't be used, `device` returns
an error rather than trying the next.

| Rank | GPU |
|---|---|
| 0 | Metal, or a discrete NVIDIA GPU |
| 1 | A discrete AMD GPU |
| 2 | An integrated NVIDIA GPU |
| 3 | An integrated AMD GPU |
| 4 | Vulkan |

Within a rank, more memory wins, then a newer CUDA architecture, then a lower index.

Everything downloads before anything opens, so a failed transfer leaves nothing
loaded. The versions of Koharu's own builds are the constants in each library's `-sys`
crate. Those builds are one archive per platform and backend, named
`{triple}-{variant}.tar.gz`, where the variant is `cpu`, `cuda`, `hip`, `metal`, or
`vulkan`.

| Package, in load order | From | Version | Needed for |
|---|---|---|---|
| CUDA | NVIDIA's wheels on PyPI | `WHEELS` in `packages/cuda.rs` | An NVIDIA GPU |
| ROCm | AMD's wheels at `stable.repo.amd.com` | `VERSION` in `packages/rocm.rs` | An AMD GPU |
| libtorch | `koharu-rs/torch` | `LIBTORCH` | Every GPU, with the CPU build on Vulkan |
| libtorch's C shim | `koharu-rs/torch` | `SHIM` | Every GPU, with the CPU build on Vulkan |
| llama.cpp | `koharu-rs/llama` | `RELEASE` | Every GPU |
| stable-diffusion.cpp | `koharu-rs/diffusion` | `RELEASE` | Every GPU |

CUDA, ROCm, and libtorch's own libraries are opened here and stay open for the life
of the process, so the bound libraries find them already loaded. The bound libraries
open through each `-sys` crate's `load`.

| Libraries | OS | libloading call | Flags |
|---|---|---|---|
| All of these | Windows | `os::windows::Library::load_with_flags` | `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR \| LOAD_LIBRARY_SEARCH_SYSTEM32` |
| CUDA, libtorch | Unix | `os::unix::Library::open` | `RTLD_NOW \| RTLD_LOCAL` |
| ROCm | Unix | `os::unix::Library::open` | `RTLD_NOW \| RTLD_GLOBAL` |

Once llama.cpp is loaded, it must list the GPU at its index and under its name, or
`device` fails. So a Vulkan loader on disk without a Vulkan device doesn't count.

## The store

The store is the `store` folder beside the executable in Windows release builds, and
the OS cache folder's `koharu/store` otherwise.

| What | Where |
|---|---|
| Koharu's builds | `{package}/{version}/{triple}-{variant}/` |
| A model file | `huggingface/{kind}/{owner}--{name}/snapshots/{commit}/{file}` |

A download returns at once if the target is valid. Otherwise it takes a lock file
beside the version's folder, shared across processes, and checks again. It then fills a staging
folder on the same volume, validates it, and renames it into place, so a target is
never half written.

Once a start has loaded its packages, it removes every other version beside the ones it
uses, in the background, under that same lock. Models stay. A failure only logs, so a
version another running Koharu holds open stays until a later start.

## Transfers

A file downloads in 64 MiB ranges over several connections when the server takes
ranges, and as one stream otherwise. Each range or stream retries up to 3 times, with
backoff from 250 ms to 5 s. Model files go through `hf-hub`, which takes Xet storage's
chunks in parallel, over the same HTTP client.
