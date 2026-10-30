# koharu-pipeline

`koharu-pipeline` runs the models that turn a scanned page into a translated
one in a [`koharu-scene`](../koharu-scene/README.md) project: detection, OCR,
translation, typesetting, and inpainting. It reads the project through a host,
writes each step's result on each page as one model edit, and decides which
models are loaded and what runs on the device when.

## Boundary

```rust,ignore
let pipeline = Pipeline::new(device);
let report = pipeline
    .run(&host, Run {
        target: Target::Pages(pages),
        steps: Steps::ALL,
        redo: false,
        cancel: cancel.clone(),
    })
    .await?;
```

```rust,ignore
pub trait Host: Send + Sync {
    /// Reads the project; the pipeline never holds a read across model work.
    fn read<T>(&self, read: impl FnOnce(&Session) -> T) -> T;
    /// Checks what the edit read and applies it, under one lock.
    fn apply(&self, edit: Edit) -> koharu_scene::Result<Change>;
    fn report(&self, progress: Progress);
}
```

| Rule | Evidence |
|---|---|
| The user keeps editing while a run goes on. A step reads what it needs, runs its model without holding the project, and hands its result back as an edit that names what it read. | FoundationDB rejects a transaction when another commit "writes a value that the second reads" [FoundationDB]; HTTP's `If-Match` prevents the lost update [RFC 9110 conditional]; an LSP edit names the document version it read, and the client answers `applied: false` when that version is stale [LSP applyEdit] |
| The host compares the reads and applies the edit under one lock, with nothing in between. | vscode-languageclient: "CRITICAL: make sure that after this check has run no async operation happens" [languageclient check] |
| A rejected step reads the project again and computes again, once; it never resends its old result. A second rejection is reported, naming the page and the step. | the client, not the host, implements retries [Kubernetes retries]; a stale result isn't shown to the user [LSP content modified], while a conflict the user can resolve is reported with enough to find it [RFC 9110 conflict] |
| One step on one page is one edit, so one undo step, labelled with the step and the page. | VS Code makes each `edit` call an undo step [VS Code edit]; Figma splits a plugin's run into undo steps with `commitUndo` [Figma commitUndo] |
| A run reports one progress stream: each page's step as it starts and finishes, and how many of the run's page steps are done. | LSP's work-done progress has begin, report, and end, a percentage, and messages such as "3/25 files" [LSP progress] |
| Cancelling stops the run between batches and between decoded tokens; finished steps stay. | Photoshop's modal context has `isCancelled` [Photoshop modal]; LSP keeps a cancelled request's partial results [LSP partial results] |

The app's host wraps its shared session: it applies the edit, sends the
change to the canvas and the frontend, and schedules the autosave. A test or
the `run` binary wraps a `Mutex<Session>`.

`Target` is a list of pages, a list of text layers (OCR, translation, or
typesetting of those blocks only), or a rectangle on a page (detection and
inpainting inside it). `Steps` is any set of the five; the pipeline doesn't add
steps the run didn't ask for.

## What a run writes

| Step | Model | Reads (ingredients) | Writes | Action |
|---|---|---|---|---|
| detection | layout segmentation (RF-DETR) | the scan's `/file`, `parentOf` | per balloon, a `shape` with its outline as `path` and no fill or stroke; per text block, a hidden `text` with no content, the block's box as `frame`, its rotation in `transform`, and its measured direction and character style; a `raster` "Cleanup" over the scan with a pixel `mask` "Text mask" painted where the blocks' text is | `c2pa.created` |
| OCR | the selected OCR model | the scan's `/file`, `parentOf`; the block's `/frame`, `/transform`, and `/direction` | the block's `/content` and `/language` | `c2pa.created` |
| translation | an LLM | the first block's `/content`, `parentOf`; the page's other blocks and, when sent, the scan, `inputTo` | a visible `text` per translated thought, with its content and the target language | `c2pa.translated`, with `sourceLanguage` and `targetLanguage` |
| typesetting | none | each balloon's `/path`; the blocks' `/frame` and `/style` | the project's "Dialogue" text style; each translation's `frame`, `text_style`, and `direction` | `c2pa.edited` |
| inpainting | the selected inpainter | nothing, as pixels aren't properties | the cleanup raster's pixels under the text mask | `c2pa.edited` |

