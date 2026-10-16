---
name: rendering
description: Koharu prepared frames, raster transport, and WebGPU presentation. Use when changing the renderer, rasterizer, canvas, PSD export, or desktop preview.
---

# Rendering

Coding rules and the skill index are in `AGENTS.md`.

`koharu-renderer` turns a scene snapshot and page ID into an immutable frame: ordered layers, entity lookup, presentation metadata, dependencies, diagnostics, and vector scenes. It prepares a portable `koharu-rasterizer` bundle for browser display and native readback.

Translation is the only visible text source. Typography, fit relations, fonts, images, visibility, and opacity come from the scene. Callers do not supply a second document model.

WebGPU presentation, PNG, layer crops, and PSD consume the same prepared frame. Incremental reuse must produce the same result as a full render.

The Tauri webview owns the visible surface. React draws the interface. `koharu-canvas` compiles to WASM and renders to a canvas with WebGPU. Camera motion, transform previews, strokes, and sampling stay transient in the browser. Completed edits cross Tauri commands.

`koharu-desktop` coordinates latest-wins preparation and exact-generation manifests and resources. A page change announces a generation before its lightweight manifest is requested. The canvas reports missing content IDs and receives resource packets asynchronously. The previous page remains visible until the staged generation is ready. Obsolete preparation cannot replace a newer generation.

Raster resources use tiled logical tiles with a one-pixel interior sampling gutter. That bounds WASM copies and GPU textures while preserving filtered edges. Coordinated CPU and GPU caches avoid resending unchanged resources when revisiting pages.

`koharu-psd` composes through the same renderer and rasterizer. It does not become a second project store.
