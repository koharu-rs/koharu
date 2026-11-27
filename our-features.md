# Our features

The expanded specs of this fork’s remaining planned features, each with motivation, behavior, boundaries, interactions, and open questions. Features already implemented in this fork are documented in [FEATURES.md](FEATURES.md) (including drag-and-drop import, CBZ WebP/JPEG XL export with automatic grayscale encoding, ComicInfo.xml 2.1 metadata, declared model sizes and VRAM fit indicators, the VRAM-aware job manager, the download manager, the request queue and persistent Activity Center, and the `koharu-cli` headless batch runner). Upstream Koharu `0.99.0` already ships FLUX.3 Klein alongside Qwen-Image 2.1 Turbo. What the reference community forks ship — and which of it this fork has deferred — is surveyed in [fork-features.md](fork-features.md).

This is a personal planning document: each entry covers what a feature is for and what it would do, at the level of user-visible behavior — no technical design yet. Decisions worked through on 2026-11-13 and 2026-11-19 are folded into the text below; new ideas from hands-on review on 2026-11-27 are included under **Job orchestration** and **AI assistance**.

## Contents

- **Job orchestration** — stage-specific rerun dropdown on the Run button · only-undone reruns
- **AI assistance** — AI repaint and typesetting of stylized text · SFX translation awareness · smart prompting
- **Optional / exploratory** — scan restoration and upscaling
- **Build order** — the sequence the remaining features land in

## Job orchestration

### Stage-specific rerun dropdown on the Run button

> A dropdown on the **Run** button next to the model selector to redo a single chosen stage for the button's current scope.

**Motivation.** Right-clicking a page in the page rail already lets you run a single stage (**Detection**, **OCR**, **Inpainting**, or **Translation**) on that one page in one click. Doing the same thing for **Selected pages** or **Entire project** currently requires opening the processing selector popover, going into **Stages**, unchecking the other three stages, clicking **Run**, and then re-enabling the other stages afterwards. A dropdown directly on the **Run** button removes that friction.

**Behavior.** The **Run** button next to the model selector always exposes a split dropdown (even when idle, not only when queuing behind an active run). Clicking the primary button runs the currently configured stages for the current scope as today. Opening the dropdown offers quick one-click stage actions — **Detection only**, **OCR only**, **Inpainting only**, and **Translation only** — each running just that stage at the button's active scope (**Selected pages** or **Entire project**) without mutating the user's saved stage checkboxes. When a run is already active, the dropdown keeps the **Queue** / **Rerun now** priority choice alongside the single-stage rerun entries.

**Boundaries.** A shortcut entry point over the existing `run_pipeline` command and current scope selection — no new job kind, no separate stage config, and no change to the saved `processingStages` preference when choosing a one-off stage from the dropdown.

**Interactions.** Mirrors the page rail's single-stage right-click menu, generalized to the processing selector's scope. Pairs naturally with **Only-undone reruns** if adopted later and with **Rerun now** priority when another run is in flight.

**Open questions.**
- When a run is already active, should single-stage actions in the dropdown submit as `Normal` (queued) or `RerunNow`, or follow a toggle inside the dropdown?

**Prior art.** In-repo: the page rail's per-page context menu (`Detect`, `Recognize`, `Inpaint`, `Translate`).

### Only-undone reruns for OCR and translation

> Re-running OCR or translation costs nothing on text blocks that stage already produced output for — avoiding redundant regeneration on untouched blocks.

**Motivation.** Upstream `koharu-pipeline` runs every requested stage fresh across everything in scope, replacing existing output (with undo as the way back). Re-running after a failure or after adding pages currently re-executes detection, OCR, inpainting, and translation on pages that were already finished, regenerating machine-authored text that was already fine.

**Behavior.** Stages gain optional only-undone semantics, tracked per text block for OCR and translation (decided 2026-11-19): a block counts as undone when it has no output from that stage, or when what the output was produced from has changed — source text edited, or a detection box moved or resized, which makes OCR stale for that block. A manual text box added to a finished page is translated on the next run without redoing the page. User-authored translations are never overwritten. Settings changes do not invalidate output: switching the translation model or editing the prompt profile leaves finished translations done — otherwise every settings tweak would silently re-translate everything. An adapted prompt from smart prompting does not make output stale either. An intentional redo rides the processing selector's scope modifier: "Missing only" vs. "All pages" — one command with one more field, not a second path.

**Boundaries.** Stage-level scoping plus one scope modifier — no new job kind, no new command, no second path: the same runs through the same surfaces (processing selector, Agent, CLI) simply skip finished work when asked, and the modifier rides the selector those runs already come from.

**Interactions.** Re-running after failed pages or interrupted unattended CLI runs becomes cheap. Rerun-now with All pages is the fast targeted redo. A spread, if ever adopted (see [fork-features.md](fork-features.md)), raises which source page counts as done. Coalescing in the request queue must respect the modifier.

