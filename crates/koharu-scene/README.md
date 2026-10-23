# koharu-scene

Koharu's project model on Loro.

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
| project | ordered pages and a `version` | a Clip Studio Paint EX work is one management file over its pages [CSP pages]; an OpenRaster image must carry a `version` [OpenRaster] |
| page | a stored id, a `size`, and its layers | each Clip Studio Paint page has its own canvas size [CSP page size]; a PSD document has a width and height [PSD spec]; each Photoshop artboard has its own size [Photoshop artboards]; a Figma page is a node with an `id` [Figma nodes] |

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
| `image` | an imported file, kept byte for byte, and its media type | Photoshop exports an embedded smart object "in its original format" [Photoshop smart objects]; SVG `image` [SVG image] |
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
| `content` | rich text: a string whose runs carry character marks | Figma `characterStyleOverrides` and `styleOverrideTable` [Figma nodes]; Clip Studio Paint colours selected letters [CSP text editing] |
| `style` | the layer-wide character and paragraph values | Figma `TypeStyle` on the node [Figma property types]; Photoshop formats "selected characters or an entire type layer" [Photoshop characters] |
| `frame` | none (point text), a box (paragraph text), or a closed path that the text fills | Photoshop point type, paragraph type [Photoshop type], and text inside a shape [Photoshop text in shapes]; Figma `textAutoResize` `WIDTH_AND_HEIGHT` and `NONE` [Figma text resize]; Clip Studio Paint point text and text boxes [CSP text] and balloon text [CSP balloons] |
| `direction` | horizontal or vertical | Clip Studio Paint Text Direction [CSP vertical text] |
| `language` | one BCP 47 tag; `und` when undetermined | SVG `systemLanguage` [SVG structure]; C2PA `sourceLanguage` and `targetLanguage` [C2PA actions] |

The character keys are font family, weight, italic, size, fill colour, stroke
colour, and stroke width. Each is both a `style` key and a mark key, and a mark
wins over `style` for its run. Alignment is a paragraph key that lives only in
`style` and applies to the whole text, like `textAlignHorizontal` in a Figma
node's `TypeStyle` [Figma property types]. Styling with no characters selected
writes `style`; styling selected characters writes marks.

SVG 2 dropped text inside shapes in 2026 because no renderer implemented it
[SVG text inside shapes dropped], so the closed-path frame follows Photoshop and
Clip Studio Paint.

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
| mask pixels | 8-bit grey, tiled like raster pixels | a PSD user mask is one 8-bit channel [PSD spec] |
| path | SVG path data | SVG [SVG paths] |
| colour | Koharu's existing RGBA colour | Figma `Color` is r, g, b, a [Figma property types] |

## 2. Generated values and overrides

- **Origin.** A layer records, in order, each model action that wrote to it,
  as C2PA lists the actions taken on an asset [C2PA actions]. Each action has
  C2PA's `action` (such as `c2pa.created` or `c2pa.translated`),
  `softwareAgent` (a name and version), `digitalSourceType`, `parameters`, and
  `ingredients`, plus `writes`, the property paths it wrote, which C2PA does
  not have. A layer the user made has an empty origin.
  - Each path belongs to the latest action that wrote it. A rerun, or another
    model writing the same path, takes the path over, and an action left with
    no paths is dropped.
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
  reading is direct and the same on every layer. A layer the user made holds
  nothing else.
  - A Blender library override likewise holds its data locally and "only
    stores override operations, not override operands" [Blender overrides
    design].
  - A model never writes into a layer the user made; it adds a layer of its
    own, as Generative Fill adds a Generative Layer [Photoshop Generative
    Fill].
- **Replaced model values.** A model layer also keeps `generated`: for each
  property the user changed, the value the model wrote there. Each entry is
  keyed by a property path, a JSON Pointer such as `/frame` or `/style/size`
  [JSON Pointer]. A path in `generated` is an override.
  - Blender override properties are likewise named by RNA path [Blender
    override API] [Blender overrides].
  - Figma instances likewise record which fields of each node are overridden
    [Figma instances].
- **Your edits.** Changing a property on a model layer first moves the model's
  value into `generated`, unless that path is already there, then writes the
  new value.
- **Overriding text.** Your first edit of a generated text, including styling
  part of it, moves a copy of the text and its marks into `generated` at
  `/content`; you keep editing the text on the layer. Later regenerations then
  go to that copy and stay hidden until Reset. Styling the whole layer
  overrides `/style` keys only, so regenerated text still shows.
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
    new `TreeID` (`loro-internal` 1.16.2, `handler.rs`). The restored map
    brings the stored id back.
  - Selection, caches, and jobs outside the document key on stored ids, so they
    survive undo.
- **Text** is `LoroText`. Character keys are Loro rich-text marks that expand
  after their run, so typing at the end of a run continues its style. Loro does
  not store mark configuration, so the keys and expand rules are registered on
  every load.
- **Replaced model values.** `generated` is a `LoroMap` from JSON Pointer to the
  model's value; `/content` holds a `LoroText` copied with `apply_delta`. The
  text on the layer stays one `LoroText` across your edits, so cursors into it
  stay valid, and Reset rewrites it in place.
- **Rasters.** Each raster's tiles are their own container, so Loro decodes
  them only when they are read.
- **Undo** is Loro's `UndoManager`: one edit is one commit, which is one undo
  step.
- **Edits.** `Session::edit(author, label, closure)` runs one edit as the user
  or as a model.
  - The user's write to a model layer first moves the model's value into
    `generated`, unless that path is already there.
  - A model's write goes onto the layer, or into `generated` at a path the user
    changed, and appends its action to the layer's origin, taking over the
    paths it writes. It first hashes each ingredient's current value and fails
    as stale on a mismatch.
  - Every operation checks its arguments before its first write, because Loro
    has no rollback.
  - A failure after a write poisons the session.
