# koharu-scene

Koharu's scene graph on Loro.

Every element of this schema copies something an established editor or format
already has, and cites it. Nothing is new. Where sources differ, Photoshop and
Clip Studio Paint win, as for all editor behaviour in Koharu. The links are
listed under [Sources](#sources).

## Goal

A schema that lasts through workflow changes. The workflow (detect, OCR,
translate, inpaint, typeset) will change; the schema should not. So the schema
holds only what image editors have kept for decades, and everything
workflow-specific is data inside it.

The schema has three parts:

1. **Layers**: pages, each holding a tree of groups, images, rasters, shapes,
   text, and masks.
2. **Generated values and overrides**: a layer records what a model wrote and
   read, so a rerun never undoes what the user changed.
3. **Attributes**: namespaced data for tools, which the renderer never reads.

## 1. Layers

### Project and pages

| Element | Model | Evidence |
|---|---|---|
| project | ordered pages, imported files, and a `version` | a Clip Studio Paint EX work is one management file over its pages [CSP pages]; a Figma file keeps each image once, fetched by hash with `getImageByHash` [Figma image paint]; an OpenRaster image must carry a `version` [OpenRaster] |
| page | a stored id, a `size`, its layers, and their pixels | each Clip Studio Paint page has its own canvas size [CSP page size]; a PSD document has a width and height [PSD spec]; each Photoshop artboard has its own size [Photoshop artboards]; a Figma page is a node with an `id` [Figma nodes] |

### Every layer

| Property | Model | Evidence |
|---|---|---|
| identity | a stored id that never changes, through undo and redo | PSD `lyid` [PSD spec]; Figma node `id` [Figma nodes]; Krita layer `uuid` [Krita file] |
| `name` | string | PSD layer name [PSD spec]; Figma `name` [Figma nodes]; OpenRaster `name` [OpenRaster] |
| `visible` | bool; hiding a group hides everything in it | PSD visible flag [PSD spec]; Figma `visible` [Figma nodes]; SVG `display: none` hides descendants [SVG rendering] |
| `locked` | bool | Figma `locked` [Figma node properties]; PSD lock settings [PSD spec] |
| `opacity` | 0 to 1 | Figma `opacity` [Figma node properties]; OpenRaster `opacity` [OpenRaster] |
| `blend` | normal, multiply, screen, and the other standard modes; pass-through for groups | PSD blend keys [PSD spec]; Figma `BlendMode`, including `PASS_THROUGH` [Figma property types]; OpenRaster `composite-op` [OpenRaster] |
| `attributes` | see [section 3](#3-attributes) | |

Children are drawn first to last, so the last child is on top, as SVG paints in
document order [SVG rendering].

### Layer kinds

| Kind | Holds | Evidence |
|---|---|---|
| `group` | child layers | PSD layer groups (`lsct`) [PSD spec]; SVG `g` [SVG structure]; OpenRaster nested `stack` [OpenRaster]; Krita group layers [Krita layers] |
| `image` | the content hash of an imported file, which the project keeps byte for byte with its media type | Photoshop exports an embedded smart object "in its original format" [Photoshop smart objects], and a PSD placed layer names its embedded file by unique ID [PSD spec]; a Figma image paint names its image by "the hash (id) of the image" [Figma image paint]; SVG `image` [SVG image] |
| `raster` | editable pixels | PSD pixel layers [PSD spec]; GIMP layers [GIMP XCF]; Krita paint layers [Krita layers] |
| `shape` | a path of open or closed subpaths, a fill rule, an optional fill, and an optional stroke | PSD shape layers: vector path, fill (`SoCo`), stroke (`vstk`), and fill rule [PSD spec]; SVG `path` [SVG paths]; Figma `VECTOR` [Figma nodes] |
| `text` | styled text in a frame | [below](#text) |
| `mask` | a mask component | [below](#masks) |

Each kind is placed the way its PSD record places it [PSD spec]:

- Text and image layers carry a `transform`, a 2D affine
  `[[a, b, c], [d, e, f]]`, like the PSD type layer transform
  `xx, xy, yx, yy, tx, ty`, a placed smart object's transform, and Figma
  `relativeTransform` [Figma node properties].
- Rasters and pixel masks sit at an integer `offset`, as a PSD layer record
  stores its bounds and Krita stores each layer's `x` and `y` [Krita file].
  Moving one changes its offset and never resamples its pixels.
- Shapes and path masks are drawn in page coordinates, as PSD stores path points
  against the document size.
- Groups have no placement of their own, as PSD groups have none.

### Text

| Property | Model | Evidence |
|---|---|---|
| `content` | rich text: spans in order, each a string with its character marks; a plain string is one unmarked span | Figma `characterStyleOverrides` and `styleOverrideTable` [Figma nodes]; Clip Studio Paint colours selected letters [CSP text editing] |
| `style` | the layer-wide character and paragraph values | Figma `TypeStyle` on the node [Figma property types]; Photoshop formats "selected characters or an entire type layer" [Photoshop characters] |
| `text_style` | the [text style](#text-styles) the text takes its other values from, by id, or none | Figma text styles [Figma text styles]; InDesign paragraph and character styles [InDesign styles] |
| `frame` | none (point text), a box (paragraph text), or a closed path that the text fills; a box or path frame also has a `fit` and an `anchor` | Photoshop point type, paragraph type [Photoshop type], and text inside a shape [Photoshop text in shapes]; Figma `textAutoResize` `WIDTH_AND_HEIGHT` and `NONE` [Figma text resize]; Clip Studio Paint point text and text boxes [CSP text] and balloon text [CSP balloons] |
| `fit` | `shrink` (the default): text that would overflow shrinks just enough to fit, down to half its set size, and past that it overflows; `none`: the set size always; `grow`, for a box only: the box keeps its inline size, and its block size follows the text, growing away from its `anchor` | PowerPoint shrinks placeholder text "as you type" by default, and with fitting stopped the text "will overfill the box" [PowerPoint placeholders] [PowerPoint autofit]; Google Slides "Shrink text on overflow" and "Resize shape to fit text" [Google Slides fitting]; Keynote fits text to most text boxes by default [Keynote autosize]; Figma `HEIGHT` [Figma text resize]; InDesign Auto-Size [InDesign auto-size]; a SwiftUI minimum scale factor of 0.5 draws text "as small as half" [SwiftUI minimum scale] |
| `anchor` | where the lines sit in a box or path frame along the block axis: `start` (the default), `center`, or `end` | OOXML `anchor`, top when omitted [OOXML anchor]; Figma vertical alignment [Figma text properties]; InDesign vertical justification, Top by default [InDesign alignment]; Clip Studio Paint Position in frame [CSP text frame]; CSS `align-content` on block containers [CSS align] |
| `direction` | horizontal or vertical | Clip Studio Paint Text Direction [CSP vertical text] |
| `language` | one BCP 47 tag; `und` when undetermined | SVG `systemLanguage` [SVG structure]; C2PA `sourceLanguage` and `targetLanguage` [C2PA actions] |

The character keys are font family, weight, italic, size, fill colour, stroke
colour, and stroke width. Each is both a `style` key and a mark key, and a mark
wins over `style` for its span. Alignment is a paragraph key that lives only in
`style` and applies to the whole text, like `textAlignHorizontal` in a Figma
node's `TypeStyle` [Figma property types]. Its values are `start`, `center`,
`end`, and `justify`, as in CSS `text-align`, where start and end follow the
writing direction [CSS text], so one value serves horizontal and vertical
text. `line_height` is the other paragraph key: a multiple of each line's
largest size, as a unitless CSS `line-height` is [CSS line-height] and Clip
Studio Paint's line space can be "a percentage of the character size" [CSP text
frame]. Photoshop's leading [Photoshop leading] and Figma's line height [Figma
text properties] are the same setting; manga letterers set it tighter than
Photoshop's 120% Auto, at the size or 1 pt more, so dialogue reads evenly
[Manga balloon basics]. Styling with no characters selected writes `style`;
styling selected characters writes marks.

A size is the size a text is set at. A `shrink` frame draws its spans smaller,
all at one scale, only while they would otherwise overflow, so dialogue keeps one
size across a page and shrinks only where a translation runs long:

- Manga lettering sets one dialogue size "in accordance with the original
  Japanese book" [Manga balloon basics], and the text stays "relatively the
  same size across balloons, despite the different lengths of text and sizes of
  balloons" [Search and Destroy letters].
- A smaller font is a last resort, used "only a few instances, at most" in a
  300-page volume, because it "alters the author's chosen balance of word and
  image" [Manga translation II]. Comic speech is one size, and a change of size
  means a shout or a whisper [Comics Kingdom].
- A translation fills a region its source laid out, the way content fills a
  slide placeholder, so `shrink` is the default.

So a balloon's size comes from its source text, not from the balloon, and the
balloon only bounds it. Start and end on the block axis follow the writing
direction, as `align` does: top and bottom for horizontal text, right and left
for vertical text.

A balloon holding several source text blocks gets one text per block, each
framed by its own region of the balloon and set at the same size. Texts never
flow from one frame into another: Clip Studio Paint adds
several texts to one balloon layer [CSP text tool], and Publisher fits text only
in a box "not connected to other text boxes" [Publisher copyfitting]. The
typesetting step cuts the balloon into regions with a watershed seeded by the
source blocks on the balloon's distance from its edge, so each border falls at
a neck [Watershed] [Watershed example], where perception places a boundary
between parts, along the shortest cut [Shortest cuts]. When the translation
reads as one thought, its blocks become one text framed by the whole balloon,
as "balloons directly joined together are generally of the same thought
process" [Blambot grammar].

SVG 2 dropped text inside shapes in 2026 because no renderer implemented it
[SVG text inside shapes dropped], so the closed-path frame follows Photoshop and
Clip Studio Paint.

### Text styles

| Property | Model | Evidence |
|---|---|---|
| place | named styles kept by the project, each with a stored id and a `name`; a text names at most one in `text_style` | Figma text styles [Figma text styles]; InDesign paragraph and character styles [InDesign styles] |
| values | any of the keys a text's `style` holds: the character keys, `align`, and `line_height` | the same |
| precedence | each key comes from a span's mark, else the text's `style`, else its text style, else the renderer's default | an InDesign override is "any change made to an existing style" [InDesign styles] |
| editing | changing a style changes every text that names it | a Figma local style's changes "are immediately applied to any text layers using that style" [Figma text styles]; InDesign's Redefine Style [InDesign styles] |
| overrides | a text's view lists the keys its `style` sets over its text style; setting them to null shows the style's values again | InDesign marks an overridden style with a plus sign and offers Clear Overrides [InDesign styles] |

The typesetting step puts a page's dialogue in one style, so one edit resizes
every balloon. A style a text names can't be removed; the edit fails, so no
text loses its look silently. A text style keeps its origin and the user's
overrides as a layer does ([section 2](#2-generated-values-and-overrides)), so
rerunning the typesetting step doesn't undo the user's change to the dialogue
size.

### Masks

| Property | Model | Evidence |
|---|---|---|
| place | a child of the layer it limits; a layer can have several | Krita masks are child nodes of a layer [Krita layers]; a Lightroom mask is made of several components [Lightroom masking] |
| `mode` | add, subtract, or intersect, applied in order | Lightroom's Add, Subtract, and Intersect [Lightroom masking]; CSS `mask-composite` [CSS masking] |
| start | a layer without masks shows whole; components apply to an empty mask when the first one adds, and to a full mask otherwise | a Lightroom mask starts from its first component [Lightroom masking]; a PSD layer mask has a default colour of 0 or 255 [PSD spec] |
| drawing | never drawn and outside the draw order; a group's masks limit the group's composite | Krita masks filter their parent [Krita layers]; a PSD group carries a layer mask like any layer [PSD spec] |
| content | pixels or a closed path | the PSD layer mask and vector mask [PSD spec] |
| PSD export | the components merge into the layer's one mask | a PSD layer has one user mask and one vector mask [PSD spec]; a GIMP layer has one mask [GIMP XCF] |

Masks take `name` and `visible` from the common properties.

### Stored values

| Value | Model | Evidence |
|---|---|---|
| raster pixels | 8-bit RGBA in 64×64 tiles keyed by tile position, each compressed with zlib on its own; only tiles holding pixels are stored | GIMP stores 64-pixel tiles, each compressed on its own, zlib being one option [GIMP XCF]; Krita stores 64×64 tiles, each recorded with its x and y [Krita tiles] |
| where pixels live | in the page, under the layer's id, apart from the layer's own properties | an OpenRaster layer names its pixel file in `src` [OpenRaster]; a Krita layer names its pixel data file [Krita file] |
| mask pixels | 8-bit grey, tiled and kept like raster pixels | a PSD user mask is one 8-bit channel [PSD spec] |
| path | SVG path data | SVG [SVG paths] |
| colour | sRGB r, g, b, and a, each a float from 0 to 1 (`color::AlphaColor<Srgb>`) | Figma `Color` has r, g, b, and a, each "between 0 and 1" [Figma property types] |

## 2. Generated values and overrides

- **Origin.** A layer records, in order, each model action that wrote to it,
  as C2PA lists the actions taken on an asset [C2PA actions]. Each action has
  C2PA's `action` (such as `c2pa.created` or `c2pa.translated`),
  `softwareAgent` (a name and version), `digitalSourceType`, `parameters`, and
  `ingredients`, plus `writes`, the property paths it wrote, which C2PA does
  not have. A layer no model has written has an empty origin.
  - Each path belongs to the latest action that wrote it. A rerun of a model
    takes over every path that model wrote on the layer, so its ingredients
    replace the old ones, as Lightroom's Update AI Masks recomputes an AI mask
    once the photo under it changes [Lightroom masking]. Another model takes
    over only the paths it writes, and an action left with no paths is
    dropped.
  - A `c2pa.translated` action carries `sourceLanguage` and `targetLanguage`
    in its parameters, as C2PA requires [C2PA actions]. An undetermined source
    language is `und`, BCP 47's tag for one.
  - Photoshop likewise keeps each Generative Fill result as its own Generative
    Layer [Photoshop Generative Fill].
  - Lightroom keeps its AI components, such as Subject or Sky, apart from Brush
    components [Lightroom masking].
- **Ingredients** are what an action read. Each names a layer by id, a property
  path, a hash of the value read, and a `relationship`, as C2PA actions
  reference their ingredients by hash and every C2PA ingredient carries a
  relationship [C2PA actions]. Every ingredient counts for staleness, whatever
  its relationship.
  - `parentOf` marks the one source the action derives from, such as the OCR
    text a translation translates. An action has at most one, as a C2PA
    manifest does.
  - `inputTo` marks context the action also read, such as the other texts on
    the page sent to a translator.
  - `componentOf` marks a part placed into the result, as C2PA requires for
    `c2pa.placed`.
  - Ingredients are the only references between layers. A translation lists the
    OCR text it read, so moving either layer keeps the link. Nesting is layout
    only.
- **Stale results.** A result is rejected if any of its ingredients changed
  before it commits. After it commits, the paths an action wrote are out of
  date once any of its ingredients changes or its ingredient layer is gone, and
  rerunning the model is the workflow's choice.
- **Values.** Every layer holds the values that show, whoever wrote them, so
  reading is direct and the same on every layer. A layer no model has written
  holds nothing else.
  - A Blender library override likewise holds its data locally and "only
    stores override operations, not override operands" [Blender overrides
    design].
  - A model may write into a layer the user made. Its value replaces the
    user's at the paths it writes, and its action takes those paths over, as
    Photoshop's Content-Aware Fill can send its result to the "Current Layer"
    instead of a new one [Photoshop Content-Aware Fill].
- **Replaced model values.** A layer a model wrote also keeps `generated`: for
  each property the user changed, the value the model wrote there. Each entry
  is keyed by a property path, a JSON Pointer such as `/frame` or `/style/size`
  [JSON Pointer]. A path in `generated` is an override.
  - Blender override properties are likewise named by RNA path [Blender
    override API] [Blender overrides].
  - Figma instances likewise record which fields of each node are overridden
    [Figma instances].
- **Your edits.** Changing a property a model wrote first moves the model's
  value into `generated`, unless that path is already there, then writes the
  new value.
- **Overriding text.** Your first edit of a generated text, including styling
  part of it, moves a copy of the text and its marks into `generated` at
  `/content`; you keep editing the text on the layer. Later regenerations then
  go to that copy and stay hidden until Reset. Styling the whole layer
  overrides `/style` keys only, so regenerated text still shows.
- **Painting.** Pixels take no override: painting over pixels a model wrote
  replaces them, a rerun replaces your painting, and undo is the only way back.
  Pixels are not a property either, so no ingredient reads them and no action
  records writing them; staleness covers properties only.
- **Rerunning a model** writes each value it produces onto the layer, except at
  a path in `generated`, where it replaces the stored model value and the
  user's value keeps showing. Which existing layer a new output replaces is the
  workflow's to decide.
  - In Blender, when the library changes, "unmodified properties" of the
    override are updated [Blender overrides].
  - Lightroom recomputes AI masks with Update [Lightroom masking].
- **Removed outputs.** When a rerun no longer produces a layer, the layer goes
  unless it or a layer inside it has an entry in `generated`, or the user added
  a layer inside it. Blender's resync likewise keeps a user-edited override
  that diverges from the linked hierarchy, unless a manual resync enforces the
  hierarchy [Blender overrides design].
- **Reset** writes the stored model value back onto the layer and deletes it
  from `generated`, so the model's value shows again: Figma's "Reset
  [property]" and "Reset all changes" [Figma reset overrides], and Blender's
  Reset and Remove Override [Blender overrides].
- **Duplicates.** A duplicate copies the layer with everything inside it, its
  pixels, and its text, and keeps its origin and `generated`. The copy is still
  the model's output with the user's changes: Reset restores the model's values
  on it, and it goes stale with the same ingredients.
  - Blender duplicates a library override as a new override of the same linked
    data [Blender 2.90 overrides].
  - Figma duplicates an instance as an instance of the same main component
    [Figma instances] [Figma insert instances].
  - Clip Studio Paint keeps duplicated file object layers linked to the same
    file [CSP file objects]. Photoshop advises duplicating a layer before
    rasterizing it, which removes its generated and Smart Object data
    [Photoshop rasterize].
  - An image copy names the same file, which is never edited, so its bytes are
    kept once.
  - The copy keeps its name. Photoshop and Clip Studio Paint add "copy"
    [Photoshop Smart Object copies] [CSP layers]; the app does that in the same
    edit, because the word is translated.
- **No conflict tracking.** Neither Blender nor Figma reports when the value
  under an override changes. A rerun replaces a stored model value silently,
  and nothing records what it was when the user made the change.

## 3. Attributes

- **Shape.** Each layer has `attributes`: namespace, then key, then value, like
  Figma's `setSharedPluginData(namespace, key, value)` [Figma plugin data] and
  GIMP parasites, whose names start with their plug-in's name [GIMP
  parasites].
- **Values** are strings, numbers, arrays, or nested maps, as in Blender custom
  properties [Blender custom properties].
- **Overrides.** Attributes take overrides like any other property, at paths
  such as `/attributes/koharu/role`; Blender custom properties can be "Library
  Overridable" [Blender custom properties].
- **Rendering.** The renderer never reads attributes, so two documents that
  differ only in attributes look the same. They are data for tools, like
  Figma plugin data [Figma plugin data] and SVG `data-*` attributes [SVG
  structure].
- **Unknown namespaces** are kept as they are, as GIMP saves persistent
  parasites [GIMP parasites]. Only core properties fail closed.

## Loro specifics

This section is implementation, not schema.

- **Identity** is a stored id in each page's and layer's map. Loro's `TreeID`s
  and container ids stay internal.
  - Undo of a delete, redo of a create, and `revert_to` re-create a node under a
    new `TreeID` (`loro-internal` 1.17.1, `handler.rs`). The restored map
    brings the stored id back.
  - Selection, caches, and jobs outside the document key on stored ids, so they
    survive undo.
- **Text** is `LoroText`, read and written as its delta, whose inserts are the
  spans. Character keys are Loro rich-text marks that expand after their span,
  so typing at the end of a span continues its style. Loro does not store mark
  configuration, so that expand rule is set as the default for every key on
  every load.
- **Replaced model values.** `generated` is a `LoroMap` from JSON Pointer to the
  model's value; `/content` holds a `LoroText` copied with `apply_delta`. The
  text on the layer stays one `LoroText` across your edits, so cursors into it
  stay valid, and Reset rewrites it in place.
- **Large data** never sits in a layer's map. Loro keeps the map of a deleted
  tree node, and everything it reaches, through compaction, because another
  peer can revive the node; entries deleted from an ordinary map or list are
  dropped (`loro-internal` 1.17.1, `state.rs`).
  - Imported files live in a project map keyed by content hash, and pixel tiles
    in a page map keyed by layer id.
  - The edit that deletes a layer deletes its tiles, and the edit that removes
    the last layer naming a file deletes the file, so undo restores them
    together.
  - Each layer's tiles are their own container, so Loro decodes them only when
    they are read. `Session::tiles_hash` hashes a layer's compressed tiles
    with BLAKE3, so a renderer can tell that pixels changed without decoding
    them.
- **Text styles** live in a project map keyed by style id. Each style's map
  holds its values, `generated`, and `origin`, as a layer's does, so edits,
  overrides, Reset, and staleness work the same on both.
- **Undo** is Loro's `UndoManager`: one edit is one commit, which is one undo
  step.
- **Edits.** `Session::apply` runs one `Edit`: its author, the user or a
  model, its undo label, what it read, and its ops.
  - The user's write to a property a model wrote first moves the model's value
    into `generated`, unless that path is already there.
  - A model's write goes onto the layer, or into `generated` at a path the user
    changed, and appends its action to the layer's origin, taking over the
    paths it writes. It first hashes each ingredient's current value and fails
    as stale on a mismatch.
  - Every operation checks its arguments before its first write, because Loro
    has no rollback.
  - A failure after a write poisons the session.
- **Change** names, by id, the pages, layers, and text styles an edit, undo,
  or redo touched, and what an edit created or kept. An edit's comes from its plan;
  an undo's or redo's is built from Loro events by walking each event's layer
  up to its page.
- **Saving.** The session's owner calls `save` when edits pause and when the
  project closes, not after every edit. Each save rewrites the whole file with
  its history, so save time grows with raster history until compaction drops
  it.
- **Versioning.** The project `version` is a SemVer version [SemVer] and
  changes only when the core does. A new layer kind or core property raises the
  minor version, and a reader opens any version up to its own with the same
  major version, since an addition needs no decoder. A change that removes or
  redefines core data raises the major version, and a reader opens only its own
  major version, because no upgrade decoder is written. The patch stays 0,
  since stored data has no bug fixes. A newer version, a pre-release, or
  unknown core data fails to open; unknown attributes never do.

## Examples

Each example uses only this crate and runs with
`cargo run -p koharu-scene --example <name>`:

- `project`: create a project, import a page, add one layer of every kind,
  write pixels, read everything back, save, reopen (one session at a time),
  and compact.
- `editing`: what the editor does: patch, lock, edit text, paint, duplicate,
  move, and remove, refresh from a change, undo and redo, and the errors an
  edit can fail with.
- `models`: provenance, overrides that survive reruns, staleness, Reset, and
  removed outputs.
- `pipeline`: detection, OCR, typesetting, translation, and inpainting as
  data: a balloon with workflow attributes, a pixel text mask, a dialogue
  style set from the source text and resized once by the user, a translation
  framed by the balloon's path, sources paired from a page, and a rerun
  replacing what a model made.
- `sharing`: one session shared by the editor and a background model, with a
  stale result and an edit that changes nothing.
- `tools`: what an agent or MCP client does: list and call the tools, and read
  compacted answers.

## Sources

Read in October 2026.

- Adobe: [PSD spec], [Photoshop type], [Photoshop characters], [Photoshop text
  in shapes], [Photoshop smart objects], [Photoshop Smart Object copies],
  [Photoshop rasterize], [Photoshop artboards], [Photoshop Generative Fill],
  [Photoshop Content-Aware Fill], [Photoshop leading], [Lightroom masking],
  [InDesign auto-size], [InDesign alignment], [InDesign styles]
- Clip Studio Paint: [CSP pages], [CSP page size], [CSP layers], [CSP file
  objects], [CSP text], [CSP text editing], [CSP vertical text], [CSP
  balloons], [CSP text frame], [CSP text tool]
- Figma: [Figma nodes], [Figma node properties], [Figma property types],
  [Figma image paint], [Figma text resize], [Figma text properties], [Figma
  text styles], [Figma plugin data], [Figma instances], [Figma insert
  instances], [Figma reset overrides]
- Microsoft: [PowerPoint placeholders], [PowerPoint autofit], [OOXML anchor],
  [Publisher copyfitting]
- Google and Apple: [Google Slides fitting], [Keynote autosize], [SwiftUI
  minimum scale]
- W3C: [SVG structure], [SVG rendering], [SVG paths], [SVG image], [SVG text
  inside shapes dropped], [CSS masking], [CSS text], [CSS align], [CSS
  line-height]
- Lettering and manga translation: [Comics Kingdom], [Blambot grammar], [Manga
  balloon basics], [Search and Destroy letters], [Manga translation II]
- Shape analysis: [Watershed], [Watershed example], [Shortest cuts]
- Others: [OpenRaster], [Krita layers], [Krita file], [Krita tiles], [GIMP
  XCF], [GIMP parasites], [Blender overrides], [Blender overrides design],
  [Blender override API], [Blender 2.90 overrides], [Blender custom
  properties], [C2PA actions], [JSON Pointer], [SemVer]

[PSD spec]: https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
[Photoshop type]: https://helpx.adobe.com/photoshop/desktop/text-typography/type-layers-creation/type-layers-in-photoshop.html
[Photoshop characters]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/format-characters.html
[Photoshop text in shapes]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/add-text-along-paths-or-inside-shapes.html
[Photoshop smart objects]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/smart-objects/export-the-contents-of-an-embedded-smart-object.html
[Photoshop Smart Object copies]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/smart-objects/duplicate-an-embedded-smart-object.html
[Photoshop rasterize]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/get-started-layers/rasterize-layers.html
[Photoshop artboards]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/get-started-artboards.html
[Photoshop Generative Fill]: https://helpx.adobe.com/photoshop/desktop/create-open-import-images/create-images/edit-images-with-generative-fill.html
[Photoshop Content-Aware Fill]: https://helpx.adobe.com/photoshop/desktop/repair-retouch/remove-objects-fill-space/adjust-content-aware-fill-settings.html
[Lightroom masking]: https://helpx.adobe.com/lightroom-classic/desktop/process-and-develop-photos/masking.html
[InDesign auto-size]: https://web.archive.org/web/20260209035437/https://helpx.adobe.com/indesign/using/text-frame-fitting.html
[InDesign alignment]: https://helpx.adobe.com/indesign/using/aligning-text.html
[InDesign styles]: https://web.archive.org/web/20231210173751/https://helpx.adobe.com/indesign/using/paragraph-character-styles.html
[Photoshop leading]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/adjust-line-and-character-spacing.html
[CSP pages]: https://help.clip-studio.com/en-us/manual_en/570_pages/Management_Files_and_Page_Files.htm
[CSP page size]: https://help.clip-studio.com/en-us/manual_en/210_file/Change_canvas_size_and_settings.htm
[CSP layers]: https://help.clip-studio.com/en-us/manual_en/180_layers/Basic_operations.htm
[CSP file objects]: https://tips.clip-studio.com/en-us/articles/1564
[CSP text]: https://help.clip-studio.com/en-us/manual_en/480_text/Adding_text.htm
[CSP text editing]: https://help.clip-studio.com/en-us/manual_en/480_text/Editing_text.htm
[CSP vertical text]: https://help.clip-studio.com/en-us/manual_en/480_text/Vertical_text_and_readings.htm
[CSP balloons]: https://help.clip-studio.com/en-us/manual_en/540_comic/Balloons.htm
[CSP text frame]: https://help.clip-studio.com/en-us/manual_en/810_subtools/L.htm
[CSP text tool]: https://help.clip-studio.com/en-us/manual_en/810_subtools/E.htm
[Figma nodes]: https://developers.figma.com/docs/rest-api/file-node-types/
[Figma node properties]: https://developers.figma.com/docs/plugins/api/node-properties/
[Figma property types]: https://developers.figma.com/docs/rest-api/file-property-types/
[Figma image paint]: https://developers.figma.com/docs/plugins/api/Paint/
[Figma text resize]: https://developers.figma.com/docs/plugins/api/properties/TextNode-textautoresize/
[Figma text properties]: https://help.figma.com/hc/en-us/articles/360039956634-Explore-text-properties
[Figma text styles]: https://help.figma.com/hc/en-us/articles/360039957034-Create-and-apply-text-styles
[PowerPoint placeholders]: https://support.microsoft.com/en-gb/powerpoint/add-text-to-a-slide
[PowerPoint autofit]: https://support.microsoft.com/en-us/powerpoint/split-body-text-between-two-slides
[OOXML anchor]: https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.bodyproperties.anchor
[Publisher copyfitting]: https://support.microsoft.com/en-au/publisher/turn-on-and-off-automatic-copyfitting
[Google Slides fitting]: https://support.google.com/docs/answer/10364036
[Keynote autosize]: https://support.apple.com/guide/keynote/change-the-font-or-font-size-tane1c9a464f/mac
[SwiftUI minimum scale]: https://developer.apple.com/documentation/swiftui/view/minimumscalefactor(_:)
[Figma plugin data]: https://developers.figma.com/docs/plugins/api/properties/nodes-setsharedplugindata/
[Figma instances]: https://developers.figma.com/docs/plugins/api/InstanceNode/
[Figma insert instances]: https://help.figma.com/hc/en-us/articles/360039150173-Create-and-insert-component-instances
[Figma reset overrides]: https://help.figma.com/hc/en-us/articles/360039150733
[SVG structure]: https://www.w3.org/TR/SVG2/struct.html
[SVG rendering]: https://www.w3.org/TR/SVG2/render.html
[SVG paths]: https://www.w3.org/TR/SVG2/paths.html
[SVG image]: https://www.w3.org/TR/SVG2/embedded.html#ImageElement
[SVG text inside shapes dropped]: https://github.com/w3c/svgwg/issues/1067
[CSS masking]: https://www.w3.org/TR/css-masking-1/#the-mask-composite
[CSS text]: https://www.w3.org/TR/css-text-3/#text-align-property
[CSS align]: https://www.w3.org/TR/css-align-3/#distribution-block
[CSS line-height]: https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/line-height
[Comics Kingdom]: https://comicskingdom.com/trending/blog/2024/07/22/do-you-speak-comics-part-2-balloons-and-bubbles
[Blambot grammar]: https://blambot.com/pages/comic-book-grammar-tradition
[Manga balloon basics]: https://www.linkedin.com/pulse/manga-lettering-balloon-basics-phil-christie
[Search and Destroy letters]: https://yattatachi.com/the-manga-letters-search-and-destroy
[Manga translation II]: https://wordswithoutborders.org/read/article/2009-02/manga-translation-only-poets-need-apply-part-ii/
[Watershed]: https://doi.org/10.1016/1047-3203(90)90014-M
[Watershed example]: https://scikit-image.org/docs/stable/auto_examples/segmentation/plot_watershed.html
[Shortest cuts]: https://doi.org/10.3758/BF03205536
[OpenRaster]: https://www.openraster.org/baseline/layer-stack-spec.html
[Krita layers]: https://docs.krita.org/en/reference_manual/layers_and_masks.html
[Krita file]: https://invent.kde.org/graphics/krita/-/blob/master/plugins/impex/libkra/kis_kra_savexml_visitor.cpp
[Krita tiles]: https://invent.kde.org/graphics/krita/-/blob/master/libs/image/tiles3/swap/kis_tile_compressor_2.cpp
[GIMP XCF]: https://developer.gimp.org/core/standards/xcf/
[GIMP parasites]: https://developer.gimp.org/core/specifications/parasites/
[Blender overrides]: https://docs.blender.org/manual/en/latest/files/linked_libraries/library_overrides.html
[Blender overrides design]: https://developer.blender.org/docs/features/core/overrides/library/
[Blender override API]: https://docs.blender.org/api/current/bpy.types.IDOverrideLibraryProperty.html
[Blender 2.90 overrides]: https://developer.blender.org/docs/release_notes/2.90/animation_rigging/
[Blender custom properties]: https://docs.blender.org/manual/en/latest/files/custom_properties.html
[C2PA actions]: https://spec.c2pa.org/specifications/specifications/2.2/specs/C2PA_Specification.html#_actions
[JSON Pointer]: https://www.rfc-editor.org/rfc/rfc6901
[SemVer]: https://semver.org/spec/v2.0.0.html