**Open questions.**
- Upstream `koharu-pipeline` explicitly specifies fresh runs ("a run never decides that a page needs no work"). Decide whether to keep fresh runs as default with an opt-in "Missing only" modifier, or make "Missing only" the default.

**Prior art.** Partial. gudoy's batch translation resumes only pages a stage has not finished and re-translates sources edited since; koharu-context's chapter workspace re-runs and overwrites by design.

## AI assistance

### AI repaint and typesetting of stylized text

> AI repaints the artwork behind stylized source text — sound effects — and the translated replacement is typeset over it, styled to match, with a generative option.

**Motivation.** Stylized lettering is the part of manga translation the pipeline handles worst. Sound effects and emphasized text are painted into the artwork, not into bubbles, so they land in cleanup: a generative model can remove them and reconstruct what was behind, but the replacement — a translated SFX with the original's weight, slant, and energy — is left to a human typesetter, or simply not translated. This feature closes the loop: remove the lettering, then put the translation in its place in a style that matches. Detection is further along than it first appeared: onomatopoeia is already a detected class in `koharu-ml` — what is missing is routing those regions into this feature's flow.

**Behavior.** Stylized source text is treated as its own category, defined by a criterion rather than a list: source text that interacts with the artwork such that normal typesetting cannot replace it — painted-in sound effects above all; chapter titles and signs qualify only when they are part of the art, and hand-lettered dialogue stays on the normal text path. Its background is repainted (generative inpainting, targeted at these regions via split lettering routing). The translated replacement is stored as a normal text record in the data model — hidden or marked artwork-rendered — and whatever is painted into the artwork is derived presentation of that record (decided 2026-11-19): re-rolls re-render from the record, and translation exchange, re-rendering, and the CLI summary keep the text. The default replacement is an editable styled text layer — outline, warp, rotation, gradient — which handles most SFX and stays editable like any text layer; generative lettering, where the model paints the replacement in the original's style, is opt-in per region (decided 2026-11-19). The generative path is a known feasibility risk and is treated as such: matching the original lettering's weight, slant, and energy in a target-language script at manga resolution is unreliable with today's models, so an acceptance bar defined by a spike on representative pages precedes the generative option shipping. The user reviews results on the canvas and adjusts them — per-region re-rolls (which submit as rerun-now, per the queue's priority rule) and undo are the control surface; there is no separate accept/reject flow. Getting a good generative result needs either that user involvement or the smart prompting feature describing the original's style well. In export the styled text layer is a text layer like any other, and a generative raster replacement is its own PSD layer, like cleanup layers — typesetters can refine either in Photoshop — with the flattened PNG staying flat.

**Boundaries.** Two halves with different maturity. Repainting behind stylized text builds on split lettering routing (bubbles vs. artwork/SFX) and existing `onomatopoeia` detection. The replacement has two tiers: the styled text layer keeps the app's rule that visible text comes only from translation text layers; a generatively lettered region is the deliberate divergence from that rule, chosen per region rather than by default. The original idea's word "typeset" is literal for the default tier.

