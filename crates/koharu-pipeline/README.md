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
| the project, or some pages | any of detect, recognize, inpaint, and translate | one click; detection again with new thresholds (`Target::Project` with `translate` also translates non-empty ComicInfo prose metadata after the last page) |
| some blocks | recognize, inpaint, and translate | reading a block you drew; translating again after fixing a line |
| a painted area | inpaint, with an optional prompt, onto a raster or a clean-up it names, or a new raster | the remove brush, as Photoshop's Content-Aware Fill [Photoshop Content-Aware Fill] and Generative Fill [Photoshop Generative Fill] fill a selection |
| metadata | translate only | translating the project's ComicInfo prose metadata |

- **A run starts fresh.** It runs every stage it's given on everything it
  names, replacing what's there. To go on with an unfinished chapter, run the
  remaining pages.
- **A stage never runs another.** A stage with nothing to do, such as
  recognition on a page of artwork, does nothing, and that isn't a failure.
- **A model can overwrite anything,** the user's own edits included, and undo
  is the way back. A lock is the exception: no stage writes a block that is
  locked or in a locked layer, and detection leaves a locked layer with the
  panels and balloons holding it, and adds no second block over a block it
  leaves.
- **A request that can't run fails at once**: no stages, detection on blocks, a
  stage other than inpainting on an area, a stage other than translation on
  metadata, or a page or block the project doesn't hold.
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
| detect | Koharu Layout HighRes, with YuzuMarker.FontDetection | the scan | panels, balloons with outlines, and blocks in reading order, in place of the ones there; each block's source frame, tilt, direction, and mask, and the kind of face its text is lettered in |
| recognize | Manga OCR AR | the scan inside each block's source frame, with other blocks' glyphs blanked | the source's wording and its language |
| inpaint | LaMa | the scan, each block's mask, its balloon, and its panel; for an area, the page as drawn without its lettering | the block's clean-up under its mask; for an area, that part of the raster or the clean-up it names, whose mask opens there, or a new raster limited to it by a layer mask |
| translate | Index-Translate 9B | the page's sources in reading order, and with vision the scan | each block's translation: wording, language, and placement |

Every stage also records its C2PA action on the layers it wrote, and detection
its scores in its own namespace. Detection records each source's size, line
width, ink as its fill, and outline as its stroke. Lettering over art is
outlined, such as in white around black, and the outline, the tone that lies
all around what it encloses, isn't its ink: on 28 pages of four works, at most
half of plain lettering's ink lies so around its counters.

Detection also reads each text's and sound effect's face: how likely each
kernel of CJK font is,
such as antique, gothic, rounded, or mincho, the probabilities of
YuzuMarker.FontDetection's 6,150 fonts summed by the kernel table published
beside its weights, and its glyph stroke over its size, both in its own
namespace. On 318 crops of Manga109 the port's kernels came within 0.001 of
PyTorch's in BF16, median, and the likeliest agreed on 314. It reads each
lettering in five crops, its box with 0, 3, 7, 12, and 20% more on each side,
and keeps their mean: YuzuMarker trains on 7%, and its reading moves with the
margin. On Manga109, moving one crop's 7% to 3% or 12% changed the style
placed for 8% of balloon texts, 17% of free texts, and 18% of sound effects,
and the mean of five for 1%, 4%, and 2%.

There's no typesetting stage. Placement puts a translation in its room in its
balloon, or in its source's frame when it's free or a sound effect, and fits it
`best`, so resizing a balloon reflows the text. A balloon text's room reaches
half the page's size past the lines splitting the balloon among its texts,
which aren't drawn, so within a three-quarter margin its letters keep a quarter
of their size off them: a short text whose share narrowed away from the
balloon's side fitted only at its wide end. It gives the translation its
source's kernel, as Japanese editors letter each kind of line in its own kind
of face [Manga font kinds], whose style Settings or the project sets, and
nothing else of its font:

