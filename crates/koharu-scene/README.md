# koharu-scene

Koharu's scene graph.

Every element of this schema copies something an established editor or format
already has, and cites it. Nothing is new. Where sources differ, Photoshop and
Clip Studio Paint win, as for all editor behaviour in Koharu. The links are
listed under [Sources](#sources).

## Goal

A schema that lasts through workflow changes. The workflow (detect, OCR,
translate, inpaint, typeset) will change, and so will the model behind each
step, but a manga page keeps its parts: panels, balloons, and texts, each text
lettered again in translation. So the schema names those parts, never a step:

1. **Layers**: pages, each with its background and a tree of groups, images,
   rasters, shapes, text, and masks, as Photoshop has them, for any editing.
2. **Manga kinds**: panels, balloons, and blocks, as Clip Studio Paint keeps
   frames and balloons beside its ordinary layers. A block is one text of the
   page, with its source, its translation, its mask, and its clean-up, so each
   step writes fields of a block, and a new model writes the same fields.
3. **Attributes**: namespaced data for tools, which the renderer never reads,
   including a C2PA record of what a model did.

The last writer of any value wins, model or user. [Section
4](#4-how-the-translation-workflow-uses-it) shows what each step and each manual
fix writes.

## 1. Layers

### Project and pages

| Element | Model | Evidence |
|---|---|---|
| project | ordered pages, imported files, and a `version` | a Clip Studio Paint EX work is one management file over its pages [CSP pages]; a Figma file keeps each image once, fetched by hash with `getImageByHash` [Figma image paint]; an OpenRaster image must carry a `version` [OpenRaster] |
| page | a stored id, a `size`, a `background`, its layers, and their pixels | each Clip Studio Paint page has its own canvas size [CSP page size]; a PSD document has a width and height [PSD spec]; each Photoshop artboard has its own size [Photoshop artboards]; a Figma page is a node with an `id` [Figma nodes] |
| `background` | none, or an imported file's content hash and `visible`; drawn first, under every layer, at the page's origin. A page made from a file has that file as its background, at its size | Photoshop's Background is the bottom-most layer, an image has only one, and its stacking order, blending mode, and opacity can't change [Photoshop Background] |

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
| `children` | the layers inside it, held inside it, masks among them | an OpenRaster `stack` holds its stacks and layers [OpenRaster]; Krita writes a group's layers inside the group's element [Krita file]; an SVG `g` holds its children [SVG structure]; Figma returns each node's `children` inside it [Figma nodes] |

Children are drawn first to last, so the last child is on top, as SVG paints in
document order [SVG rendering]. Masks are skipped in drawing and apply in their
own order.

### Layer kinds

| Kind | Holds | Evidence |
|---|---|---|
| `group` | child layers | PSD layer groups (`lsct`) [PSD spec]; SVG `g` [SVG structure]; OpenRaster nested `stack` [OpenRaster]; Krita group layers [Krita layers] |
| `image` | the content hash of an imported file, which the project keeps byte for byte | Photoshop exports an embedded smart object "in its original format" [Photoshop smart objects], and a PSD placed layer names its embedded file by unique ID [PSD spec]; a Figma image paint names its image by "the hash (id) of the image" [Figma image paint]; SVG `image` [SVG image] |
| `raster` | editable pixels | PSD pixel layers [PSD spec]; GIMP layers [GIMP XCF]; Krita paint layers [Krita layers] |
| `shape` | a path of open or closed subpaths, a fill rule, an optional fill, and an optional stroke | PSD shape layers: vector path, fill (`SoCo`), stroke (`vstk`), and fill rule [PSD spec]; SVG `path` [SVG paths]; Figma `VECTOR` [Figma nodes] |
| `text` | styled text in a frame | [below](#text) |
| `mask` | a mask component | [below](#masks) |
| `panel` | a panel of the page, holding what's in it | [section 2](#panels) |
| `balloon` | a balloon, holding its texts | [section 2](#balloons) |
| `block` | one text of the page: its source, translation, mask, and clean-up | [section 2](#blocks) |

A page, a group, and a panel hold any layers but masks, and a balloon holds
blocks and text layers. A group, an image, a raster, a shape, and a text can
also hold masks. A block and a mask hold nothing.

Each kind is placed the way its PSD record places it [PSD spec]:

- Text and image layers carry a `transform`, a 2D affine
  `[[a, b, c], [d, e, f]]`, like the PSD type layer transform
  `xx, xy, yx, yy, tx, ty`, a placed smart object's transform, and Figma
  `relativeTransform` [Figma node properties]. A block's source and translation
  each carry one too.
- Rasters and masks sit at an integer `offset`, as a PSD layer record stores
  its bounds and Krita stores each layer's `x` and `y` [Krita file]. Moving
  one changes its offset and never resamples its pixels.
- A block's mask and clean-up sit on the page's own grid, with no offset,
  because they line up with the background.
- Shapes and the outlines of panels and balloons are drawn in page
  coordinates, as PSD stores path points against the document size.
- Groups have no placement of their own, as PSD groups have none.

### Text

A text layer holds one text value, and so do a block's source and its
translation.

| Property | Model | Evidence |
|---|---|---|
| `content` | rich text: spans in order, each a string with its character marks; a plain string is one unmarked span | Figma `characterStyleOverrides` and `styleOverrideTable` [Figma nodes]; Clip Studio Paint colours selected letters [CSP text editing] |
| `style` | the layer-wide character and paragraph values | Figma `TypeStyle` on the node [Figma property types]; Photoshop formats "selected characters or an entire type layer" [Photoshop characters] |
| `frame` | none (point text), a box (paragraph text), or a closed path that the text fills; a box or path frame also has a `fit` and an `anchor` | Photoshop point type, paragraph type [Photoshop type], and text inside a shape [Photoshop text in shapes]; Figma `textAutoResize` `WIDTH_AND_HEIGHT` and `NONE` [Figma text resize]; Clip Studio Paint point text and text boxes [CSP text] and balloon text [CSP balloons] |
| `fit` | `shrink` (the default): text that would overflow shrinks just enough to fit, down to half its set size, and past that it overflows; `none`: the set size always; `grow`, for a box only: the box keeps its inline size, and its block size follows the text, growing away from its `anchor`; `best`: the text grows or shrinks to the largest size the frame holds, down to half its set size | PowerPoint shrinks placeholder text "as you type" by default, and with fitting stopped the text "will overfill the box" [PowerPoint placeholders] [PowerPoint autofit]; Google Slides "Shrink text on overflow" and "Resize shape to fit text" [Google Slides fitting]; Keynote fits text to most text boxes by default [Keynote autosize]; Figma `HEIGHT` [Figma text resize]; InDesign Auto-Size [InDesign auto-size]; a SwiftUI minimum scale factor of 0.5 draws text "as small as half" [SwiftUI minimum scale]; Publisher's Best Fit shrinks or grows text to fill its box [Publisher copyfitting] |
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

A text holds its own values. Nothing is shared between texts, because a
balloon's text length and size differ from its neighbours', so no one style
fits them all.

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

So a set size comes from the source text, not from the balloon, and the
balloon only bounds it. A translation is the exception: it can't know the
page's dialogue size, and its length differs from its source's, so it's
written `best` and takes the size its room holds, as Publisher's Best Fit
fills a box [Publisher copyfitting]. Setting a size and `shrink` gives it one
again. Start and end on the block axis follow the writing
direction, as `align` does: top and bottom for horizontal text, right and left
for vertical text.

SVG 2 dropped text inside shapes in 2026 because no renderer implemented it
[SVG text inside shapes dropped], so the closed-path frame follows Photoshop and
Clip Studio Paint.

### Masks

| Property | Model | Evidence |
|---|---|---|
| place | among the children of the layer it limits; a layer can have several | Krita masks are child nodes of a layer [Krita layers]; a Lightroom mask is made of several components [Lightroom masking] |
| `mode` | add, subtract, or intersect, applied in order | Lightroom's Add, Subtract, and Intersect [Lightroom masking]; CSS `mask-composite` [CSS masking] |
| start | a layer without masks shows whole; components apply to an empty mask when the first one adds, and to a full mask otherwise | a Lightroom mask starts from its first component [Lightroom masking]; a PSD layer mask has a default colour of 0 or 255 [PSD spec] |
| drawing | never drawn and outside the draw order; a group's masks limit the group's composite | Krita masks filter their parent [Krita layers]; a PSD group carries a layer mask like any layer [PSD spec] |
| content | 8-bit grey pixels at an integer offset; there are no path masks | a Photoshop and a Clip Studio Paint layer mask is grey pixels [PSD spec] [CSP layer masks]; Krita's transparency mask is pixels [Krita layers]; Photoshop's vector mask has no Clip Studio Paint counterpart [CSP layer masks] |
| PSD export | the components merge into the layer's one mask | a PSD layer has one user mask and one vector mask [PSD spec]; a GIMP layer has one mask [GIMP XCF] |

Masks take `name` and `visible` from the common properties. A block's mask is
its own field, not a mask layer ([Blocks](#blocks)).

### Stored values

| Value | Model | Evidence |
|---|---|---|
| raster pixels | 8-bit RGBA in 64×64 tiles keyed by tile position, each compressed with zlib on its own; only tiles holding pixels are stored | GIMP stores 64-pixel tiles, each compressed on its own, zlib being one option [GIMP XCF]; Krita stores 64×64 tiles, each recorded with its x and y [Krita tiles] |
| where pixels live | in the page, under the id of the layer that owns them, apart from its own properties: a raster, a mask, or a block's mask or clean-up, each with one picture | a PSD layer record lists its channels by ID, its user mask being channel -2 [PSD spec]; an OpenRaster layer names its pixel file in `src` [OpenRaster]; a Krita layer names its pixel data file [Krita file] |
| mask pixels | 8-bit grey, tiled and kept like raster pixels | a PSD user mask is one 8-bit channel [PSD spec] |
| path | SVG path data | SVG [SVG paths] |
| colour | sRGB r, g, b, and a, each a float from 0 to 1 (`color::AlphaColor<Srgb>`) | Figma `Color` has r, g, b, and a, each "between 0 and 1" [Figma property types] |

### Writes

Every layer holds only its values, whoever wrote them, so reading is direct and
the same on every layer. A write replaces the values it writes, and the last
writer wins: a model's run replaces the user's change, the user's change
replaces a model's value, and undo is the way back. Pixels are written the same
way, so a rerun replaces your painting.

- Figma resolves two writers to one property the same way: "the document will
  just end up with the last value that was sent to the server" [Figma
  multiplayer].
- Photoshop's Content-Aware Fill can send its result to the "Current Layer"
  instead of a new one, over what's there [Photoshop Content-Aware Fill].

Which fields a run writes, whether it leaves alone what the user changed, and
whether it removes a layer it no longer produces are the workflow's choices. The
scene enforces none of them.

### Duplicates

A duplicate copies the layer with everything inside it, its pixels, its text,
and its attributes.

- An image copy names the same file, which is never edited, so its bytes are
  kept once, as Clip Studio Paint keeps duplicated file object layers linked to
  the same file [CSP file objects].
- The copy keeps its name. Photoshop and Clip Studio Paint add "copy"
  [Photoshop Smart Object copies] [CSP layers]; the app does that in the same
  edit, because the word is translated.

## 2. Manga kinds

Three kinds hold the parts of a manga page. Panels and balloons follow Clip
Studio Paint's frame border folders [CSP frames] and balloons [CSP balloons]. A
block holds one text the way a document-analysis format records a text region
[PAGE-XML], a translation file pairs a source with its target [XLIFF], and
Photoshop keeps a fill on its own layer with its mask [Photoshop combine
variations].

### Panels

| Property | Model | Evidence |
|---|---|---|
| `outline` | a closed path in page coordinates | a Clip Studio Paint frame is an outline edited with the Object tool, and transforming it leaves the layers in its folder as they are [CSP frames] |
| clipping | its children show only inside the outline, so a layer that crosses panels sits on the page, outside them | inside a frame border folder "only the areas within the frame will show", and a layer above the folder draws across frames [CSP frames] |
| border | never drawn, because the background already shows it; a new border is drawn as a shape | turning off Draw border leaves the frame lines invisible [CSP frames] |

### Balloons

| Property | Model | Evidence |
|---|---|---|
| `outline` | a closed path in page coordinates | a Clip Studio Paint balloon's outline has control points, edited as on vector layers [CSP balloons] |
| children | blocks and text layers, several to a balloon | Clip Studio Paint adds several texts to one balloon layer [CSP text tool] |
| drawing | the outline is never drawn or filled, because the background already shows it; only its children draw, and a new balloon is drawn as a shape | a Clip Studio Paint balloon has a line and a fill colour [CSP balloons] |

### Blocks

| Property | Model | Evidence |
|---|---|---|
| place | in its balloon, or free: in its panel, or on the page outside every panel | Clip Studio Paint's Text tool adds text wherever you click on the canvas, and a balloon drawn over it later turns its text layer into a balloon layer [CSP text] [CSP balloons] |
| `role` | `text` (the default), or `sfx` for a sound effect | a PAGE-XML text region's `type` gives "the nature of the text" [PAGE-XML]; manga lettering treats sound effects as a craft of their own, apart from typesetting [Search and Destroy letters] |
| `source` | the original text, never drawn: one [text](#text) value whose frame is a box or a closed path, never a point, with its tilt as its `transform` | a PAGE-XML text region has a polygon outline, an `orientation` for its skew, its text, a `readingDirection`, and a `primaryLanguage` [PAGE-XML]; an XLIFF segment's source and target share one content model [XLIFF]; Acrobat's OCR keeps the scan and places invisible text over it [Acrobat OCR] |
| `translation` | none, or one [text](#text) value, drawn | an XLIFF segment holds one source followed by zero or one target [XLIFF] |
| `mask` | a mask layer of its own: where the source's glyphs are; hidden, it stops limiting the clean-up | a Generative Fill result's layer mask [Photoshop combine variations]; a PSD user mask is one 8-bit channel [PSD spec]; Clip Studio Paint's Enable Mask turns a layer mask off [CSP layer masks] |
| `cleanup` | a raster layer of its own, shown only inside the mask, under the translation; hidden or faded, it shows the original under it | Photoshop's Generative Fill puts each result on its own layer with a layer mask, and painting the mask black reveals what's under it [Photoshop combine variations] |

The mask and the clean-up have the values every layer has, and their own ids:
the editor shows and edits them as layers, a patch on the block reaches them, as
`{"cleanup": {"opacity": 0.4}}`, and painting takes their ids. A block draws its
clean-up through its mask, then its translation over it, and its `opacity` and
`blend` apply to the two together, as to a group.

A block is one text, so a balloon holding three texts holds three blocks, and
each is cleaned, translated, fixed, and removed on its own:

- Deleting a block deletes its clean-up with it, so the original shows again.
- Removing only its translation keeps its clean-up, so the area stays clean for
  new lettering.
- Painting its mask hides its clean-up there at once, so an over-reaching
  clean-up is fixed where it happened, without a rerun.
- Each text gets its own translation, framed by its own room in its balloon, or
  by its source's frame when it's free, and filling it. Texts never flow from one frame into
  another, as Clip Studio Paint adds several texts to one balloon [CSP text
  tool], and Publisher fits text only in a box "not connected to other text
  boxes" [Publisher copyfitting].

These are fields of one layer, not children, so their types hold the rules: a
block always has one source, at most one translation, one mask, and one
clean-up, its mask and clean-up can't be dragged away from it or deleted on
their own, and no tool finds them by convention.

### PSD export

PSD has no panels, balloons, or blocks, so each exports as ordinary layers
[PSD spec]:

| Kind | Exports as |
|---|---|
| background | the bottom pixel layer |
| `panel` | a group whose vector mask is its outline |
| `balloon` | a group holding a hidden shape layer of its outline, then its texts |
| `block` | a group holding its clean-up as a pixel layer with its own name, visibility, opacity, and blend, whose layer mask is the block's mask while it's visible, its source as a hidden type layer, and its translation as a type layer |

## 3. Attributes

- **Shape.** Each layer has `attributes`: namespace, then key, then value, like
  Figma's `setSharedPluginData(namespace, key, value)` [Figma plugin data] and
  GIMP parasites, whose names start with their plug-in's name [GIMP
  parasites].
- **Values** are strings, numbers, arrays, or nested maps, as in Blender custom
  properties [Blender custom properties].
- **Writes.** Attributes are written like any other property, and the last
  writer wins.
- **Links.** The scene relates layers only by holding them: a block's balloon,
  when it has one, is its parent, and its translation is its own field. A workflow that needs
  another link names layers by id in its own namespace.
- **Rendering.** The renderer never reads attributes, so two documents that
  differ only in attributes look the same. They are data for tools, like
  Figma plugin data [Figma plugin data] and SVG `data-*` attributes [SVG
  structure].
- **Owners.** A namespace's owner reads its values into its own types and fails
  closed on a value it can't read, so bad data never reads as missing.
- **Unknown namespaces** are kept as they are, as GIMP saves persistent
  parasites [GIMP parasites]. Only core properties fail closed.

### Provenance

A model that writes a layer records what it did in the layer's `c2pa`
namespace, keyed by the step that ran it, such as `ocr` or `translation`: one
action, in C2PA's vocabulary [C2PA actions], with `action` (such as
`c2pa.created`, `c2pa.edited`, or `c2pa.translated`), `softwareAgent` (a name
and version), `digitalSourceType`, `parameters`, and `ingredients`.

- It's a record, not a rule. The scene never reads it, and nothing is refused or
  kept because of it.
- Several steps write one block, so each keeps its own action. A step writes it
  in the same edit as its results, so a rerun replaces it, whichever model ran,
  and the latest run wins.
- A `c2pa.translated` action carries `sourceLanguage` and `targetLanguage` in its
  parameters, as C2PA requires [C2PA actions]. An undetermined source language
  is `und`, BCP 47's tag for one.
- Because the vocabulary is C2PA's, an export can carry it as Content
  Credentials, as Photoshop records edits in a document and attaches them when
  it exports [Photoshop Content Credentials], and as Photoshop marks a
  Generative Fill result as a Generative Layer [Photoshop Generative Fill].

## 4. How the translation workflow uses it

The workflow writes the kinds above, so every tool finds a page's parts by kind.
Facts only the workflow reads, such as a detector's confidence, live in its
`koharu` namespace.

Layers are listed first to last, which is reading order, and the last is drawn
on top.

```text
Page              background: the scan     import
├── Panel         panel                    detection
│   ├── Balloon   balloon                  detection
│   │   ├── Text  block                    detection, or the box tool
│   │   └── Text  block
│   ├── SFX       block, sfx               free, in its panel
│   └── Text      block                    narration, free, in its panel
├── Balloon       balloon                  crosses panels, so on the page
│   └── Text      block
├── Panel         panel                    the next panel
└── Text          block                    free, outside every panel
```

### What each step writes

Every step runs on the pages or blocks it's given, and writes only its own
fields of them.

| Step | Reads | Writes |
|---|---|---|
| Import | the file | a page, its size, and its background |
| Detection | the background | a `panel` for each panel it finds; a `balloon` for each balloon, inside its panel, or on the page when it crosses panels; a `block` for each text or sound effect, inside its balloon if it has one, else free in its panel or on the page, with its `role`, its source's frame, tilt, and direction, and its mask; all in reading order |
| OCR | the background inside each block's source frame | the source's `content` and `language` |
| Inpainting | the background, each block's mask, and its balloon's outline when it has one | the block's clean-up, only in the tiles it changes |
| Translation | the page's sources in reading order, for context, and in a balloon, the balloon's outline and its other texts | each given block's translation: `content` and `language`; its `frame`, its room in its balloon, or its source's frame when it's free, with `best` fit; `anchor` and `direction` |

Each model also writes its `c2pa` action on the layers it wrote.

### Manual work

| You want to | You do | Which writes |
|---|---|---|
| Add a text detection missed | Draw a box on the page | a block in the balloon under the box, else free in the panel or on the page there, whose source frame and mask are the box. Paint the mask smaller to keep art |
| Read it | Run OCR on that block | its source's wording |
| Fix what OCR read | Edit the source in the Story panel | its source's wording |
| Translate again after a fix | Run translation on that block | its translation's wording, leaving its frame |
| Undo an over-reaching clean-up | Paint the block's mask, or clear its clean-up, and run inpainting on it again if needed | that block's clean-up only |
| Reletter a text | Delete its translation | the block keeps its clean-up, and its area stays clean |
| Drop a text entirely | Delete its block | its clean-up goes with it, so the original shows again |

### Why it's shaped this way

- **A block is one text**, with its own mask and clean-up ([Blocks](#blocks)).
  A run on part of a page writes only the tiles it changes, so a block's
  clean-up stores no more than its area.
- **Panels and balloons are kinds with their own outlines**, so a tool reads a
  balloon's outline from the balloon, and the texts in it are its children, as Clip Studio Paint keeps a balloon's texts in its balloon layer
  [CSP balloons].
- **A text needn't have a balloon or a panel.** Narration and sound effects are
  often free, so a block is free in its panel, or on the page outside every
  panel, as Clip Studio Paint's text needs no balloon [CSP text].
- **A panel clips its children**, so a balloon or a free text that crosses
  panels sits on the page, outside them, as a Clip Studio Paint layer above the
  frame border folders draws across frames [CSP frames].
- **The background is the scan**, kept byte for byte, so clean-up lives in
  blocks, and hiding or deleting a block shows the original under it.
- **Reading order is the tree's order.** Detection writes panels, balloons, and
  blocks in reading order, so a balloon or text on the page sits between the
  panels it's read between, and since they rarely overlap, the order doesn't
  change what's drawn. Moving a layer moves it in reading order too.
- **Translation places a balloon's texts from its outline, and a free text from
  its source's frame**, and resizing the frame reflows the text, as resizing a
  Clip Studio Paint balloon moves its text [CSP balloons].
- **Translation stores its decisions, not their result:** the frame, the
  anchor, the direction, and `best` fit. The size a `best` or `shrink` frame
  draws at is worked out at each render, and PSD export writes the size it
  drew. PowerPoint caches its shrink as `fontScale` [OOXML autofit], but Koharu
  has one renderer, and a cached scale would go stale when a font loads.
- **Reruns find their fields by kind.** A translation rerun rewrites each given
  block's translation, its wording and its placement. Whether a rerun leaves alone what you changed is
  the workflow's choice ([Writes](#writes)).
- **Names** are written in the app's language, as the copy suffix is.

### How the app shows it

- **Layers panel:** the tree as it is, as Photoshop's and Clip Studio Paint's
  layer palettes show it. A block's row shows its clean-up and its mask, and
  painting goes to whichever is selected.
- **Story panel:** each page's blocks in reading order, each with its source's
  wording and its translation's, as Clip Studio Paint's Story Editor "shows all
  text on each page" in text fields [CSP Story Editor]. Editing a row edits that
  block, and removing a row deletes it.
- **Agent tools** read a page as `get_page` answers it, its layers nested in
  reading order, and edit one block at a time by its id.

## Implementation

This section is implementation, not schema.

- **The project file** is one SQLite database named `*.khr`, page images
  included, as Clip Studio Paint's `.clip` wraps one [CSP file format]. It has
  its own application ID, and a second window opening it gets `ProjectLocked` at
  once.
  - Writes go through a write-ahead log beside the project, which goes away on a
    clean close. With `synchronous=NORMAL`, a crash of the app loses nothing
    committed, and a power loss can lose the latest commits without damaging
    the file.
  - Each page is a row holding its layer tree, as an OpenRaster stack holds its
    layers [OpenRaster], and tile positions by the layer that owns them are
    rows. Tiles'
    and files' bytes are blobs by content hash, out of undo, so a step holds
    hashes rather than pixels. Blobs nothing names are swept on the next open,
    when no step can need them.
  - Tiles are compressed with `zlib-rs` at level 3, in parallel.
- **In memory** the project is an immutable value built from `imbl` maps, each
  page holding its tree as plain values. An edit copies a page the first time it
  changes it, and the pages it leaves alone stay shared. An index of which page
  holds each layer, a block's mask and clean-up included, is derived from the
  pages and never stored. Reading takes no
  lock and composes nothing, because every layer holds only its values.
- **Edits.** `Session::apply` runs one `Edit`, its undo label and its ops, from
  whoever made it: the editor, the pipeline, or an agent.
  - It copies the project in O(1), runs the ops on the copy, and writes what
    differs from the current project in one transaction. If anything fails,
    nothing is written and the project is as it was.
  - A patch is a JSON Merge Patch [RFC 7396] over the layer's values, parsed
    back into the layer's types, so a misspelt property or a value of the
    wrong shape fails and the layer stays as it was. A patch can't name the
    layer's id, children, kind, or attributes. A text's `content`, a block's
    translation included, changes only through the text ops; a block's
    source takes its wording from a patch, as OCR writes it.
  - An edit that changes nothing adds no undo step.
- **Undo** keeps the project as it was before each edit. Undo and redo write
  the difference back and make that project current, so they can't conflict.
  `History` lists every step's label, oldest first, and how many the project
  holds, and `Session::go_to` reaches any of them in one write, as selecting a
  state in Photoshop's History panel does. History keeps the 20 most recent
  steps, Photoshop's default, and lasts as long as the session, as closing a
  Photoshop document clears its states [Photoshop History panel]. Every edit is
  committed before `apply` returns, so there's no save.
- **Identity** is a stored id on each page and layer, kept through undo, so
  selection, caches, and jobs outside the document key on it.
- **Pixel hashes.** `Session::tiles_hash` hashes a picture's tile positions and the
  BLAKE3 hashes of their bytes, so the renderer can tell that pixels changed
  without decoding them.
- **Change** names, by id, the pages and layers an edit, undo, or redo touched,
  and what an edit created, from the difference between the two projects.
- **Opening** reads every row into its types, so a value of the wrong shape, a
  misspelt property, or an unknown kind fails to open, as do a tile out of
  range, a missing blob, and a file that isn't a project, which is left
  unchanged. The tree isn't checked again: the ops keep its rules, which kind
  holds which among them, and nothing else writes the file. A nested tree can't
  hold a cycle or a layer in two places.
- **Versioning.** The project `version` is a SemVer version [SemVer] and
  changes only when the core does. A new layer kind or core property raises the
  minor version, and a reader opens any version up to its own with the same
  major version, since an addition needs no decoder. A build that writes to an
  older file raises its version to its own, so an older build refuses what it
  can't read as too new. A change that removes or
  redefines core data raises the major version, and once a release has written
  a version, the change ships an upgrade to it. The patch stays 0, since stored
  data has no bug fixes. A newer version, a pre-release, or unknown core data
  fails to open; unknown attributes never do.

## Examples

Each example uses only this crate and runs with
`cargo run -p koharu-scene --example <name>`:

- `project`: create a project, import a page, add one layer of every kind,
  panels, balloons, and blocks included, write pixels, read everything back,
  and reopen (one session at a time).
- `editing`: what the editor does: patch, lock, edit text, paint a raster and a
  block's mask, duplicate, move, and remove, refresh from a change, undo and
  redo, and the errors an edit can fail with.
- `models`: a model's run recording its C2PA action, a rerun replacing the
  user's change, and the user's change replacing a model's.
- `pipeline`: detection, OCR, inpainting, and translation writing
  a page's panels, balloons, and blocks as [section
  4](#4-how-the-translation-workflow-uses-it) lays it out, and a rerun
  replacing what a model made.
- `sharing`: one session shared by the editor and a background model, with an
  edit that changes nothing.
- `tools`: what an agent or MCP client does: list and call the tools, and read
  their answers, which are the project's own values.

## Sources

Read in October and November 2026.

- Adobe: [PSD spec], [Photoshop Background], [Photoshop type], [Photoshop
  characters], [Photoshop text in shapes], [Photoshop smart objects],
  [Photoshop Smart Object copies], [Photoshop artboards], [Photoshop Generative
  Fill], [Photoshop Content-Aware Fill], [Photoshop Content Credentials],
  [Photoshop leading], [Photoshop History panel], [Photoshop combine
  variations], [Lightroom masking],
  [InDesign auto-size], [InDesign alignment], [Acrobat OCR]
- Clip Studio Paint: [CSP pages], [CSP page size], [CSP layers], [CSP layer
  masks], [CSP file objects], [CSP text], [CSP text editing], [CSP vertical
  text], [CSP balloons], [CSP text frame], [CSP text tool], [CSP frames], [CSP
  Story Editor], [CSP file format]
- Figma: [Figma nodes], [Figma node properties], [Figma property types],
  [Figma image paint], [Figma text resize], [Figma text properties], [Figma
  plugin data], [Figma multiplayer]
- Microsoft: [PowerPoint placeholders], [PowerPoint autofit], [OOXML anchor],
  [OOXML autofit], [Publisher copyfitting]
- Google and Apple: [Google Slides fitting], [Keynote autosize], [SwiftUI
  minimum scale]
- W3C: [SVG structure], [SVG rendering], [SVG paths], [SVG image], [SVG text
  inside shapes dropped], [CSS masking], [CSS text], [CSS align], [CSS
  line-height]
- Document and translation formats: [PAGE-XML], [XLIFF]
- Lettering and manga translation: [Comics Kingdom], [Manga balloon basics],
  [Search and Destroy letters], [Manga translation II]
- Others: [OpenRaster], [Krita layers], [Krita file], [Krita tiles], [GIMP
  XCF], [GIMP parasites], [Blender custom properties], [C2PA actions],
  [SemVer], [RFC 7396]

[PSD spec]: https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
[Photoshop Background]: https://web.archive.org/web/20241224024227/https://helpx.adobe.com/photoshop/using/layer-basics.html
[Photoshop type]: https://helpx.adobe.com/photoshop/desktop/text-typography/type-layers-creation/type-layers-in-photoshop.html
[Photoshop characters]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/format-characters.html
[Photoshop text in shapes]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/add-text-along-paths-or-inside-shapes.html
[Photoshop smart objects]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/smart-objects/export-the-contents-of-an-embedded-smart-object.html
[Photoshop Smart Object copies]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/smart-objects/duplicate-an-embedded-smart-object.html
[Photoshop artboards]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/layout-design-tools/get-started-artboards.html
[Photoshop Generative Fill]: https://helpx.adobe.com/photoshop/desktop/create-open-import-images/create-images/edit-images-with-generative-fill.html
[Photoshop Content-Aware Fill]: https://helpx.adobe.com/photoshop/desktop/repair-retouch/remove-objects-fill-space/adjust-content-aware-fill-settings.html
[Photoshop Content Credentials]: https://helpx.adobe.com/photoshop/desktop/save-and-export/metadata-content-credentials/use-content-credentials.html
[Lightroom masking]: https://helpx.adobe.com/lightroom-classic/desktop/process-and-develop-photos/masking.html
[InDesign auto-size]: https://web.archive.org/web/20260209035437/https://helpx.adobe.com/indesign/using/text-frame-fitting.html
[InDesign alignment]: https://helpx.adobe.com/indesign/using/aligning-text.html
[Photoshop leading]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/adjust-line-and-character-spacing.html
[Photoshop History panel]: https://helpx.adobe.com/photoshop/desktop/get-started/set-up-toolbars-panels/history-panel-overview.html
[CSP pages]: https://help.clip-studio.com/en-us/manual_en/570_pages/Management_Files_and_Page_Files.htm
[CSP page size]: https://help.clip-studio.com/en-us/manual_en/210_file/Change_canvas_size_and_settings.htm
[CSP layers]: https://help.clip-studio.com/en-us/manual_en/180_layers/Basic_operations.htm
[CSP layer masks]: https://help.clip-studio.com/en-us/manual_en/180_layers/Layer_masks.htm
[CSP file objects]: https://tips.clip-studio.com/en-us/articles/1564
[CSP text]: https://help.clip-studio.com/en-us/manual_en/480_text/Adding_text.htm
[CSP text editing]: https://help.clip-studio.com/en-us/manual_en/480_text/Editing_text.htm
[CSP vertical text]: https://help.clip-studio.com/en-us/manual_en/480_text/Vertical_text_and_readings.htm
[CSP balloons]: https://help.clip-studio.com/en-us/manual_en/540_comic/Balloons.htm
[CSP text frame]: https://help.clip-studio.com/en-us/manual_en/810_subtools/L.htm
[CSP text tool]: https://help.clip-studio.com/en-us/manual_en/810_subtools/E.htm
[CSP frames]: https://help.clip-studio.com/en-us/manual_en/540_comic/Frames_and_Panels.htm
[CSP Story Editor]: https://help.clip-studio.com/en-us/manual_en/570_pages/Use_Story_Editor.htm
[CSP file format]: https://docs.rs/crate/csp-format/latest/source/src/lib.rs
[Acrobat OCR]: https://helpx.adobe.com/au/acrobat/desktop/create-documents/scan-documents-to-pdfs/recognize-text.html
[Photoshop combine variations]: https://www.adobe.com/learn/photoshop/web/combine-variations-generative-fill
[OOXML autofit]: https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.normalautofit?view=openxml-2.16.0
[Figma nodes]: https://developers.figma.com/docs/rest-api/file-node-types/
[Figma node properties]: https://developers.figma.com/docs/plugins/api/node-properties/
[Figma property types]: https://developers.figma.com/docs/rest-api/file-property-types/
[Figma image paint]: https://developers.figma.com/docs/plugins/api/Paint/
[Figma text resize]: https://developers.figma.com/docs/plugins/api/properties/TextNode-textautoresize/
[Figma text properties]: https://help.figma.com/hc/en-us/articles/360039956634-Explore-text-properties
[PowerPoint placeholders]: https://support.microsoft.com/en-gb/powerpoint/add-text-to-a-slide
[PowerPoint autofit]: https://support.microsoft.com/en-us/powerpoint/split-body-text-between-two-slides
[OOXML anchor]: https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.bodyproperties.anchor
[Publisher copyfitting]: https://support.microsoft.com/en-au/publisher/turn-on-and-off-automatic-copyfitting
[Google Slides fitting]: https://support.google.com/docs/answer/10364036
[Keynote autosize]: https://support.apple.com/guide/keynote/change-the-font-or-font-size-tane1c9a464f/mac
[SwiftUI minimum scale]: https://developer.apple.com/documentation/swiftui/view/minimumscalefactor(_:)
[Figma plugin data]: https://developers.figma.com/docs/plugins/api/properties/nodes-setsharedplugindata/
[Figma multiplayer]: https://www.figma.com/blog/how-figmas-multiplayer-technology-works/
[SVG structure]: https://www.w3.org/TR/SVG2/struct.html
[SVG rendering]: https://www.w3.org/TR/SVG2/render.html
[SVG paths]: https://www.w3.org/TR/SVG2/paths.html
[SVG image]: https://www.w3.org/TR/SVG2/embedded.html#ImageElement
[SVG text inside shapes dropped]: https://github.com/w3c/svgwg/issues/1067
[CSS masking]: https://www.w3.org/TR/css-masking-1/#the-mask-composite
[CSS text]: https://www.w3.org/TR/css-text-3/#text-align-property
[CSS align]: https://www.w3.org/TR/css-align-3/#distribution-block
[CSS line-height]: https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/line-height
[PAGE-XML]: https://www.primaresearch.org/schema/PAGE/gts/pagecontent/2019-07-15/pagecontent.xsd
[XLIFF]: https://docs.oasis-open.org/xliff/xliff-core/v2.1/os/xliff-core-v2.1-os.html#segment
[Comics Kingdom]: https://comicskingdom.com/trending/blog/2024/07/22/do-you-speak-comics-part-2-balloons-and-bubbles
[Manga balloon basics]: https://www.linkedin.com/pulse/manga-lettering-balloon-basics-phil-christie
[Search and Destroy letters]: https://yattatachi.com/the-manga-letters-search-and-destroy
[Manga translation II]: https://wordswithoutborders.org/read/article/2009-02/manga-translation-only-poets-need-apply-part-ii/
[OpenRaster]: https://www.openraster.org/baseline/layer-stack-spec.html
[Krita layers]: https://docs.krita.org/en/reference_manual/layers_and_masks.html
[Krita file]: https://invent.kde.org/graphics/krita/-/blob/master/plugins/impex/libkra/kis_kra_savexml_visitor.cpp
[Krita tiles]: https://invent.kde.org/graphics/krita/-/blob/master/libs/image/tiles3/swap/kis_tile_compressor_2.cpp
[GIMP XCF]: https://developer.gimp.org/core/standards/xcf/
[GIMP parasites]: https://developer.gimp.org/core/specifications/parasites/
[Blender custom properties]: https://docs.blender.org/manual/en/latest/files/custom_properties.html
[C2PA actions]: https://spec.c2pa.org/specifications/specifications/2.2/specs/C2PA_Specification.html#_actions
[SemVer]: https://semver.org/spec/v2.0.0.html
[RFC 7396]: https://www.rfc-editor.org/rfc/rfc7396
