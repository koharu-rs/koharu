---
name: scene
description: Koharu scene meaning, patches, and storage publication. Use when changing project structure, undo, revisions, or .khrproj persistence.
---

# Scene

Coding rules and the skill index are in `AGENTS.md`.

`koharu-scene` owns what a project means. `koharu-storage` owns how complete states and immutable bytes become durable.

## Separate analysis, content, and presentation

- Analysis: region, geometry, OCR.
- Content: text content, source text, translation.
- Presentation: text layout, typography, optional authored geometry.

Detection geometry stays analysis. It does not become a movable visible layer. Translation can change without dropping OCR provenance. Typography can change without rewriting semantic text.

## Apply an edit

Snapshots are immutable. A patch binds to a project and a base revision, records operation preconditions, and carries inverses for session undo.

A stale patch is not accepted silently. Derived work rebases explicitly and fails when anything the patch observed has changed since its base revision.

## Publish a state

Storage is domain-agnostic. It saves an opaque complete scene payload into alternating state slots, with immutable content-addressed blobs, checksums, and a referenced-blob set.

1. Publish missing blobs before the state that needs them.
2. Write the inactive slot beside its destination and flush it.
3. Publish atomically, and keep the previously valid slot if publication fails.

Opening selects the newest valid state and can fall back to the other slot if the newer one is corrupt. Blob reads may use read-only memory mapping without exposing that detail to scene consumers.

Renderer, pipeline, and Agent consume snapshots and submit semantic patches. They do not write storage files.