- A balloon is a box when four-fifths or more of its outline, simplified to
  within a fiftieth of its diagonal, runs within 10° of horizontal or
  vertical. Narration and monologue sit in boxes, one or two run together,
  often in a face of their own, where speech balloons curve and a shout's
  spikes run diagonal. In chapter 119 of Tensei Goblin, 22 of the 28 balloon
  texts read as mincho, its narration, sat in boxes, against 1 of the 36 read
  as antique and none of the 10 gothic shouts.
- A source's reading is first multiplied by the square root of the mean of
  the project's other readings of its kind: texts in speech balloons, texts
  in boxes, texts outside both, or sound effects. A reading the crop swings
  then doesn't stand alone.
- A text or a sound effect takes the likeliest kernel of its likeliest group,
  the scene's, when that group's kernels sum to 0.6 or more, as detection
  splits a face among the kernels of its look; else, or with none read,
  antique.
- A text in a speech balloon that this places as antique or mincho takes the
  project's dialogue face of the two instead, as a book keeps one face from
  balloon to balloon. The font network knows 21 antique fonts against 1,189
  mincho ones and reads antique's kana as mincho, so one book's dialogue would
  otherwise change face from line to line: in Dorohedoro's first volume, 117
  of 150 pages mixed the two in their balloons. The face is mincho when five
  or more of the dialogue's kanji-heavy texts, those whose Han is 40% or more
  of their Han and kana, read as the mincho kernels at 0.7 on average.
  Antique sets its kanji in gothic, so on Manga109 the books in mincho read
  0.92 and 1.00 and those in antique 0.26 at most; otherwise it's antique. A
  text placed in any other face, such as a gothic shout or a rounded thought,
  and a box's or a free text, keeps its own.
- The mincho group takes 0.995: antique kana with gothic kanji, the commonest
  dialogue face, often reads as it past 0.9, as the font network knows few
  antique fonts. On Manga109 that letters 87 of 91 mincho texts as mincho and
  27 of 48 antique ones, where 0.9 lettered all of both. Kana alone can't tell
  mincho, fangsong, kaisho, or textbook from antique, so a source with no Han
  character counts them as antique.
- A source whose stroke is 1.3 times the median of the dialogue's or more is
  set at weight 700, as a shout's heavy face is. The dialogue is the texts in
  speech balloons read as antique or mincho, or every text outside them when
  none is in one:
  the project's, not a page's, as a page of mostly monologue would weigh
  rounded gothic as its dialogue.

- **Size.** A letterer keeps one size for a page's dialogue [Manga balloon
  basics]. A page's is the median of the largest its texts' rooms hold in whole
  words, measured with the renderer, whatever its sources' own: its texts in
  balloons, or every text when none is in one, as a running head's or a title's
  widened frame holds far more than a balloon: two had set a page's one
  balloon text past it at nearly three times its source's size. A text that
  would draw smaller is brought up to it as far as its room allows, and one that
  fits larger stays so: small text can't be read, and a large line in a large
  balloon is a shout. One whose lines would span under half its room across
  them at that size grows toward its source's size, as large as the balloon
  holds up to it within an eighth of an em of its border, and when its lines
  still span under half the balloon, past the border until they span half or
  reach the source's size: a word as wide as a narrow balloon left it mostly
  empty, where the original shouted, and its source's size whole overfilled
  tall balloons whose lines it multiplied. Of a chapter's 92 balloon
  texts, those that looked empty spanned at most 0.38 of their rooms, the rest
  0.59 or more. A run on some blocks measures the page's other translations
  too.
- **Margin.** A balloon's text has a letter's width of air [Comicraft glossary].
  One under the page's size has as much of it as holds that size, down to three
  quarters, and one its balloon holds at only about the page's size narrows it
  the same way until it draws 15% larger: half held text 15 to 17% larger but
  set letters against the border, where a line height of 1 rather than 1.2 gives
  2%.
