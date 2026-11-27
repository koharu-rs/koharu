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
| breaking | where Unicode allows a line to break, chosen over the whole text rather than one line at a time; Chinese by word, and before an opening quotation mark and after a closing one between East Asian characters | Unicode 17's line breaking [UAX 14], its LB19a for quotation marks, through ICU4X [ICU4X]; Chinese words by jieba [jieba] |
| fitting | `shrink` lowers the size just enough to fit, down to half; `best` also grows it to the largest size that fits, and a word too wide for its line stops the growth; past every step at half size, `best` shrinks on until the text fits | the scene's fit modes ([koharu-scene](../koharu-scene/README.md#text)) |
| hyphens | whole words shrink first; where that draws them under nine tenths of their size, one word may take one hyphen, near its middle, if that makes the text at least a tenth larger, up to its size, as more cost more than they gain; and at half size a word too wide for its line takes as many as it needs; a word never breaks without a hyphen, and what still doesn't fit runs past its frame, hyphenated where that keeps it nearer, unless it fits best | Comicraft avoids hyphenation wherever it can [Comicraft glossary] |
| hyphenation | words of more than five letters, two letters kept on each side, at most two lines in a row ending with a hyphen; a drawn-out sound, such as "Gwaaaaah", in the middle of its run of four or more of one letter, in any language and as a word breaks, though growing to fill a frame never adds such a break | Photoshop's defaults [Photoshop hyphenation]; dictionaries have no points in a drawn-out sound |
| a balloon's edge | a path's inset halves at half size, short of letters on the border, which read as cut off | |
| vertical text | columns run right to left; what Unicode sets sideways in vertical text, such as Latin letters and digits, turns a quarter clockwise, its font's ideographic em box centred across the column as an upright character's em is, and so does what it transforms or else rotates, such as `〜`, in a font without a vertical form for it; what stands upright takes its font's `vert` forms and never `vrt2`'s, whose proportional Latin comes already turned; `!` and `?` set full width, and a run of two or three sits upright in one cell, as does a number of one or two digits outside a Latin word; a hyphen, a dividing mark, a middle dot, or an inseparable mark, such as `〜`, `！`, `・`, `：`, `—`, or `…`, and Chinese's `·`, `⋯`, and `～`, centres its ink across the column, as does a turned `(`, `)`, `[`, `]`, `{`, or `}` among upright characters, with no letter or digit beside it in its run, in place of its vertical form; `、` and `。` keep their font's vertical forms in the column's corner, but Chinese centres its commas and full stops in their cells; ASCII `:` and `;` stay on a sideways run's baseline | Unicode's Vertical_Orientation [UAX 50]; OpenType has a CJK font for vertical text set its ideographic em box as its typo ascender and descender [OpenType OS/2]; CSS's `text-orientation: mixed` and `text-combine-upright: digits 2`, and its leaving `vrt2` unused [CSS writing modes]; JLREQ's character classes cl-03 to cl-08 and its Western characters, cl-27 [JLREQ]; CLREQ's interpunct, ellipsis, and connector, and its stops in the middle of their cells as Taiwan and Hong Kong set them [CLREQ] |
| a span's orientation | a span set `upright` stands every character upright, Latin included, `sideways` turns it all a quarter clockwise as one line, CJK included, and `tate_chu_yoko` sets each of its runs in one upright cell; a cell wider than an em takes its font's half-, third-, or quarter-width forms for two, three, or four characters where they narrow it, and is squeezed into the em past that, the automatic groups too | CSS's `text-orientation: upright` and `sideways`, `text-combine-upright: all`, and its rules for fitting combined text into an em [CSS writing modes] |
| emphasis marks | a span's mark beside each of its characters but white space, controls, and punctuation, though `#`, `%`, `&`, `@`, and the like take one, and one for a tate-chu-yoko cell; half its character's size, in its font chain, upright, its ink centred on its character along the line and on an em box past the character's across it, or past a ruby on that side; over a line and right of a column, under a Chinese line; a line opens for its marks as for ruby; a mark never spreads its character, so one wider than a narrow letter hangs over its neighbours | CSS's emphasis marks, set as if each character had its mark as a centred ruby, their language's preferred positions, the characters they skip, and the line height they add as ruby does [CSS text decoration] |
| line head | a line opening with a full-width opening bracket, such as `「` or `『`, starts at the bracket's ink, without the half em before it | JLREQ counts an opening bracket half an em wide with half an em of space before it, and starts every line with no space, as JIS X 4051 does [JLREQ] |
| right to left | a line's runs show in the order the bidirectional algorithm gives, and a right-to-left paragraph's lines start on the right | [UAX 9] |
| ruby | half its base's size, over a line or on the right of a column, its em box on its base's; whichever of the two is shorter is spread to the other's length, full-width characters taking equal shares with half a share at each end and other text staying together, so they're centred and a ruby never reaches over its base's neighbours; a base never breaks; a line with ruby opens as far as its ruby and base need, and a path's lines, which share one pitch, all open with it | W3C's rules for simple ruby, which centre the two, space them 1-2-1, keep Western letters together, and let no ruby overhang [Simple ruby]; CSS's `ruby-align: space-around`, and the leading it adds so that equal lines of ruby don't overlap [CSS ruby]; JLREQ's half size [JLREQ] |
| strokes | under every fill, so a stroke never covers a neighbouring letter | |
| overflow | text that nothing holds still draws, past its frame: in a path, in as many lines as its widest span holds, past its top and foot rather than its sides, a long word split over lines rather than one line running far past; a centred line on both sides alike, any other past its end | a box's text runs past its foot |
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
| face | where a text's marks and style set no family, weight, or italic, its kernel's face for the script of its language in the project's styles does, with what that leaves out from antique's, and antique's for a text with no kernel; the `[renderer]` preference holds the styles a new project starts with, whose defaults are, for Latin, the bundled font scanlation groups' font guides set each kind of line in, for Cyrillic and Greek, which those fonts lack, the dialogue font's bold and italics, and for Chinese and Japanese a bundled face of the kernel, a group's first kernel's standing for each of its kernels with none of its own | Japanese editors set each kind of line in its own kind of face [Manga font kinds]; scanlation groups' guides set narration in a lowercase italic font apart from dialogue's, and name fonts for narration, thoughts, shouts, and handwriting [QSS font cards] [Gemelli font guide] [Vorbis fonts] |
| chain | a text's face, then the dialogue families of its script and of Latin, then Arial, then the system's fallback for the text's scripts in its language, then symbol and emoji families | the system's fallback through fontique [fontique] |
| language | a text with none takes Japanese from kana and Korean from Hangul, and Han alone falls back as Simplified Chinese, so Chinese never takes the system language's font, such as a Japanese one | |
| graphemes | fallback never splits one, and a character no font in the chain has is listed for the inspector | |
| marks | a character of no script of its own, such as a mark, a digit, or a space, takes the font of the text beside it when that font has it, so a CJK sentence's marks come from its CJK font even where a Latin font earlier in the chain has them | |
| faux bold | a text at 600 or more in a face under 600 draws 0.02 em bolder on each side and is set 0.04 em wider a character, as CC Wild Words' own bold is; a full em, a column's cells, and an upright group keep their width, as a full em does in a CJK family's own bold; a variable face's own weights come first | CSS synthesizes the face a family lacks [CSS font synthesis]; Firefox counts 600 and up as bold [Firefox fonts]; HarfBuzz's synthetic bold widens every advance by what it adds [HarfBuzz fonts] |
| faux italic | italic text in an upright face leans 14° to the right, about each glyph's centre where it stands upright in a column, so it stays on the column's centre line; a text's bounds hold what leans past its line | CSS's default oblique angle, and its skew about the centre in vertical text [CSS font synthesis]; Clip Studio Paint's Italic leans letters to the right [CSP font], where CSS skews a column's along it |
| bundled fonts | a family of the chain the system lacks can come from Koharu's font dataset, in its place in the chain, pinned to one commit so its index, faces, and previews match | the `mayocream/fonts` dataset [Koharu fonts] |
| font downloads | a face a text needs downloads as the text lays out, and the render waits for it; a family the dataset lacks, or one that fails to download, falls back down the chain | |

