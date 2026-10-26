# koharu-renderer

`koharu-renderer` draws a page of a [`koharu-scene`](../koharu-scene/README.md)
project with [vello]. The desktop canvas, PNG and CBZ export, and PSD export all
draw the same `Frame`, so every export writes what the editor shows. The scene
README says what a page holds and means. This crate lays out its text,
composites its layers, and turns it into pixels.

## Ownership

| Owner | Owns |
|---|---|
| `koharu-scene` | what the page holds and means; the renderer reads a `Session` and never writes to it |
| `Renderer` | fonts (discovery, the bundled fonts, fallback, and their caches), decoded files, raster tiles, retained layer scenes, and the one wgpu device and vello renderer that export and the canvas share |
| `Frame` | one page at one revision, immutable: its layers' scenes, bounds, and resolved text; cloning it is cheap |
| the desktop canvas | showing frames in the window under the web UI, which CEF paints off-screen [CEF OSR]; camera, tools, hit testing, selection, overflow marks, and transient previews |

## API

```rust,ignore
let renderer = Renderer::new(typesetting);

let frame = renderer.render(&session, page)?;

// The canvas draws what one view of the page needs.
frame.draw(&mut scene, View { rect, scale });

// Exports.
let composite = renderer.rasterize(&frame)?;
let own = renderer.rasterize(&frame.cropped(layer)?)?;
if let Some(mask) = frame.mask(layer)? {
    let coverage = renderer.rasterize(&mask)?;
}
let text = frame.layer(layer).and_then(Layer::text);

let families = renderer.available_fonts()?;
let preview = renderer.font_preview(family)?;
```

- `render` is synchronous. Files and pixel tiles live in memory in the Loro
  document, so drawing does no storage I/O. The caller renders on a worker
  thread while it holds a shared borrow of the session, which is `Sync`, and
  the renderer spreads decoding and layout over rayon.
