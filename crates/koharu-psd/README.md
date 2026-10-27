# koharu-psd

`koharu-psd` writes a page of a [`koharu-scene`](../koharu-scene/README.md)
project as a layered PSD. [`koharu-renderer`](../koharu-renderer/README.md)
draws the pixels and lays out the text; [photocraft-psd] writes the file. The
scene tree decides what the file holds: every layer is written, hidden ones
hidden, and nothing is added.

```rust,ignore
let bytes = koharu_psd::export_page(&session, &renderer, page)?;
```

`export_page` renders the page once and draws every layer, so the caller runs
it off the UI thread.

## Mapping

| Scene | PSD | Source |
|---|---|---|
| page | an 8-bit RGB document at the page's size and 72 dpi, its merged image the renderer's composite; a page wider or taller than 30,000 pixels fails, as PSD holds no more | [PSD spec] |
| group | a folder record over its children and a closing divider (`lsct`) | [PSD spec] |
| raster, image | a pixel layer: what the renderer draws for it, at its place on the page | [PSD spec] |
| shape | a shape layer: its path as the vector mask (`vmsk`), its fill as a solid colour (`SoCo`), its stroke as vector stroke data (`vstk`, with the fill in `vscg`), and the renderer's pixels | [PSD spec]; [ag-psd blocks] |
| text | a type layer (`TySh`): a transform, a text descriptor holding the EngineData, and the renderer's pixels, without the stroke when it is written as an effect | [PSD spec]; [ag-psd type] |
| masks, every one a path | one vector mask, a subpath per component in order: add combines, subtract subtracts, intersect intersects, and it starts filled unless the first one adds | [ag-psd vector mask]; [photocraft-psd paths] |
| masks, any of pixels or an empty path | the renderer's merged coverage as the layer's user mask, cropped to what it covers; an empty path has no subpath to carry its operation | scene README "PSD export" |
| hidden mask | left out, as the renderer leaves it out | renderer README |
| name, visible, opacity, blend | both name fields, the hidden flag, the opacity byte, and the blend key | [PSD spec] |
| locked | `lspf` with "lock all" | [ag-psd lock] |
| attributes, origin, overrides, language, text styles | not written; a text's own and its style's values are written as it was drawn | no PSD equivalent |

Every block the crate builds counts its own padding in its length, as the
spec rounds a block's length up to an even byte count; photocraft-psd would
write the pad byte after an odd length, which psd-tools then misreads.

### Shapes

- A path fills by its own rule: bytes 4–5 of a subpath's length record hold 1
  for even-odd and 2 for non-zero, as ag-psd writes them.
- Without a fill or a stroke, a shape draws nothing, and is written as an empty
  pixel layer.
- A shape's layer sets the "pixel data irrelevant" flag, as Photoshop marks a
  shape's pixels as derived from its path; psd-tools takes the flag for a
  shape layer [psd-tools kinds].
- A stroke's alpha becomes its opacity; a fill has no opacity of its own.
- A shape's masks go into its user mask, since its vector mask is its path.

### Text

- Each run the renderer drew becomes a style run with the PostScript name of
  its face, the size it was drawn at after fitting, and its fill. A run reaches
  the next one's start, so white space the renderer didn't draw keeps the style
  before it.
- Line breaks become Photoshop's paragraph marks (`\r`), a `\r\n` pair one
  mark, as the renderer takes it for one break.
- A text that draws nothing has no runs to style, and is written as an empty
  pixel layer.
- Alignment becomes Photoshop's justification: start left, centre centre, end
  right, and justify with the last line left; in right-to-left text, as the
  renderer sets it, start is right and justify ends right. The line height
  becomes auto leading.
- Point text is point type, anchored on its first line's baseline (in vertical
  text, its first column's centre line) at the start, middle, or end of its
  lines by its alignment.
- Box text is paragraph type in its box, a grown box at its grown size.
- Path text is paragraph type in the path's bounds; Photoshop's own type in a
  path lives in an undocumented block.
