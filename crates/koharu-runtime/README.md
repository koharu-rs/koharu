# koharu-runtime

Gets a machine ready to run models.

It picks the GPU models run on, downloads and opens the native libraries that
run on it, and fetches model files. Everything is pinned to a version and kept
in one store, so once it's downloaded, nothing goes over the network again.
Where it follows an established product, it cites it; the links are under
[Sources](#sources).

## Goal

The first launch downloads what this machine needs, and every later launch
runs from disk. A model runs on the same library builds on every machine of a
kind, so a result on one machine is the result on another.

## 1. The device

One GPU runs every model, the best this machine has. The models share its
memory, so splitting them across GPUs would make whether they fit depend on
which ran first. GPUs rank:

| Rank | GPU |
|---|---|
| 1 | Metal, or a discrete NVIDIA GPU |
| 2 | a discrete AMD GPU |
| 3 | an integrated NVIDIA GPU |
| 4 | an integrated AMD GPU |
| 5 | Vulkan |

Within a rank, more memory wins, then a newer CUDA architecture. If the best
GPU can't be used, starting fails rather than falling back to the next, so a
model never runs somewhere the user didn't expect. llama.cpp must list the GPU
too, so a Vulkan loader without a Vulkan device doesn't count. libtorch has no
Vulkan build, so on Vulkan its models run on the CPU.

## 2. Packages

| Package | From | Needed for |
|---|---|---|
| CUDA | NVIDIA's wheels on PyPI | an NVIDIA GPU |
| ROCm | AMD's wheels, from TheRock [TheRock] | an AMD GPU |
| libtorch, and Koharu's C shim to it | the koharu-rs/torch releases | every GPU |
| llama.cpp | the koharu-rs/llama releases | every GPU |
| stable-diffusion.cpp | the koharu-rs/diffusion releases | every GPU |

- **Pinned.** Each package's version is a constant: the `-sys` crate's for
  Koharu's own builds, the runtime's for NVIDIA's and AMD's. Koharu's builds are
  one archive per platform and GPU backend.
- **All or nothing.** Every package downloads before any opens, so a failed
  download leaves nothing half loaded.
- **Open for good.** The libraries stay open for the life of the process, in
  the order each needs the ones before it.

## 3. The store

| Part | Model | Cite |
|---|---|---|
| place | the `store` folder beside the program in Windows release builds, where setup installs Koharu, and the system cache folder's `koharu/store` otherwise | |
| packages | a folder per package, per version, per platform and backend | |
| models | Hugging Face repositories at a pinned commit, laid out as Hugging Face's own cache lays them out, by repository and commit snapshot, so a newer commit never mixes into an older one | the `huggingface_hub` cache's `snapshots` by revision [HF cache] |
| writing | into a staging folder on the same disk, validated, then renamed into place, so nothing is ever half written; a lock other processes respect holds off a second download of the same thing | |
| clearing | once a start has loaded its packages, every other version of them goes, in the background; models stay | |

A version another running Koharu has open can't be removed, so it stays until
a later start.

## 4. Transfers

- **Ranges.** A file downloads in ranges over several connections when the
  server takes ranges, and as one stream otherwise. Each retries a few times,
  backing off.
- **Model files** come through Hugging Face's own client at the pinned commit,
  one after another, each as one stream.
- **Progress.** Every transfer's state goes to whoever is listening, which the
  app shows as its list of downloads.

## Sources

Read in November 2026.

- [HF cache], [TheRock]

[HF cache]: https://huggingface.co/docs/huggingface_hub/guides/manage-cache
[TheRock]: https://github.com/ROCm/TheRock
