# Koharu Project Rules

Document only durable, repository-specific constraints here. Do not record current file layouts, temporary paths, model inventories, helper names, or other implementation details that may change during a refactor. Normal Rust, TypeScript, testing, formatting, and Git practices are assumed.

## Change Policy

- Never add backward compatibility. When an API, schema, configuration, or ownership boundary changes, update every in-repository consumer and remove the replaced form.
- Prefer a coherent ownership redesign over aliases, forwarding layers, compatibility parsers, or cosmetic renaming.
- Keep responsibilities self-contained. Defaults and provider-specific behavior belong to the component that owns them rather than a central list of special cases.
- Remove dead abstractions and one-use helpers when direct code is clearer.
- Make the smallest change that solves the problem. Do not edit unrelated configuration, documentation, generated files, or workflows in the same change. A feature does not land beside drive-by cleanup.
- If the current owner has moved, rebase onto that owner. Do not port a patch onto a deleted tree.
- When two changes implement the same workflow, keep one and extend the existing command or configuration. Do not land both.
- For editor behavior that is not already specified, match Photoshop or Clip Studio Paint. Prefer an in-app edit over an export/import round trip when the task is ordinary editing.
- Desktop API keys go through the OS credential store. Do not add a project environment-file fallback for the desktop app.
- Once a subsystem is removed, delete its calls and dependency together. Do not leave a stub, alias, or disabled flag behind.

## Ownership

- The UI invokes typed commands. Native code owns durable project state and processing. Do not add an HTTP client or a generic event envelope.
- Scene owns what a project means. Storage owns opaque payloads and immutable blobs. Analysis, content, and presentation stay separate: detection geometry is not a visible layer, translation does not drop OCR provenance, and typography does not rewrite semantic text.
- A patch binds to a project and a base revision, records preconditions, and carries inverses. Reject a stale patch. Derived work rebases explicitly and fails if anything it observed has changed.
- Publish missing blobs first, write the inactive state slot, then publish atomically. Opening uses the newest valid slot and can fall back. Renderer, pipeline, and Agent consume snapshots and submit patches. They do not write storage files.
- A pipeline stage runs on one page: detection, then OCR and translation, or detection, then inpainting. Pages enter in project order. Completed detection makes that page's later stages eligible immediately. At most one job per stage runs at a time. Each committed result is its own undo step. Do not add a second translation prompt path.
- Accelerator inference uses one admission lane, including remote translation. On out-of-memory, release the lane, unload the other models, wait for a fresh resource sample, and retry once. Any other failure unloads the failing stage. Stopping is cooperative and discards an uncommitted result.
- One prepared frame feeds presentation, PNG, layer crops, and PSD. Translation is the only visible text. Incremental reuse must match a full render. The previous page stays visible until the staged generation is ready. An obsolete preparation cannot replace a newer generation. Transient camera, transform, stroke, and sampling stay in the browser. Completed edits cross commands.

## Interface

- User-visible strings go through the translation function and the locale files. Hardcoded English in a translated surface is incomplete.
- An interface language's display name is that language's own name in every locale. Do not translate it into the surrounding locale.
- The translation-target language list stays in the active UI language. Drop a country name unless the entry is a real variant.
- Do not duplicate status the current selection already implies. Do not add a second job kind, cancel path, or counter for an operation the existing command already covers.
- Use the existing control library for standard interaction. Commit on selection or an uncanceled drop, not on focus or hover. Ignore a canceled drag. Do not let a shortcut steal keys from a focused control.
- Reuse the existing numeric color and typography representation. Do not introduce a parallel string format.
- A page switch uses the existing selection and optimistic cache update. Do not add a second navigation path that leaves the rail, layer selection, or query data behind.

## Source Boundaries