Detection makes the blocks, and OCR fills their text in place, so a box the
user corrects before OCR is read where the user put it, and a rerun of OCR
takes over only `/content` and `/language`. Workflow data, such as a
detection's confidence or whether a block is dialogue, goes in
`attributes.koharu`, which the renderer never reads.

- **What runs.** A step does what's missing or out of date on a page: detection
  where the page has no detection, OCR where a block has no text or its text is
  stale, translation where a block has no translation or it's stale,
  typesetting where a translation has no frame or it's stale, and inpainting
  where the cleanup is empty or the run names a rectangle. `redo` reruns all
  of it. Lightroom likewise flags AI masks whose photo changed and recomputes
  them on Update [Lightroom masking].
- **Reruns.** A rerun writes into the layers the step made before, so what the
  user changed keeps showing, adds what's new, and removes what the step no
  longer produces; the scene keeps a removed layer the user changed.
- **Inputs.** A step reads its inputs from the project when it runs. One whose
  inputs aren't there, such as OCR on a page without blocks, skips that page
  and says so.

### OCR

- **Crops.** Each block is cropped at its box, turned upright by its
  `transform`, and read in the block's `direction`. A crop's shape never
  decides it: PaddleX and MinerU turn any crop at least 1.5 times as tall as
  it is wide [PaddleX crops] [MinerU crops], which would turn vertical
  Japanese on its side.
- **Batches.** The blocks of every page ready for OCR go to the model
  together, and the results go back by page and block. MinerU, docTR, and
  Surya likewise send every page's crops to one call [MinerU OCR] [docTR
  predictor] [Surya recognition]; PaddleX's page-by-page loop carries "TODO:
  Process all sub-images in the batch together" [PaddleX OCR].

### Translation

`koharu-translator` merges into this step, without its machine-translation
services: DeepL, Google Cloud Translation, and Caiyun go. The model is any
LLM `koharu-llm` serves: a local GGUF, or a hosted provider the user chose.

- **A page per request.** The blocks in reading order, grouped by the balloon
  each sits in, with the scan when vision is on. Professional translators
  preferred paragraph-at-once output 71.1% of the time over sentence by
  sentence, which made 12 times as many consistency errors [Karpinska and
  Iyyer]; whole-document prompts beat sentence prompts on most metrics [Wang
  et al.].
- **Thoughts.** Each balloon's translation is a list of texts, each naming the
  first and last of the balloon's blocks it covers, so blocks that read as one
  thought become one text, as "balloons directly joined together are
  generally of the same thought process" [Blambot grammar]. Subtitle
  translation likewise merges a sentence split across cues, translates it,
  and maps it back [Matusov et al.]. Block ids are numbers, and come before
  the text in each answer, so the model commits to the mapping first; OpenAI
  writes keys in schema order [OpenAI structured outputs].
- **Checking the answer.** A JSON schema constrains the shape but can't say
  "every block once": Anthropic's arrays take a `minItems` of only 0 or 1
  [Claude structured outputs]. So the answer is checked after decoding: each
  balloon's ranges cover its blocks in order without gaps or overlaps, each
  text is in the target language, and a refusal or an answer cut off at the
  token limit is no answer. A page that fails the check fails, as joint
  translation is known to drop content [Hu et al.].
- **Context.** A page carries no context from other pages, so pages translate
  independently and decode together. Source-primed multi-turn translation
  would add context but make each page wait for the one before [Hu et al.].
- **Vision.** The scan is sent only when vision is on; on Gemma 4, the image
  is the part of a request that can't share a decode with other pages.
- **Settings.** The model, target language, instructions, and generation
  settings are `[pipeline.translation]`.

### Typesetting

Typesetting follows the scene's text model without a model of its own.

