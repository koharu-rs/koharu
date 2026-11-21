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
| parameters | set by key on sd.cpp's option objects, so anything left unset is what sd.cpp would use; only the options Koharu's models need are named | sd.cpp's option keys [sd.cpp C API] |
| loading | every weight loads up front, so a model that doesn't fit, or can't make images, fails when it loads and not partway through a generation | sd.cpp's eager loading [sd.cpp header] |
| placing weights | sd.cpp fits the weights into the device's free memory itself, then system memory | |
| kernels | sd.cpp picks flash attention and direct convolution for each module and backend, so none is forced on or off | sd.cpp's automatic selection [sd.cpp backends] |
| a schedule | a model whose checkpoint saves its own noise levels steps through them, in place of a step count | sd.cpp's custom sigmas [sd.cpp C API] |
| stopping | a canceller stops a generation under way from another thread; sd.cpp clears the request as a generation starts | `sd_cancel_generation` [sd.cpp header] |
| prompts | sd.cpp keeps its last few prompts' encodings, so a prompt used again isn't encoded again | |
| a model | one at a time per context: a context can move between threads, but generating takes it whole | |
| images | inputs are copied into sd.cpp's tensors; outputs are copied out of its result, which sd.cpp frees, so the C runtime that made them frees them | sd.cpp's tensors and results [sd.cpp C API] |
| logs | sd.cpp's log goes to `tracing` under `stable-diffusion.cpp` from the first model loaded, since sd.cpp keeps one log callback for the process; nothing from it unwinds into C | |
| errors | one type for each operation, loading, generating, and setting an option, as koharu-llama has; a status sd.cpp fails a call with is its own variant, with the reason sd.cpp gives for it | sd.cpp's `sd_status_t` and `sd_get_last_error` [sd.cpp header] |

## Sources

Read in November 2026.

- [sd.cpp], [sd.cpp header], [sd.cpp C API], [sd.cpp backends]

[sd.cpp]: https://github.com/leejet/stable-diffusion.cpp
[sd.cpp header]: https://github.com/leejet/stable-diffusion.cpp/blob/532d2c1/include/stable-diffusion.h
[sd.cpp C API]: https://github.com/leejet/stable-diffusion.cpp/blob/532d2c1/docs/guides/c_api.md
[sd.cpp backends]: https://github.com/leejet/stable-diffusion.cpp/blob/532d2c1/docs/guides/backend.md
