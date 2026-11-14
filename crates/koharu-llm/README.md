# koharu-llm

Koharu's language models.

A language model runs on this machine, in-process on llama.cpp, so nothing
listens on a port and one GPU's memory is planned in one place, or at a hosted
provider the user chose. Every model is a rig model [rig], so translation and
the agent work the same whichever one answers. Where it follows an established
product, it cites it; the links are under [Sources](#sources).

## Goal

The user picks a model by name, local or hosted, and nothing else in Koharu
changes. A local model is one more provider, with the same requests, streams,
tools, and structured answers as a hosted one, and a page's text goes to a
provider only when the user picked that provider.

## 1. Naming and connecting

| Part | Model | Cite |
|---|---|---|
| name | `vendor/model`, as rig names a model, such as `anthropic/claude-opus-5-5` or `koharu/index-translate-9b` | |
| quantization | one of Koharu's own models takes it after a colon, `koharu/index-translate-9b:Q8_0`, and otherwise loads its first | Ollama names a model `model:tag` [Ollama API] |
| a server's address | `[providers]` holds a vendor's `base_url` by its rig name, such as a remote Ollama | Codex's `[model_providers]` [Codex config] |
| credentials | API keys and the ChatGPT sign-in live in Koharu's encrypted secrets, under the vendor's name; no environment variable is read | |
| listing | every model the user can pick, from each vendor with a key; one that needs a key and has none lists nothing, and a list that fails is left out | LM Studio's own list says what its models can do [LM Studio list] |
| what a model can do | rig's catalog says; a model it knows as neither calling tools nor keeping to a schema, such as an embedding model, is dropped, and one it doesn't know is offered with its limits unknown | |
| retries | a 429 or 5xx reply is sent again up to 3 times, after the wait the reply names or a growing one | |

## 2. Structured answers

A translation answers in a shape a schema describes, kept to it the way each
vendor keeps to one:

| Vendor | Credential | Keeps to a schema by |
|---|---|---|
| `koharu` | none | a grammar compiled from the schema, which sampling can't leave, so even a small model writes an answer translation can read |
| `openai`, `anthropic`, `gcp.gemini`, `xai` | API key | the provider's own structured outputs [OpenAI structured outputs] |
| `openrouter` | API key | the provider's own, routed only to hosts that keep to it |
| `chatgpt` | sign-in | asking in the prompt |
| `deepseek`, `minimax` | API key | asking in the prompt |
| `ollama`, `lmstudio` | optional key | the server's own |

An answer is checked against its schema whichever way it was kept, because rig
fails open on structured output.

## 3. Local models

| Part | Model | Cite |
|---|---|---|
| catalog | each model pinned to a Hugging Face commit, with its quantizations, its vision projector if it reads images, how its template writes reasoning and tool calls, its sampling defaults, and the languages it translates into | |
| translation models | only translate, and say so: they take no tools, whatever their template writes | |
| loading | llama.cpp's fit measures free memory and shrinks the context, down to 4,096 tokens a sequence, then moves layers to system memory, leaving 1 GiB and the projector free; a projector loads only when images are asked for | llama.cpp's own fit [llama.cpp fit] |
| prompts | the model's own chat template from its GGUF, rendered as llama.cpp renders it with its Jinja engine | |
| decoding | one thread owns the model, and its sequences decode together | llama.cpp's parallel decoding [llama.cpp parallel] |
| reuse | a request reuses the longest prefix any sequence holds, at least 64 tokens; a recurrent model keeps a checkpoint of its state short of each prompt's end to go back to, with the same answer as decoding the whole prompt again | |
| reasoning and tool calls | read from the reply as it streams, by markers its template writes; a reply held to a schema reasons freely, and the grammar holds it from the reasoning's end | |

## 4. Context

A conversation keeps inside its model's window:

- **Fitting** clears old tool outputs first, then summarizes older messages if
  that isn't enough. **Compacting** summarizes now, because the user asked or
  the model refused a request as too long.
- **A translation model** can't summarize, so its oldest turns are dropped,
  each reply with its turn.
- **Counting** starts from the tokens a provider last reported, because an
  estimate of four bytes a token undercounts Japanese and Chinese.
- **Summaries** are written for a translator, not a coding agent, and the room
  they keep is capped at a quarter of the model's window, which rig's defaults,
  made for large windows, would overrun.

## Sources

Read in November 2026.

- [rig], [Ollama API], [Codex config], [OpenAI structured outputs], [LM Studio list]
- llama.cpp: [llama.cpp fit], [llama.cpp parallel]

[rig]: https://github.com/0xPlaygrounds/rig
[Ollama API]: https://github.com/ollama/ollama/blob/main/docs/api.md
[Codex config]: https://developers.openai.com/codex/config-reference
[OpenAI structured outputs]: https://developers.openai.com/api/docs/guides/structured-outputs
[LM Studio list]: https://lmstudio.ai/docs/developer/rest/list
[llama.cpp fit]: https://github.com/ggml-org/llama.cpp/blob/785322d17f2d/common/fit.h
[llama.cpp parallel]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476