- **Change** names, by id, the pages an edit touched. It is built from Loro
  events by walking each event's layer up to its page.
- **Saving.** The session saves when edits pause and when the project closes,
  not after every edit. Each save rewrites the whole file with its history, so
  save time grows with raster history until compaction drops it.
- **Versioning.** The project `version` changes only when the core does: a new
  layer kind or core property. A reader opens any version up to its own, since
  an added kind or property needs no decoder. A newer version or unknown core
  data fails to open; unknown attributes never do. A change that removes or
  redefines core data leaves older files unopenable, because no upgrade decoder
  is written.

## Sources

Read in October 2026.

- Adobe: [PSD spec], [Photoshop type], [Photoshop characters], [Photoshop text
  in shapes], [Photoshop smart objects], [Photoshop artboards], [Photoshop
  Generative Fill], [Lightroom masking]
- Clip Studio Paint: [CSP pages], [CSP page size], [CSP text], [CSP text
  editing], [CSP vertical text], [CSP balloons]
- Figma: [Figma nodes], [Figma node properties], [Figma property types],
  [Figma text resize], [Figma plugin data], [Figma instances], [Figma reset
  overrides]
- W3C: [SVG structure], [SVG rendering], [SVG paths], [SVG image], [SVG text
  inside shapes dropped], [CSS masking]
- Others: [OpenRaster], [Krita layers], [Krita file], [Krita tiles], [GIMP
  XCF], [GIMP parasites], [Blender overrides], [Blender overrides design],
  [Blender override API], [Blender custom properties], [C2PA actions], [JSON
  Pointer]

[PSD spec]: https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
[Photoshop type]: https://helpx.adobe.com/photoshop/desktop/text-typography/type-layers-creation/type-layers-in-photoshop.html
[Photoshop characters]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/format-characters.html
[Photoshop text in shapes]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/add-text-along-paths-or-inside-shapes.html
[Photoshop smart objects]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/smart-objects/export-the-contents-of-an-embedded-smart-object.html
[Photoshop artboards]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/get-started-artboards.html
[Photoshop Generative Fill]: https://helpx.adobe.com/photoshop/desktop/create-open-import-images/create-images/edit-images-with-generative-fill.html
[Lightroom masking]: https://helpx.adobe.com/lightroom-classic/desktop/process-and-develop-photos/masking.html
[CSP pages]: https://help.clip-studio.com/en-us/manual_en/570_pages/Management_Files_and_Page_Files.htm
[CSP page size]: https://help.clip-studio.com/en-us/manual_en/210_file/Change_canvas_size_and_settings.htm
[CSP text]: https://help.clip-studio.com/en-us/manual_en/480_text/Adding_text.htm
[CSP text editing]: https://help.clip-studio.com/en-us/manual_en/480_text/Editing_text.htm
[CSP vertical text]: https://help.clip-studio.com/en-us/manual_en/480_text/Vertical_text_and_readings.htm
[CSP balloons]: https://help.clip-studio.com/en-us/manual_en/540_comic/Balloons.htm
[Figma nodes]: https://developers.figma.com/docs/rest-api/file-node-types/
[Figma node properties]: https://developers.figma.com/docs/plugins/api/node-properties/
[Figma property types]: https://developers.figma.com/docs/rest-api/file-property-types/
[Figma text resize]: https://developers.figma.com/docs/plugins/api/properties/TextNode-textautoresize/
[Figma plugin data]: https://developers.figma.com/docs/plugins/api/properties/nodes-setsharedplugindata/
[Figma instances]: https://developers.figma.com/docs/plugins/api/InstanceNode/
[Figma reset overrides]: https://help.figma.com/hc/en-us/articles/360039150733
[SVG structure]: https://www.w3.org/TR/SVG2/struct.html
[SVG rendering]: https://www.w3.org/TR/SVG2/render.html
[SVG paths]: https://www.w3.org/TR/SVG2/paths.html
[SVG image]: https://www.w3.org/TR/SVG2/embedded.html#ImageElement
[SVG text inside shapes dropped]: https://github.com/w3c/svgwg/issues/1067
[CSS masking]: https://www.w3.org/TR/css-masking-1/#the-mask-composite
[OpenRaster]: https://www.openraster.org/baseline/layer-stack-spec.html
[Krita layers]: https://docs.krita.org/en/reference_manual/layers_and_masks.html
[Krita file]: https://invent.kde.org/graphics/krita/-/blob/master/plugins/impex/libkra/kis_kra_savexml_visitor.cpp
[Krita tiles]: https://invent.kde.org/graphics/krita/-/blob/master/libs/image/tiles3/swap/kis_tile_compressor_2.cpp
[GIMP XCF]: https://developer.gimp.org/core/standards/xcf/
[GIMP parasites]: https://developer.gimp.org/core/specifications/parasites/
[Blender overrides]: https://docs.blender.org/manual/en/latest/files/linked_libraries/library_overrides.html
[Blender overrides design]: https://developer.blender.org/docs/features/core/overrides/library/
[Blender override API]: https://docs.blender.org/api/current/bpy.types.IDOverrideLibraryProperty.html
[Blender custom properties]: https://docs.blender.org/manual/en/latest/files/custom_properties.html
[C2PA actions]: https://spec.c2pa.org/specifications/specifications/2.2/specs/C2PA_Specification.html#_actions
[JSON Pointer]: https://www.rfc-editor.org/rfc/rfc6901