- **Free text.** A vertical source's column holds a word or two of a horizontal
  translation, so its frame widens until it holds the page's size, or a sound
  effect's its source's when that's smaller: along the lines to two and a half
  times its width but not past its longer side, then across them to three lines,
  in its panel and off the page's other texts and balloons. Without that limit a
  text covered a face beside it. A margin narrows and a frame widens as far as
  they may before a word is hyphenated to reach the size.
- **Overflow.** A balloon's text its room still holds under nine tenths of the
  page's size, as a word wider than a narrow balloon leaves it, keeps the size
  and runs past the balloon, above and below it rather than past its sides,
  centred, hyphenated as far as that keeps it nearer, and outlined as text on
  art is: smaller text stands out among its page's more than text past a
  balloon's edge. It stays on the page and out of other balloons. A free text
  doesn't run past its frame, as one still small in its widest, such as a
  chapter's subtitle under its title, is a smaller text.
- **Clear of each other.** Last, a free text, a sound effect, or a balloon's
  text running past it or lifted to its source's size shrinks as far as keeps
  its lines, grown by an eighth of its size, off the other texts' lines and
  sources, whose clean-up, drawn over an earlier layer, hid an overflowing
  letter. Sound effects move first and then free texts, each the largest
  first, so a balloon's text keeps its size. A free text or sound effect
  shrinks to half at most, as a quarter drew some at 2 to 4 pixels; a sound
  effect no size clears takes the size covering least dialogue; a balloon's
  text shrinks back to what its room holds or the page's size at most. Sound
  effects filling widened frames had covered balloons' texts and each other. A
  remark written across the edge of a balloon holding other texts is a free
  text, as its share of the balloon would be its narrow source box.
- **Lines** are centred, or in Japanese and Chinese started together at the top
  or the left, as JLREQ aligns a line to its head [JLREQ].
- **Fill and stroke** are its source's, the stroke's width in proportion to its
  size, at most an eighth of it, as a sound effect's measured outline scaled to
  a size read small from its source had come to a third. One whose source has no
  outline gets a stroke, black around a light fill and white around any other,
  which keeps it legible on art, tone, or a balloon's border: a sound effect a
  sixteenth of its size, and a text an eighth, in a balloon too, where lifted
  and overflowing texts had lost their letters on tone. On paper of the stroke's
  colour it can't be seen, and looking for low contrast under the text missed
  one over foliage and those crossing a panel's border; placement doesn't read
  the page's pixels.
- **Direction.** It's vertical only when its source is and the target is
  Japanese or Chinese; any other language runs horizontally, turned by its
  source's tilt.

## 3. Inpainting

A block's mask reaches 3 pixels past its glyphs, or slivers of letter outlines
stay outside it, but never into another lettering's box, where its clean-up
would cover the neighbour's translation. A glyph two masks take in is the one's
whose box is nearer it, and the smaller box's inside both, or it would be in
neither.

Most lettering sits on plain paper or a gradient, which a fill rebuilds
exactly without a model call, so the model comes last. Each lettering's
surroundings are read on a ring of unmasked pixels just past its mask, inside
its balloon or panel, and the first tier that can rebuild the ring fills the
mask:

| Tier | When | Fill |
|---|---|---|
| one colour | every ring pixel, in its balloon or not, is within 16 levels of their median; or the ring is within 10 levels of its median on average, with no pixel 64 away | the median |
| a smooth background | the same, after a plane fitted to the ring | Telea's fast marching, ported from OpenCV [OpenCV inpaint] |
| the model | anything else: lettering crossing its balloon's outline, a line or art against it, screentone, or a ring too small to read | the inpainting model |

The second is read again around each separate piece of the mask, since
detection can mask a stray piece on other paper or on a balloon's line. What of
a piece is past its balloon's outline, or is a line or art that goes on outside
the mask, is left to the model.

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

