# koharu-pipeline

Koharu's models, run over a project's pages.

One click runs every stage on every page, and every stage also runs by hand on
a page, a block, or a painted area. The app's buttons, its canvas, and the
agent all start runs the same way. Where it follows an established product, it
cites it; the links are under [Sources](#sources).

## Goal

A run does what it was asked, on what it was given, and writes the scene's
own fields, so a new model for a stage writes what the old one did. Nothing
reruns because something it read changed, and a run never decides that a page
needs no work.

## 1. Requests

A request is a target, a set of stages, and optionally settings for that run
only.

| Target | Stages | For |
|---|---|---|
| the project, or some pages | any of detect, recognize, inpaint, and translate | one click; detection again with new thresholds |
| some blocks | recognize, inpaint, and translate | reading a block you drew; translating again after fixing a line |
| a painted area | inpaint, with an optional prompt | the remove brush, as Photoshop's Content-Aware Fill [Photoshop Content-Aware Fill] and Generative Fill [Photoshop Generative Fill] fill a selection |

- **A run starts fresh.** It runs every stage it's given on everything it
  names, replacing what's there. To go on with an unfinished chapter, run the
  remaining pages.
- **A stage never runs another.** A stage with nothing to do, such as
  recognition on a page of artwork, does nothing, and that isn't a failure.
- **A model can overwrite anything,** the user's own edits included, and undo
  is the way back. A locked panel, balloon, or block is the exception:
  detection leaves it.
- **A request that can't run fails at once**: no stages, detection on blocks, a
  stage other than inpainting on an area, or a page or block the project
  doesn't hold.
- **What a run told** is kept in order as it's told, the last event saying how
  it ended, so a caller that looks late still sees everything, as ComfyUI keeps
  a prompt's history [ComfyUI routes].
- **Each run is traced**: a span from its queuing to its end, under whatever
  asked for it, and one for each page stage while it runs.

## 2. Stages

Each stage reads a page, runs its model, and writes its results as one undo
step per page, labelled with the stage's name. A stopped run writes nothing
more. Only a stage knows its models.

| Stage | Default model | Reads | Writes |
|---|---|---|---|
| detect | Koharu Layout HighRes | the scan | panels, balloons with outlines, and blocks in reading order, in place of the ones there; each block's source frame, tilt, direction, and mask |
| recognize | Manga OCR AR | the scan inside each block's source frame, with other blocks' glyphs blanked | the source's wording and its language |
| inpaint | LaMa | the scan, each block's mask, its balloon, and its panel; for an area, the page as drawn without its lettering | the block's clean-up under its mask; for an area, a new raster limited to it by a layer mask |
| translate | Index-Translate 9B | the page's sources in reading order, and with vision the scan | each block's translation: wording, language, and placement |

Every stage also records its C2PA action on the layers it wrote, and detection
its scores in its own namespace. Detection records each source's size, line
width, ink as its fill, and outline as its stroke.

There's no typesetting stage. Placement puts a translation in its room in its
balloon, or in its source's frame when it's free or a sound effect, and fits it
`best`, so resizing a balloon reflows the text.

- **Size.** A new translation starts at the page's lettering size, the median
  of its sources', as a letterer keeps one size for a page's dialogue [Manga
  balloon basics], with its lines centred, or in Japanese and Chinese
  started together at the top or the left, as JLREQ aligns a line to its head
  [JLREQ], and with its source's fill and stroke. A
  sound effect whose source has no outline gets a stroke a sixteenth of its
  size, black around a light fill and white around any other, which keeps it
  legible over art.
- **Direction.** It's vertical only when its source is and the target is
  Japanese or Chinese; any other language runs horizontally, turned by its
  source's tilt.

## 3. Inpainting

A block's mask reaches 3 pixels past its glyphs, or slivers of letter outlines
stay outside it, but never into another lettering's box, where its clean-up
would cover the neighbour's translation.

Most lettering sits on plain paper or a gradient, which a fill rebuilds
exactly without a model call, so the model comes last. Each lettering's
surroundings are read on a ring of unmasked pixels just past its mask, inside
its balloon or panel, and the first tier that can rebuild the ring fills the
mask:

| Tier | When | Fill |
|---|---|---|
| one colour | the ring is within 10 levels of its median on average, with no pixel 64 away | the median |
| a smooth background | the same, after a plane fitted to the ring | Telea's fast marching, ported from OpenCV [OpenCV inpaint] |
| the model | anything else: lettering crossing its balloon's outline, a line or art against it, screentone, or a ring too small to read | the inpainting model |

The model redraws in windows at the page's own scale, each lettering with 256
pixels of context around it, merged with the windows it overlaps while the
merge stays within a megapixel. A window never spans two panels.

| Inpainter | Redraws | Takes a prompt | Keeps |
|---|---|---|---|
| LaMa | the mask | no | the mask's pixels |
| FLUX.3 Klein, Qwen-Image 2.1 Turbo | the whole window, edited with the window as its reference, the way both models' makers publish editing, since neither ships inpainting [FLUX.3 Klein] [Qwen-Image 2.1] | yes: an area's own, or else the settings' | the window, its colours matched to the original's away from the mask: a block the part within 256 pixels of its lettering, an area all of it |

What a model keeps shows through the mask, as Photoshop keeps a Generative
Fill result whole on its layer, under a layer mask the user can paint to show
more of it [Photoshop combine variations].

## 4. Translation

Translation is one conversation per open project, a turn per page in page
order, and translating blocks is a turn of the same conversation. Changing the
model, the target, or the instructions starts a new one.

- **A page** is its texts as a JSON object keyed by reading order, and the
  answer must keep the keys. A broken answer, a missing key, or a blank
  translation fails the page, which never enters the history.
- **An answer copying the page before's** for different texts is asked again
  without the history it copied.
- **A text given back unchanged**, such as a copyright line, keeps its own
  lettering: its clean-up is hidden and it gets no translation.
- **Vision** shows the model the scan, 1,280 pixels on its longer side, ahead
  of the texts. **Reasoning** lets it think first, and the answer is held to the
  schema once the thinking ends, at the model's own level unless set, or off.
  On 16 pages with Index-Translate 9B, neither read better and both cost time,
  so vision is off and reasoning the model's own unless set.
- **The history** keeps each page's texts and answer, not its image or the
  model's reasoning, so it grows as it does without them, and a local model
  reuses it from turn to turn.
- **The system prompt** is Index-Translate's structured-data template
  [Index-Translate] in English, fixed, naming the target by its plain English
  name from CLDR [CLDR]. It keeps the JSON the answer is parsed by, which a
  user's edit could otherwise drop. The user's instructions, in any language
  and form, follow it as written.
- **A full conversation** is recycled: a general model summarizes older pages,
  and a translation model, which can't, drops them.

## 5. Running

- **Models** load the first time a stage needs one, on the one thread that owns
  them, so each load sees the memory actually free. Nothing is unloaded to make
  room: a model that doesn't fit fails, naming the free memory. A model leaves
  when the settings switch away from it, when asked, or when the pipeline ends.
  One no setting uses, which a run brought as Generative Fill's does, leaves
  five minutes after a run last ended, as Ollama keeps a model five minutes
  after its last request [Ollama keep_alive], so filling again doesn't load it
  again.
- **One call at a time** on that thread, so peak memory is every loaded model
  plus the largest working set, and running out never depends on timing. A
  local language model decodes on its own thread beside it.
- **Pages in flight.** Up to three: one reading its input, one on the model,
  and one writing its result.
- **Order.** A run is a graph of its pages' stages: detection before a page's
  other stages, recognition before its translation, and each page's translation
  before the next page's. Runs on blocks or an area go first, then runs on
  pages in the order they came, so an interactive run goes ahead at its next
  model call. Recognition takes up to eight pages waiting for the same model at
  once.
- **Failure.** The first failure ends its run, naming the page, the stage, and
  the reason, a panic included. What it finished stays. Stopping is the same
  without the error, for every run, and cuts short a diffusion model's redraw
  under way.

## 6. Settings

The section fails to parse when a threshold isn't between 0 and 1 or the target
is a language CLDR doesn't name in English.

| Key | Default |
|---|---|
| `detector.model` | `koharu-layout-highres` |
| `detector.text`, `onomatopoeia`, `bubble`, `panel` | the model's own thresholds |
| `recognizer.model` | `hayai-ocr`, or `manga-ocr-ar` or `baberu-ocr` |
| `inpainter.model` | `lama`, or `flux3-klein` or `qwen-image` |
| `inpainter.fill` | `flux3-klein`, or another inpainter, the one Generative Fill's bar picks and keeps, as Photoshop's Generative Fill keeps its own model [Photoshop Generative Fill] |
| `inpainter.prompt` | asks the model to remove all text and keep everything else, for a run that brings none |
| `translator.model` | `koharu/index-translate-9b` |
| `translator.target` | `en` |
| `translator.instructions` | none |
| `translator.vision` | `false` |
| `translator.reasoning` | the model's own, or `off` or an effort level, one word |

Manga OCR AR is CC BY-NC-SA 4.0, so a user who picks it is held to
non-commercial use. Telea's port keeps OpenCV's BSD-style license, so this
crate's license is `MIT OR Apache-2.0 AND BSD-3-Clause`.

## Sources

Read in November 2026.

- Adobe: [Photoshop Content-Aware Fill], [Photoshop Generative Fill],
  [Photoshop combine variations]
- Editing models: [FLUX.3 Klein], [Qwen-Image 2.1]
- [ComfyUI routes], [Ollama keep_alive], [OpenCV inpaint], [CLDR], [Index-Translate], [JLREQ]
- Lettering: [Manga balloon basics]

[Photoshop Content-Aware Fill]: https://helpx.adobe.com/photoshop/desktop/repair-retouch/remove-objects-fill-space/adjust-content-aware-fill-settings.html
[Photoshop Generative Fill]: https://helpx.adobe.com/photoshop/desktop/create-open-import-images/create-images/edit-images-with-generative-fill.html
[Photoshop combine variations]: https://www.adobe.com/learn/photoshop/web/combine-variations-generative-fill
[FLUX.3 Klein]: https://docs.comfy.org/tutorials/flux/flux-3-klein
[Qwen-Image 2.1]: https://github.com/QwenLM/Qwen-Image-2.1
[ComfyUI routes]: https://docs.comfy.org/development/comfyui-server/comms_routes
[Ollama keep_alive]: https://docs.ollama.com/faq
[OpenCV inpaint]: https://github.com/opencv/opencv/blob/fe38fc608f6acb8b68953438a62305d8318f4fcd/modules/photo/src/inpaint.cpp
[CLDR]: https://cldr.unicode.org/
[Index-Translate]: https://huggingface.co/IndexTeam/Index-Translate-9B
[Manga balloon basics]: https://www.linkedin.com/pulse/manga-lettering-balloon-basics-phil-christie
[JLREQ]: https://www.w3.org/TR/jlreq/