## Sources

Read in November 2026.

- Libraries: [vello], [HarfRust], [HarfBuzz fonts], [ICU4X], [jieba], [fontique],
  [Koharu fonts], [Firefox fonts]
- Unicode and W3C: [UAX 14], [UAX 9], [UAX 50], [CSS writing modes], [CSS ruby], [Simple ruby],
  [CSS text decoration], [CSS font synthesis], [JLREQ], [CLREQ]
- OpenType: [OpenType OS/2]
- Adobe: [Photoshop hyphenation], [Photoshop text in shapes]
- Clip Studio Paint: [CSP font]
- Lettering: [Comicraft glossary], [Manga font kinds]
- Scanlation font guides: [QSS font cards], [Gemelli font guide], [Vorbis fonts]

[vello]: https://github.com/linebender/vello
[HarfRust]: https://github.com/harfbuzz/harfrust
[HarfBuzz fonts]: https://github.com/harfbuzz/harfbuzz/blob/main/src/hb-font.hh
[ICU4X]: https://github.com/unicode-org/icu4x
[jieba]: https://github.com/fxsjy/jieba
[fontique]: https://github.com/linebender/parley/tree/main/fontique
[Manga font kinds]: https://manga.jp.net/knowhow/150924font/
[QSS font cards]: https://quicksandscans.wordpress.com/resources/
[Gemelli font guide]: https://docs.google.com/presentation/d/1hOvjfXMLuvTO6-XofTf4T6asAOp2DOOiYkTvLLj-XEc/htmlpresent
[Vorbis fonts]: https://www.insidescanlation.com/etc/Vorbis-Typesetting-Guide/fonts.htm
[Koharu fonts]: https://huggingface.co/datasets/mayocream/fonts
[Firefox fonts]: https://github.com/mozilla-firefox/firefox/blob/main/gfx/thebes/gfxFontEntry.h
[UAX 14]: https://www.unicode.org/reports/tr14/
[UAX 9]: https://www.unicode.org/reports/tr9/
[UAX 50]: https://www.unicode.org/reports/tr50/
[CSS writing modes]: https://www.w3.org/TR/css-writing-modes-4/
[CSS ruby]: https://drafts.csswg.org/css-ruby-1/
[CSS text decoration]: https://www.w3.org/TR/css-text-decor-3/#emphasis-marks
[CSS font synthesis]: https://www.w3.org/TR/css-fonts-4/#font-synthesis-intro
[Simple ruby]: https://www.w3.org/TR/simple-ruby/
[JLREQ]: https://www.w3.org/TR/jlreq/
[CLREQ]: https://www.w3.org/TR/clreq/
[OpenType OS/2]: https://learn.microsoft.com/en-us/typography/opentype/spec/os2
[Photoshop hyphenation]: https://developer.adobe.com/photoshop/uxp/2022/ps_reference/objects/options/hyphenationproperties/
[Photoshop text in shapes]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/add-text-along-paths-or-inside-shapes.html
[CSP font]: https://www.clip-studio.com/site/gd_en/csp/toolguide/csp_toolguide/100_reference/Font.htm
[Comicraft glossary]: https://balloontales.com/?p=418
