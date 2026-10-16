# Coding style and preferences

Normal Rust, TypeScript, formatting, and Git practice is assumed. These are the repository-specific rules.

## Change shape

- Never add backward compatibility. When an API, schema, config key, or ownership boundary changes, update every in-repository consumer and delete the replaced form.
- Do not add aliases, forwarding layers, compatibility parsers, or cosmetic renames. Prefer one coherent owner over a shim.
- Remove dead abstractions and one-use helpers when the direct code is clearer.
- Make the smallest change that solves the problem. Do not edit unrelated Tauri config, docs, generated files, or workflows in the same change.
- Split mixed work into separately reviewable changes. A feature does not land beside drive-by cleanup.
- If `main` has moved and the patched files are gone, rebase onto the current owner. Do not port a patch onto deleted trees such as top-level `ui/`, `koharu-rpc`, `koharu-llm`, or hand-written OpenAPI clients.
- When two changes implement the same workflow, keep one and close or retarget the other. Extend the existing config or command. Do not land both.
- Provider defaults and provider-specific request behavior stay inside that provider. Do not grow a central list of special cases in the engine.
- For editor behavior that is not already specified, match Photoshop or Clip Studio Paint rather than inventing a second interaction model.
- Prefer an in-app edit over an export/import round trip when the task is ordinary editing.
- Desktop API keys go through the OS credential store. Do not add a `.env` fallback for the desktop app.

## Comments and model code

- Comments explain ownership, invariants, upstream mapping, or a deliberate divergence. Do not narrate straightforward code.
- A model port stays structurally traceable to a commit-pinned upstream. Preserve checkpoint-affecting names, construction order, parameter paths, tensor layouts, execution order, and postprocessing.
- Treat missing or unexpected weights as an architecture or parameter-name mismatch before changing the loader.
- Accept a device abstraction at the model boundary, convert once, and avoid extra transfers or synchronization. Disable gradient tracking during inference.
- Create generated token IDs and index tensors on the model device. A CPU tensor used against accelerator embeddings is a bug, not a later optimization.
- Compare ports on identical inputs with structured outputs: shapes, ranges, boxes, scores, masks, and ordering.

## Interface and copy

- UI strings go through `t()` and the locale files. Hardcoded English in a translated surface is incomplete.
- An interface language's display name is that language's own name in every locale. Do not translate it into the surrounding locale.
- The translation-target language list stays in the active UI language. Drop a country name unless the entry is a real variant.
- Do not duplicate status the current selection already implies. Do not add a second job kind, cancel path, or counter for an operation the existing command already covers.
- Use the existing control library for standard interaction, such as sortable drag-and-drop, instead of a hand-written pointer state machine.
- Commit on selection or an uncanceled drop, not on focus or hover. Ignore a canceled drag. Do not let a shortcut steal keys from a focused slider or other control.
- Reuse the existing numeric color and typography representation. Do not introduce a parallel string format.
- A page switch uses the existing selection and optimistic cache update. Do not add a second, slimmer navigation path that leaves the rail, layer selection, or query data behind.

## Runtime and release

- Safe public APIs stay separate from unsafe FFI, dynamic loading, and build integration.
- Native llama.cpp and diffusion runtimes come from prebuilt packages. Do not compile those CUDA trees inside the app build. Release and publish jobs do a fresh build. Do not cache them.
- Hardware discovery stays separate from managed runtime package activation. Do not load a cached HIP runtime during the probe.
- Sentry error sampling is off in debug builds and on in release builds.
- Once a subsystem is removed, delete its calls and dependency together. Do not leave a stub, alias, or disabled flag behind.
- Do not commit credentials, model weights, datasets, generated outputs, or machine-specific artifacts.
