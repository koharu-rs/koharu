# koharu-leptonica

Leptonica's document-image algorithms, in Rust.

Ports of the Leptonica algorithms no Rust crate covers [Leptonica], from
Leptonica 1.87.0. Each keeps Leptonica's integer and float arithmetic, so it
gives Leptonica's result on the same image. The links are under
[Sources](#sources).

## Goal

Measurements on a page's lettering, such as its skew and the width of its
strokes, match what Leptonica measures, without linking a C library.

## 1. What it ports

| Algorithm | Leptonica | Does |
|---|---|---|
| skew | `pixFindSkew`, `pixFindSkewSweepAndSearch` | finds the angle that deskews an image: a coarse sweep of angles on a reduced image, then a binary search to a hundredth of a degree |
| stroke width | `pixFindStrokeWidth` | the average width of an image's strokes |
| Otsu thresholding | `pixOtsuAdaptiveThreshold` | binarizes each tile at its own Otsu threshold, optionally smoothed over its neighbours |
| Sauvola thresholding | `pixSauvolaBinarize` | binarizes each pixel by its neighbourhood's mean and deviation |
| colour segmentation | `pixColorSegment` | reduces an image to a few colours, and says which colour each pixel took |

- **Binary images.** A binary input's nonzero pixels are foreground, and a
  binary result marks foreground 255. Both binarizers make pixels below the
  threshold foreground.
- **Skew** is the rotation that deskews the image, clockwise positive, with a
  confidence: the best score over the worst, and zero when the measurement
  isn't valid.

## 2. Where it differs from Leptonica

Results match. These change only what happens on input Leptonica doesn't
handle, or how a result is computed:

| Case | Leptonica | This crate |
|---|---|---|
| a skew sweep of 45° or more, or a step that isn't positive | never finishes | an error |
| stroke width with no foreground | reads an unset ratio | none |
| skew's vertical shear | makes the sheared image | sums each sheared row straight from the source |

Skew also finds none when no foreground survives the reduction, and colour
segmentation finds none when 20 growing distances don't fit the colours. It
refuses a closing of 32 pixels or more, since only smaller closings match
Leptonica's.

## 3. Fidelity

Each port's source links the C file it follows, and the tests compare every
port with exact values recorded from the C library. The test images use only
integer arithmetic, so they're the same on every platform, and any drift from
Leptonica fails.

Leptonica's code is BSD-2-Clause, so this crate's license is
`MIT OR Apache-2.0 AND BSD-2-Clause`.

## Sources

Read in November 2026.

- [Leptonica]

[Leptonica]: https://github.com/DanBloomberg/leptonica/tree/13275a2
