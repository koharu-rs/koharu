# koharu-leptonica

Rust ports of the Leptonica algorithms no Rust crate covers, from Leptonica 1.87.0
(commit `13275a2`). Each keeps Leptonica's integer and float arithmetic, so it gives
Leptonica's result on the same image.

## API

| Function | Ports | What it does |
|---|---|---|
| `find_skew` | `pixFindSkew` | Sweeps ±7° in 1° steps at 4× reduction, then binary searches to 0.01° at 2× |
| `find_skew_sweep_and_search` | `pixFindSkewSweepAndSearch` | The same, with the range, steps, and reductions as arguments. A reduction is 1, 2, 4, or 8 |
| `find_stroke_width` | `pixFindStrokeWidth` | Averages the area over half the boundary with a width read from the 8-connected distance histogram |
| `otsu_adaptive_threshold` | `pixOtsuAdaptiveThreshold` | Binarizes each tile at its own Otsu threshold, optionally smoothed over neighboring tiles |
| `sauvola_binarize` | `pixSauvolaBinarize` | Thresholds at `mean × (1 − factor × (1 − deviation / 128))`, with a mirrored border |
| `color_segment` | `pixColorSegment` | Reduces an `RgbImage` to at most `final_colors` colors |

Inputs and results use `image` buffers:

- **Binary images.** A binary input is a `GrayImage` whose nonzero pixels are
  foreground, and a binary result marks foreground 255. Both binarizers make pixels
  below the threshold foreground.
- **`Skew`.** `angle` is the rotation that deskews the image, in degrees, clockwise
  positive. `confidence` is the best score over the worst, and zero when the
  measurement isn't valid.
- **`Segments`.** `indices` holds each pixel's index into `colors`.

## Where it differs from Leptonica

Results match. These only change what happens on input Leptonica doesn't handle, or
how a result is computed:

| Case | Leptonica | This crate |
|---|---|---|
| A skew sweep of 45° or more, or a step that isn't positive | Never finishes | `Error::Invalid` |
| Stroke width with no foreground | Reads an unset ratio | `None` |
| Skew's vertical shear | Makes the sheared image | Sums each sheared row straight from the source |

`find_skew` also returns `None` when no foreground survives the reduction, and
`color_segment` returns `None` when 20 growing distances don't fit the colors. It
rejects a `sel_size` of 32 or more, since this crate's closing only matches
Leptonica's below that.

## Tests

`tests/leptonica.rs` compares each port with exact values recorded from the C library.
The test images use only integer arithmetic, so they're the same on every platform,
and any drift from Leptonica fails. Each source file links the C files it ports.

Leptonica's code is BSD-2-Clause, so this crate's license is
`MIT OR Apache-2.0 AND BSD-2-Clause`.
