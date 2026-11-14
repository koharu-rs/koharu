# koharu-desktop

Koharu's window and the commands its interface calls.

One window shows the page on a native canvas and the React interface over it.
The interface reads and edits the open project through the same protocol as
the agent, and everything that changes the project tells the window. Where it
follows an established product, it cites it; the links are under
[Sources](#sources).

## Goal

The canvas draws pages at display rate on the GPU, and the interface draws
everything else in a web view. Pages are large and drawn at every zoom, so they
draw on the GPU the renderer already holds, and the web view only composites
over them. Neither waits on the other, and both show the project as it is,
whoever changed it last: the user, a model's run, or an agent.

## 1. The window

| Part | Model | Cite |
|---|---|---|
| interface | the React interface renders off-screen in CEF, which hands over each paint's pixels and the rectangles that changed | a windowless CEF browser paints into a BGRA buffer, with the rectangles that need repainting [CEF render handler]; Electron renders a page off-screen on the same engine [Electron offscreen rendering] |
| canvas | `koharu-canvas` draws the page with the renderer's GPU | |
| frame | the canvas, then the interface over it, then the interface's popup, such as an open `<select>`, on the window's own surface | |
| pointer | goes to the canvas where the interface painted nothing over it, and to the interface everywhere else | |
| keys | go to the interface, the input method's composition included | |

The page shows through wherever the interface's page is transparent, so the
interface lays out panels and leaves the middle empty, and the canvas never
needs to know where the panels are.

## 2. One protocol

The interface edits the project the way the agent and an outside agent over MCP
do: with the scene's own queries and edits, its undo and redo, and its history.

- **Owners' types cross the boundary.** A model is `koharu-llm`'s, a run's event
  the pipeline's, the camera the canvas's, so there's no type of the desktop's
  own to keep in step with them. The interface's TypeScript is generated from
  the commands' Rust types, so the two can't drift.
- **An edit is one undo step**, labelled by a key the interface translates, so
  the History panel reads in the app's language.
- **Changes are told, not polled.** The open project tells every change,
  whichever writer made it, and the window redraws and tells the interface.
  The renderer tells when fonts it fetched arrive, and the page draws again.
- **Outside agents** reach the open project over MCP at `127.0.0.1:7878`, always
  on, with the same tools as the agent panel. The port is fixed so a client's
  settings stay valid from one launch to the next.

## 3. Projects

- **One folder** holds every project, one `.khr` file each, and the start view
  lists it. A project's name is its file's name, so a name that isn't a file
  name on every platform is refused rather than changed.
- **Where it lives.** A Windows release build keeps it in the folder Koharu is
  installed in, beside the model store, so it holds Koharu's program, models,
  and projects together; a per-user install goes to the user's own
  `AppData\Local` [Tauri installer]. That folder isn't writable on
  macOS or Linux [Tauri app directories], so there it's the app's local data
  folder, and so is it in debug builds, which then share the installed app's
  projects.
- **Import** makes a page of each image, of each image in a ZIP, CBZ, RAR, or
  CBR archive, and of each page of a PDF, in name order.
- **Export** writes every page as PNG or PSD files in a folder, or as a CBZ of
  PNGs.

## 4. Around the project

- **Preferences** are `config.toml`'s sections, each handed to the component
  that owns it when saved.
- **Runs.** The pipeline's runs and the agent's run on the open project are told
  to the interface as they go; closing the project stops them.
- **One instance.** Opening Koharu again brings its window forward, and the
  window keeps its size, position, and maximized or full-screen state.
- **Debug builds** listen for the DevTools protocol on port 4000, which UI
  automation drives, in a browser profile of their own: the installed app pins
  remote debugging off in its profile on every launch.

## Sources

Read in November 2026.

- Chromium Embedded Framework and Electron: [CEF render handler], [Electron
  offscreen rendering]
- Tauri: [Tauri installer], [Tauri app directories]

[CEF render handler]: https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/include/cef_render_handler.h#L132-L150
[Electron offscreen rendering]: https://github.com/electron/electron/blob/6d9ec3d7433a01587615a23881ed64ed94e1fa6d/docs/tutorial/offscreen-rendering.md
[Tauri installer]: https://github.com/tauri-apps/tauri/blob/17a527316d3aee46f7185c178d3b3122d8ebfd7b/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi#L546-L547
[Tauri app directories]: https://github.com/tauri-apps/tauri/blob/17a527316d3aee46f7185c178d3b3122d8ebfd7b/crates/tauri-utils/src/config.rs#L3617-L3637
