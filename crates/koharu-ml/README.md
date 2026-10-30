# koharu-ml

`koharu-ml` holds the vision models Koharu runs on the user's machine for
the pipeline's detection, OCR, and inpainting steps. Each is a port that stays
traceable to a pinned upstream, with one lifecycle for all. They run on libtorch (`koharu-torch`),
stable-diffusion.cpp (`koharu-diffusion`), or, for PaddleOCR-VL, llama.cpp
through `koharu-llm`'s engine.

```rust,ignore
let device = koharu_runtime::device().await?;
let ocr = MangaOcr::load(&device).await?;
let texts = ocr.inference(&crops)?;        // one call for a batch of crops
```

## Models

| Role | Module | Runs on | Upstream code | Used by |
|---|---|---|---|---|
| layout and instance masks | `koharu_layout_rfdetr_seg_2xl` | libtorch | rf-detr `4ab7c18` | detection |
| OCR | `manga_ocr` | libtorch | manga-ocr `847e093` | OCR |
| OCR | `baberu_ocr` | libtorch | transformers DINOv2 `c6c8503` | OCR |
| OCR | `hayai_ocr` | libtorch | SigLIP2 at a commit (today `main`) | OCR |
| OCR | `paddle_ocr_vl_quantized` | llama.cpp, through `koharu-llm` | the model card's task prompts, `66317ac` | OCR |
| inpainting | `lama` | libtorch | IOPaint `61a759f` | inpainting |
| inpainting | `aot_inpainting` | libtorch | BallonsTranslator `4bcc635` | inpainting |
| inpainting | `rorem_mixed` | stable-diffusion.cpp | RORem `891ab8a` | inpainting |
| inpainting and generation | `flux2_klein` | stable-diffusion.cpp | diffusers `a37f6f8` | inpainting |

## Lifecycle

| Rule | Evidence |
|---|---|
| `load(&Device)` resolves the model's pinned files, converts the device once into a libtorch device and a precision chosen from that device, builds the network, and loads its weights, which never track gradients; network and weights are apart from pre- and post-processing. | `AGENTS.md` "Models" |
| Loading fails on a missing weight and on an unexpected one, except where upstream itself allows it: a BatchNorm's `num_batches_tracked`, which PyTorch fills in when a checkpoint leaves it out. | `AGENTS.md`: missing or unexpected weights mean a name mismatch; PyTorch's BatchNorm loading [PyTorch batch norm] |
| `inference(&self, …)` runs under `no_grad` and returns a `Result`: a port calls `koharu-torch`'s `f_` ops and this crate's layers, never its panicking forms. | `AGENTS.md`; `tch`'s fallible ops [tch fallible] |
| Layers and the weight store are this crate's own (`nn`): a constructor declares each weight under the checkpoint's name, in the store's device and precision, holding nothing until the store fills it from the checkpoint once. | `tch`'s `nn` starts each weight random and tracking gradients [tch var], and its loading ignores weights the model doesn't declare [tch load] |
| Token ids and index tensors are made on the model's device. | `AGENTS.md`: "a CPU tensor used against accelerator embeddings is a defect" |
| Each model has one loader, and detectors default their thresholds the same way. | `AGENTS.md`: the same public lifecycle for every model |

Precision is BF16 where the given device supports it and F32 elsewhere, as
now, but read from that device rather than a fresh scan of the machine. A
diffusion model takes the pipeline's memory budget when it loads.
PaddleOCR-VL 1.6 keeps its prompt and result parsing here and runs on
`koharu-llm`'s engine, so its crops decode together.

## Devices

The runtime hands out the one device it started on; there's no device before
it starts. The cuDNN and MIOpen setting is applied once, when libtorch loads,
instead of on every model load. A device libtorch can't run on, such as Vulkan,
fails the load instead of falling back to the CPU.

## Batches

The OCR models take a batch of crops in one call: their inputs are fixed-size
(Manga OCR's 224 pixels square, Baberu's square input, Hayai's padded patches),
so crops stack. Each batched port is compared with single-crop runs on the
same crops, as `AGENTS.md` asks of a port. Crops arrive upright in the block's
direction from the pipeline; no model turns a crop by its shape.

## Checks

Benchmarks run on the device the runtime started, synchronising before the
clock stops, on representative pages (`AGENTS.md` "Performance"). The
command-line programs become examples, so their dependencies (`clap`,
`tracing-subscriber`, `ab_glyph`) are development dependencies only [Cargo
examples].

[Cargo examples]: https://doc.rust-lang.org/cargo/reference/cargo-targets.html#examples
[PyTorch batch norm]: https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/torch/nn/modules/batchnorm.py#L117-L141
[tch fallible]: https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/gen/gen.ml#L786
[tch var]: https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/src/nn/var_store.rs#L560-L569
[tch load]: https://github.com/LaurentMazare/tch-rs/blob/4227b89b72059b0651ff83a38637693574e0a2d6/src/nn/var_store.rs#L204-L222
