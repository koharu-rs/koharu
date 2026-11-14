# koharu-psd

A page as a layered Photoshop document.

It writes one page of a project as a PSD that opens in Photoshop with every
layer editable. The renderer draws the pixels and lays out the text, so the
file shows what the editor shows. Where it follows an established format or
product, it cites it; the links are under [Sources](#sources).

## Goal

A letterer can take a page into Photoshop and keep working on it: every layer
the project has, hidden ones hidden, and every text still text.

## 1. Layers

PSD has no panels, balloons, or blocks, so each becomes a folder of ordinary
layers [PSD spec], and nothing a block holds is lost: its clean-up, its mask,
its source, and its translation each stay a layer.

| Scene layer | PSD layer |
|---|---|
| background | the bottom pixel layer, hidden when the background is |
| group | a folder, with its children, its blend mode, and its masks |
| panel | a folder whose vector mask is its outline |
| balloon | a folder holding a hidden white shape layer of its outline, then its texts |
| block | a folder holding its clean-up as a pixel layer, whose layer mask is the block's mask while that's shown, its source as a hidden type layer, and its translation as a type layer |
| image, raster | a pixel layer, cropped to what it draws |
| shape | a shape layer: its path as a vector mask, its fill, and its stroke |
| text | a type layer, with its stroke as a layer effect |
| mask | no layer of its own: it's its parent's mask |

- **Every layer** keeps its name, visibility, opacity, and blend mode, and a
  locked layer is locked. Pass-through means something only for a folder, so
  any other layer writes it as normal.
- **Pixels are a layer's own**, without its opacity, blend mode, or masks,
  which the file records as properties. Under a stroke effect they're the
  glyphs alone, as Photoshop draws the effect around them.
- **Masks.** A PSD layer has one layer mask [PSD spec], so a layer's shown masks
  merge into one, cropped to what it covers.
- **Names.** A type layer is named by its text, as Photoshop names one. The
  layers the scene has no name for, such as the background and a block's
  clean-up, are named in the app's language.

## 2. The file

| Part | Model | Cite |
|---|---|---|
| size | at most 30,000 pixels on a side; a larger page fails, since it would need PSB | a PSD's width and height range from 1 to 30,000 [PSD spec] |
| merged image | the renderer's composite of the page, at 72 pixels an inch | |
| type layers | the text engine's data, in the keys, order, and resource sets ag-psd writes; Photoshop fills in what isn't written from its own defaults | ag-psd writes text layers [ag-psd] |
| blocks | laid out as the specification says, and as ag-psd does where it's silent; a block's length counts its padding, because readers such as psd-tools skip exactly that many bytes | [PSD spec], [ag-psd], [psd-tools] |

## Sources

Read in November 2026.

- Adobe: [PSD spec]
- Readers and writers: [ag-psd], [psd-tools]

[PSD spec]: https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
[ag-psd]: https://github.com/Agamnentzar/ag-psd
[psd-tools]: https://github.com/psd-tools/psd-tools
