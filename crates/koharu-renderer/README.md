# koharu-renderer

Draws a page of a `koharu-scene` project with vello: it lays out the text, composites the
layers, and turns the page into pixels. The desktop canvas, image export, and PSD export all
draw the same frame, so every export matches what the editor shows.

## API

| Item | What it does |
|---|---|
| `Renderer::new(config)` | A renderer. Cloning it shares its fonts, caches, and GPU, which start on first use |
| `Renderer::render(&session, page)` | A `Frame` of the page at its current revision: its background, then its layers |
| `Renderer::lay_out(layer, &text)` | A text laid out as a render would, for one nothing draws, such as a block's source |
| `Renderer::rasterize(&frame)` | The frame's area at one output pixel per page pixel, as a `Raster` of straight-alpha RGBA and its corner |
| `Renderer::gpu()` | The `Gpu`, which the desktop canvas draws frames with |
| `Renderer::load_fonts(&frame).await` | Downloads the bundled fonts the frame's text asked for |
| `Renderer::available_fonts().await` | Every font family, system and bundled |
| `Renderer::font_preview(family).await` | A WebP of the family's name set in it, 96 pixels tall |
| `Renderer::set_config(config)` | Follows a changed `[renderer]` preference, and text lays out again |
| `Frame::draw(view)` | The `Strip`s that draw a `View`: a page rectangle at a scale |
| `Frame::layers()` | Everything the frame drew, as `Drawn` handles, parents first |
| `Frame::layer(id)`, `background()`, `translation(block)` | What a layer drew, a block's clean-up among them; the page's background; a block's translation |
| `Drawn::bounds()`, `visible()`, `text()` | What it covers, whether it and everything above it show, and a text's `ResolvedText` |
| `Drawn::cropped()`, `unstroked()` | It alone, as a PSD's pixel layer needs it |
| `Drawn::mask()` | Its masks merged into one coverage |
| `Frame::needs_fonts()` | Whether some text draws with a fallback until `load_fonts` |

A block draws its clean-up through its mask, then its translation, each apart, as PSD export
takes them apart. A panel shows its children only inside its outline, and no outline is drawn.

A frame is immutable and cheap to clone. Layer trees are fixed when it renders, so a view can
be drawn again without touching the session.

## Frames

`render` builds each layer's content in parallel, then assembles the page's tree. Content is
retained by what changes it, such as a text's canonical JSON hash and the font generation, so a render after an
edit reuses every layer the edit didn't touch. Retained content may take 2 GiB. A page or an imported image may be up to 32,768 pixels on a side and 268,435,456 pixels.

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
A view whose images need more becomes several strips across it. A layer that composites apart
is clipped to what it draws: vello's buffers are fixed and give a layer every tile its clip
covers, so a page of layers each clipped to the whole page drew nothing.

[vello#1948]: https://github.com/linebender/vello/issues/1948
[vello#2011]: https://github.com/linebender/vello/issues/2011

## Text

A text layer is cut into runs of one span, font, script, direction, and orientation, and each
run is shaped once with HarfRust. Lines are measured from those glyphs, fitted to the frame,
arranged as positioned glyphs, and painted; a line edge where HarfRust marks breaking unsafe is
shaped again on its own. `ResolvedText` keeps what that resolved for the inspector and PSD export.

| Frame | Lines |
|---|---|
| Point | Break only where the text does |
| Box | Wrap inside the box. Centred text is balanced, and a paragraph's last line may otherwise stop short |
| Path | Fill a closed path: every line count its height holds is tried and the cheapest that fits wins, every line centred on one axis, the one the lines reach furthest on both sides of, each as far as the outline reaches on both sides of it along its band, where Photoshop centres each line in its own span |

- **Breaking.** Line breaks come from ICU4X, with jieba for Chinese. Lines are chosen over the
  whole text rather than one at a time.
- **Fitting.** `Shrink` lowers the size just enough to fit, down to half. `Best` also raises it to
  the largest size that fits, breaking only between words, so a word that outgrows its line
  stops the growth. Whole words shrink first, since Comicraft avoids a hyphen whenever it can.
  At half size a path's inset then halves, short of letters on a balloon's border, which read
  as cut off, and last a word hyphenates where its line can't hold it whole. A word never
  breaks without a hyphen: what still doesn't fit runs past its frame whole.
- **Hyphenation.** In a language with patterns, words of more than five letters break, keeping two
  letters on each side, and at most two lines in a row end with a hyphen. These are Photoshop's
  defaults.
- **Overflow.** Text that nothing holds still draws, running past its frame.
- **Paragraphs.** An empty one keeps its span's leading, as Photoshop's does.
- **Vertical text.** Columns run right to left. What Unicode's Vertical_Orientation rotates, such
  as Latin letters and digits, is set sideways, a quarter turn clockwise, as Photoshop and CSS's
  `text-orientation: mixed` set it. `!` and `?` set in full-width forms, or the font's own
  upright when it has none, and a run of two or three sits upright side by side in one cell.
  Text in a language with hyphenation patterns, such as Latin set sideways, hyphenates as
  horizontal text does; CJK has none.
- **Right-to-left text.** A line's runs show in the order the Unicode Bidirectional Algorithm gives
  their levels, and the lines of a paragraph that runs right to left start on the right.
- **Strokes.** They draw under every fill, as Photoshop draws an outside stroke under its type.

## Fonts

A span's font chain is its family, then `[renderer]`'s `font_families`, then Arial, then
fontique's fallback for the text's scripts in its language, then a list of symbol and emoji
families. A text with no language takes Japanese from kana and Korean from Hangul. Fallback never
splits a grapheme, and a character no font of the chain has is listed in `ResolvedText::uncovered`.

fontique keeps Han's fallback for no language and for Simplified Chinese in one place, and the
first query to ask fills it. The renderer asks for Simplified Chinese first, so Chinese text never
gets the system language's font, such as a Japanese one.

If the system doesn't have the chain's first family, it can come from the `mayocream/fonts`
dataset, pinned to one commit so its index, faces, and previews all match. Faces download only when
`load_fonts` asks, and up to 64 MiB of them stay in memory. Until then the text draws with a
fallback, and `Frame::needs_fonts` says so.
