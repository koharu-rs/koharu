# koharu-canvas

Koharu's page viewport.

It draws a page with the renderer at display rate and turns the pointer into
edits. It owns the camera, hit-testing, the selection and its handles, the
brush cursor, what a gesture previews, and the gestures themselves, since
they need the page's geometry on every move, so the web interface only picks
tools and shows panels. Where it follows an established product, it cites it;
the links are under [Sources](#sources).

## Goal

Every edit a tool makes goes through the same door as every other writer's. The
canvas reads the project and never writes it: it answers edits, and its host
applies them, so a stroke, a drag, and a model's run land in the project the
same way and undo the same way.

## 1. What it owns

| Part | Model | Cite |
|---|---|---|
| input | W3C pointer events: one stream for mouse, pen, and touch, each with its position, buttons, and pressure; and W3C keyboard events while the host gives the canvas the keys. A key it doesn't take goes back to the host, and a key whose press it took stays its own until released, so holding it never reaches the host halfway. While typing, the text takes its keys, repeats included, and the input method's composition | Pointer Events [Pointer Events]; UI Events [UI Events key] |
| typing | the text typed into: its rich text as typed, its caret and selection, the style held at the caret for the next characters, and the input method's composition, laid out on every change and drawn in its layer's place. The type bar styles its selected characters. Leaving it commits one step, none when nothing changed, and a new text is added only with characters in it; Escape drops what was typed | Photoshop edits a type layer in place on its canvas, its input method composing inline [Photoshop inline input], and styles selected characters from its Character panel [Photoshop type] |
| cursor | CSS cursors, which the host shows | the CSS `cursor` property [CSS UI] |
| view | where the page shows: the canvas's place in the window, its scale factor, and the camera from page points to the screen's pixels, with pan and zoom. Pointer events become page points here and nowhere else | |
| what's shown | the frame the renderer last drew, the page and project it drew, and what each shown layer offers: its frame, its handles, and how it resizes and turns | |
| selection | the selected layers, in the order they were picked, the one the select tool would pick under the pointer, the anchors the direct selection tool picked on the selected text's frame, and where the selected layers lie on the page as they're laid out, a point text's lines included, for the host to place its own controls beside, as Photoshop's Contextual Task Bar sits below a selection. A block's clean-up is picked only from the host's Layers, never by a click or a marquee | a layer mask is targeted by its thumbnail in the Layers panel, not on the canvas [Photoshop layer masks] |
| area | what the marquee and the lasso marked of the page's pixels, a byte a pixel: a new shape replaces it, adds to it with Shift held at its press, or takes from it with Alt; a click without a drag, Deselect, or another page drops it. Its edge draws as marching ants, black dashes over white, held still rather than moving, and where it lies on the page goes to the host, which places Generative Fill below it. The host reads it for a fill, which runs as the pipeline's area run, and the area stays to fill again | Photoshop's selection tools make a new selection, add to it, or subtract from it [Photoshop marquee] [Photoshop lasso]; a dashed border shows a selection, which Deselect, Ctrl+D, drops [Photoshop selections]; Generative Fill in the Contextual Task Bar fills a selection after a prompt, or from what surrounds it with none [Photoshop Generative Fill] |
| story | while the host shows the Story panel, each text's frame outlined faintly, and its number in the scene's reading order in a pill centred on the frame's top-left corner, filled for dialogue and outlined for a sound effect; the frame of the text whose entry the pointer is on drawn stronger. The frames draw under the selection's outlines and handles, and the numbers over them, so none hides behind a handle; the handles keep their place and their reach | |
| overlay | outlines, handles, a frame's anchors, the marquee, and each gesture's preview, drawn over the page in screen pixels, so lines and handles keep their size at any zoom. A selected block's mask shows in place of its bounds as a half-red overlay over what it hides, and a selected clean-up with the rest of the page greyed out. A text that runs past its frame has a plus in a hollow square on the frame's top-left corner where the frame is outlined, or beside its Story number, which takes that corner, while Story shows; the text past it still draws, so no line goes missing from an export | selected anchors are filled squares and the rest hollow [Photoshop path selection]; a mask shows as a rubylith, a 50% red overlay [Photoshop layer masks]; "the overflow icon appears on the bounding box" when type doesn't fit it [Photoshop create type], on its lower-right corner, and hides the type past it |
| settings | the tool, the brush, and the theme's colours, which the host owns and the canvas only reads, so the panels and the canvas show one value | |

Tools read the frame that was drawn, never a newer project, so what the
pointer hits is what the user sees.

## 2. Tools

| Tool | Does | Cite |
|---|---|---|
| select | picks a layer with a click, moves the selection with a drag, picks several with a marquee, and shows what the pointer is over | |
| transform | moves, resizes, and turns the selected layers by their handles; each kind moves by its own values, and a text's frame resizes as the scene's frame does | the scene's per-kind transform follows Photoshop's Free Transform [Photoshop transform] |
| direct selection | reshapes the selected text's frame by its anchors: a press on an anchor or a segment drags what it picks, Shift adds to the picked anchors, and a marquee picks those it encloses; the text fits the new shape when the drag ends. A box becomes a path through its corners at its first edit. Delete removes the picked anchors and joins their neighbours, never leaving a ring fewer than three, so a frame stays closed | the Direct Selection tool reshapes a text's path by dragging its anchors [Photoshop text paths], picks segments and anchors by click or marquee, Shift adding [Photoshop path selection], and drags a segment [InDesign paths]; its Delete key would cut the segments at a point, where the Delete Anchor Point tool joins them [Photoshop anchor points] |
| marquee, lasso | mark a rectangle, or a freehand shape closed as the pointer lifts, of the page's pixels as the area | the Rectangular Marquee [Photoshop marquee] and the Lasso, which closes its border on release [Photoshop lasso] |
| text | types point text from a click and a text box over a drag, added when typing commits; a click on a text types into it at the click, as a double click with the select tool does | Photoshop's point type starts at the point you select on the canvas, and its paragraph type sits inside a bounding box [Photoshop type] |
| draw, eraser | paint and erase a raster's or a block's clean-up's pixels along a stroke; on a block's mask, the draw tool uncovers the clean-up whatever its colour, and the eraser hides it | a layer mask is drawn on an alpha scale: the eraser masks more, and any colour uncovers more [CSP layer masks] |
| remove | paints the area for the inpainting model to redraw, as the pipeline's area run takes it | |
| colour picker | the page's own pixel under the pointer, not the screen's | |
| pan | drags the view; the wheel and a pinch move and zoom it | |

## 3. Gestures

A press starts a gesture of its tool, and the gesture takes the moves and the
release that follow.

- **One type per gesture.** Every kind of gesture has its own type in its tool,
  so each step of each gesture is handled, or the build fails.
- **A gesture changes nothing.** It reads the canvas and answers effects: an
  edit, a new selection, a camera, a cursor, or a request to the host. The
  canvas applies those on its own state and passes the rest on.
- **A new layer arrives selected**, in the same answer as the edit that makes
  it, so a text typed with the text tool is selected the moment it's added.
- **The brush stays the host's.** A request such as Alt and the wheel asks the
  host to change the brush, and the host decides.

## Sources

Read in November 2026.

- W3C: [Pointer Events], [UI Events key], [CSS UI]
- Adobe: [Photoshop type], [Photoshop create type], [Photoshop transform], [Photoshop text
  paths], [Photoshop path selection], [Photoshop anchor points], [InDesign paths], [Photoshop
  inline input], [Photoshop marquee], [Photoshop lasso], [Photoshop selections], [Photoshop
  Generative Fill]
- Clip Studio Paint: [CSP layer masks]

[Pointer Events]: https://www.w3.org/TR/pointerevents3/
[UI Events key]: https://www.w3.org/TR/uievents-key/
[CSS UI]: https://www.w3.org/TR/css-ui-4/#cursor
[Photoshop type]: https://helpx.adobe.com/photoshop/desktop/text-typography/type-layers-creation/type-layers-in-photoshop.html
[Photoshop create type]: https://helpx.adobe.com/photoshop/using/creating-type.html
[Photoshop transform]: https://helpx.adobe.com/photoshop/using/free-transformations-images-shapes-paths.html
[Photoshop text paths]: https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/modify-text-paths.html
[Photoshop path selection]: https://helpx.adobe.com/photoshop/desktop/draw-shapes-paths/create-shapes/select-paths-and-path-segments.html
[Photoshop anchor points]: https://helpx.adobe.com/photoshop/desktop/draw-shapes-paths/create-shapes/add-delete-and-convert-anchor-points.html
[InDesign paths]: https://helpx.adobe.com/indesign/desktop/create-lines-and-shapes/edit-and-style-paths/edit-and-reshape-paths.html
[Photoshop inline input]: https://community.adobe.com/bug-reports-711/p-characters-are-not-displayed-when-typing-japanese-in-photoshop-23-0-and-22-5-2-for-windows-658744
[CSP layer masks]: https://help.clip-studio.com/en-us/manual_en/180_layers/Layer_masks.htm
[Photoshop layer masks]: https://helpx.adobe.com/photoshop/using/masking-layers.html
[Photoshop marquee]: https://helpx.adobe.com/photoshop/desktop/make-selections/get-started-selections/make-selections-with-marquee-tools.html
[Photoshop lasso]: https://helpx.adobe.com/photoshop/desktop/make-selections/get-started-selections/isolate-specific-areas-in-images-with-lasso-tool.html
[Photoshop selections]: https://helpx.adobe.com/photoshop/desktop/make-selections/get-started-selections/selection-tools-overview.html
[Photoshop Generative Fill]: https://helpx.adobe.com/photoshop/desktop/create-open-import-images/create-images/edit-images-with-generative-fill.html