- Keep safe public APIs separate from unsafe FFI, dynamic loading, and build integration.
- Do not hand-edit generated or derived source. Change its authoritative input and run the generator.
- Do not commit credentials, model weights, datasets, generated outputs, or machine-specific artifacts.
- Native llama.cpp and diffusion runtimes come from prebuilt packages. Do not compile those CUDA trees inside the app build. Release and publish jobs do a fresh build.
- Hardware discovery stays separate from managed runtime package activation. Do not load a cached accelerator runtime during the probe.
- Error sampling is off in debug builds and on in release builds.

## ML Architecture

- Keep a consistent public lifecycle across models while allowing model-specific inputs and outputs.
- Separate network ownership and weight loading from preprocessing, postprocessing, slicing, and public result types.
- Avoid pass-through types and layers that do not own a real responsibility.
- Accept a device abstraction at the model boundary, convert it once, and avoid unnecessary transfers or synchronization.
- Use the established runtime and variable-store loading paths unless they are proven insufficient.
- Disable gradient tracking during inference.
- Create generated token IDs and index tensors on the model device. A CPU tensor used against accelerator embeddings is a defect, not a later optimization.
- Invalid model JSON fails closed. Do not add a silent repair layer that can change meaning.
- Enforce a translation output schema through each provider's native structured-output API. A bounded retry belongs only to a provider that already has one. Hosted machine-translation providers fail immediately.

## Upstream Alignment

- Keep ports structurally traceable to a commit-pinned authoritative implementation.
- Preserve checkpoint-affecting names, construction order, parameter paths, tensor layouts, execution order, and postprocessing semantics.
- Treat missing or unexpected weights as an architecture or parameter-name mismatch before changing the loader.
- Explain intentional divergences next to the affected code.
- Compare ports on identical inputs using structured outputs such as shapes, ranges, boxes, scores, masks, and ordering.

## Performance

- Optimize and benchmark the actual target device with representative inputs.
- Remove redundant transfers, synchronization, allocations, and per-pixel host loops before adding concurrency or caching.
- Account for asynchronous accelerator execution when timing work.
- Load assets and warm models outside measured regions.
- Report the device, input size, baseline, result, and correctness difference.

## Verification

- Optimize for fast development and iteration. By default, run the smallest relevant check or focused test once using the debug profile.
- Do not run full test suites, repeatedly rerun unchanged tests or builds, or build and test profiles other than debug unless the user explicitly requests it.
- Run end-to-end tests only when the user explicitly asks for them.
- Add a test when it protects a real regression boundary. Do not lock a translated phrase, a language display name, a registration string, or a one-line behavior the owner can read directly. Do not add a second harness for behavior the crate's existing tests already cover. Do not assert a compatibility shim.
- A UI change needs a screenshot of the affected state. If the bug is truncation, include a long-label case.
- A model port needs a structured comparison on identical inputs. A green build is not that comparison.
- A green check is not acceptance. Small bug fixes, documentation, translations, and tests can be opened directly. A new product feature, significant UI change, model or provider integration, platform integration, dependency, public API, schema change, or architectural refactor needs an issue discussion first.
- Keep the pull-request text short: problem, change, and verification. A human reviews the whole diff and can explain every change. Do not submit an unreviewed autonomous patch.

## Desktop UI Debugging

- Debug builds must expose the CEF remote debugging endpoint at `http://127.0.0.1:4000` through CEF's command-line arguments.
- Connect `chrome-devtools-mcp` with `--browser-url=http://127.0.0.1:4000` and prefer its tools for WebView inspection and automation. Use semantic targets and observable conditions instead of coordinate-only actions or fixed delays.
- Use a lower-level CDP client only when `chrome-devtools-mcp` does not expose a required protocol operation. Use native window capture when CDP cannot observe the final WebGPU output.

## Desktop Rendering

- Koharu presents canvas pixels through the `koharu-canvas` WASM module and WebGPU inside the standard Tauri webview. Keep durable scene preparation and export native, keep transient canvas interaction in the browser, and validate WebGPU presentation through the final desktop window.

## Documentation

- Comments should explain ownership, invariants, upstream mapping, or deliberate divergence; do not narrate straightforward code.
- Keep this file focused on long-lived decision rules rather than the current implementation.
