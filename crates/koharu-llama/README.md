# koharu-llama

llama.cpp for Koharu's language models.

Koharu's own fork of llama-cpp-rs's `llama-cpp-2` [llama-cpp-rs]: safe wrappers
over nearly direct bindings to llama.cpp, so it keeps up with llama.cpp rather
than hiding it behind an API of its own. The links are under
[Sources](#sources).

## Goal

`koharu-llm` runs local models in-process on llama.cpp as llama.cpp runs them.
This crate makes that safe and stays close enough to llama.cpp's API that a
new llama.cpp release is a small change.

## 1. Model

| Part | Model | Cite |
|---|---|---|
| shape | llama.cpp's own objects, each a Rust type that frees what it owns: the backend, a model, a context, a batch, the vocabulary, a sampler, and a multimodal context | llama-cpp-2 wraps llama.cpp the same way [llama-cpp-rs] |
| loading | through `koharu-llama-sys`, whose functions resolve once the runtime opens the libraries, so nothing links at build time | |
| the common library | fitting a model's parameters to free memory, a JSON Schema as a grammar, and multi-token prediction, through the C wrapper the release builds in | |
| grammars | sampling held to a grammar compiled by llguidance, besides llama.cpp's own grammars | llguidance [llguidance] |
| multimodal | images and audio into a model's context through llama.cpp's multimodal library | |
| logs | llama.cpp's, ggml's, and the multimodal library's logs go to `tracing`, each library's continuation lines kept with the message they continue | |

## 2. Where it differs from llama-cpp-2

The fork is Koharu's to change. It diverges where llama.cpp has moved on, as
when llama.cpp v1.0.0 removed the deprecated functions `llama-cpp-2` still
calls, and where Koharu loads at runtime what upstream links.

- **No upstream sources kept unchanged.** It builds under the workspace's
  edition and lints, and what a lint finds is fixed rather than allowed.
- **Tests run without llama.cpp.** The library isn't present when Koharu
  builds, so a test that would call it isn't kept.

## Sources

Read in November 2026.

- [llama-cpp-rs], [llguidance]

[llama-cpp-rs]: https://github.com/utilityai/llama-cpp-rs
[llguidance]: https://github.com/guidance-ai/llguidance
