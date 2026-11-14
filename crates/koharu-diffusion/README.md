# koharu-diffusion

stable-diffusion.cpp for Koharu's diffusion models.

A safe Rust layer over stable-diffusion.cpp's image generation [sd.cpp]: load
a model, then generate images from a prompt, an image to start from, a mask,
and reference images. The links are under [Sources](#sources).

## Goal

Koharu's diffusion models run on sd.cpp as sd.cpp runs them, so a model that
works in sd.cpp's own command line works here, with the same results. This
crate adds safety, not behaviour.

## 1. Model

| Part | Model | Cite |
|---|---|---|
| parameters | start from sd.cpp's own defaults, so anything left unset is what sd.cpp would use; only the options Koharu's models need are named | sd.cpp's `sd_ctx_params_init` and `sd_img_gen_params_init` [sd.cpp header] |
| loading | every weight loads up front, so a model that doesn't fit, or can't make images, fails when it loads and not partway through a generation | sd.cpp's eager loading [sd.cpp header] |
| placing weights | sd.cpp fits the weights into the device's free memory itself, then system memory | |
| prompts | sd.cpp keeps its last few prompts' encodings, so a prompt used again isn't encoded again | |
| a model | one at a time per context: a context can move between threads, but generating takes it whole | |
| images | inputs are lent for the call; outputs are copied out and sd.cpp frees its own, so the C runtime that made them frees them | |
| logs | sd.cpp's log goes to `tracing` under `stable-diffusion.cpp`; nothing from it unwinds into C | |
| errors | a string with a NUL, a path that isn't Unicode, a size out of C's range, and sd.cpp's own failures, whose reason is in its log | |

## Sources

Read in November 2026.

- [sd.cpp], [sd.cpp header]

[sd.cpp]: https://github.com/leejet/stable-diffusion.cpp
[sd.cpp header]: https://github.com/leejet/stable-diffusion.cpp/blob/b9d7176/include/stable-diffusion.h
