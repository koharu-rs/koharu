# koharu-diffusion

Safe Rust bindings to stable-diffusion.cpp's image generation. Calls go through
`koharu-diffusion-sys`, so its `load` must have succeeded first.

## API

`Context::new(&ContextParams)` loads a model, and `Context::generate_image(&ImageParams)`
returns a `Vec<RgbImage>`. A `Context` is `Send` but not `Sync`, and generating takes
`&mut self`.

Parameters start from `sd_ctx_params_init` and `sd_img_gen_params_init`, so anything
left unset keeps sd.cpp's default. `Scheduler`, `SampleMethod`, and `VaeFormat` have
only the variants in use: `Discrete`, `Euler`, and `Flux2`.

| `ContextParams` | sd.cpp field |
|---|---|
| `model`, `diffusion_model`, `vae`, `clip_l`, `clip_g`, `llm` | `model_path`, `diffusion_model_path`, `vae_path`, `clip_l_path`, `clip_g_path`, `llm_path` |
| `device` | `backend`, by ggml's device name, such as `CUDA0` |
| `vae_format` | `vae_format` |
| `mmap` | `enable_mmap` |
| `flash_attention` | `flash_attn`, `diffusion_flash_attn` |
| `conv_direct` | `diffusion_conv_direct`, `vae_conv_direct` |

| `ImageParams` | sd.cpp field |
|---|---|
| `prompt`, `negative_prompt`, `width`, `height`, `strength`, `seed` | The field of the same name |
| `init`, `mask` | `init_image`, `mask_image`. White in the mask is repainted |
| `references`, `reference_args` | `ref_images`, `ref_image_args` |
| `cfg` | `sample_params.guidance.txt_cfg` |
| `scheduler`, `sample_method`, `flow_shift` | The field of the same name in `sample_params` |
| `steps` | `sample_params.sample_steps` |
| `count` | `batch_count` |

## Behavior

- **Loading.** `eager_load` is always on, so a model that doesn't fit fails in
  `Context::new` and not partway through a generation. A model that can't generate
  images also fails there.
- **Prompt cache.** sd.cpp keeps its last few prompt encodings
  (`conditioning_cache_size`, 4 by default), so reusing a prompt encodes it once.
- **Images.** Inputs are borrowed for the call. Outputs are copied into `RgbImage`, and
  `free_sd_image_generation_result` frees sd.cpp's copies, so the C runtime that
  allocated them also frees them.
- **Logging.** sd.cpp's log goes to `tracing` under the target `stable-diffusion.cpp`.
  The callback is installed once per process, and a panic in it never unwinds into C.

## Errors

| `Error` | When |
|---|---|
| `Nul` | A string argument holds a NUL byte |
| `NonUnicodePath` | A path isn't valid Unicode |
| `OutOfRange` | A count or size doesn't fit in an `i32` |
| `Load`, `Generate` | sd.cpp reports failure. Its log has the reason |
| `Output` | sd.cpp returned an image it can't have made, such as one without 3 channels |