- **Dialogue style.** One "Dialogue" text style for the project, at the
  median size of the dialogue blocks, centred, with line height 1.05, as
  letterers set dialogue "in accordance with the original Japanese book"
  [Manga balloon basics]. A rerun updates it, and the user's overrides stay.
- **Frames.** Each balloon is cut into regions by a watershed seeded by its
  blocks on its distance from its edge [Watershed] [Watershed example]. A
  translation is framed by the union of the regions of the blocks it covers,
  centred, shrinking only if it would overflow. A text outside any balloon is
  framed by its block's box.
- **Direction.** Horizontal, unless the target language is Chinese, Japanese,
  or Korean and the block was vertical.

### Inpainting

- **Mask.** Inpainting reads the text mask from the project, so a mask the
  user painted is cleaned the same way. The mask is grown by a few pixels
  first, as AOT-GAN "tends to propagate the boundaries" left outside it
  [AOT-GAN].
- **Fill.** A balloon whose inside is one colour is filled with it without the
  model.
- **LaMa and AOT-GAN** run at the page's own resolution, around each group of
  masked areas: a core of at most 512 pixels with 128 pixels of context,
  IOPaint's default margin [IOPaint crop], mirror-padded to the model's
  multiple [IOPaint padding], and written back only on the group's masked
  pixels. LaMa works "in a single pass" up to 1536 pixels [LaMa], and on thin
  masks refinement changes nothing "because there is sufficient surrounding
  context" [LaMa refinement]. A masked area larger than the core is inpainted
  in one crop, downscaled to fit, and written back on its masked pixels.
- **Diffusion inpainters** take the same groups with 32 pixels of context,
  scaled to the model's native size at its multiple (16 for FLUX.2 at up to
  2 MP [BFL resolution], 64 for SDXL [SDXL]), then scaled back and blended
  with a 4-pixel feather onto the masked pixels only, as A1111's "Only
  masked" [A1111 inpaint] and ComfyUI's Crop and Stitch [Crop and Stitch] do.
  Every crop gets the model's one prompt, which `koharu-diffusion` encodes
  once and reuses.
- **Pixels.** It writes only the tiles it changed.

## Running

The baseline is today's pipeline on `main`, in a release build, over eight
Manga109 spreads of 1654×1170 with 16–34 text blocks each, on an RTX 5090 with
the default models: RF-DETR, PaddleOCR-VL 1.6, Gemma 4 12B with vision, and
LaMa.

| Step | Load | Running | Per spread (median) | Share |
|---|---|---|---|---|
| detection | 0.23 s | 1.23 s | 0.09 s | 2% |
| OCR | 0.97 s | 9.93 s | 1.08 s | 18% |
| translation | 3.35 s | 33.79 s | 3.81 s | 63% |
| inpainting | 0.16 s | 4.24 s | 0.58 s | 8% |

The run took 53.95 s, of which 0.05 s fell between jobs: one job at a time,
nothing overlapping. The GPU averaged 64% busy. Translation and OCR are both
language models decoding one request at a time, and are 81% of the run.

- **One device lane.** Model calls run one at a time on the device, as now:
  the current design measured two different models running at once taking
  2.5–4× as long as running them one after the other. Clockwork found that
  running inferences concurrently "gains up to 25% throughput" while making
  the slowest requests about 100 times slower [Clockwork], and NVIDIA's MPS,
  which shares a GPU between processes, runs only on Linux and QNX [NVIDIA
  MPS].
- **Order.** The lane takes next a job for a model that's already loaded, then
  the earliest step, then the earliest page, so a run goes step by step across
  its pages. A job passed over for three batches of another model runs next,
  as a shared lane serving models of very different speeds otherwise queues
  the fast ones behind the slow [AlpaServe].
- **Windows of pages.** A run takes its pages in windows of 16, so a chapter
  of hundreds of pages holds a bounded number of decoded pages between steps.
  MinerU likewise runs every step over a 64-page window and frees memory
  between windows [MinerU window].
- **Batches.** The lane hands a model every job ready for it in the window:
  every page's OCR blocks at once, and several pages' translations in one
  decode. Clockwork likewise serves several models on one GPU by batching
  each model's requests [Clockwork].
