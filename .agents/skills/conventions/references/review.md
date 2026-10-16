# Tests, review, and acceptance

## What to run

Run the smallest relevant debug check once. Do not run the full suite, a non-debug profile, or an end-to-end test unless the user explicitly asks. State any check that could not run.

```bash
cargo check -p <crate>
cargo test -p <crate> <test_name>
cargo fmt --all --check
git diff --check
```

Frontend work uses that package's focused test or typecheck. `bun run check` is the repository format check. `bun run lint` is frontend lint.

Invalid model JSON fails closed. Do not add a silent repair layer that can change meaning. Enforce the translation output schema through each provider's native structured-output API. A bounded retry belongs only to a provider that already has one. Hosted machine-translation providers fail immediately.

## Evidence

- A UI change needs a screenshot of the affected state. If the bug is truncation, include a long-label case.
- Performance work reports device, input size, baseline, result, and correctness difference. Remove redundant transfers, synchronization, allocations, and per-pixel host loops before adding concurrency or caching. Load assets and warm models outside the measured region. Account for asynchronous accelerator execution.
- A model port needs a structured comparison on identical inputs. A green build is not that comparison.

## Test boundaries

- Add a test when it protects a real regression boundary, such as global style versus selected-node style, or a generation mask that changes model output.
- Do not add a test that only locks a translated phrase, a language display name, a registration string, or a one-line behavior the owner can read directly. Remove that test when it does not earn its keep.
- Do not add a second harness for behavior the crate's existing tests already cover.
- Do not assert a compatibility shim. There is no old behavior to freeze.
- Focused crate tests are the default. A passing CI matrix does not replace the focused check, and the focused check does not replace review.

## What review accepts

A change can land when it fits the current owner, stays small, preserves behavior that was not part of the request, and can be maintained after the contributor leaves.

Review asks for changes when a patch:

- adds a parallel command, config, job kind, or prompt path
- is based on a deleted tree and needs a rebase onto the current owner
- hand-edits generated protocol or another generated binding
- keeps a compatibility alias or a removed subsystem
- duplicates status, controls, or APIs
- hardcodes user-visible copy that belongs in locales
- adds an unnecessary test
- adds a provider login or account flow the product did not ask for
- changes unrelated files

Small bug fixes, documentation, translations, and tests can be opened directly. A new product feature, significant UI change, model or provider integration, platform integration, dependency, public API, schema change, or architectural refactor needs an issue discussion first. An open issue or a working implementation is not acceptance.

## How a submission is judged

Maintainers weigh product fit, breadth of user value, UX, architecture, performance, security, privacy, platform support, tests, documentation, dependencies, and operational complexity.

A proposal may be declined, postponed, reduced, redesigned, or kept outside the core repository. After merge, the project must be able to maintain the feature without the original contributor.

Keep the pull-request and issue text short: problem, change, and verification. Do not paste an article-length model writeup. A human contributor reviews the whole diff, checks generated claims against the code, and can explain every change. Do not submit an unreviewed autonomous patch.

A green check is not acceptance.
