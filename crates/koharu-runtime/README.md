# koharu-runtime

`koharu-runtime` gets Koharu ready to run models on this machine. It finds the
GPU, installs the native libraries that run on it (libtorch, llama.cpp,
stable-diffusion.cpp, and the CUDA or ROCm libraries they need), fetches
model files pinned to a commit, and reports how much of the device's memory is
free. Everything it fetches is pinned to a version, and after the first
install nothing goes over the network.

```rust,ignore
use koharu_runtime::Repository;

const MANGA_OCR: Repository = Repository::model("mayocream/manga-ocr", "4380edb…");

let device = koharu_runtime::device().await?;
let [config, weights] = MANGA_OCR.resolve(["config.json", "model.safetensors"]).await?;
let free = device.memory().free;

let mut transfers = koharu_runtime::transfers();
while let Some(transfer) = transfers.next().await {
    // send it to the frontend
}
```

Everything a caller uses is at the crate's root, and there's nothing to set up
first: the first call that needs the GPU or a file does the work.

## The device

`device().await?` returns the `&'static Device` every model runs on. Its first
call installs and loads what the best GPU needs; every later call, from any
crate, returns the same device at once. A start that failed is tried again on
the next call. It fails only with `Error::NoGpu`, whose message says what
Koharu supports and why each GPU it found couldn't be used.

A `Device` is plain data:

| Field | |
|---|---|
| `name` | llama.cpp's name for it, such as `CUDA0`; stable-diffusion.cpp takes this |
| `index` | its index as CUDA and HIP count devices, which is also its place in llama.cpp's device list; libtorch and `koharu-llama` take this |
| `description` | the driver's name for it, such as `NVIDIA GeForce RTX 5090` |
| `backend` | `Cuda { compute_capability }`, `Rocm { target }`, `Vulkan`, or `Metal` |
| `integrated` | whether it shares the system's memory |

A model chooses its precision from the backend's own data, such as a compute
capability of 80 or more on CUDA. `device.memory()` reads the device's free
and total memory now, from the driver through llama.cpp; it can't fail, as the
device is one llama.cpp listed when it started.

## Model files

A `Repository` is a Hugging Face model or dataset at a pinned commit, made
with `Repository::model(id, commit)` or `Repository::dataset(id, commit)` and
declared once, as a constant, by the crate that uses it.

- **`resolve(names)`** returns the paths of the files it names, in the same
  order, so they destructure into one `let`. A file the store doesn't have is
  fetched first; files transfer one after another, each over many
  connections.
- **`exists(name)`** says whether the store has a file, without going over
  the network, such as to show which models are already in the store.

## Transfers

`transfers()` gives a receiver of every transfer's state, for the app's list
of them. `next().await` returns the next `Transfer`:

| Field | |
|---|---|
| `id` | the same for every state of one transfer |
| `name` | the file's name |
| `completed`, `total` | bytes; `total` is 0 while the size isn't known |
| `state` | `Running`, `Finished`, or `Failed(message)` |

Each `Transfer` is the whole state, so the app sends it to the frontend as it
is. A receiver that falls behind skips the oldest states it missed and goes
on.

## HTTP

`http_client()` is the app's one HTTP client, which `koharu-llm`'s hosted
providers send through: Koharu's user agent, the proxies the OS and the
environment set, and the timeouts `[http]` holds. The client itself retries
nothing, as a generation request mustn't be replayed.

## The store

Everything installs into one store: the OS cache folder's `koharu/store`,
or the `store` folder beside the executable in Windows release builds.
Nothing chooses another.

## How it works

### Finding the GPU

- **Candidates.** CUDA (driver 13.3 or later, compute capability 7.5 or
  later), ROCm, Vulkan, and Metal on Apple silicon, ranked best first: Metal
  or a discrete NVIDIA GPU, a discrete AMD GPU, integrated GPUs, then Vulkan.
  Within a rank, more memory and then a newer CUDA architecture come first.
- **A device is one llama.cpp found.** After its library loads, the runtime
  keeps a candidate only if llama.cpp lists it at the candidate's index under
  the candidate's name, so a Vulkan loader on disk without a Vulkan device
  doesn't count, and one index serves libtorch and llama.cpp alike.
- **Falling back.** One candidate whose packages fail to install or load gives
  way to the next, until a library the runtime binds has loaded; a loaded
  library can't be unloaded, so a failure after that is final.

### Packages

| Package | Pinned to | From |
|---|---|---|
| CUDA's libraries (runtime, NVRTC, nvJitLink, cuBLAS, cuFFT, cuRAND, cuSPARSE, cuSOLVER, cuDNN) | a PyPI version per wheel | NVIDIA's wheels on PyPI |
| ROCm's libraries and the GPU target's kernels | a ROCm version | AMD's wheel index |
| libtorch with the koharu-rs/torch shim | a release tag | `koharu-rs/torch` |
| llama.cpp | a release tag | `koharu-rs/llama` |
| stable-diffusion.cpp | a release tag | `koharu-rs/diffusion` |
| model files | a commit | the Hugging Face Hub |

- **Addresses.** A file's address is built from its version, the
  platform, and the backend, and for ROCm's kernels from the GPU's target,
  which discovery limits to the targets Koharu supports. No file is checked
  against a pinned hash; a file cut short fails where it's read, as an archive
  that doesn't extract or a model its loader refuses.
- **Layout.** Every package is installed at `<package>/<version>/…` in the
  store, and each of CUDA's wheels counts as a package, at
  `cuda/<project>/<version>`. A package is installed when its folder holds the
  files its loader opens.
- **Installing.** An install fetches and extracts into a staging folder on
  the store's own volume, off the async threads, and renames it into place
  under a lock that holds across processes, so two Koharus starting at once
  install it once.
- **Loading.** Everything installs before anything loads, so a transfer that
  fails leaves nothing loaded. Libraries then load in dependency order: CUDA
  or ROCm, libtorch, llama.cpp, then stable-diffusion.cpp, each resolving
  every function it binds before anything runs. stable-diffusion.cpp must
  report the commit its header came from, and libtorch's cuDNN switch, which
  also drives MIOpen, is off for ROCm on Windows.
- **Cleanup.** Once the device is running, one cleanup removes every other
  version of each package it installed. A version another running Koharu still
  holds open stays, and a later start removes it. Model files stay, as the
  runtime doesn't know which commits other crates still pin.

### Transfers

- **Ranges.** A large file arrives in 64 MiB ranges over up to 32
  connections, each range retried on its own and checked to be the range
  asked for, at its length; a server that doesn't take ranges sends it as one
  stream. A 7 GB model that fails near the end resumes that range instead of
  starting over. A transfer that fails removes what it wrote.
- **Retries.** One policy (`backon`, exponential with jitter): network
  failures, server errors, and 429 are retried, and other client errors
  aren't. `[http]` holds `connect_timeout` (20 s), `read_timeout` (300 s), and
  `max_retries` (3); a zero timeout is invalid.
