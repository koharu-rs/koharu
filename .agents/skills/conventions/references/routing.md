# Entry routing and component boundaries

Koharu is a desktop application. React invokes typed Tauri commands. Native crates own durable project state and processing.

Toolchain versions are not pinned. Read `packages/docs/development/setup.mdx` for the current Rust, Bun, and LLVM expectation.

## How a change enters the app

```text
packages/koharu (React / Next.js)
        │ typed commands and channels
        ▼
packages/bridge (generated protocol, canvas adapter, WASM)
        │
        ▼
crates/koharu (startup, diagnostics, Tauri config, build)
        │
        ▼
koharu-app (state, lifecycle, jobs, sync, Agent host)
        │
        ├── koharu-scene ── koharu-storage
        ├── koharu-pipeline ── koharu-ml ── koharu-runtime ── native packages
        ├── koharu-translator
        ├── koharu-agent
        ├── koharu-desktop ── koharu-renderer ── koharu-rasterizer
        └── koharu-psd ── koharu-renderer / koharu-rasterizer

packages/bridge ── koharu-canvas (WASM) ── koharu-rasterizer
```

- `bun install` then `bun dev` starts the Next.js UI and the Tauri app. Debug builds expose CEF remote debugging at `http://127.0.0.1:4000`.
- `packages/koharu` owns the project browser, page rail, canvas controls, inspector, settings, activity, Agent UI, and transient interaction state.
- `packages/ui` is shared React primitives and styling. It does not own product state.
- `packages/docs` is the Mintlify site. English is served from the site root. Do not add a second documentation tree.
- The frontend does not use an HTTP client or a generic application event envelope. Project, canvas, job, download, and resource updates use independent typed channels. Agent sign-in and runs stream through per-call channels. Preferences are fetched and saved through commands.

## Generated boundaries

Rust command signatures and Specta types are authoritative.

```bash
cargo run -p koharu-app --bin generate
```

Do not hand-edit `packages/bridge/src/protocol.ts`. `bun dev` builds `koharu-canvas` for WASM into `packages/bridge/src/wasm` and watches `koharu-canvas` and `koharu-rasterizer`. Change the Rust or WASM source, then regenerate. Do not commit generated output that the generator can reproduce, except the bindings the repository already tracks as the generated protocol.

## Who owns what

Keep each responsibility in the crate that already owns it. A feature extends that owner. It does not add a parallel type, command, or config path.

| Owner | Owns | Must not |
| --- | --- | --- |
| `koharu-scene` | Project meaning: hierarchy, components, relations, patches, revisions, session undo | Persist files, draw pixels, schedule models |
| `koharu-storage` | Opaque complete scene payloads and immutable blobs | Interpret scene types or UI selection |
| `koharu-app` | `.khrproj` directories, names, active page, undo grouping, UI projection, command serialization | Become a second scene model or renderer |
| `koharu-pipeline` | Page-stage scheduling, model lifetime, progress, cancellation, incremental commits | Write storage files or own translation providers |
| `koharu-ml` | Models and the shared device abstraction | Own provider HTTP defaults |
| `koharu-translator` | Local and hosted translation connections | Grow a second prompt path beside `TranslationConfig` |
| `koharu-renderer` | Scene snapshot to an immutable frame and a portable rasterizer bundle | Accept a second document model from callers |
| `koharu-rasterizer` | Portable prepared frame consumed by browser transport and native readback | Own project edits |
| `koharu-canvas` | Retained browser resources and WebGPU presentation | Commit durable edits by itself |
| `koharu-desktop` | Latest-wins preparation and exact-generation manifests | Own the visible window surface; the webview does |
| `koharu-runtime` | Discover, download, validate, and load native packages | Probe hardware by loading the managed package early |
| `koharu-torch`, `koharu-llama`, `koharu-diffusion` | Safe wrappers | Contain unsafe FFI or dynamic loading |
| `*-sys` crates | Unsafe FFI, dynamic loading, and build integration | Leak into public model APIs |
| `koharu-config` | Typed sections of `config.toml` | Store secrets |
| `koharu-secrets` | OS credential store | Read a project `.env` as the desktop key store |
| `koharu-agent`, `koharu-psd` | Agent hosting and PSD composition through the existing renderer | Become a second project store |

## Scene, pipeline, and presentation

`koharu-scene` separates three layers:

- Analysis: region, geometry, OCR.
- Content: text content, source text, translation.
- Presentation: text layout, typography, optional authored geometry.

Detection geometry stays analysis. It does not become a movable visible layer. Translation can change without dropping OCR provenance. Typography can change without rewriting semantic text.

A patch binds to a project and a base revision, records preconditions, and carries inverses for session undo. A stale patch is rejected. Derived work rebases explicitly and fails if anything it observed has changed.

Storage writes an opaque payload into alternating `state-a.khr` and `state-b.khr` slots, with immutable content-addressed blobs. Publish missing blobs first, write the inactive slot, then publish atomically. Opening selects the newest valid slot and can fall back if the newer one is corrupt. Renderer, pipeline, and Agent consume snapshots and submit semantic patches. They do not write storage files.

`koharu-pipeline` schedules one stage on one page:

```text
detection → OCR → translation
detection → inpainting
```

Pages enter in project order. Completed detection makes that page's later stages eligible immediately. At most one job per stage runs at a time. The number of pages in flight is bounded by the number of selected stages. Stages return semantic patches. The application commits each result, synchronizes the canvas, and records each commit as its own undo step.

Accelerator inference uses one admission lane, including remote translation providers. CPU-only work may keep per-model lanes. On out-of-memory, release the lane, unload the other stages' models, wait for a fresh resource sample, and retry once. Any other failure unloads the failing stage's model. A settings change rebuilds the stage runner. Stopping is cooperative: do not start new stages, and discard an uncommitted native result.

Translation is the only visible text source. WebGPU presentation, PNG, layer crops, and PSD consume the same prepared frame. Incremental reuse must match a full render. The previous page stays visible until the staged generation is ready. An obsolete preparation cannot replace a newer generation.

Camera motion, transform previews, strokes, and sampling stay in the browser. Completed edits cross Tauri commands.
