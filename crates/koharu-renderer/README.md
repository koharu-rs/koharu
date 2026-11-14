# koharu-renderer

Koharu's one renderer.

It draws a page of a project with vello [vello]: it lays out the text,
composites the layers, and turns the page into pixels. The canvas, image
export, and PSD export all draw the same frame, so every export matches what
the editor shows. Where it follows an established product or standard, it
cites it; the links are under [Sources](#sources).

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
  apart, as PSD export takes them apart. A panel shows its children only inside
  its outline, and no outline is drawn.
- **Limits.** A page or an imported image may be up to 32,768 pixels on a side
  and 268,435,456 pixels in all.

## 2. Text

A text is cut into runs of one span, font, script, direction, and orientation,
each shaped once with HarfRust [HarfRust]. Lines are measured from those
glyphs, fitted to the frame, and painted. Lettering's conventions win over
typesetting's: one size held across a page, whole words before hyphens, and
lines centred on one axis in a balloon.

| Frame | Lines |
|---|---|
| point | break only where the text does |
| box | wrap inside the box; centred text is balanced |
| path | fill a closed path: every line count its height holds is tried and the cheapest that fits wins, and every line is centred on one axis |

| Part | Model | Cite |
|---|---|---|
| breaking | where Unicode allows a line to break, chosen over the whole text rather than one line at a time; Chinese by word | Unicode line breaking [UAX 14], through ICU4X [ICU4X]; Chinese words by jieba [jieba] |
| fitting | `shrink` lowers the size just enough to fit, down to half; `best` also grows it to the largest size that fits, and a word too wide for its line stops the growth | the scene's fit modes ([koharu-scene](../koharu-scene/README.md#text)) |
| hyphens | whole words shrink first, and only at half size does a word too wide for its line hyphenate; a word never breaks without a hyphen, and what still doesn't fit runs past its frame whole | Comicraft avoids hyphenation wherever it can [Comicraft glossary] |
| hyphenation | words of more than five letters, two letters kept on each side, at most two lines in a row ending with a hyphen | Photoshop's defaults [Photoshop hyphenation] |
| a balloon's edge | a path's inset halves at half size, short of letters on the border, which read as cut off | |
| vertical text | columns run right to left; what Unicode sets sideways in vertical text, such as Latin letters and digits, turns a quarter clockwise; `!` and `?` set full width, and a run of two or three sits upright in one cell | Unicode's Vertical_Orientation [UAX 50]; CSS's `text-orientation: mixed` and `text-combine-upright` [CSS writing modes] |
| right to left | a line's runs show in the order the bidirectional algorithm gives, and a right-to-left paragraph's lines start on the right | [UAX 9] |
| strokes | under every fill, so a stroke never covers a neighbouring letter | |
| overflow | text that nothing holds still draws, past its frame | |

A path frame centres every line on one axis, where Photoshop centres each line
in its own span of the shape [Photoshop text in shapes], so the lines of a
lopsided balloon don't wander along its outline.

## 3. Fonts

| Part | Model | Cite |
|---|---|---|
| chain | a span's family, then the preferred families, then Arial, then the system's fallback for the text's scripts in its language, then symbol and emoji families | the system's fallback through fontique [fontique] |
| language | a text with none takes Japanese from kana and Korean from Hangul, and Han alone falls back as Simplified Chinese, so Chinese never takes the system language's font, such as a Japanese one | |
| graphemes | fallback never splits one, and a character no font in the chain has is listed for the inspector | |
| bundled fonts | when the system lacks the chain's first family, it can come from Koharu's font dataset, pinned to one commit so its index, faces, and previews match | the `mayocream/fonts` dataset [Koharu fonts] |
| arriving fonts | a face the page needs downloads in the background; the page draws with a fallback until it lands, then draws again | |

## Sources

Read in November 2026.

- Libraries: [vello], [HarfRust], [ICU4X], [jieba], [fontique], [Koharu fonts]
- Unicode and W3C: [UAX 14], [UAX 9], [UAX 50], [CSS writing modes]
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
[Photoshop hyphenation]: https://developer.adobe.com/photoshop/uxp/2022/ps_reference/objects/options/hyphenationproperties/
[Photoshop text in shapes]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/add-text-along-paths-or-inside-shapes.html
[Comicraft glossary]: https://balloontales.com/?p=418
