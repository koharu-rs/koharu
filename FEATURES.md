# Koharu Fork Features

User-facing documentation of the features added in this fork on top of upstream Koharu `0.99.0`.

For planned features still on our roadmap (such as Phase 2 stylized SFX repainting, SFX translation awareness, smart prompting, and the stage-specific Run dropdown), see [our-features.md](our-features.md). For the full comparative survey of other community forks, see [fork-features.md](fork-features.md).

## Contents

- [Import and export](#import-and-export)
  - [Drag-and-drop file import](#drag-and-drop-file-import)
  - [CBZ image formats and automatic grayscale encoding](#cbz-image-formats-and-automatic-grayscale-encoding)
  - [ComicInfo.xml 2.1 and comic metadata](#comicinfoxml-21-and-comic-metadata)
- [Models, downloads, and VRAM](#models-downloads-and-vram)
  - [Model sizes, quantization menus, and VRAM fit indicators](#model-sizes-quantization-menus-and-vram-fit-indicators)
  - [VRAM-aware job manager](#vram-aware-job-manager)
  - [Download manager](#download-manager)
- [Translation](#translation)
  - [Cross-page translation context](#cross-page-translation-context)
- [Job orchestration and Activity Center](#job-orchestration-and-activity-center)
  - [Persistent Activity tab and status bar](#persistent-activity-tab-and-status-bar)
  - [Request queue, stage-batched execution, and priority modes](#request-queue-stage-batched-execution-and-priority-modes)
- [Headless batch CLI (`koharu-cli`)](#headless-batch-cli-koharu-cli)
- [Features from other forks (TODO)](#features-from-other-forks-todo)

---

## Import and export

### Drag-and-drop file import

Drag files or folders directly from your file manager onto an open project window to add pages without going through the file dialog.

- **Supported inputs**: raster images, folders (scanned recursively), comic archives (`.cbz`, `.zip`, `.rar`, `.cbr`, `.7z`, `.cb7`), and `.pdf` files — the same formats accepted by **File → Import**.
- **Positional insertion on the page rail**: hovering over the left page rail shows an insertion line indicating where the dropped pages will be inserted.
- **Append on the canvas**: dropping onto the main canvas appends pages to the end of the project.
- **Mixed drops and active runs**: unsupported files in a mixed drop are skipped cleanly and summarized in a toast notification rather than failing the whole import. Dropping files works even while a pipeline run is already active (the active run continues on the pages it started with).

### CBZ image formats and automatic grayscale encoding

Exporting a project or selected pages as CBZ (**File → Export Project → CBZ** or **File → Export Selected Pages → CBZ**) opens a **CBZ export options** dialog where you can choose how page images are encoded inside the archive:

- **PNG** (default) — lossless, maximum reader compatibility.
- **WebP** — supports lossy quality (`1–100`, default `90`) or **Lossless** encoding.
- **JPEG XL (`.jxl`)** — supports lossy quality (`1–100`, default `90`) or **Lossless** encoding.

Your last chosen codec, quality, and lossless setting are saved automatically under `[export]` in Koharu's preferences.

**Automatic 8-bit grayscale (`L8`) encoding**: when exporting PNG or CBZ pages, Koharu checks each rendered page in a single pass. Pages that are opaque and purely grayscale (typical black-and-white manga) are encoded as 8-bit grayscale instead of 32-bit RGBA, dramatically shrinking PNG and JPEG XL file sizes with no visual loss.

### ComicInfo.xml 2.1 and comic metadata

Koharu stores, edits, translates, and exports full **ComicInfo 2.1** metadata so exported CBZ archives index cleanly in comic servers and readers such as Kavita, Komga, and ComicRack.

- **Automatic import adoption**: when you import a comic archive containing `ComicInfo.xml`, Koharu reads and adopts all non-empty fields automatically. When importing multiple archives into one project, the first archive's metadata is kept and later archives do not overwrite it.
- **Metadata editor**: open it from **File → Metadata…** or the **Metadata** button in the title bar. Common fields (**Series**, **Number**, **Volume**, **Title**, **Summary**, **Writer**, **LanguageISO**, **Manga**) appear upfront, and all remaining ComicInfo 2.1 fields are available under **More fields**.
- **Side-by-side original and translated prose**: translatable prose fields (such as **Title**, **Series**, **Summary**, **Notes**, **Genre**, **Tags**, **Characters**, **StoryArc**) store both the original text and its translation side by side. Editing the original text marks its translation stale so you always know what still needs updating.
- **Metadata translation**: translate metadata directly from the Metadata dialog, or let it run automatically at the end of a full-project **Translate** run. Structural fields (such as **Number**, **Volume**, **Year**, and identifiers) are never sent for prose translation.
- **Automatic regeneration on CBZ export**: every exported CBZ includes `ComicInfo.xml` using your translated prose fields where present (falling back to original values otherwise). Derived fields are always recomputed accurately for the exported archive: `PageCount`, `<Pages>` entries (page index, filename key, pixel dimensions, byte size, and `DoublePage="true"` for wide spreads), and `LanguageISO` (set from your target translation language).
- **Project format compatibility**: metadata is persisted inside `.khr` projects (schema `1.2.0`). Projects saved by earlier `1.1.0` releases open read-only without modifying the file on disk and upgrade atomically on the first edit.

---

## Models, downloads, and VRAM

### Model sizes, quantization menus, and VRAM fit indicators

Picking models in **Settings → Pipeline** and the translation model selector now shows disk cost and measured VRAM fit before you commit:

- **Declared disk sizes**: every ML model and local LLM shows its total pinned download size across all required files before downloading.
- **Grouped local LLM quantizations**: local translation model families with multiple quantizations appear as one family entry with an expandable quantization submenu listing each variant's disk size, download status, and VRAM fit.
- **Measured VRAM fit indicator**: once a model has been run on your GPU, Koharu displays a VRAM fit badge (icon + short label, never color alone) with a tooltip showing the measured footprint against currently available VRAM alongside the other selected pipeline stages. Models that have never been measured yet display only their disk size. Available VRAM is sampled when opening the picker so indicators do not flicker while browsing.

### VRAM-aware job manager

Instead of keeping every loaded model resident until an out-of-memory error happens, Koharu learns each model's real VRAM usage on your machine and manages model residency proactively:

- **Shared measurement store**: on discrete CUDA and ROCm GPUs with cross-process VRAM accounting, Koharu measures each model's load footprint, peak working memory, and per-megapixel inpainting working cost and saves them in `~/.koharu/vram.json` keyed by `(device, model, revision)`.
- **Planned eviction**: before loading or running a measured model, Koharu clears libtorch's cache and evicts resident models whenever free VRAM would drop below the needed footprint plus a `512 MiB` safety margin. Eviction prefers models not needed by any active run (least-recently used first), then active-run models furthest from the current stage.
- **Clean local LLM teardown**: when evicting a local llama.cpp model, Koharu waits synchronously for its engine thread to free VRAM before sampling free memory and loading the next model.
- **Graceful fallback**: on backends without cross-process VRAM accounting (such as Apple unified memory or integrated GPUs) or on unmeasured first runs, Koharu falls back cleanly to reactive OOM eviction and retry.

### Download manager

Every model download now appears as its own tracked entry inside the **Activity** tab under **Downloads**:

- **Per-model visibility**: downloads are grouped per model with the model name, pipeline stage badge, progress bar, completed/total bytes, file counter (for multi-file models), and live smoothed download speed.
- **Controls**: **Cancel** active downloads, **Retry** failed or cancelled downloads, and **Dismiss** finished or failed entries. Completed downloads remain visible during the current session until dismissed.
- **Pre-admission downloads**: if a submitted pipeline run needs models that are not yet on disk, those models download off the GPU thread while the run waits in the queue as **Waiting for download**. Cancelling a sole-owner download cancels the waiting run (and cancelling the waiting run cancels any downloads only it needed).

---

## Translation

### Cross-page translation context

*(Multi-page request batching inspired by [NeKoharu](https://github.com/nekojin116/NeKoharu); adapted natively for Koharu `0.99.0`'s multi-turn conversation, dynamic model limits, and Vision workflow.)*

Under **Settings → Pipeline → Translate**:

- **Combine pages** (enabled by default): gives the translator both look-back and look-ahead dialogue context across pages for better character-name, pronoun, terminology, and plot consistency:
  - **Text-only mode (Vision off)**: packs consecutive pages into multi-page JSON batches automatically sized to the active model's context and output token limits (using CJK-aware token estimation), while keeping Koharu's multi-turn conversation history across batches. General LLMs receive explicit page-boundary continuity guidance; specialized translation models (`Index-Translate`) keep their bare numbered JSON prompt under a tighter auto budget. If a combined batch is refused, cut off, or overflows, Koharu automatically falls back to translating that batch page by page so a single refused page never fails neighboring pages.
  - **Vision mode (Vision on)**: keeps each request focused on **only the current page's scan** while attaching preceding dialogue (when starting mid-chapter) and upcoming pages' OCR text as clearly marked read-only `<previous-pages>` and `<upcoming-pages>` context on general multimodal models (stripped before recording into conversation history so history never duplicates text). Specialized `Index-Translate` models keep bare JSON + conversation history only.
- **Max pages per request**: leave blank for **Auto** (dynamically sized from the model's context/output limits), or enter an explicit page cap (`1–64`) for smaller chunks.

---

## Job orchestration and Activity Center

### Persistent Activity tab and status bar

The transient bottom-right notification popup has been replaced by a persistent **Activity** tab in the right sidebar:

- **Always reachable**: open it from the **Activity** tab in the right sidebar or by clicking the live activity status pill in the bottom status bar.
- **One surface for all work**: displays **Running** work (pipeline runs, active imports/exports, chapter loads, and dismissible failures), **Queued** work, and **Downloads** in a single scrollable view.

### Request queue, stage-batched execution, and priority modes

You no longer have to wait for a run to finish before submitting more work, and multi-page runs execute cleanly stage by stage:

- **Stage-batched execution**: within a multi-page run, Koharu finishes all pages for one stage before moving to the next (**Detection** across all pages → **OCR** across all pages in multi-page batches → **Inpainting** across all pages → **Translation** across all pages). This loads each stage's model once per run, avoids per-page VRAM swaps, and ensures every page's OCR text is ready before translation begins.
- **Per-page queued job list and individual cancellation**: the **Queued** section in the Activity tab lists every upcoming page-stage step in execution order with its stage icon and name (for example, `Page 2 Inpainting`, `Page 3 Translation`, `Metadata Translation`). You can cancel any individual queued step with its `×` button without stopping the rest of the run, or stop an entire active/queued run at once.
- **Queue and Rerun now**: while work is active, the processing button switches to **Queue** with a dropdown offering:
  - **Queue** — adds the run to the normal batch queue.
  - **Rerun now** — jumps ahead of queued batch work and runs at the next stage/page boundary (also used automatically for interactive work such as Generative Fill, block reruns, and metadata translation).
- **Automatic coalescing**: overlapping normal batch runs submitted on the same project and configuration merge per `(page, stage)` instead of duplicating work.
- **VRAM-aware queue ordering**: queued batch runs are scheduled to prefer already-resident models and lower VRAM swap cost, with FIFO aging (`256 MiB/s`) so large-VRAM runs never starve.
- **Unified cancellation**: stopping an active run or cancelling queued/download-waiting runs or individual page-stage jobs preserves pages that already finished.

---

## Headless batch CLI (`koharu-cli`)

`koharu-cli` runs the complete Koharu pipeline and export workflow unattended from the command line, outside the GUI, sharing the desktop app's settings, model store, and `~/.koharu/vram.json`.

### Basic usage

```bash
# Build the CLI
cargo build --release -p koharu-cli

# Process a folder of manga pages and export as a JPEG XL CBZ archive
koharu-cli ./chapter-01 \
  --project ./chapter-01.khr \
  --export-format cbz \
  --export-out ./chapter-01.cbz \
  --cbz-codec jxl --quality 85

# Re-run only translation on an existing project with JSONL progress
koharu-cli ./chapter-01.khr \
  --stages translate \
  --target en \
  --json
```

### Common options

| Flag | Description |
|---|---|
| `INPUT...` | Existing `.khr` project, or one or more images, folders, archives (`.cbz`/`.zip`/`.rar`/`.7z`), or `.pdf` files |
| `--project PATH` | `.khr` project path to open or create when importing raw inputs (uses a temporary project if omitted) |
| `--stages LIST` | Comma-separated stages: `detect,recognize,inpaint,translate` (default: all four) |
| `--detector ID` | Override detection model ID |
| `--recognizer ID` | Override OCR recognizer model ID |
| `--inpainter ID` | Override inpainting model ID |
| `--translator MODEL` | Override translation model reference (`vendor/model`) |
| `--target LANG` | Override target language (CLDR BCP-47 tag, e.g. `en`, `zh-Hans`, `fr`) |
| `--vision` | Enable vision input (send page scans alongside text) for translation |
| `--reasoning LEVEL` | Override reasoning level (`off`, `minimal`, `low`, `medium`, `high`) |
| `--export-format FMT` | Export format: `png`, `psd`, `cbz`, or `pdf` |
| `--export-out PATH` | Destination file (`cbz`, `pdf`) or folder (`png`, `psd`) |
| `--cbz-codec CODEC` | CBZ image codec: `png`, `webp`, or `jxl` |
| `--quality 1-100` | Lossy quality for WebP and JPEG XL |
| `--lossless` | Force lossless encoding for WebP and JPEG XL |
| `--json` | Emit JSON-lines progress events and a final summary object on stdout (logs go to stderr) |

### Exit codes and safety guarantees

- **Exit codes**: `0` = all pages succeeded, `1` = fatal error or invalid arguments, `2` = partial completion (some pages failed while others succeeded), `130` = interrupted by `SIGINT`/`SIGTERM` (gracefully cancels active work and waits for clean shutdown).
- **Automatic downloads**: missing pinned models download automatically during headless runs.
- **Process coexistence**: an advisory project lock refuses to open a `.khr` project currently open in the GUI, and a cross-process accelerator lock prevents two Koharu processes from fighting over VRAM.
- **Credentials**: hosted API keys are read strictly from the OS credential store and fail closed if the keyring is unavailable or locked (no `.env` file fallback).

---

## Features from other forks (TODO)

> **TODO**: The items below come from six reference community forks of Koharu — [NeKoharu](https://github.com/nekojin116/NeKoharu), [spgsroot/koharu](https://github.com/spgsroot/koharu), [koharu-context](https://github.com/Wkingxc/koharu-context), [gudoy/koharu](https://github.com/gudoy/koharu), [koharu-ex](https://github.com/Gray-Rice/koharu-ex), and [koharu-edge](https://github.com/MOULINIER/koharu-edge) — and are **not yet implemented** in this fork. They are tracked here as candidates for evaluation and selective adoption. See [fork-features.md](fork-features.md) for full details and deferred-design notes.

### Translation and context (TODO)

- [x] **Cross-page / multi-page translation context** *(inspired by NeKoharu; shipped under [Cross-page translation context](#cross-page-translation-context) — koharu-context's dedicated chapter workspace with manual summary proofreading remains TODO)*
- [ ] **Translation validation** *(spgsroot/koharu, koharu-context)* — reject responses that still contain blocked scripts or mismatched/empty block IDs and retry with feedback.
- [ ] **Content profiles and processing presets** *(spgsroot/koharu, koharu-context)* — saveable presets covering model, language, reading order, system prompt, and rendering style.
- [ ] **Token and cost tracking** *(spgsroot/koharu)* — filterable journal of model requests with reported token counts and cost.
- [ ] **Additional hosted providers** *(spgsroot/koharu)* — extra translation/agent sign-in providers.

### Editor and canvas conveniences (TODO)

- [ ] **Arrow-key page navigation** *(NeKoharu, koharu-context)* — navigate pages from the keyboard while ignoring inputs.
- [ ] **Multi-layer and rectangle selection** *(NeKoharu, koharu-ex)* — drag-box multi-selection for layers, eraser, and remove tools.
- [ ] **Copy and paste layers or text styles between pages** *(NeKoharu, spgsroot/koharu, koharu-edge)*.
- [ ] **Project color palette** *(koharu-edge)* — save reusable manga-specific swatches in the project file.
- [ ] **Page flags and page reset** *(NeKoharu, koharu-edge)* — mark chapter starts or done pages in the rail, and undoably reset a page to its original scan.
- [ ] **Live font preview on selection** *(NeKoharu)* — preview fonts live on the canvas while browsing candidates.
- [ ] **Per-frame OCR and cleaner overlapping detection** *(koharu-edge)* — one-click OCR button per text layer and deduplication of nested paragraph/box detections.
- [ ] **Merged spreads** *(NeKoharu)* — merge adjacent split-panel pages into one wide spread (deferred until export and stage-eligibility rules are decided; see [fork-features.md](fork-features.md#merged-spreads-nekoharu)).
- [ ] **Live resource monitor** *(spgsroot/koharu)* — CPU, RAM, GPU, and VRAM telemetry in the status bar.

### Cleanup and inpainting (TODO)

- [ ] **Split lettering routes** *(spgsroot/koharu)* — separate masks and models for bubble text vs. lettering/SFX over artwork (planned to pair with Phase 2 SFX repainting).
- [ ] **Additional artwork models and repair brushes** *(spgsroot/koharu)* — extra diffusion/ComfyUI backends and preset repair brushes.

### Batch, export, and performance (TODO)

- [ ] **Incremental export** *(gudoy/koharu)* — render each page once across multiple requested export formats.
- [ ] **Translation exchange** *(koharu-ex, spgsroot/koharu)* — JSON / `.xlsx` / `.csv` export and re-import of translations for collaborative editing.
- [ ] **Folder batch-project translation UI** *(gudoy/koharu)* — in-app dialog to batch-translate a folder of `.khr` projects with pause and time estimates.
- [ ] **Editor and runtime performance optimizations** *(gudoy/koharu)* — memory-bounded canvas page cache, neighbour prefetching, faster encoding, llama.cpp context reuse, and unreferenced blob cleanup.
