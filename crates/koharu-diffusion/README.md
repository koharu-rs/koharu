# koharu-diffusion

`koharu-diffusion` runs stable-diffusion.cpp [sd.cpp] for Koharu's diffusion
models, FLUX.2 Klein and RORem, loaded when the runtime starts.
`koharu-diffusion-sys` holds the header, copied verbatim from sd.cpp `be48649`,
and the bindings generated from it. No crate on crates.io loads sd.cpp at run
time, so these stay.

```rust,ignore
let mut model = Context::new(&ContextParams {
    model: Some(&weights),
    max_vram: Some(budget),          // bytes, from the pipeline's lane
    ..ContextParams::default()       // sd.cpp's own defaults
})?;
let images = model.generate_image(&ImageParams {
    init: Some(&crop),
    mask: Some(&mask),
    ..ImageParams::default()
})?;
```

## Rules

| Rule | Evidence |
|---|---|
| Parameters start from sd.cpp's own initialisers, `sd_ctx_params_init` and `sd_img_gen_params_init`, with Koharu's values on top, so a default can't drift from upstream's. | sd.cpp's initialisers [sd init] |
| The pipeline's memory budget is passed as `max_vram`, in bytes, which this crate writes as the per-device budget sd.cpp reads, and sd.cpp places weights to fit it with `auto_fit` on, its default. | sd.cpp's `max_vram` and `auto_fit` [sd memory] |
| The budget isn't a hard cap: sd.cpp leaves some buffers out of it, so the pipeline's reserve covers them. | "it is not a hard physical" cap [sd backend] |
| Weights load when the model loads (`eager_load`), so a model that doesn't fit fails then, not partway through a page. | sd.cpp's `eager_load` [sd memory] |
| A prompt is encoded once per model. sd.cpp keeps a model's last few prompt encodings (`conditioning_cache_size`, 4 by default), and Koharu sends one prompt for every crop, so the text encoder, Qwen3 4B for FLUX.2 Klein, runs on the first crop and is skipped on the rest. The cache reuses an encoding exactly, so the image doesn't change. | sd.cpp's conditioning cache [sd conditioning cache], added for this request [sd prompt cache issue] |

sd.cpp reports no typed failure, only log lines, so running out of memory is
prevented by the budget rather than caught. The crate binds what Koharu uses: creating a context, generating an image from
an image and a mask, the parameters Koharu sets, and logging, sent to
`tracing`. Native images are freed by sd.cpp's own functions, and a panic in a
callback never crosses into C, as now. One sd.cpp commit is cited everywhere,
and the runtime checks the library's `sd_commit()` matches it when it loads.

[sd.cpp]: https://github.com/leejet/stable-diffusion.cpp/tree/be486498eadc29dc3cd234d944c2180ed42b1415
[sd init]: https://github.com/leejet/stable-diffusion.cpp/blob/be486498eadc29dc3cd234d944c2180ed42b1415/include/stable-diffusion.h#L538-L554
[sd memory]: https://github.com/leejet/stable-diffusion.cpp/blob/be486498eadc29dc3cd234d944c2180ed42b1415/include/stable-diffusion.h#L241-L247
[sd backend]: https://github.com/leejet/stable-diffusion.cpp/blob/be486498eadc29dc3cd234d944c2180ed42b1415/docs/backend.md#L45-L49
[sd conditioning cache]: https://github.com/leejet/stable-diffusion.cpp/blob/be486498eadc29dc3cd234d944c2180ed42b1415/include/stable-diffusion.h#L256
[sd prompt cache issue]: https://github.com/leejet/stable-diffusion.cpp/issues/1774