- **CPU and network work off the lane.** Decoding, masks and contours, the
  watershed, compositing, and building edits run on a CPU pool, and a hosted
  translation is a network request, at most three in flight, as Marker's
  default [Marker LLM]; neither holds the device. PaddleOCR-VL runs its
  rendering, layout, and recognition as threads joined by bounded queues
  [PaddleOCR-VL queues], and Surya post-processes one batch while the GPU runs
  the next [Surya detection].
- **Edits don't stall the lane.** One task applies finished edits through the
  host, in order, while the lane runs the next batch.

### Language models

Translation and PaddleOCR-VL OCR both run in llama.cpp, and the work this
needs is in local LLM inference, wherever it lives.

- **One context per loaded model.** Every new llama.cpp context allocates its
  whole KV cache and reserves its worst-case graphs [llama.cpp context], which
  today's context per call repeats every time.
- **Pages decode together.** One context holds a sequence per page and decodes
  them all in each step, with a sampler per sequence, as llama.cpp's
  `parallel` example does [llama.cpp parallel]. Pages share no prefix but the
  instructions, so each sequence gets its own KV cache (`kv_unified` off, as
  the header advises for several sequences [llama.h kv_unified]). The number
  of sequences comes from the memory budget, as the KV cache for all of them
  is allocated with the context.
- **Images go one at a time.** Gemma 4 decodes an image with non-causal
  attention, which llama.cpp sets for the whole context [mtmd decode], so each
  page's image is read on its own and its text is then generated with the
  others. PaddleOCR-VL's projector can't batch images of different sizes
  [mtmd batch], so each crop's image is encoded alone and their text decoded
  together.
- **Shared instructions first.** The fixed instructions come before the image,
  decoded once and copied to each sequence [llama.cpp parallel].
- **Expected.** On an RTX 3090, a small model's decoding throughput grew 1.5×
  at 2 sequences, 2.2× at 4, and 2.9× at 8 [llama.cpp batched bench]. No
  published sweep covers a 12B model on a 5090, so translation's gain is an
  estimate of 1.5–2.5×, to be measured with `llama-batched-bench` at the real
  prompt lengths. The first measurement splits each language-model step into
  context setup, image encoding, prefill, and generation.

### Residency

- **What stays.** A loaded model stays loaded between runs until it has been
  idle for five minutes, Ollama's default [Ollama FAQ], or the lane unloads it
  to make room.
- **Room.** A model loads only when the device's free memory, less a reserve
  of 1 GiB, holds its footprint. ComfyUI keeps 0.8 GiB for inference and a
  further 600 MiB on Windows "because of the shared vram issue" [ComfyUI
  memory]; Ollama counts only 80% of free memory as usable [Ollama
  scheduler].