- **A page** is its unlocked texts as a JSON object keyed by reading order, and
  the answer must keep the keys. One still without an answer after it's asked
  again, or with a blank translation, fails and never enters the history.
- **A page that got no answer** is asked again by why:

  | Why | Asked again |
  |---|---|
  | its answer is broken or misses a key | once, told what's wrong |
  | its answer copies the page before's for different texts | once, without the history it copied |
  | its provider blocked it, or its answer, for its content | once, without the history, which may be what's blocked |
  | it doesn't fit the model's window, or a model that doesn't say its window cut its answer off | once, with the older half of the history dropped |
  | a local model's answer was cut off, as one that repeats itself is | twice, at 0.4 and 0.8, as Whisper falls back [Whisper]: greedy, it would come out the same |
- **A text given back unchanged**, such as a copyright line, keeps its own
  lettering: its clean-up is hidden and it gets no translation.
- **Vision** shows the model the scan, 1,280 pixels on its longer side, ahead
  of the texts; a model that reads no images goes without it. **Reasoning** lets
  it think first, and the answer is held to the schema once the thinking ends,
  at the model's own level unless set, or off. On 16 pages with Index-Translate
  9B, neither read better and both cost time, so vision is off and reasoning the
  model's own unless set.
- **Cross-page combining (`combine_pages`, on by default).**
  - With Vision off, consecutive pages in a run are packed dynamically to fit
    the model's `input_limit` and `output_limit` (using CJK-aware token
    estimates and an optional `max_pages_per_request` cap). General LLMs
    receive page-boundary continuity guidance; specialized `TRANSLATION` models
    (`Index-Translate`) keep their bare numbered JSON prompt under a tighter
    auto budget. If a combined batch is refused, cut off, or overflows, Koharu
    automatically falls back to translating that batch page by page so a single
    refused page never fails its neighbours.
  - With Vision on, translation stays one page per request (showing only the
    current page's scan); for general models, preceding dialogue (when starting
    mid-chapter) and upcoming pages' OCR text are attached as read-only
    `<previous-pages>` and `<upcoming-pages>` context and stripped before
    storing the turn in conversation history. Specialized `TRANSLATION` models
    keep bare JSON + conversation history only.
- **The history** keeps each page's texts and answer, not its image, transient
  neighbour context, or the model's reasoning, so it grows cleanly without them,
  and a local model reuses its prefix from turn to turn.
- **The system prompt** is Index-Translate's structured-data template
  [Index-Translate] in English, fixed, naming the target by its plain English
  name from CLDR [CLDR]. It keeps the JSON the answer is parsed by, which a
  user's edit could otherwise drop. The user's instructions, in any language
  and form, follow it as written.
- **A full conversation** is recycled: a general model summarizes older pages,
  and a translation model, which can't, drops them.

## 5. Running

- **Models** load the first time a stage needs one, on the one thread that owns
  them, so each load sees the memory actually free. On devices with reliable
  cross-process VRAM accounting (discrete CUDA and ROCm GPUs), each model's
  load footprint, peak working memory, and per-megapixel inpainting working
  cost are measured and persisted at `~/.koharu/vram.json` keyed by
  `(device, model, revision)` (with `512 MiB` kept as safety margin). Before
  loading or running a measured model, `Models::prepare` empties libtorch's
  caching allocator and evicts resident models while free VRAM is below the
  needed budget, choosing victims in strict order: models not needed by any
  active run (LRU first), then active-run models whose stage is furthest from
  the current stage (LRU tie-breaker). Evicting a local language model drops its
  handle and waits synchronously for its engine thread to free its llama.cpp
  context and model and exit before free memory is sampled again. A model also
  leaves when the settings switch away from it, when asked, or when the
  pipeline ends; one no setting uses, which a run brought as Generative Fill's
  does, leaves five minutes after a run last ended, as Ollama keeps a model five
  minutes after its last request [Ollama keep_alive], so filling again doesn't
  load it again.
