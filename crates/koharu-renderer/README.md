# koharu-renderer

Koharu's one renderer.

It draws a page of a project with vello_gpu [vello]: it lays out the text,
composites the layers, and turns the page into pixels. The canvas, image
export, PSD export, thumbnails, and the pipeline's painted areas all draw on
the GPU through one painter, so
every export matches what the editor shows, and Koharu needs a GPU to draw.
Where it follows an established product or standard, it cites it; the links
are under [Sources](#sources).

## Goal

There's one answer to what a page looks like. A text's size, its line breaks,
and its place in its balloon are worked out here at every render, and nowhere
else, so nothing can store a result that goes stale.

## 1. Frames

- **A frame is a page at one moment.** It holds the page's background and its
  layers as they were when it rendered, can't change, and is cheap to share,
  so a view draws it again without touching the project.
- **What changed is all that's built again.** A layer's content is kept by
  what it depends on, such as its text and the fonts, so a render after an edit
  reuses every layer the edit didn't touch.
- **Only what a view needs is drawn**, at the scale it needs: a view is a
  rectangle of the page at a zoom, and the page's pixels come at that zoom's
  level of detail.
- **What each layer drew can be asked**: its bounds, whether it shows, a text's
  resolved layout, its masks merged into one, and its pixels alone, as a PSD's
  pixel layer needs them.
- **A block** draws its clean-up through its mask, then its translation, each
  apart, as PSD export takes them apart. A panel and a balloon draw their
  children as a group does, and no outline is drawn.
- **Limits.** A page or an imported image may be up to 32,768 pixels on a side
  and 268,435,456 pixels in all.

## 2. Text

A text is cut into runs of one span, font, script, direction, and orientation,
each shaped once with HarfRust [HarfRust]. Lines are measured from those
glyphs, fitted to the frame, and painted. Lettering's conventions win over
typesetting's: one size held across a page, whole words before hyphens, and
lines centred on one axis in a balloon, or started together on it.

| Frame | Lines |
|---|---|
| point | break only where the text does |
| box | wrap inside the box; centred text is balanced |
| path | fill a closed path: every line count its height holds is tried and the cheapest that fits wins, and every line is centred on one axis, or, aligned to an edge, starts or ends with the others in a block centred on it |

| Part | Model | Cite |
|---|---|---|
| breaking | where Unicode allows a line to break, chosen over the whole text rather than one line at a time; Chinese by word | Unicode line breaking [UAX 14], through ICU4X [ICU4X]; Chinese words by jieba [jieba] |
| fitting | `shrink` lowers the size just enough to fit, down to half; `best` also grows it to the largest size that fits, and a word too wide for its line stops the growth; past every step at half size, `best` shrinks on until the text fits | the scene's fit modes ([koharu-scene](../koharu-scene/README.md#text)) |
| hyphens | whole words shrink first, and only at half size does a word too wide for its line hyphenate; a word never breaks without a hyphen, and what still doesn't fit runs past its frame whole, unless it fits best | Comicraft avoids hyphenation wherever it can [Comicraft glossary] |
| hyphenation | words of more than five letters, two letters kept on each side, at most two lines in a row ending with a hyphen | Photoshop's defaults [Photoshop hyphenation] |
| a balloon's edge | a path's inset halves at half size, short of letters on the border, which read as cut off | |
| vertical text | columns run right to left; what Unicode sets sideways in vertical text, such as Latin letters and digits, turns a quarter clockwise, and so does what it transforms or else rotates, such as `〜`, in a font without a vertical form for it; `!` and `?` set full width, and a run of two or three sits upright in one cell, as does a number of one or two digits outside a Latin word; a hyphen, a dividing mark, a middle dot, or an inseparable mark, such as `〜`, `！`, `・`, `：`, `—`, or `…`, and Chinese's `·`, `⋯`, and `～`, centres its ink across the column; `、` and `。` keep their font's vertical forms in the column's corner, but Chinese centres its commas and full stops in their cells; ASCII `:` and `;` stay on a sideways run's baseline | Unicode's Vertical_Orientation [UAX 50]; CSS's `text-orientation: mixed` and `text-combine-upright: digits 2` [CSS writing modes]; JLREQ's character classes cl-03 to cl-08 and its Western characters, cl-27 [JLREQ]; CLREQ's interpunct, ellipsis, and connector, and its stops in the middle of their cells as Taiwan and Hong Kong set them [CLREQ] |
| line head | a line opening with a full-width opening bracket, such as `「` or `『`, starts at the bracket's ink, without the half em before it | JLREQ counts an opening bracket half an em wide with half an em of space before it, and starts every line with no space, as JIS X 4051 does [JLREQ] |
| right to left | a line's runs show in the order the bidirectional algorithm gives, and a right-to-left paragraph's lines start on the right | [UAX 9] |
| strokes | under every fill, so a stroke never covers a neighbouring letter | |
| overflow | text that nothing holds still draws, past its frame | |
| typing | a text laid out as a render lays it out says where a caret before any character stands, what a selection covers, which character a point lands nearest, and each line's characters; a ligature's characters share its width evenly, and an empty text has a caret a line tall where its first line would start | |

A path frame centres every line on one axis, where Photoshop centres each line
in its own span of the shape [Photoshop text in shapes], so the lines of a
lopsided balloon don't wander along its outline; the axis is one every line
reaches where there is one, so no line is left without room. Lines aligned to
an edge start together, or end together, where Photoshop starts each where the
shape does, so a balloon's Japanese or Chinese columns hang from one line as
JLREQ aligns lines to their head [JLREQ]: they share the edge every line's span
reaches, each runs from it to its own span's far end, so a balloon cut away on
one side still fills its longer side, and the block is centred on the axis as
far as that allows.

## 3. Fonts

| Part | Model | Cite |
|---|---|---|
| chain | a span's family, then the preferred families, then Arial, then the system's fallback for the text's scripts in its language, then symbol and emoji families | the system's fallback through fontique [fontique] |
| language | a text with none takes Japanese from kana and Korean from Hangul, and Han alone falls back as Simplified Chinese, so Chinese never takes the system language's font, such as a Japanese one | |
| graphemes | fallback never splits one, and a character no font in the chain has is listed for the inspector | |
| marks | a character of no script of its own, such as a mark, a digit, or a space, takes the font of the text beside it when that font has it, so a CJK sentence's marks come from its CJK font even where a Latin font earlier in the chain has them | |
| bundled fonts | a family of the chain the system lacks can come from Koharu's font dataset, in its place in the chain, pinned to one commit so its index, faces, and previews match | the `mayocream/fonts` dataset [Koharu fonts] |
| font downloads | a face a text needs downloads as the text lays out, and the render waits for it; a family the dataset lacks, or one that fails to download, falls back down the chain | |

## Sources

Read in November 2026.

- Libraries: [vello], [HarfRust], [ICU4X], [jieba], [fontique], [Koharu fonts]
- Unicode and W3C: [UAX 14], [UAX 9], [UAX 50], [CSS writing modes], [JLREQ], [CLREQ]
- Adobe: [Photoshop hyphenation], [Photoshop text in shapes]
- Lettering: [Comicraft glossary]

[vello]: https://github.com/linebender/vello
[HarfRust]: https://github.com/harfbuzz/harfrust
[ICU4X]: https://github.com/unicode-org/icu4x
[jieba]: https://github.com/fxsjy/jieba
[fontique]: https://github.com/linebender/parley/tree/main/fontique
[Koharu fonts]: https://huggingface.co/datasets/mayocream/fonts
[UAX 14]: https://www.unicode.org/reports/tr14/
[UAX 9]: https://www.unicode.org/reports/tr9/
[UAX 50]: https://www.unicode.org/reports/tr50/
[CSS writing modes]: https://www.w3.org/TR/css-writing-modes-4/
[JLREQ]: https://www.w3.org/TR/jlreq/
[CLREQ]: https://www.w3.org/TR/clreq/
[Photoshop hyphenation]: https://developer.adobe.com/photoshop/uxp/2022/ps_reference/objects/options/hyphenationproperties/
[Photoshop text in shapes]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/add-text-along-paths-or-inside-shapes.html
[Comicraft glossary]: https://balloontales.com/?p=418
