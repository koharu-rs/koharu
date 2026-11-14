# koharu-ml

Koharu's vision models.

The models that run on this machine to lay out a page, read its text, and
redraw what's under the text. Each is a port that stays traceable to a pinned
commit of its upstream, or runs on stable-diffusion.cpp as upstream's own
pipeline does. The links are under [Sources](#sources).

## Goal

A model gives the result its upstream gives on the same input. A port keeps
upstream's names, construction order, tensor layouts, execution order, and
postprocessing, so a checkpoint loads as it is and a divergence is written next
to the code that makes it.

## 1. Models

| Model | Task | Runs on | Upstream |
|---|---|---|---|
| Koharu Layout HighRes, an RF-DETR segmenter | text, onomatopoeia, balloons, and panels | libtorch | [RF-DETR], [Koharu Layout] |
| manga-ocr-ar-experimental | Japanese OCR | libtorch | [manga-ocr-ar] |
| Baberu OCR, with DINOv2 | Japanese OCR | libtorch | [Baberu OCR] |
| Hayai OCR v2.5 Nova, with SigLIP2 | Japanese, Chinese, Korean, and English OCR | libtorch | [Hayai OCR] |
| LaMa, fine-tuned on manga | inpainting | libtorch | [LaMa], as [IOPaint] runs it |
| RORem | inpainting | stable-diffusion.cpp | [RORem] |
| FLUX.3 Image Klein | inpainting and generation | stable-diffusion.cpp | [FLUX.3 Klein] |
| MOSS-Image SFT, with Qwen3.5 9B | inpainting and generation | stable-diffusion.cpp | [MOSS-Image] |
| Qwen-Image 2.1, with Qwen3-VL 8B | inpainting, generation, and editing | stable-diffusion.cpp | [Qwen-Image] |

Weights come from Hugging Face at a pinned commit, downloaded the first time a
model loads. manga-ocr-ar-experimental is CC BY-NC-SA 4.0, and Qwen-Image 2.1
is under the Qwen Research License, for research and evaluation only.

## 2. One lifecycle

Every model loads on a device, downloading any of its files that are missing,
then runs on its own inputs and returns its own results. The network and its
weights stay apart from the preprocessing and postprocessing, so the work on a
page's pixels can run off the device's thread.

| Part | Model |
|---|---|
| device | the runtime's, given once when the model loads |
| precision | BF16 where the device computes in it, F32 elsewhere; libtorch has no Vulkan build, so on Vulkan its models run on the CPU |
| weights | every weight a port declares comes from the checkpoint; a missing one, an unexpected one, or a shape that differs is an error, since each means the port's names don't match |
| inference | gradients off, and no tensor leaves inference mode |
| diffusion models | stable-diffusion.cpp places their weights in the device's free memory, then system memory |

## 3. What the models return

- **Layout** returns the page's lettering in reading order, each with its kind,
  score, box, glyph mask, and the balloon and panel it's in; the balloons, each
  with its outline and what it holds; and the panels. A balloon's outline is
  divided among the groups of text in it, one room each, where translations go.
  Each class has its own confidence threshold.
- **OCR** returns each crop's text.
- **Inpainting** returns the image redrawn under its mask.

FLUX.3 and MOSS-Image ship no inpainting pipeline, so both inpaint as diffusers'
flow-matching image-to-image does with a mask [diffusers Flux2 Klein inpaint]:
the image fitted under a megapixel, optionally cropped around the mask, and the
result blended back by the mask. Qwen-Image 2.1 edits by generating from
condition images, as its diffusers pipeline does [diffusers Qwen-Image inpaint],
a mask being one more condition image the prompt names.

## Sources

Read in November 2026.

- Layout: [RF-DETR], [Koharu Layout]
- OCR: [manga-ocr-ar], [Baberu OCR], [Hayai OCR]
- Inpainting and generation: [LaMa], [IOPaint], [RORem], [FLUX.3 Klein],
  [MOSS-Image], [Qwen-Image], [diffusers Flux2 Klein inpaint], [diffusers
  Qwen-Image inpaint]

[RF-DETR]: https://github.com/roboflow/rf-detr/tree/4ab7c18729de9d02ffd0495795d0831b5630f01b
[Koharu Layout]: https://huggingface.co/mayocream/koharu-layout-highres
[manga-ocr-ar]: https://huggingface.co/muscgab/manga-ocr-ar-experimental
[Baberu OCR]: https://huggingface.co/genshiai-daichi/baberu-ocr
[Hayai OCR]: https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2.5-nova
[LaMa]: https://github.com/advimman/lama/tree/786f5936b27fb3dacd2b1ad799e4de968ea697e7
[IOPaint]: https://github.com/Sanster/IOPaint/tree/61a759fb3f332bacdce8b2813f4837495c9b86e0
[RORem]: https://github.com/leeruibin/RORem/tree/891ab8a17bbcfb16a773c078cb256aae8cb30468
[FLUX.3 Klein]: https://github.com/black-forest-labs/flux-3-image-klein/tree/62e90b9278b73ecc52927770c6965dc6b1966498
[MOSS-Image]: https://huggingface.co/OpenMOSS-Team/MOSS-Image-SFT
[Qwen-Image]: https://huggingface.co/Qwen/Qwen-Image-2.1
[diffusers Flux2 Klein inpaint]: https://github.com/huggingface/diffusers/blob/a37f6f8394ac2a7ee8360c3abea811efe54512b1/src/diffusers/pipelines/flux2/pipeline_flux2_klein_inpaint.py
[diffusers Qwen-Image inpaint]: https://github.com/huggingface/diffusers/blob/74fb49b5506a6960d55de0f7e6f4e0631213dc63/src/diffusers/pipelines/qwenimage/pipeline_qwenimage_edit_inpaint.py
