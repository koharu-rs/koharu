# koharu-pipeline

Runs Koharu's models over a project's pages. One click runs every step on every page, and
every step also runs by hand on a page, a block, or a painted area. The app's canvas, its
buttons, and the agent all use it the same way. The work graph is internal: nothing reruns
because something it read changed, and a run never decides that a page needs no work.

## API

| Item | What it does |
|---|---|
| `Pipeline::new(config)` | Starts the scheduler and the thread owning the models. It needs a Tokio runtime; models load on first use |
| `Pipeline::set_config(config)` | Follows a changed `[pipeline]`. A model switched away from leaves memory once the work queued with it ends |
| `Pipeline::run(session, request)` | Queues a run on the project a session holds and returns its `RunId` |
| `Pipeline::events()` | Every run's `Event`s: a step started or finished on a page, and a run ended done, failed, or stopped |
| `Pipeline::stop()` | Stops every run, running and queued |
| `Pipeline::unload()` | Unloads every model once the GPU's current work ends |
| `Pipeline::close(project)` | Forgets a closed project's translation conversation |
| `placement(project, block, target)` | The op placing a block's translation. Translation writes it, and the editor asks again when a balloon or a frame changes |
| `PipelineConfig` | The `[pipeline]` section |

`Pipeline` is a cheap handle to clone; the scheduler and the model thread end when the last
one drops. The canvas needs no event to preview a run: each step's result is an edit, and the
scene's change notification redraws it.

## Requests

A `Request` is a `Target`, a set of steps, and optionally a `PipelineConfig` for this run
only.

| `Target` | Steps | Examples |
|---|---|---|
| `Project`, `Pages` | Any of detect, recognize, inpaint, translate | One click. Detection again with new thresholds, given as the request's config |
| `Blocks` | Recognize, inpaint, translate | Recognition on a block you drew. Translation after fixing a line |
| `Area` | Inpaint, with an optional prompt | The magic eraser, Content-Aware Fill, Generative Fill |

- **A run starts fresh.** It runs every step it's given on everything it names, replacing
  what's there. To go on with an unfinished chapter, run the remaining pages.
- **A step never runs another.** A step with nothing to do, such as recognition on a page of
  artwork, does nothing, and that isn't a failure.
- **A model can overwrite anything,** the user's own edits included; undo is the way back. A
  locked panel, balloon, or block is the exception: detection leaves it.
- **A request that can't run fails at once:** an empty set of steps, detection on blocks, a
  step other than inpainting on an area, a page or block the project doesn't hold, or a
  prompt to an inpainter that takes none.

## Steps

Each step runs in four stages. `read` turns a page into the model's input on a blocking
thread, `infer` runs the model on the model thread, `finish` does the rest of the work on
its output on a blocking thread, as detection lays out the page from the network's detections
and translation takes the page's turn, and
`write` turns that and the project as it is then into ops, applied as one undo step per page
labelled with the step's name. A stopped run writes nothing.

| Step | Default model | Reads | Writes |
|---|---|---|---|
| Detect | Koharu Layout HighRes | The scan | Panels, balloons with outlines, and blocks in reading order, in place of the ones there. A block gets its source's box turned back by its tilt, its direction, and its mask |
| Recognize | manga-ocr-ar | The scan inside each block's source frame | The source's wording and the language the model reads |
| Inpaint | LaMa | The scan, each block's mask, its balloon and panel | The block's clean-up under its mask. On an area, a new raster layer limited to it by a layer mask |
| Translate | Index-Translate 9B | The page's sources in reading order, and with vision the scan | Each block's translation: wording, language, and its placement |

Each step also records its C2PA action on the layers it wrote, and detection its scores in
the `koharu` namespace. Detection records each source's size, how broad its lines are, and
its outline, a band of one colour around lettering over art, as its stroke.
There's no typesetting step. Placement puts a translation in its room in its balloon, the
layout model's share of the outline among the source boxes standing in it, a remark over its
edge included, or in its source's frame when it's free or a sound effect, and fits it `best`:
it fills the frame at every render, so resizing a balloon reflows the text. A new translation
starts at the page's lettering size, the median of its sources', as a letterer keeps one
size for a page's dialogue, with its lines centred and its source's stroke, the width scaled
to its size; what its style already sets is kept. It's vertical only when its source is and
the target is Japanese or Chinese; any other language runs horizontally, turned by its
source's tilt, in a balloon as outside one. The renderer centres every line on one axis, so
the lines of a lopsided room don't wander along its outline.

## Inpainting

Detection writes a block's mask 3 pixels past its glyphs, or slivers of letter outlines stay
outside it, cut first to the lettering's box and a glyph past its lines' ends, where a
box can stop short of a trailing ellipsis, but not past its sides nor into another
lettering's box: there the model's mask has spilled onto a neighbour, and its clean-up would
cover the neighbour's translation, and inpainting fills the mask as it is, the region the clean-up shows through.
Each lettering's surroundings are read on a ring of unmasked pixels 4 pixels past the mask,
inside its balloon and 3 pixels from the outline, or inside its panel. The first
tier that can rebuild the ring fills the mask:

| Tier | When | Fill |
|---|---|---|
| One colour | Within 10 levels of its median on average, and no pixel 64 away | The median |
| Smooth background | The same, from a plane fitted to the ring | Telea's fast marching with a radius of 5, ported from OpenCV 4.13.0 |
| The model | Anything else: a lettering crossing its balloon's outline, a line or art against it, screentone, or a ring under 16 pixels | The inpainting model, a line's lettering with a pixel more around it |