**Interactions.** Routing onomatopoeia regions from detection into this flow enables [SFX translation awareness](#sfx-translation-awareness) below. Strong generative models already in-tree (FLUX.3 Klein, Qwen-Image 2.1 Turbo) raise the ceiling on both halves. Smart prompting describes the original's style to the generating model. The canvas remains the review surface; undo, regeneration, and paint-over are the controls.

**Open questions.**
- The acceptance bar for generative lettering: a spike on representative pages and masks defines "good enough" before the opt-in path ships.

**Prior art.** Partial. spgsroot routes lettering over artwork separately, removes sound effects, and regenerates the artwork behind them (FLUX.2 Klein 9B, ComfyUI SDXL backends, repair brush with presets). No fork typesets a stylized replacement.

### SFX translation awareness

> Let the user choose whether detected sound effects (SFX) are translated and repainted or left untouched in the artwork.

**Motivation.** Not every reader or scanlation workflow wants sound effects translated: translating SFX requires heavier generative inpainting over complex artwork and extra typesetting review, and many readers prefer keeping original Japanese/Korean hand-drawn sound effects intact while translating only dialogue and narration. Because the Phase 2 SFX repainting pipeline already distinguishes sound effects (`Role::Sfx` / onomatopoeia) from normal dialogue and free text, exposing an explicit user choice falls out naturally.

**Behavior.** A user-facing setting (available in pipeline preferences and/or per project) controls how SFX regions are handled during pipeline runs:
- **Translate & repaint SFX** — sound effects flow through OCR, generative artwork cleanup, and styled-text / opt-in generative replacement.
- **Keep original SFX** — sound effects are still recognized as SFX regions so dialogue detection stays clean, but they are skipped by cleanup/inpainting, translation, and typesetting, leaving the original artwork untouched.

Individual SFX blocks on the canvas can still be manually selected and translated or repainted on demand even when automatic SFX translation is off.

**Boundaries.** Controls pipeline routing for SFX-classified blocks; does not alter detection of dialogue bubbles or normal free text, and does not add a separate pipeline stage.

**Interactions.** Phase 2 companion to [AI repaint and typesetting of stylized text](#ai-repaint-and-typesetting-of-stylized-text). Skipping SFX saves substantial generative inpainting time and VRAM when the user only wants dialogue translated, and keeps the Story tab focused on dialogue when SFX translation is disabled.

**Open questions.**
- Should the SFX toggle live in global `[pipeline]` settings, per-project settings (stored in `.khr`), or global default with a per-project override?
- When "Keep original SFX" is active, should SFX blocks still be hidden/grouped separately in the Story panel or omitted from the translation prompt altogether?

**Prior art.** Partial. Upstream detection already separates the `onomatopoeia` class; spgsroot routes lettering over artwork separately.

### Smart prompting

> An AI adapts prompts for the pipeline's models based on the current page's image and extracted text.

**Motivation.** Generative inpainting quality depends on the prompt, and prompts today are static per model: the user writes a reconstruction prompt once, kept in the model's settings profile, and it applies to every page. The translation prompt is static in the same way. A flat screentone background and a detailed crowd scene need different guidance; a gag panel and a solemn one need different tone. Per-page tuning is expert work most users never do, so most pages get a mediocre default.

**Behavior.** Based on the current page — the extracted text, and the image where the adapter can see it — an AI adapts the prompts given to the pipeline's promptable models: the generation prompts of the SFX repaint feature above, and the translation prompts. It can add character tags to the dialogue, supply plot context, and set the appropriate tone. Images are used only when the adapter model is multimodal (decided 2026-11-19): many local translation LLMs are text-only, so the adapter falls back to extracted text alone when the configured model cannot read images. The feature is enabled per project, and enabling it with an API adapter shows what that means — every page image leaves the machine, and cost goes up by a request per page. Once enabled it runs automatically: each page's job carries a prompt suited to that page, without the user tuning anything manually. The adapted prompt is kept per page and viewable read-only in the UI, so a bad translation can be traced back to the prompt that caused it. The user's configured prompts become the baseline or fallback rather than the per-page decision. The adapting model is its own setting: it defaults to the configured translation model, and another model can be chosen for it. Cross-page memory is out of the first cut (decided 2026-11-19): per-page adaptation ships first, and plot-detail consistency is a later extension — the forks' cross-page translation context is the proven mechanism to consider adopting for it, rather than building memory into the adapter.

**Boundaries.** Adaptation of prompts for the promptable models — the SFX-repaint generation prompts and the translation prompts; anything else waits for a real consumer. Adapted prompts flow through the existing per-model prompt profiles: the adapter composes the prompt the pipeline already consumes, and no second prompt path is added. No new pipeline models or tuning knobs beyond the adapter model choice, and no automatic changes to non-prompt settings such as detection thresholds or sampling parameters; those stay manual unless decided otherwise. An adapted prompt does not invalidate existing output — same rule as settings changes under only-undone reruns.

**Interactions.** The AI repaint feature is the flagship beneficiary — describing style and surroundings well is exactly its need. The adapter defaults to the configured translation model and can be pointed at another; API adapters add a request per page. A local adapter is a resident the VRAM manager plans around. Cross-page memory (the plot-detail consistency) overlaps the forks' cross-page translation context — prior art for the mechanism, not the prompt-adaptation use.

**Open questions.** None remaining.

**Prior art.** None. spgsroot keeps separate, editable prompts and sampling settings per model and stage, but the adaptation is manual, not driven by the images.

## Optional / exploratory

### Scan restoration and upscaling

Use AI image models to upscale scanned paper manga and remove scanning artifacts. Old print scans become cleaner, higher-resolution source images. Deliberately not expanded into a spec — none of the surveyed forks implements it — and revisited only when there is a real need.

## Build order

Phase 1 (drag-and-drop import, CBZ WebP/JPEG XL export with grayscale encoding, ComicInfo.xml 2.1, download manager, declared model sizes + VRAM fit, VRAM-aware job manager, request queue + persistent Activity Center, and `koharu-cli`) is shipped and documented in [FEATURES.md](FEATURES.md).

Remaining sequence:

1. Stage-specific rerun dropdown on the Run button (small UI convenience over existing `run_pipeline`)
2. Only-undone reruns (optional — pending decision on fresh vs. missing-only default)
3. Smart prompting (Phase 2A)
4. AI repaint and typesetting of stylized text + SFX translation awareness (Phase 2B, with generative lettering gated on the feasibility spike)
