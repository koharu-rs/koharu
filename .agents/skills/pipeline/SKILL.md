---
name: pipeline
description: Koharu page-stage scheduling, model residency, and incremental commits. Use when changing detection, OCR, translation, inpainting, cancellation, or pipeline undo.
---

# Pipeline

Coding rules and the skill index are in `AGENTS.md`.

`koharu-pipeline` schedules one stage on one page. `koharu-ml` owns models and the shared device abstraction. `koharu-translator` owns local and hosted translation connections. Provider defaults stay inside that provider.

```text
detection → OCR → translation
detection → inpainting
```

Pages enter in project order. Completed detection makes that page's dependent branches eligible immediately, without waiting for detection on the whole project. At most one job per stage runs at a time. The number of pages in flight is bounded by the number of selected stages.

Stages return semantic patches. The application commits each result, synchronizes the canvas, and records each commit as its own undo step. Do not add a second translation context, prompt format, or run-request path beside the existing translation config.

Accelerator inference uses one admission lane, including remote translation providers. CPU-only work may keep independent per-model lanes. Models load lazily and remain reusable.

When a stage fails with an out-of-memory error, the runner releases the accelerator lane, unloads the other stages' models, waits briefly for a fresh resource sample, and retries once. Any other failure unloads the failing stage's model. A settings change rebuilds the stage runner, so models reload on the next run.

Stopping is cooperative. New stages do not start after a stop request. Active native inference returns safely. An uncommitted result is discarded.
