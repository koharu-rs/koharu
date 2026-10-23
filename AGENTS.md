# Working on Koharu

Koharu is a local-first manga translator: a Rust workspace with a TypeScript interface, shipped as a desktop app. This file holds durable decision rules only. Architecture, file layouts, crate and helper names, model inventories, commands, and anything else a refactor can change belong in crate READMEs and the docs. Normal Rust, TypeScript, testing, formatting, and Git practices are assumed.

## Product

- Models run on the user's machine. Pages and text leave it only through a hosted provider the user chose.
- Editor behavior that is not already specified matches Photoshop or Clip Studio Paint. Ordinary editing happens in the app, not through an export and re-import round trip.
- Desktop API keys live in the OS credential store. Do not fall back to a project environment file.

## Changes

- Never add backward compatibility. When an API, schema, file format, configuration, or ownership boundary changes, update every in-repository consumer and delete the replaced form: no alias, forwarding layer, compatibility parser, upgrade decoder, stub, or disabled flag.
- Removing a subsystem removes its calls and dependencies in the same change.
- Prefer a coherent ownership redesign over cosmetic renaming. Defaults and provider-specific behavior belong to the component that owns them, not to a central list of special cases.
- Make the smallest change that solves the problem. Leave unrelated configuration, documentation, generated files, and workflows alone. A feature does not land beside drive-by cleanup.
- When two changes implement the same workflow, keep one and extend the existing command or configuration.
- If the code you are changing has moved, rebase onto its new owner. Do not port a patch onto a deleted tree.
- Invalid input fails closed, whether it is a project file, configuration, or model output. Do not add a repair or defaulting layer that can change meaning.

## Code

- Write the smallest code that does the job. Inline a helper with one caller, keep a small module in its parent, and do not wrap a type without adding a responsibility.
- Reach for a crate before writing code: a well-known external crate first, then one in this repository. Hand-write only what no crate covers, and prefer a cross-platform crate to per-platform branches.
- Keep safe public APIs apart from unsafe FFI, dynamic loading, and build integration.
- Do not hand-edit generated or derived source. Change its input and rerun the generator.
- Comments explain ownership, invariants, upstream mapping, or a deliberate divergence. Do not narrate straightforward code.
- Do not commit credentials, model weights, datasets, generated outputs, or machine-specific files.

## Interface

- User-visible strings go through the translation function and the locale files. Hardcoded English in a translated surface is incomplete.
- An interface language's display name is that language's own name in every locale.
- The translation-target language list stays in the active UI language. Drop a country name unless the entry is a real variant.
- Do not show status the current selection already implies. Do not add a second job kind, cancel path, or counter for an operation an existing command covers.
- Use the existing control library for standard interaction. Commit on selection or an uncanceled drop, never on focus or hover. Ignore a canceled drag. A shortcut never steals keys from a focused control.

## Models

- A port stays structurally traceable to a commit-pinned upstream implementation. Preserve checkpoint-affecting names, construction order, parameter paths, tensor layouts, execution order, and postprocessing semantics. Explain each intentional divergence next to the affected code.
- Missing or unexpected weights mean an architecture or parameter-name mismatch. Fix that before changing the loader.
- Every model has the same public lifecycle, with model-specific inputs and outputs. Network and weight loading stay apart from preprocessing, postprocessing, slicing, and result types. Do not add pass-through types or layers.
- Accept a device at the model boundary and convert it once. Create token IDs and index tensors on the model device; a CPU tensor used against accelerator embeddings is a defect.
- Use the established loading paths unless they are proven insufficient.
- Disable gradient tracking during inference.

## Performance

- Optimize and benchmark on the actual target device with representative inputs. Load assets and warm models outside the measured region, and account for asynchronous accelerator execution.
- Remove redundant transfers, synchronization, allocations, and per-pixel host loops before adding concurrency or caching.
- Report the device, input size, baseline, result, and correctness difference.

## Verification

- Run the smallest relevant check or focused test once, in the debug profile. Run full suites, other profiles, or end-to-end tests only when asked.
- Add a test only where it protects a real regression boundary. Do not lock a translated phrase, a language display name, a registration string, or a one-line behavior the owner can read. Do not add a second harness for behavior existing tests cover, a test of a compatibility shim, or a benchmark nobody asked for.
- A model port needs a structured comparison with upstream on identical inputs: shapes, ranges, boxes, scores, masks, and ordering. A green build is not that comparison.
- A UI change needs a screenshot of the affected state. For truncation, include a long label.
- A green check is not acceptance.

## Testing

- Automate the UI through a Chrome DevTools Protocol MCP server connected to `http://127.0.0.1:4000`, the debug build's remote debugging endpoint.
- Capture the canvas with the harness's built-in screenshot tool.
- Do not use computer use on Windows.

## Contributing

- A new product feature, significant UI change, model or provider integration, platform integration, public API, schema change, or architectural refactor needs an issue discussion first. Small fixes, documentation, translations, and tests can go straight to a pull request.
- A human reviews the whole diff and can explain every change. Do not submit an unreviewed autonomous patch.
- Keep pull-request text short: problem, change, and verification.
- Never add an AI assistant as a commit co-author, for example through a `Co-Authored-By` trailer.