The model redraws in windows at the page's own scale: each lettering with 256 pixels of
context, clipped to its panel, merged with the windows it overlaps in the same panel while the
merge stays within a megapixel. A window never spans two panels, every lettering in it is
already filled or masked, and only the masked pixels come back.

| Inpainter | Takes a prompt |
|---|---|
| LaMa | No |
| RORem | No |
| FLUX.3 Image Klein | Yes |
| MOSS-Image | Yes |

## Running

- **Models.** A model loads the first time a step needs it, on the one thread that owns the
  models, so each load sees the memory actually free. stable-diffusion.cpp and llama.cpp fit
  their weights to the device's free memory themselves, and before either loads, libtorch's
  cache is emptied, so its freed memory counts as free. Nothing is unloaded to make room: a
  model that doesn't fit fails, naming the device's free memory and what libtorch holds. A
  model leaves memory when the settings switch away from it, when `unload` is called, or when
  the pipeline ends.
- **The model thread runs one call at a time.** Peak memory is every loaded model plus the
  largest working set, so running out of memory never depends on timing. A local language
  model decodes on its own engine thread with memory it set aside when it loaded, so its
  turn runs beside the model thread's calls: on 16 pages with Index-Translate 2B that took
  peak memory up 134 MiB and the run from 19.3 to 16.5 seconds. Up to three pages are in
  flight: one reading its input, one on the model thread, and one writing its result.
- **Order.** Each run is a graph of its pages' steps, with detection before a page's other
  steps, recognition before its translation, and each page's translation before the next
  page's. Runs on blocks or an area go first, then runs on pages in the order they came;
  within a run, the oldest page goes first, in step order. Whenever the model thread is free
  it takes the prepared step first in that order, so hand work goes ahead at its next call.
  Recognition takes every page waiting for the same model with it, up to eight. A local
  translation waits for the model thread only to load its model, and a hosted one not at all.
- **Failure.** The first failure ends its run with the page, the step, and the reason. What
  it finished stays, and nothing more is written. Stop is the same without the error, for
  every run. A model call on the model thread finishes, but a local translation stops at its
  next token.

## Translation

Translation is one conversation per open project, kept in memory, a turn per page in page
order; translating blocks is a turn of the same conversation. A project's next translation
waits while its conversation is in use. Changing the model, the target, or the prompt starts
a new one; turning vision or reasoning on or off doesn't, as the history holds neither.

- **A page** is its texts as a JSON object, keyed by zero-padded numbers in reading order. The
  answer must keep the keys, held to a schema the way the model's provider keeps one. A
  broken answer, a missing key, or a blank translation fails the page, which never enters the
  history. An answer repeating the page before's for different texts is asked again without
  the history it copied, and a text given back unchanged, such as a copyright line, keeps its
  lettering: its clean-up is hidden and it gets no translation.
- **Vision** shows the model the scan ahead of the page's texts: a JPEG 1,280 pixels on its
  longer side, each pixel the average of the scan's under it, about 1,120 tokens to
  Index-Translate and over the 1,024 llama.cpp asks of a Qwen-VL model. A model that reads no
  images fails the page. A local model fetches and loads its projector only while vision is
  on: for Index-Translate 9B a 620 MB download, 840 MiB of GPU memory, and 0.7 seconds of
  loading. Turning vision on or off reloads the model and keeps the conversation.
- **Reasoning** at the set effort lets the model think before it answers, and the answer is
  held to the schema once the thinking ends.
- **The history** keeps each page's texts and its answer. The page's image and the model's
  reasoning serve its own turn only, as Index-Translate's template drops earlier reasoning, so
  the history grows as it does without them and a local model reuses it from turn to turn.
  The conversation counts from the last request's usage, which held its image and reasoning,
  as the next request holds its own.
- **What they cost.** On 16 Blue Archive pages with Index-Translate 9B, the run took 27.7
  seconds, 35.4 with vision, 73.5 with reasoning, and 78.7 with both. Neither read better:
  with the page in view it wrote アロナ as Aroana, and reasoning dropped stutters and left
  ケチンボ untranslated, so both are off unless set.
- **The system prompt** is the user's to rewrite. `{target}` becomes the target's plain English
  name from CLDR: Chinese by its script, and other languages without a region their language
  implies. The default is Index-Translate's instTrans template in English.
- **A full conversation** is recycled by koharu-llm's `Context`: a general model summarizes
  older pages, and a translation model, which can't, drops them.
- **A local model** runs with one sequence of 32,768 tokens.

## Settings

The section fails to parse when a threshold isn't between 0 and 1, the target is a language
CLDR doesn't name in English, or the prompt has no `{target}`.

| Key | Default |
|---|---|
| `detector.model` | `koharu-layout-highres` |
| `detector.text`, `onomatopoeia`, `bubble`, `panel` | The model's own thresholds |
| `recognizer.model` | `manga-ocr-ar`, or `baberu-ocr` or `hayai-ocr` |
| `inpainter.model` | `lama`, or `rorem-mixed`, `flux3-klein`, or `moss-image` |
| `translator.model` | `koharu/index-translate-9b`, its first quantization |
| `translator.target` | `en` |
| `translator.prompt` | `DEFAULT_PROMPT` |
| `translator.vision` | `false` |
| `translator.reasoning` | Unset, the model's own: a local model doesn't reason. Or `minimal`, `low`, `medium`, `high`, `xhigh`, `max` |

manga-ocr-ar is CC BY-NC-SA 4.0, so a user keeping the default is held to non-commercial use.
Telea's port keeps the BSD-style license of OpenCV's `inpaint.cpp`, Intel's, so this crate's
license is `MIT OR Apache-2.0 AND BSD-3-Clause`.
