# koharu-ml

The vision models Koharu runs locally for page layout, OCR, and inpainting. Weights come
from Hugging Face at a pinned commit, and each ported network's source links the upstream
commit it follows.

## Models

| Module | Model | Task | Runs on |
|---|---|---|---|
| `koharu_layout_highres` | Koharu Layout HighRes, an RF-DETR | Layout | libtorch |
| `manga_ocr_ar` | manga-ocr-ar-experimental (CC BY-NC-SA 4.0) | OCR | libtorch |
| `baberu_ocr` | Baberu OCR, with DINOv2 | OCR | libtorch |
| `hayai_ocr` | Hayai OCR v2.5 Nova | OCR | libtorch |
| `lama` | LaMa | Inpainting | libtorch |
| `rorem_mixed` | RORem | Inpainting | stable-diffusion.cpp |
| `flux3_klein` | FLUX.3 Image Klein | Inpainting and generation | stable-diffusion.cpp |
| `moss_image` | MOSS-Image SFT, with Qwen3.5 9B | Inpainting and generation | stable-diffusion.cpp |
| `qwen_image` | Qwen-Image 2.1, with Qwen3-VL 8B (Qwen research license) | Inpainting, generation, and editing | stable-diffusion.cpp |

## Lifecycle

Every model has `load(...).await`, which downloads any of its files that are missing. They
differ in what `load` needs and how they run:

| Models | `load` takes | Runs with | Returns |
|---|---|---|---|
| Layout | `&Device` | `inference(&image, &Options)`, then `layout(&page)` | `Detected`, then `Layout` |
| manga-ocr-ar, Baberu, Hayai | `&Device` | `inference(&[image])` | `Vec<String>` |
| LaMa | `&Device` | `inference(&image, &mask)` | `RgbImage` |
| RORem | `&Device` | `inference(&image, &mask, &Options)` | `RgbImage` |
| FLUX.3 | `&Device` | `generate(&references, prompt, &GenerateOptions)` | `Vec<RgbImage>` |
| | | `inpaint(prompt, &image, reference, &mask, &InpaintOptions)` | `RgbImage` |
| MOSS-Image | `&Device` | `generate(prompt, &GenerateOptions)` | `Vec<RgbImage>` |
| | | `inpaint(prompt, &image, &mask, &InpaintOptions)` | `RgbImage` |
| Qwen-Image 2.1 | `&Device` | `generate(prompt, &images, &Options)` | `Vec<RgbaImage>` |

`Device` is the runtime's, re-exported here. stable-diffusion.cpp places its models'
weights in the device's free memory itself, then in system memory.

Neither FLUX.3 nor MOSS-Image ships an inpainting pipeline, so both inpaint as diffusers'
flow-matching img2img does with a mask, in `diffusers.rs`: the image fitted under a
megapixel, optionally cropped around the mask, and the result blended back by the mask at
that size. MOSS-Image takes its own VAE, whose encoder and latent statistics differ from
FLUX.2's, and shares its Qwen3.5 9B GGUF with koharu-llm's catalog.

Qwen-Image 2.1 edits by generating from condition images, which its Qwen3-VL text encoder
reads through its vision projector; a mask is one more condition image the prompt names. It
decodes four channels, so its images carry alpha.

A model's folder keeps its parts apart: `mod.rs` holds the public API, `model.rs` the
network and weight loading, `processor.rs` the preprocessing and postprocessing, and
`config.rs` the checkpoint's configuration.

## libtorch models

- **Precision.** A model runs in BF16 where the device computes in it, and in F32 elsewhere.
  On Vulkan, libtorch has no build, so its models run on the CPU.
- **Weights.** `VarStore::load` fills every weight from a `.safetensors` file. A weight the
  file doesn't have, a shape that differs, or a weight the model doesn't declare is an error,
  because each means the port's names don't match the checkpoint's.
- **Inference.** Every call runs in libtorch's inference mode, and no tensor leaves it.

## Layout

`KoharuLayoutHighres::inference` returns the network's `Detected`, and its `layout(&page)`,
the CPU work on the page's pixels that a caller can run off the device's thread, returns a
`Layout`:

| Field | Holds |
|---|---|
| `lettering` | Text and onomatopoeia in reading order: `kind`, `score`, `bbox`, a glyph `mask`, and the indices of its bubble and panel. A text also has its `room` in its bubble |
| `bubbles` | `score`, `bbox`, `mask`, a simplified `outline`, and the lettering it holds |
| `panels` | The page's panels |

`Options` overrides each class's confidence threshold. The defaults are the checkpoint's,
except onomatopoeia at 0.4 and bubbles at 0.4, both tuned on Blue Archive's comics. `rooms`
divides a bubble's outline among groups of text, one room per group.

## Examples and benchmarks

Each libtorch and diffusion model has an example in `examples/`, and most have a benchmark
in `benches/`. They download the weights they need the first time they run.
