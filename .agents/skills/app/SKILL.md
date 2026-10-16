---
name: app
description: Koharu desktop entry, typed Tauri commands, and generated protocol. Use when changing startup, the React shell, bridge bindings, or command channels.
---

# App

Coding rules and the skill index are in `AGENTS.md`.

Koharu is a desktop application. React invokes typed Tauri commands. Native crates own durable project state and processing.

```text
packages/koharu
        │ typed commands and channels
        ▼
packages/bridge
        │
        ▼
crates/koharu
        │
        ▼
koharu-app
        ├── scene, pipeline, translator, agent, desktop, psd
        └── see the matching skill for each feature
```

- `packages/koharu` owns the project browser, page rail, canvas controls, inspector, settings, activity, Agent UI, and transient interaction state.
- `packages/ui` is shared React primitives. It does not own product state.
- `crates/koharu` owns startup, diagnostics, Tauri configuration, and build integration.
- `koharu-app` owns Tauri state, project lifecycle, command serialization, jobs, synchronization, and Agent hosting. It owns `.khrproj` directories, names, active-page selection, undo grouping, and UI projection. It is not a second scene model or renderer.
- The frontend does not use an HTTP client or a generic application event envelope. Project, canvas, job, download, and resource updates use independent typed channels. Agent sign-in and runs stream through per-call channels. Preferences are fetched and saved through commands.
- `packages/docs` is the documentation site. English is served from the site root. Do not add a second documentation tree.

Rust command signatures and Specta types are authoritative.

```bash
cargo run -p koharu-app --bin generate
```

Do not hand-edit `packages/bridge/src/protocol.ts`. Change `koharu-canvas` or `koharu-rasterizer` source rather than editing derived WASM by hand. `koharu-agent` hosts the agent through the existing project store. It does not become a second one.

Toolchain versions are not pinned. Read the development setup doc for the current Rust, Bun, and LLVM expectation. `bun dev` starts the UI and the desktop app.
