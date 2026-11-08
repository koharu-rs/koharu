# koharu-psd

Writes a page of a `koharu-scene` project as a layered PSD. `koharu-renderer` draws the
pixels and lays out the text, and photocraft-psd writes the file. Every layer in the scene is
written, hidden ones hidden. PSD has no panels, balloons, or blocks, so each becomes a folder of
ordinary layers.

## API

`export(&session, &renderer, page, &labels)` returns the file's bytes. `Labels` names, in the
app's language, the layers the scene has no name for: the background, a block's clean-up, and a
balloon's outline. A type layer takes its text as its name, as Photoshop names it. It renders
the page once, so call it off the UI thread. A page over 30,000 pixels on a side fails with
`Error::TooLarge`, since a larger document would need PSB.

The merged image is the renderer's composite, at 72 pixels an inch. The `export` example writes
a page and prints its layers, bottom to top.

## Layers

| Scene layer | PSD layer |
|---|---|
| Page background | The bottom pixel layer, hidden when the background is |
| Group | A folder, with its children, its blend mode, and its masks |
| Panel | A folder whose vector mask is its outline |
| Balloon | A folder holding a hidden white shape layer of its outline, then its texts |
| Block | A folder holding its clean-up as a pixel layer with the clean-up's own name, visibility, opacity, and blend, whose user mask is the block's mask while it's visible, its source as a hidden type layer, and its translation as a type layer |
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

A layer's shown masks merge into one user mask the renderer draws, cropped to what it covers,
since a PSD layer has one layer mask.

## Type layers

A type layer's EngineData is the PDF-like dictionary Photoshop's text engine reads. Its keys,
their order, and its resource sets follow ag-psd's encoder, and Photoshop fills in what isn't
written from its own defaults. Strings are UTF-16 big-endian with a byte-order mark, and runs
count UTF-16 units.

Block layouts follow Adobe's Photoshop File Formats Specification, and ag-psd where it is
silent. A block's length counts its padding, because readers such as psd-tools skip exactly
that many bytes.