- **One call at a time** on that thread, so working sets don't overlap on the
  GPU; when an unmeasured load, an external GPU allocation, or a backend without
  cross-process VRAM accounting hits an out-of-memory failure on load or
  inference (or a local language model partially spills off GPU while other
  models are resident), Koharu evicts all other loaded models, empties
  libtorch's cache, and retries once. A local language model decodes on its own
  thread beside it, taking turns on the GPU with stable-diffusion.cpp's models
  on CUDA, and with every model on ROCm.
- **Pre-admission downloads.** Before entering the GPU queue, the scheduler checks `stages::files_needed` off the GPU thread; if any pinned model files are missing, the run enters `WaitingForDownload`, joins or starts grouped entries in `DownloadManager`, and moves to the queue once its downloads succeed (or ends as stopped/failed, cancelling sole-owner remaining downloads).
- **Pages in flight.** Up to three: one reading its input, one on the model,
  and one writing its result.
- **Order.** A run is a graph of its pages' stages: detection before a page's
  other stages, recognition before its translation, and each page's translation
  before the next page's. Within a run, execution is stage-batched in stage
  order (`detect`, then `recognize`, then `inpaint`, then `translate`) and page
  order within each stage, so every page completes one stage before the next
  stage loads its model and starts (maximizing multi-page OCR batching and
  avoiding per-page VRAM swaps). Runs on blocks, an area, metadata, or with
  `RunPriority::RerunNow` are interactive and preempt at the next model call; at
  most one `Normal` batch run executes at a time while additional batch runs
  wait in `queued`. Overlapping `Normal` batch runs on the same project and
  config coalesce per `(page, stage)` without cross-producting unrelated stages.
  When admitting the next batch run, `Scheduler` scores candidates against
  resident GPU models and `~/.koharu/vram.json` footprints (falling back to
  declared file bytes for unmeasured models) with a `256 MiB/s` FIFO aging
  bonus. Recognition takes up to eight pages waiting for the same model at once.
- **Failure and cancellation.** A page the translation model blocks, can't fit,
  or answers with nothing it can take, fails alone: it's named with the reason,
  and the run goes on. Any other failure would come again with the next page, so
  it ends its run, a panic included. What it finished stays. `Pipeline::cancel`
  cancels one run (`Some(id)`) or every run (`None`) across waiting-for-download
  (cancelling sole-owner downloads), queued, and active runs without an error,
  and cuts short a diffusion model's redraw under way. `Pipeline::cancel_job`
  cancels an individual `(page, stage)` job (or metadata translation) inside an
  active, queued, or download-waiting run without stopping the rest of the run,
  bridging translation sequence edges and emitting `Event::JobCancelled`.

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
| `translator.combine_pages` | `true` |
| `translator.max_pages_per_request` | unset (`Auto`, sized from model limits), or `>= 1` |

Manga OCR AR is CC BY-NC-SA 4.0, so a user who picks it is held to
non-commercial use. Telea's port keeps OpenCV's BSD-style license, so this
crate's license is `MIT OR Apache-2.0 AND BSD-3-Clause`.

## Sources

Read in November 2026.

- Adobe: [Photoshop Content-Aware Fill], [Photoshop Generative Fill],
  [Photoshop combine variations]
- Editing models: [FLUX.3 Klein], [Qwen-Image 2.1]
- [ComfyUI routes], [Ollama keep_alive], [OpenCV inpaint], [CLDR], [Index-Translate], [JLREQ], [Whisper]
- Lettering: [Manga balloon basics], [Comicraft glossary], [Manga font kinds]

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
[Manga font kinds]: https://manga.jp.net/knowhow/150924font/
[Comicraft glossary]: https://balloontales.com/?p=418
[JLREQ]: https://www.w3.org/TR/jlreq/
[Whisper]: https://github.com/openai/whisper/blob/86098128c0b4f24f0e2aa2994de830614b474227/whisper/transcribe.py#L184-L224
