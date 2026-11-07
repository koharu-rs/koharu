# koharu-psd

Writes a page of a `koharu-scene` project as a layered PSD. `koharu-renderer` draws the
pixels and lays out the text, and photocraft-psd writes the file. Every layer in the scene is
written, hidden ones hidden, and nothing is added.

## API

`export(&session, &renderer, page)` returns the file's bytes. It renders the page once, so
call it off the UI thread. A page over 30,000 pixels on a side fails with `Error::TooLarge`,
since a larger document would need PSB.

The merged image is the renderer's composite, at 72 pixels an inch. The `export` example writes
a page and prints its layers, bottom to top.

## Layers

| Scene layer | PSD layer |
|---|---|
| Group | A folder, with its children, its blend mode, and its masks |
| Image, raster | A pixel layer, cropped to what it draws |
| Shape | A pixel layer marked as derived from its path, with a `vmsk` vector mask, and `SoCo` for its fill, or `vscg` and `vstk` when it has a stroke |
| Text | A pixel layer with a `TySh` type layer, and `lfx2` for a stroke effect. A text that draws nothing stays an empty pixel layer |
| Mask | No layer of its own: it becomes its parent's mask |

Every layer also keeps its name, visibility, opacity, and blend mode. A locked layer gets
`lspf`'s lock-all flag. Pass-through only means something for a group, so any other layer
writes it as normal.

A layer's pixels are its own content without its opacity, blend mode, or masks, which the PSD
records as properties. Under a stroke effect they are the glyphs alone, as Photoshop draws the
effect around them.

## Masks

A layer's shown masks become one vector mask when every one is a path and the layer has no
vector mask of its own. Otherwise they merge into one user mask the renderer draws, cropped to
what it covers.

## Type layers

A type layer's EngineData is the PDF-like dictionary Photoshop's text engine reads. Its keys,
their order, and its resource sets follow ag-psd's encoder, and Photoshop fills in what isn't
written from its own defaults. Strings are UTF-16 big-endian with a byte-order mark, and runs
count UTF-16 units.

Block layouts follow Adobe's Photoshop File Formats Specification, and ag-psd where it is
silent. A block's length counts its padding, because readers such as psd-tools skip exactly
that many bytes.
