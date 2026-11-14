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
| input | W3C pointer events: one stream for mouse, pen, and touch, each with its position, buttons, and pressure | Pointer Events [Pointer Events] |
| cursor | CSS cursors, which the host shows | the CSS `cursor` property [CSS UI] |
| view | where the page shows: the canvas's place in the window, its scale factor, and the camera from page points to the screen's pixels, with pan and zoom. Pointer events become page points here and nowhere else | |
| what's shown | the frame the renderer last drew, the page and project it drew, and what each shown layer offers: its frame, its handles, and how it resizes and turns | |
| selection | the selected layers, in the order they were picked, and the one the select tool would pick under the pointer | |
| overlay | outlines, handles, the marquee, and each gesture's preview, drawn over the page in screen pixels, so lines and handles keep their size at any zoom | |
| settings | the tool, the brush, and the theme's colours, which the host owns and the canvas only reads, so the panels and the canvas show one value | |

Tools read the frame that was drawn, never a newer project, so what the
pointer hits is what the user sees.

## 2. Tools

| Tool | Does | Cite |
|---|---|---|
| select | picks a layer with a click, moves the selection with a drag, picks several with a marquee, and shows what the pointer is over | |
| transform | moves, resizes, and turns the selected layers by their handles; each kind moves by its own values, and a text's frame resizes as the scene's frame does | the scene's per-kind transform follows Photoshop's Free Transform [Photoshop transform] |
| text | point text at a click, a text box over a drag | Photoshop's point type starts at the point you select on the canvas, and its paragraph type sits inside a bounding box [Photoshop type] |
| draw, eraser | paint and erase a raster's pixels along a stroke | |
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
  it, so a box drawn with the text tool is selected the moment it exists.
- **The brush stays the host's.** A request such as Alt and the wheel asks the
  host to change the brush, and the host decides.

## Sources

Read in November 2026.

- W3C: [Pointer Events], [CSS UI]
- Adobe: [Photoshop type], [Photoshop transform]

[Pointer Events]: https://www.w3.org/TR/pointerevents3/
[CSS UI]: https://www.w3.org/TR/css-ui-4/#cursor
[Photoshop type]: https://helpx.adobe.com/photoshop/desktop/text-typography/type-layers-creation/type-layers-in-photoshop.html
[Photoshop transform]: https://helpx.adobe.com/photoshop/using/free-transformations-images-shapes-paths.html
