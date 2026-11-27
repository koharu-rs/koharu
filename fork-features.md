# Fork features

> What the six reference forks add on top of upstream Koharu, grouped by feature — and the fork features this fork has considered and deliberately deferred.

Companion to [our-features.md](our-features.md), the specs of this fork’s planned features — their Prior art sections compare each planned feature against what the forks ship. This file is the other direction. Where several forks ship the same feature, it is listed once with all forks named. Based on each fork’s README and fork documentation, checked 2026-11-13.

Reference forks: [NeKoharu](https://github.com/nekojin116/NeKoharu), [spgsroot/koharu](https://github.com/spgsroot/koharu), [koharu-context](https://github.com/Wkingxc/koharu-context), [gudoy/koharu](https://github.com/gudoy/koharu), [koharu-ex](https://github.com/Gray-Rice/koharu-ex), [koharu-edge](https://github.com/MOULINIER/koharu-edge).

| Fork | Focus |
|---|---|
| NeKoharu | Editor and export conveniences; tracks upstream `main` |
| spgsroot/koharu | Production translation workflow (cleanup, orders, automation); based on upstream 0.79.4 |
| koharu-context | Chapter-level contextual translation; Chinese-language fork |
| gudoy/koharu | Performance, stability, batch folder translation |
| koharu-ex | Experimental UX and robustness features; based on upstream 0.83.x |
| koharu-edge | Reviewed cherry-picks of pending or closed upstream PRs plus own fixes; tracks upstream `main` |

## Editor and canvas

- **Arrow-key page navigation** — NeKoharu (active page stays visible in the page rail), koharu-context (switch pages with ←/→ on the main page; ignored while typing in inputs).
- **Drag-and-drop reordering** — NeKoharu (pages in the page rail), koharu-edge (layers, including into groups; from upstream PR #959).
- **Multi-layer and rectangle selection** — NeKoharu (drag a rectangle from empty canvas with the select tool; Shift/Ctrl/Cmd adds; ported from upstream PR #1148), koharu-ex (drag to multi-select layers; rectangle selection for the eraser and remove tools).
- **Copy and paste between pages** — NeKoharu (selected text layers with text, placement, and formatting), spgsroot (layer clipboard for text and raster layers), koharu-edge (text style only: font, weight, color, and border via Ctrl+Alt+C / Ctrl+Alt+V).
- **Saved colors** — NeKoharu (last eight colors as recent swatches, eyedropper included; ported from upstream PR #989), koharu-edge (project colors: save a color to the manga with **+**, reuse it on any page, stored in the project file).
- **Page flags and reset** — NeKoharu (manually mark chapter starts in the page rail), koharu-edge (Mark as Done with a thumbnail checkmark; Reset returns a page to its imported image, undoable).
- **History panel** — spgsroot (Photoshop-style list of named states, snapshots, clear history).
- **Resource monitor** — spgsroot (CPU, RAM, GPU, and VRAM in the status bar).
- **Consistent text styles** — koharu-edge (detection gives same-looking text the same color and border within and across pages; manual edits propagate to later same-style detections; thin borders that used to be dropped are kept).
- **Font preview on selection** — NeKoharu (browse candidates with keyboard or pointer, each applied live to the selected text).
- **Per-frame OCR** — koharu-edge (an OCR button on each text layer, also for hand-drawn frames; clearing a frame's source text lets OCR fill it again; from upstream PR #1065).
- **Cleaner overlapping detection** — koharu-edge (a paragraph detected as both one large box and smaller inner boxes becomes a single text block; from upstream PR #1070).
- **Spreads** — NeKoharu (merge adjacent split-panel pages into one wide spread from the page menu, keeping text layers, translations, cleanup layers, and analysis regions).
- **Brush conveniences** — NeKoharu (cleanup layer auto-selected when switching to brush or eraser), spgsroot (three manual repair brushes — Fast (LaMa/AOT), Inpainting (FLUX) with saved presets, and Decensor — strokes queue and run while drawing).

## Translation

- **Cross-page translation context** — NeKoharu (optionally translate multiple pages in one request, with a configurable page limit, for name and plot consistency), koharu-edge ("Context from previous pages" setting, off by default), koharu-context (whole-chapter translation; see the next section).
- **Page-level failure isolation** — koharu-ex (a failing page is skipped instead of failing the run, with a summary of per-page errors, success/failed counts, and retry for failed pages), gudoy (a processing failure is confined to its page; real stage errors survive Stop), spgsroot (a provider's refusal of one page leaves the other pages translated; refused page numbers reported).
- **Translation validation** — spgsroot (rejects translations that still contain a blocked script — Latin, kana, Han, Hangul presets or custom rules — and retries with feedback), koharu-context (verifies returned text-block IDs for completeness, duplicates, and empty translations before writing back).
- **Profiles and presets** — spgsroot (Manga, Comic, and Hentai content profiles, each with an editable system prompt; per-model reasoning effort), koharu-context (saveable processing configurations covering pipeline engine, model, target language, reading order, and font/rendering style; ships Japanese-manga and Korean presets with matched OCR models).
- **Token and cost tracking** — spgsroot (journal of every model request with reported tokens and cost, filterable by provider, model, operation, and date; prompts and responses never stored).
- **Extra providers** — spgsroot (Cursor as a translation provider; agent sign-in via Grok and Kimi Code, all with device-code OAuth).

## Chapter-level translation workspace — koharu-context

The fork's core feature, as a dedicated top-bar page: detection and OCR run across the whole project first, then translation is written back per text-block ID, and re-running overwrites old translations (a translate → refine → re-translate loop). Requests cover the whole chapter in one go, or batches sized by page count; each batch returns a summary the user proofreads and edits before continuing, and confirmed summaries are carried into later batches. Uses API models only (OpenAI, OpenAI-compatible, Claude, Gemini, DeepSeek) with favorites, target language, and max output tokens, plus prompt fields for work background, fixed name translations, honorifics, and tone. Progress shows real page counts, and results inherit the editor's global font and per-block styles.

## Cleanup and inpainting — spgsroot

- **Split lettering routes:** detection writes separate masks for text inside bubbles and lettering over artwork; bubbles get a flat fill or LaMa/AOT, artwork gets a generative model, and sound effects upstream leaves on the page are removed.
- **More artwork models:** a FLUX.2 Klein 9B profile (optional LoRA) and ComfyUI SDXL backends (Animagine XL 4.0, Illustrious XL 2.0) alongside upstream's models, each stage keeping its own prompt and sampling settings.
- **Censor detection and decensor:** black bars, white strokes, and optional pixel mosaics are found into a reviewable mask; reconstruction then applies only to that mask.

## Batch, automation, and collaboration

- **Batch translation of many projects** — gudoy (batch-translate a folder of projects; dialog with progress, pause, and time estimate; resumes only pages a stage has not finished; re-translates sources edited since).
- **Automation API** — spgsroot (loopback HTTP API to create a project, import pages, run the pipeline, and export results from another program, such as a chat bot).
- **Orders dashboard** — spgsroot (orders, stages, deadlines, notes, and an audit log).
- **Translation exchange** — koharu-ex (JSON-based import/export with page-level granularity, for collaborative workflows), spgsroot (.xlsx/.csv translation exchange for outside translators, via the orders dashboard).
- **Project organization** — koharu-ex (project archiving), spgsroot (isolated configuration and project directories via `KOHARU_CONFIG_DIR` / `KOHARU_PROJECTS_DIR`).
- **Gallery import** — spgsroot (download e-hentai, exhentai, and nhentai galleries into one project; cookies kept in the system keychain).

## Export

- **Selective page export** — NeKoharu (export selected pages as PNG, PSD, or CBZ from the export menu), koharu-ex (individual or grouped export of selected pages without exporting the whole project), gudoy (incremental page export; each page rendered once per requested format).

## Performance and stability — gudoy

gudoy's fork is almost entirely performance and reliability work: the editor no longer re-renders on pan, zoom, or hover; the canvas page cache is memory-bounded; the model picker mounts only visible rows; neighbour pages are prefetched; PSD and PNG encoding is faster; llama.cpp contexts are reused per loaded model; detection and inpainting run in bfloat16 on Apple GPUs; the UI opens before the ML runtime finishes loading; models stay loaded when a config reload leaves a stage unchanged; inpainting crop sizes are bucketed to bound accelerator memory; text drafts autosave on quit and periodically; one undo step per text-edit session; unreferenced blobs are collected when a project opens; HTTP read timeout; and telemetry can be disabled with `KOHARU_DISABLE_TELEMETRY`.

## Distribution and packaging

- **NeKoharu** — self-updating releases served from its own repository.
- **koharu-edge** — installs as a separate app with an "EDGE" badge, sharing settings, models, API keys, and projects with upstream Koharu.
- **spgsroot/koharu** — upstream auto-updater removed (built and installed locally); bundles the Er0mangaSeg censor model.
- **koharu-ex** — distributes under MIT only; builds for Linux and Windows only.
- **koharu-context** — ships a Windows installer and macOS app builds; UI text maintained in Simplified Chinese and English.

## Deferred adoption candidates

Fork features considered for adoption but deliberately postponed, with what to decide when they come back — so the decision is not re-derived from scratch. Add an entry whenever a fork feature comes up and the answer is "later."

### Merged spreads (NeKoharu)

**What it is.** Merge two adjacent split-panel pages into one wide spread from the page menu, keeping text layers, translations, cleanup layers, and analysis regions (NeKoharu's implementation).

**Why deferred.** Not on the fork's feature list, and the feature needs to be seen in use first — what merging actually does to the rail, the layers, and the processing flow — before deciding whether this fork wants it.

**To decide when it returns.**
- How a spread exports in the comic formats: one wide page, or split back into two pages on export. This is the one interaction with the planned features — comic-format export — which is why the export spec points here.
- How a merged page interacts with per-page stage eligibility and only-undone scoping: a spread is one canvas built from two source pages, so which page counts as done?

### Selective page export (NeKoharu, koharu-ex, gudoy)

**What it is.** Export a subset of pages instead of the whole project — NeKoharu exports the selected pages as PNG, PSD, or CBZ from the export menu; koharu-ex exports individually or in groups; gudoy exports incrementally, each page rendered once per requested format.

**Why deferred.** Upstream's export command takes only a format and exports every page. Decided on 2026-11-14 to keep whole-project export as the only scope for the comic-format work — the smallest change; selection is a fork feature that can be adopted wholesale later.

**To decide when it returns.**
- Whether selection rides the export menu (NeKoharu's shape) or a dedicated flow.
- Whether incremental export (gudoy's shape — render each page once per requested format) comes with it.
