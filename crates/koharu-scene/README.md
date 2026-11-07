# koharu-scene

Koharu's scene graph on Loro. A project is pages, each a tree of layers modeled on what
established editors such as Photoshop and Clip Studio Paint already have. A layer records what
a model wrote, so rerunning a step never undoes what the user changed.

A `Session` alone owns an open project's document. Reads return owned values, and every change
is an `Edit`. The UI, models, MCP clients, and the in-app agent all use the same `Op` and `Query`
types.

## API

| Item | What it does |
|---|---|
| `Session::create(path)`, `open(path)` | A project, through `koharu-storage`'s lock and file |
| `Session::apply(edit)` | Checks the edit in full, then writes it as one undo step, and returns a `Change` |
| `Session::undo()`, `redo()`, `history()` | Undo and redo, with each step's label |
| `Session::save()` | Saves with history. Containers never read are exported from their loaded bytes, so no pixels decode |
| `Session::compact()` | Saves without history, dropping files and tiles nothing names. It decodes everything, so it suits rare moments such as closing |
| `Session::query(query)` | Answers a `Query`: `project()`, `page(id)`, `layer(id)`, and `style(id)` read the same way |
| `Session::read(layer, path)` | The value at a path, and a `Read` of it |
| `Session::tiles(layer, rect)`, `tiles_hash(layer)` | A layer's decoded pixels, or a hash of them taken without decoding |
| `Session::file(hash)` | An imported file's bytes |
| `tools()`, `tool_call(name, arguments)`, `compact(&answer)` | The agent and MCP tools |

A session saves only when its caller says so: when edits pause, and when the project closes.

## Layers

| Kind | What it is |
|---|---|
| `group` | A folder of layers, drawn first to last |
| `image` | An imported file, kept byte for byte |
| `raster` | Editable RGBA pixels |
| `shape` | A vector path in page coordinates, with an optional fill and stroke |
| `text` | Styled text in a point, box, or path frame |
| `mask` | Limits its parent's visibility, from pixels or a path. It's never drawn, and masks apply in order |

Layers share a name, visibility, a lock, opacity, a blend mode, and `attributes`, though a mask
takes only the name, visibility, and attributes. Attributes are tools' own data, by namespace then
key, and the renderer never reads them. A text style is kept by the project, and a text takes its
unset values from the style it names.

## Edits

An `Edit` names its project, an undo label, its author, what it read, and its ops. Loro has no
rollback, so an edit is checked in full and planned into writes before any is made. An edit
that changes nothing adds no undo step. If a write still fails partway, the session is poisoned
and refuses further edits and saves until the project is reopened.

| Ops | What they do |
|---|---|
| `add_page`, `move_page`, `remove_page` | Pages |
| `add_layer`, `duplicate_layer`, `move_layer`, `remove_layer` | The layer tree. A duplicate keeps what models wrote and what the user overrode |
| `update_layer` | Properties, with a JSON Merge Patch |
| `set_text`, `edit_text` | A text's content, as plain text diffed against the old, or a Loro delta from the rich editor |
| `write_pixels` | RGBA8 for a raster, or grey8 for a pixel mask |
| `reset` | Deletes overrides, so the model's values show again |
| `add_file` | An imported file, stored once by its content hash |
| `add_text_style`, `update_text_style`, `reset_text_style`, `remove_text_style` | Text styles. A style a text names can't be removed |

A `Ref` names an existing page or layer, or `$label` for one created earlier in the same edit.

`Error` covers what a check can refuse: `NotFound`, `Invalid`, `Locked`, `Stale`, and
`WrongProject`. A project also fails to open with `Version` or `Corrupt`. An error's code is its
variant's name in snake case.

## Provenance

A model's edit carries a C2PA-style `Action`: the software that ran, and what it read as its
ingredients. Each layer keeps the actions that wrote it, and each path belongs to the latest
action that wrote it.

- **Overrides.** When the user changes a path a model wrote, the user's value shows and the
  model's is kept. A later model run updates the kept value and leaves the user's in place, until
  `reset` removes the override. Paths are recorded where the values first differ in shape, so
  resetting one always restores a valid value.
- **Reruns.** A rerun of a model takes over every path that model wrote on the layer, and another
  model takes over only the paths it writes.
- **Removal.** A model removes only what it created and the user left alone. `Change::kept` lists
  the layers it skipped.
- **Staleness.** A `Read` holds a value's hash, BLAKE3 over its RFC 8785 canonical JSON. An edit
  whose read no longer matches fails with `Stale` and writes nothing. `LayerView::stale` lists the
  paths a model wrote whose ingredients have changed since.

## Tools

`tools()` generates each tool from `Op` and `Query`: its name is the serde tag, its description
the variant's doc comment, and its input schema the variant's part of the schemars schema. No tool
has code of its own, and what tools never send, such as bytes and editor deltas, is skipped by
schemars attributes. `tool_call` parses a call into an edit or a query, and the host adds the
author, its reads, and a label. `compact` gives an answer as agents see it, without hashes.

## Storage

The document's layout is `project`, `files` by content hash, `styles` by id, and `pages`, a movable
list where each page holds its size, a `layers` tree, and its layers' `tiles`. Large data never
sits in a tree node's map, because Loro keeps a deleted node's map through compaction.

Pixels are 64×64 tiles, each compressed with zlib on its own as GIMP and Krita store them, and only
tiles holding pixels are stored. Tiles decode and encode in parallel. `Revision` is the document's
opaque Loro frontiers: equal revisions are the same document, and revisions have no order.

The schema version is `1.0.0`. A build opens only its own major version, up to its own version.