- **libtorch keeps what it frees.** libtorch's allocator keeps the memory of
  the tensors and models it drops for its own next allocations, and gives it
  back to the device only when one of its own allocations would otherwise
  fail, on CUDA and HIP [libtorch CUDA memory] [libtorch HIP memory] [libtorch
  CUDA retry] as on Metal [libtorch Metal retry]. The shim release Koharu
  binds has no `empty_cache` (`koharu-torch-sys`'s headers), so libtorch's share of
  the device grows to the most its models have needed, and llama.cpp and
  stable-diffusion.cpp don't get it back while Koharu runs.
- **Footprint.** A model's footprint is how much free memory drops from an
  idle, synchronised device to the end of its first inference at the largest
  input, including the KV cache for its sequences; llama.cpp keeps its peak
  scratch memory until its context goes [ggml pool]. A libtorch model's
  footprint is how much libtorch's share grew, as the model reuses what
  libtorch already holds. Free memory is read live from the runtime's device,
  as `koharu-runtime`'s README describes. stable-diffusion.cpp is given that
  budget as `max_vram`, as it otherwise sizes itself to all free memory
  [sd.cpp max_vram].
- **Warm-up.** A model runs that first inference, and a llama.cpp model one
  image as well, right after it loads, so its memory grows then rather than
  partway through a page.
- **Eviction.** The lane unloads the idle llama.cpp or stable-diffusion.cpp
  model whose next queued job is furthest away, then the least recently used.
  Belady showed that evicting what is needed furthest ahead is the best choice
  when the future is known [Belady], and a run's queue is. An idle libtorch
  model isn't unloaded to make room: its memory would stay in libtorch's
  share, and the release reports nothing about that share to tell how much of
  it the next libtorch model could reuse.
- **Out of memory.** Prevented, and never retried. On Windows a CUDA
  allocation past the device's memory can fall back to system memory and run
  slowly, which NVIDIA calls "expected functionality" [WDDM fallback], and
  ggml's CUDA pool aborts the process when it runs out [ggml pool], which
  covers llama.cpp and stable-diffusion.cpp; stable-diffusion.cpp reports
  other failures only in its log, and libtorch's errors reach Koharu only as
  messages (`koharu-torch-sys`'s headers). So nothing tells the lane that a failure was
  memory, and a retry would repeat any other failure: stable-diffusion.cpp
  retries with smaller tiles only on its typed allocation failure, as
  "Execution failures can leave the device unusable" [sd.cpp retry]. The lane
  reads no error's message for its cause.
- **Failures.** A failed step fails the steps that need it on that page; other
  pages go on, and the report lists every failure.

### Model work this needs

| Change | Where | Why |
|---|---|---|
| One context per loaded model, a sequence per page with its own sampler, images read one at a time, shared instructions copied | `koharu-llm`'s engine on `koharu-llama` | translation and PaddleOCR-VL OCR are 81% of the baseline |
| Live free memory for the device | `koharu-runtime` | the reserve and footprints need it; today it's always 0 |
| `max_vram` from the lane's budget | `koharu-diffusion` | it otherwise takes all free memory |

Detection's mask copies and the inpainters' per-pixel loops measured 2% and 8%
of the run, so they wait until a measurement says otherwise. No reference
batches LaMa crops, and padding crops to one size changes LaMa's output, as
its Fourier convolutions span the whole input [LaMa].

### Measuring

```text
cargo run --release -p koharu-pipeline --bin run -- --input <page>...
```

`run` puts the pages in one project, runs every step, and prints each step's
load and running time per page. A change to how the pipeline runs reports the
device, the pages, the baseline above, the new times, and whether the output
differs: the same blocks and masks, the same OCR text, and translations that
pass the check.

## Configuration

`[pipeline]` names the detection, OCR, and inpainting models and their
settings, and `[pipeline.translation]` the translation model, target
language, instructions, and generation settings. A run reads the section
from `koharu-config` when it starts, so nobody watches for changes, and models
stay loaded while their settings stay the same. Provider connections belong to
`koharu-llm`.

## Sources

- Concurrency and editors: [FoundationDB], [RFC 9110 conditional], [RFC 9110
  conflict], [LSP applyEdit], [LSP content modified], [LSP progress], [LSP
  partial results], [languageclient check], [Kubernetes retries], [VS Code
  edit], [Figma commitUndo], [Photoshop modal], [Lightroom masking]
- Serving and memory: [Clockwork], [AlpaServe], [NVIDIA MPS], [Belady],
  [Ollama FAQ], [Ollama scheduler], [ComfyUI memory], [libtorch CUDA memory],
  [libtorch HIP memory], [libtorch CUDA retry], [libtorch Metal retry], [WDDM
  fallback]
- llama.cpp and stable-diffusion.cpp: [llama.cpp parallel], [llama.h
  kv_unified], [llama.cpp context], [llama.cpp batched bench], [mtmd decode],
  [mtmd batch], [ggml pool], [sd.cpp max_vram], [sd.cpp retry]
- Document pipelines: [MinerU window], [MinerU OCR], [MinerU crops], [docTR
  predictor], [Surya recognition], [Surya detection], [PaddleX OCR], [PaddleX
  crops], [PaddleOCR-VL queues], [Marker LLM]
- Translation: [Karpinska and Iyyer], [Wang et al.], [Hu et al.], [Matusov et
  al.], [OpenAI structured outputs], [Claude structured outputs]
- Inpainting: [LaMa], [LaMa refinement], [AOT-GAN], [IOPaint crop], [IOPaint
  padding], [A1111 inpaint], [Crop and Stitch], [BFL resolution], [SDXL]
- Lettering and geometry: [Blambot grammar], [Manga balloon basics],
  [Watershed], [Watershed example]

[FoundationDB]: https://apple.github.io/foundationdb/developer-guide.html
[RFC 9110 conditional]: https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.1
[RFC 9110 conflict]: https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.10
[LSP applyEdit]: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#workspace_applyEdit
[LSP content modified]: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#implementationConsiderations
[LSP progress]: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#workDoneProgressBegin
[LSP partial results]: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#partialResults
[languageclient check]: https://github.com/microsoft/vscode-languageserver-node/blob/cc5c69a6a9b336ba7adcf302c906a9829b4fd3f7/client/src/common/client.ts#L2271-L2307
[Kubernetes retries]: https://kubernetes.io/docs/reference/using-api/api-concepts/#update-mechanism-update
[VS Code edit]: https://code.visualstudio.com/api/references/vscode-api#TextEditor
[Figma commitUndo]: https://developers.figma.com/docs/plugins/api/properties/figma-commitundo/
[Photoshop modal]: https://developer.adobe.com/photoshop/uxp/2022/ps_reference/media/executeasmodal/
[Lightroom masking]: https://helpx.adobe.com/lightroom-classic/desktop/process-and-develop-photos/masking.html
[Clockwork]: https://arxiv.org/abs/2006.02464
[AlpaServe]: https://arxiv.org/abs/2302.11665
[NVIDIA MPS]: https://docs.nvidia.com/deploy/mps/when-to-use-mps.html
[Belady]: https://doi.org/10.1147/sj.52.0078
[Ollama FAQ]: https://github.com/ollama/ollama/blob/8ec9ecdf692e417798169ff8d5972159078d3754/docs/faq.mdx#L289-L336
[Ollama scheduler]: https://github.com/ollama/ollama/blob/8ec9ecdf692e417798169ff8d5972159078d3754/server/sched.go#L589-L615
[ComfyUI memory]: https://github.com/Comfy-Org/ComfyUI/blob/426f5eea52a7ece1dd599fff4cb6cd1f352ddd24/comfy/model_management.py#L877-L1038
[libtorch CUDA memory]: https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/docs/source/notes/cuda.md?plain=1#L506-L516
[libtorch HIP memory]: https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/docs/source/notes/hip.rst?plain=1#L92-L102
[libtorch CUDA retry]: https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/c10/cuda/CUDACachingAllocator.cpp#L1776-L1792
[libtorch Metal retry]: https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/aten/src/ATen/mps/MPSAllocator.mm#L245-L263
[WDDM fallback]: https://forums.developer.nvidia.com/t/cudamalloc-with-sysmem-fallback/347791
[llama.cpp parallel]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476
[llama.h kv_unified]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/include/llama.h#L412-L417
[llama.cpp context]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/src/llama-context.cpp#L425-L885
[llama.cpp batched bench]: https://github.com/ggml-org/llama.cpp/discussions/18030
[mtmd decode]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/tools/mtmd/mtmd.cpp#L2426-L2447
[mtmd batch]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/tools/mtmd/mtmd.h#L338-L354
[ggml pool]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/ggml/src/ggml-cuda/ggml-cuda.cu#L532-L640
[sd.cpp max_vram]: https://github.com/leejet/stable-diffusion.cpp/blob/be486498eadc29dc3cd234d944c2180ed42b1415/include/stable-diffusion.h#L233-L246
[sd.cpp retry]: https://github.com/leejet/stable-diffusion.cpp/blob/be486498eadc29dc3cd234d944c2180ed42b1415/src/core/backend_fit.cpp#L481-L490
[MinerU window]: https://github.com/opendatalab/MinerU/blob/b7f3dd0c5f6e6d66a298239ffd02b328b873918e/mineru/backend/analysis/pdf/window.py#L186-L313
[MinerU OCR]: https://github.com/opendatalab/MinerU/blob/b7f3dd0c5f6e6d66a298239ffd02b328b873918e/mineru/backend/analysis/pdf/ocr.py#L175-L267
[MinerU crops]: https://github.com/opendatalab/MinerU/blob/b7f3dd0c5f6e6d66a298239ffd02b328b873918e/mineru/model/ocr/image.py#L15-L148
[docTR predictor]: https://github.com/mindee/doctr/blob/323de1f783807066ba5a16cd67e7b9c1f5d6d91c/doctr/models/predictor/pytorch.py#L91-L230
[Surya recognition]: https://github.com/datalab-to/surya/blob/0d4f77793071efd1543916afaf406383da81c7c7/surya/recognition/__init__.py#L179-L265
[Surya detection]: https://github.com/datalab-to/surya/blob/0d4f77793071efd1543916afaf406383da81c7c7/surya/detection/__init__.py#L60-L182
[PaddleX OCR]: https://github.com/PaddlePaddle/PaddleX/blob/ffb64904d23708863ff5b8da312a5cbd52a7f462/paddlex/inference/pipelines/ocr/pipeline.py#L451-L480
[PaddleX crops]: https://github.com/PaddlePaddle/PaddleX/blob/ffb64904d23708863ff5b8da312a5cbd52a7f462/paddlex/inference/pipelines/components/common/crop_image_regions.py#L165-L207
[PaddleOCR-VL queues]: https://github.com/PaddlePaddle/PaddleX/blob/ffb64904d23708863ff5b8da312a5cbd52a7f462/paddlex/inference/pipelines/paddleocr_vl/pipeline.py#L1074-L1125
[Marker LLM]: https://github.com/datalab-to/marker/blob/6d40b01930dd45f4fc3b014f2ede8fb0d8cb6b81/marker/processors/llm/__init__.py#L42-L170
[Karpinska and Iyyer]: https://arxiv.org/abs/2304.03245
[Wang et al.]: https://arxiv.org/abs/2304.02210
[Hu et al.]: https://arxiv.org/abs/2503.10494
[Matusov et al.]: https://aclanthology.org/W19-5209/
[OpenAI structured outputs]: https://developers.openai.com/api/docs/guides/structured-outputs
[Claude structured outputs]: https://platform.claude.com/docs/en/build-with-claude/structured-outputs
[LaMa]: https://arxiv.org/abs/2109.07161
[LaMa refinement]: https://arxiv.org/abs/2206.13644
[AOT-GAN]: https://arxiv.org/abs/2104.01431
[IOPaint crop]: https://github.com/Sanster/IOPaint/blob/61a759fb3f332bacdce8b2813f4837495c9b86e0/iopaint/schema.py#L293-L306
[IOPaint padding]: https://github.com/Sanster/IOPaint/blob/61a759fb3f332bacdce8b2813f4837495c9b86e0/iopaint/helper.py#L209-L244
[A1111 inpaint]: https://github.com/AUTOMATIC1111/stable-diffusion-webui/blob/82a973c04367123ae98bd9abdf80d9eda9b910e2/modules/processing.py#L1606-L1660
[Crop and Stitch]: https://github.com/lquesada/ComfyUI-Inpaint-CropAndStitch/blob/6005efc02f7745649118edd89cdd084c2b9bd3ae/README.md?plain=1#L43-L113
[BFL resolution]: https://help.bfl.ai/articles/8531149640-what-are-the-resolution-limits
[SDXL]: https://arxiv.org/abs/2307.01952
[Blambot grammar]: https://blambot.com/pages/comic-book-grammar-tradition
[Manga balloon basics]: https://www.linkedin.com/pulse/manga-lettering-balloon-basics-phil-christie
[Watershed]: https://doi.org/10.1016/1047-3203(90)90014-M
[Watershed example]: https://scikit-image.org/docs/stable/auto_examples/segmentation/plot_watershed.html
