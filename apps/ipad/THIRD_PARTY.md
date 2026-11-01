# Third-party components

- Koharu: upstream [koharu-rs/koharu](https://github.com/koharu-rs/koharu), MIT OR Apache-2.0. Original icon and shared translation prompt remain under the repository licenses.
- llama.cpp: [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp/tree/b12024), MIT. The official prebuilt XCFramework is downloaded by bootstrap, not committed. Archive SHA-256: `b7005ad25dab07d273794afc7ac24d55aeac1b52376301907c5fb4762e42545d`. The upstream license is bundled in `Resources/llama-LICENSE.txt`.
- Qwen 3.5 catalog downloads: [unsloth/Qwen3.5-0.8B-GGUF](https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF) and [unsloth/Qwen3.5-4B-GGUF](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF), Apache-2.0 according to their model cards. Exact revisions and filenames are recorded in `Resources/local-models.json`. Model weights are optional downloads, not app assets. Imported models have their own licenses and hardware requirements.
- Optional LaMa CoreML conversion: [Dadm-n/lama-dilated-coreml](https://huggingface.co/Dadm-n/lama-dilated-coreml/tree/51853822134974a99c9a473175228104f6c97767), derived from [advimman/lama](https://github.com/advimman/lama), Apache-2.0 according to the conversion's model card. Exact file URLs/revision are in `Resources/cleanup-model.json`.
- SwiftUI, WebKit, Vision, CoreML and Security are Apple platform frameworks supplied by the OS/SDK.

Rawkuma is an external content source. This fork is not affiliated with the site or model publishers.