- `render` reuses every retained layer whose inputs are unchanged
  ([Retention](#retention)), so the canvas renders again after every change,
  whoever made it.
- `rasterize` renders at the page's pixel size and returns RGBA with its
  page-space origin. A page and a cropped layer give the same `Raster` type.
- The renderer follows the typesetting preference as it changes; a new font
  chain rebuilds text.

## Drawing

| Scene | Drawn as |
|---|---|
| page | a transparent surface of the page's `size`, with layers drawn first to last |
| group | a vello layer with the group's blend mode and opacity: its children composite first, then the group blends as one image, as Photoshop composites a group in any mode but Pass Through [Photoshop blending] |
| pass-through group | its children blend with the layers below the group, as Photoshop's Pass Through [Photoshop blending] and Clip Studio Paint's Through [CSP through] do |
| raster | its tiles at its `offset` |
| image | the file, decoded once per content hash, cut into tiles, and drawn through its `transform` |
| shape | the path filled with its fill rule; a stroke is centred on the path, as Clip Studio Paint draws vector lines [CSP vector lines] and Photoshop's Center alignment does, with miter joins and butt caps, two of Photoshop's stroke options [Photoshop shape strokes] |
| text | glyph runs from [Text](#text), drawn through its `transform` |
| hidden layer | nothing, and nothing inside it |

The scene's blend modes are W3C Compositing's separable and non-separable modes,
which vello's `Mix` implements one for one [W3C compositing].

Neither Photoshop nor Clip Studio Paint documents how a pass-through group's own
opacity and masks apply. The renderer draws the backdrop again inside the
group's layer, as a non-isolated group includes its backdrop [W3C compositing],
so the group's opacity and masks fade between the backdrop and the result.

A layer's masks are never drawn. They build one coverage in a vello layer, and
the layer's content is composited into it with `DestIn`. Components apply in
order, as the scene README says: add with `SrcOver`, subtract with `DestOut`,
intersect with `DestIn`, starting empty when the first one adds and full
otherwise. A pixel mask's grey becomes coverage, and a path mask fills its path.
A hidden component is left out, as Photoshop disables a mask [Photoshop disable
mask] and Clip Studio Paint unchecks Enable Mask [CSP masks].

## Rasters and images

vello 0.11 draws every image in a frame from one atlas of at most 8192×8192
pixels [vello atlas], skips an image that doesn't fit [vello overflow], and
copies even a registered wgpu texture into that atlas each frame [vello
textures]. A whole page wider or taller than 8192 pixels could never show, and
an A4 page at 600 dpi (4961×7016) fits only one full-page image per frame. So:

- Rasters, pixel masks, and decoded images are cut into 512×512 tiles,
  assembled from the scene's 64×64 tiles. Each tile carries a one-pixel border
  from its neighbours, so filtered sampling leaves no seams.
- Each layer's tiles have levels at half, quarter, and smaller scales, down to
  a single tile, built the first time a view needs them.
- A pass draws only the tiles its rectangle meets, at the smallest level at or
  above the pass's scale.
- A pass whose tiles hold more pixels than the atlas is split into strips that
  each fit. The canvas at any zoom and an export at full size take the same
  path.

## Text

The renderer keeps its own shaping and line breaking (HarfRust, ICU4X
segmentation, and fontique) because Parley, the rich-text layout crate on the
same stack, has no vertical writing yet [Parley vertical text], and much manga
text is vertical.

Each value comes from a span's mark, else the text's `style`, else its text
style, else this table:

| Value | When unset |
|---|---|
| family | the typesetting preference's font chain, then fontique's fallback for the text's script and `language` |
| weight | the family's regular face, weight 400 [CSS font-weight] |
| italic | upright |
| size | 24 px; Photoshop picks its default from the document's resolution and zoom [Photoshop type defaults], and Clip Studio Paint doesn't document one |
| fill | opaque black |
| stroke | none |
| `line_height` | 1.2, Photoshop's Auto leading of 120% [Photoshop spacing] |

The scene stores no kerning or tracking. Kerning uses the font's own pairs, as
Photoshop's Metrics kerning does [Photoshop spacing], and tracking is 0.

### Frames

- **Point text** breaks lines only where the text has a line break, as
  Photoshop's point type treats each line as a paragraph [Photoshop
  paragraphs].
- **Box text** wraps at the box's inline edges.
- **Path text** sets each line across the inside of the closed path at that
  line, as Photoshop sets type inside a path [Photoshop type in a path], less
  half the size on each side: about one letter width, the air letterers leave
  inside a balloon [Comicraft glossary]. The whole height of the line must fit,
  so curved edges keep their air too. Where a line meets the inside in several
  spans, it takes the widest, as CSS Exclusions' `wrap-flow: maximum` flows
  only beside "the largest available space" [CSS exclusions], so a line never
  reads across a gap. SVG 2 instead fills every span of a broken line [SVG
  broken lines], which would carry a line across a balloon's neck.
- **Several texts in one balloon** each fit their own frame; they share no
  layout, and each shrinks on its own, so their drawn sizes can differ.
- **Anchor.** The block of lines sits at the start, centre, or end of the
  frame's block axis: top, middle, or bottom for horizontal text, and right,
  middle, or left for vertical text.

### Fit

Box and path text that doesn't fit its frame goes through these steps in
order, each only when the ones before can't make it fit:

1. **Breaks between words**, at the set size, including after a hyphen the
   text already has.
2. **Shrinking**, for `fit: shrink`: every span at one scale, the largest that
   fits, never below half the set size. It fits again on every render, so
   typing, deleting, restyling, and resizing the frame all refit live, and a
   shortened text grows back to its set size, as PowerPoint fits placeholder
   text "as you type" [PowerPoint placeholders].
3. **Hyphenation** by the text's `language`, at the largest size it lets the
   text fit. Letterers hyphenate only when they must: Blambot advises avoiding
   it "if at all possible, and even then, only if it's a compound word that
   breaks well" [Blambot grammar], and a manga letterer divides and hyphenates
   "longer words as necessary" [Manga lettering]. A word is split only if it
   has more than five letters, keeping at least two on each side, and at most
   two lines in a row end with a hyphen, as Photoshop's hyphenation defaults
   are [Photoshop hyphenation]. CJK text needs none: it already breaks between
   characters.
4. **Breaking inside a word** with no hyphenation point, such as a long scream
   or a URL, between any two characters, as CSS `overflow-wrap: anywhere`
   breaks "an otherwise unbreakable string of characters" when nothing else
   prevents overflow [CSS overflow-wrap], so the word stays inside its balloon.
5. **Overflow.** What still doesn't fit is drawn past the frame and never
   clipped, as PowerPoint lets text "overfill the box" [PowerPoint autofit] and
   CSS shows overflow by default [CSS overflow]. Hiding it would drop translated
   words from an export without a trace.

`fit: none` skips step 2. With `fit: grow`, the box keeps its inline size and
its block size follows the text, growing away from the anchor: down from a top
anchor, both ways from a middle one, and up from a bottom one; only a word wider
than the box reaches steps 3 to 5. The text reports its scale, its hyphens and
breaks inside words, and whether it overflowed, and the canvas marks each:
Vertical shrinks a balloon's text only when rephrasing can't make it fit
[Manga translation II], and a hyphen is as much a last resort.

### Lines

- **Spans.** Each span is shaped with its own font and size, and a mark wins
  over `style` for its span. Lines break across spans, and a line's leading
  comes from its largest span.
- **Breaks** fall at Unicode line-break opportunities. A break after
  punctuation is preferred to one inside a phrase, as subtitle guides ask
  [BBC subtitles].
- **Shape.** Breaks are chosen over the whole paragraph, not line by line:
  - Path text fills every line in the same proportion to its span, so in a
    balloon the block takes the outline's round shape: "the center line of text
    should be the widest, and each line above or below it should be slightly
    narrower" [Comicraft word arrangement]. A last line shorter than the lines
    above it is avoided, the "parking space" letterers remove [Comicraft
    glossary].
  - Centred box text evens out its line lengths, as InDesign suggests
    balancing centred paragraphs [InDesign alignment] and CSS `text-wrap:
    balance` does [CSS text-wrap].
- **Direction.** Vertical text sets columns right to left with the font's
  vertical forms (`vert` and `vrt2`). Chinese and Japanese text uses CJK
  punctuation layout.
- **Alignment** works along the inline axis: left, centre, or right in
  horizontal text, and top, centre, or bottom in vertical text, as in Photoshop
  [Photoshop paragraphs] and Clip Studio Paint [CSP text frame].
- **Justify** stretches every line but the last to both edges and sets the last
  at the start, in both directions: Photoshop's Justify last left (top in
  vertical type) [Photoshop paragraphs] and Clip Studio Paint's Align last line
  left [CSP text frame].
- **Stroke.** A character stroke is drawn under the fill at twice its width, so
  `stroke_width` of it shows outside the glyphs, as Photoshop's Outside stroke
  position [Photoshop stroke] and Clip Studio Paint's text edge [CSP text edge]
  sit outside the characters.

### Resolved text

`Layer::text` gives the laid-out text as runs: each run's UTF-16 range, the
PostScript name of the face that drew it, the size it was drawn at, its fill,
and its stroke and width. It also gives the box the text was laid out in, the
scale `shrink` drew it at, where it hyphenated or broke inside a word, and
whether it overflowed. The canvas's inspector
shows the set size beside the drawn one. A fallback face gets a run of its own,
so the PSD's type layer names every font it uses.

## Retention

A render assembles the frame from the retained scenes of the layers that draw,
and composes groups and masks around them. A layer's scene is reused while its
key is unchanged:

| Layer | Key |
|---|---|
| text | the layer's hash (the hash of `""`, which its view carries), its text style's values, and the typesetting preference |
| shape | the layer's hash |
| image | the layer's hash; the decoded file is cached by its content hash |
| raster, pixel mask | the hash of the layer's stored tiles, which the scene reports |

Keys are content, so a frame is right whoever edited the session (the user, the
agent, an MCP client, or the pipeline), and no change log has to reach the
renderer. Caches are bounded, and eviction costs time, never output.

## Exports

- PNG and CBZ rasterize the frame.
- PSD export takes the composite, each layer's `cropped` frame, each masked
  layer's `mask` frame, and the resolved text. `cropped` draws only the layer's
  own content, without its opacity, blend mode, or masks, which the PSD records
  as layer properties. `mask` draws the layer's components merged into one
  coverage.

## Modules

| Module | Owns |
|---|---|
| `renderer` | `Renderer`, the scene walk, and the caches |
| `frame` | `Frame`, `Layer`, drawing a view, `cropped`, and `mask` |
| `tiles` | raster tiles, their levels, and strips |
| `text` | spans to layout, frames, fitting, glyph runs, and resolved text |
| `layout`, `shape`, `segment`, `script` | line breaking, shaping, Unicode segmentation, and scripts |
| `fonts` | discovery, the bundled fonts, fallback, and previews |
| `images` | decoding files, with their sizes checked first |
| `raster` | the wgpu device, vello rendering, and readback |
| `config` | the typesetting preference |
| `error` | errors that keep their causes |

A page is at most 32,768 pixels on a side and 268,435,456 pixels in all.

## Checks

```text
cargo check -p koharu-renderer --all-targets
cargo test -p koharu-renderer
```

GPU tests run only on a machine with a usable adapter.

## Sources

- Adobe: [Photoshop blending], [Photoshop shape strokes], [Photoshop disable
  mask], [Photoshop type defaults], [Photoshop spacing], [Photoshop
  paragraphs], [Photoshop type in a path], [Photoshop stroke], [Photoshop
  hyphenation], [InDesign alignment]
- Celsys: [CSP through], [CSP vector lines], [CSP masks], [CSP text frame],
  [CSP text edge]
- Microsoft: [PowerPoint placeholders], [PowerPoint autofit]
- Lettering, manga translation, and subtitles: [Comicraft glossary],
  [Comicraft word arrangement], [Blambot grammar], [Manga lettering], [Manga
  translation II], [BBC subtitles]
- vello: [vello atlas], [vello overflow], [vello textures]
- Linebender: [Parley vertical text]
- W3C and MDN: [W3C compositing], [CSS font-weight], [CSS overflow], [CSS
  overflow-wrap], [CSS text-wrap], [CSS exclusions], [SVG broken lines]
- Tauri: [CEF OSR]

[vello]: https://github.com/linebender/vello
[Photoshop blending]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/apply-layer-effects/set-layer-opacity-and-blending-modes.html
[Photoshop shape strokes]: https://helpx.adobe.com/photoshop/desktop/draw-shapes-paths/create-shapes/set-and-manage-shape-stroke-options.html
[Photoshop disable mask]: https://helpx.adobe.com/photoshop/desktop/create-masks/layer-masks/disable-or-enable-layer-masks.html
[Photoshop type defaults]: https://web.archive.org/web/20241226200333/https://helpx.adobe.com/photoshop/using/editing-text.html
[Photoshop spacing]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/adjust-line-and-character-spacing.html
[Photoshop paragraphs]: https://helpx.adobe.com/photoshop/desktop/text-typography/characters-glyphs/format-paragraphs.html
[Photoshop type in a path]: https://web.archive.org/web/20241213102001/https://helpx.adobe.com/photoshop/using/creating-type-effects.html
[Photoshop stroke]: https://helpx.adobe.com/photoshop/desktop/create-manage-layers/apply-layer-effects/layer-style-effects-and-options-overview.html
[Photoshop hyphenation]: https://developer.adobe.com/photoshop/uxp/2022/ps-reference/objects/options/hyphenationproperties
[InDesign alignment]: https://helpx.adobe.com/indesign/using/aligning-text.html
[CSP through]: https://help.clip-studio.com/en-us/manual_en/390_filters/Tonal_Correction.htm
[CSP vector lines]: https://tips.clip-studio.com/en-us/articles/600
[CSP masks]: https://help.clip-studio.com/en-us/manual_en/180_layers/Layer_masks.htm
[CSP text frame]: https://help.clip-studio.com/en-us/manual_en/810_subtools/L.htm
[CSP text edge]: https://help.clip-studio.com/en-us/manual_en/480_text/Editing_text.htm
[PowerPoint placeholders]: https://support.microsoft.com/en-gb/powerpoint/add-text-to-a-slide
[PowerPoint autofit]: https://support.microsoft.com/en-us/powerpoint/split-body-text-between-two-slides
[Comicraft glossary]: https://balloontales.com/the-comicraft-glossary-of-lettering-terms/
[Comicraft word arrangement]: https://balloontales.com/word-arrangement/
[Blambot grammar]: https://blambot.com/pages/comic-book-grammar-tradition
[Manga lettering]: https://swet.jp/members/article/manga_lettering_and_translation
[Manga translation II]: https://wordswithoutborders.org/read/article/2009-02/manga-translation-only-poets-need-apply-part-ii/
[BBC subtitles]: https://www.bbc.co.uk/accessibility/forproducts/guides/subtitles/
[vello atlas]: https://github.com/linebender/vello/blob/v0.11.0/research/vello_encoding/src/image_cache.rs#L10
[vello overflow]: https://github.com/linebender/vello/blob/v0.11.0/research/vello_encoding/src/resolve.rs#L306-L311
[vello textures]: https://github.com/linebender/vello/blob/v0.11.0/research/vello_research/src/lib.rs#L564-L574
[Parley vertical text]: https://github.com/linebender/parley/issues/634
[W3C compositing]: https://www.w3.org/TR/compositing-1/#isolation
[CSS font-weight]: https://www.w3.org/TR/css-fonts-4/#font-weight-prop
[CSS overflow]: https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/overflow
[CSS overflow-wrap]: https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/overflow-wrap
[CSS text-wrap]: https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/text-wrap
[CSS exclusions]: https://www.w3.org/TR/css3-exclusions/#wrap-flow-property
[SVG broken lines]: https://www.w3.org/TR/SVG2/text.html#TextLayoutAutoNotesBrokenLines
[CEF OSR]: https://github.com/tauri-apps/tauri/pull/16225