- A stroke shared by every run is written as an Outside stroke effect
  (`lfx2`), Photoshop's stroke outside the glyphs that the renderer matches
  [ag-psd effects], at the stroke's opacity and its width on the page. The
  layer's pixels are then the glyphs alone, which Photoshop draws the effect
  around.
- The four values after the warp are zeros, as Photoshop writes them for type
  without a warp [ag-psd test files].
- Vertical type is written as ag-psd writes it, which ag-psd warns can make a
  file Photoshop crashes on [ag-psd limitations]. Nothing here opens one in
  Photoshop, so check one before relying on it.

### Where the PSD isn't exact

A layer's pixels are what the renderer drew, except a shape's, which
Photoshop draws again from its path, fill, and stroke. What Photoshop draws or
edits can differ:

- A shape's fill is opaque, as `SoCo` holds no opacity.
- Path text wraps in the path's bounds rather than its shape.
- Photoshop sets paragraph type from the top of its box, so text set in the
  middle or at the bottom of its box or path moves up once Photoshop lays it
  out again.
- Photoshop lays out type with its own engine, so lines may break elsewhere
  once it does.
- A synthetic bold or italic face is written as the face it came from.
- Strokes that differ between runs stay in the pixels only, and are lost once
  Photoshop lays the type out again.
- Right-to-left text keeps its alignment, but no paragraph direction is
  written.
- A vector mask on a pixel layer has no user mask beside it, as ag-psd writes
  one; psd-tools only applies such a mask alongside a user mask, so it shows
  those layers unmasked [psd-tools masks].

## Checks

```text
cargo test -p koharu-psd
cargo run -p koharu-psd --example export -- page.psd
```

The test reads the file back with photocraft-psd. Opening the example's file in
Photoshop and GIMP is the check that counts; they can't be driven from here.

## Sources

[PSD spec]: https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
[photocraft-psd]: https://github.com/storytold/photocraft/tree/59cb4c49bd236df04d8e37d4764253f29ca6600a/crates/psd
[photocraft-psd paths]: https://github.com/storytold/photocraft/blob/59cb4c49bd236df04d8e37d4764253f29ca6600a/crates/psd/src/path.rs
[ag-psd blocks]: https://github.com/Agamnentzar/ag-psd/blob/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/src/additionalInfo.ts#L152-L208
[ag-psd type]: https://github.com/Agamnentzar/ag-psd/blob/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/src/additionalInfo.ts#L54-L148
[ag-psd vector mask]: https://github.com/Agamnentzar/ag-psd/blob/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/src/additionalInfo.ts#L288-L330
[ag-psd effects]: https://github.com/Agamnentzar/ag-psd/blob/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/src/descriptor.ts#L1339-L1388
[ag-psd lock]: https://github.com/Agamnentzar/ag-psd/blob/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/src/additionalInfo.ts#L760-L768
[ag-psd test files]: https://github.com/Agamnentzar/ag-psd/tree/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/test/read
[ag-psd limitations]: https://github.com/Agamnentzar/ag-psd/blob/c82ba4e44d00d9ad48236ec183c7e3a1a5c7fd04/README.md#text-layer-issues
[psd-tools kinds]: https://github.com/psd-tools/psd-tools/blob/0385aac27d6e28921516c7a045e2fe460d15f0ad/src/psd_tools/api/psd_image.py#L998-L1007
[psd-tools masks]: https://github.com/psd-tools/psd-tools/blob/0385aac27d6e28921516c7a045e2fe460d15f0ad/src/psd_tools/composite/composite.py

- Adobe: [PSD spec]
- photocraft: [photocraft-psd], [photocraft-psd paths]
- ag-psd: [ag-psd blocks], [ag-psd type], [ag-psd vector mask], [ag-psd
  effects], [ag-psd lock], [ag-psd test files], [ag-psd limitations]
- psd-tools: [psd-tools kinds], [psd-tools masks]
