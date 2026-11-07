# koharu-renderer

Draws a page of a `koharu-scene` project with vello: it lays out the text, composites the
layers, and turns the page into pixels. The desktop canvas, image export, and PSD export all
draw the same frame, so every export matches what the editor shows.

## API

| Item | What it does |
|---|---|
| `Renderer::new(typesetting)` | A renderer. Cloning it shares its fonts, caches, and GPU, which start on first use |
| `Renderer::render(&session, page)` | A `Frame` of the page at its current revision |
| `Renderer::rasterize(&frame)` | The frame's area at one output pixel per page pixel, as a `Raster` of straight-alpha RGBA and its corner |
| `Renderer::gpu()` | The `Gpu`, which the desktop canvas draws frames with |
| `Renderer::load_fonts(&frame).await` | Downloads the bundled fonts the frame's text asked for |
| `Renderer::available_fonts().await` | Every font family, system and bundled |
| `Renderer::font_preview(family).await` | A WebP of the family's name set in it, 96 pixels tall |
| `Renderer::set_typesetting(config)` | Follows a changed `[typesetting]` preference, and text lays out again |
| `Frame::draw(view)` | The `Strip`s that draw a `View`: a page rectangle at a scale |
| `Frame::layers()`, `layer(id)` | Each `Layer` as the frame drew it: bounds, visibility, and a text's `ResolvedText` |
| `Frame::cropped(id)`, `unstroked(id)` | One layer alone, as a PSD's pixel layer needs it |
| `Frame::mask(id)` | A layer's masks merged into one coverage |
| `Frame::needs_fonts()` | Whether some text draws with a fallback until `load_fonts` |

A frame is immutable and cheap to clone. Layer trees are fixed when it renders, so a view can
be drawn again without touching the session.

## Frames

`render` builds each layer's content in parallel, then assembles the page's tree. Content is
retained by what changes it, such as a layer's hash, its text style's, and the font generation,
so a render after an edit reuses every layer the edit didn't touch. Retained content may take
2 GiB. A page or an imported image may be up to 32,768 pixels on a side and 268,435,456 pixels.

The scene's 64×64 tiles assemble into one image, which is drawn as 512×512 tiles at full size
and at halved levels, so a pass draws only the tiles it meets at the level its scale needs.

## The GPU

`Gpu` holds one wgpu device and vello renderer. Every vello render goes through `Gpu::render`,
which works around two faults of vello 0.11's 8192×8192 image atlas:

- A render with no image, gradient, or glyph frees the atlas while vello still counts its
  images as uploaded ([vello#1948]). Such a scene gets a transparent pixel.
- vello never evicts an image either of the last two renders drew ([vello#2011]). Two renders
  of the pixel alone come first when a render's images wouldn't fit beside theirs.

vello skips an image that doesn't fit the atlas, so one pass draws at most seven eighths of it.
A view whose images need more becomes several strips across it.

[vello#1948]: https://github.com/linebender/vello/issues/1948
[vello#2011]: https://github.com/linebender/vello/issues/2011

## Text

A text layer's spans are shaped with HarfRust, broken into lines, fitted to the frame, and drawn
as glyph runs. `ResolvedText` keeps what that resolved for the inspector and PSD export.

| Frame | Lines |
|---|---|
| Point | Break only where the text does |
| Box | Wrap inside the box. Centred text is balanced, and a paragraph's last line may otherwise stop short |
| Path | Fill a closed path: every line count its height holds is tried and the cheapest that fits wins, each line taking the widest span inside the outline along its band |

- **Breaking.** Line breaks come from ICU4X, with jieba for Chinese. Lines are chosen over the
  whole text rather than one at a time.
- **Fitting.** `Shrink` lowers the size just enough to fit, down to half. A word wider than its
  line is hyphenated before the text shrinks, and breaking anywhere in a word is the last
  resort. Lines that are merely full shrink instead of hyphenating.
- **Hyphenation.** In a language with patterns, words of more than five letters break, keeping two
  letters on each side, and at most two lines in a row end with a hyphen. These are Photoshop's
  defaults.
- **Overflow.** Text that nothing holds still draws, running past its frame.
- **Vertical text.** Columns run right to left. `!` and `?` set in full-width forms, and a run of
  two or three sits upright side by side in one cell. It doesn't hyphenate.
- **Right-to-left text.** Runs are ordered by each byte's bidi level, and a first paragraph that
  runs right to left starts its lines on the right.
- **Strokes.** They draw under every fill, as Photoshop draws an outside stroke under its type.

## Fonts

A span's font chain is its family, then `[typesetting]`'s `font_families`, then Arial, then
fontique's fallback for the text's scripts, then a list of symbol and emoji families. Fallback
never splits a grapheme.

If the system doesn't have the chain's first family, it can come from the `mayocream/fonts`
dataset, pinned to one commit so its index, faces, and previews all match. Faces download only when
`load_fonts` asks, and up to 64 MiB of them stay in memory. Until then the text draws with a
fallback, and `Frame::needs_fonts` says so.
