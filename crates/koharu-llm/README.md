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
| a server's address | `[providers]` holds a vendor's `base_url` by its rig name, such as a remote Ollama; the interface offers it for the servers the user names: Ollama, LM Studio, and an OpenAI-compatible one | Codex's `[model_providers]` [Codex config] |
| OpenAI-compatible | one server at the base URL the user gives, such as a relay, spoken to in Chat Completions, which such servers speak far more widely than Responses; its `/models` list says nothing of what a model can do | Zed's OpenAI-compatible provider [Zed providers] |
| credentials | API keys and sign-ins live in Koharu's encrypted secrets, under the vendor's name; no environment variable is read | |
| sign-in | `chatgpt` and `grok` sign in with the user's plan in place of a key: ChatGPT through rig's sign-in, the route Codex takes, and Grok, a SuperGrok or X Premium plan, with xAI's device code through xAI's shared client, its requests going to the endpoint the Grok CLI uses | Hermes Agent and OpenClaw sign in to xAI so [Hermes xAI] [OpenClaw xAI] |
| listing | every model the user can pick, from each vendor with a key; one that needs a key and has none lists nothing, and a list that fails is left out | LM Studio's own list says what its models can do [LM Studio list] |
| what a model can do | rig's catalog says; a model it knows as neither calling tools nor keeping to a schema, such as an embedding model, is dropped, and one it doesn't know is offered with its limits unknown | |
| retries | a request is sent again when its reply says it failed for now, as a 429 or 5xx does, or its connection was lost before one: up to 9 times, or 3 for a model list, which the interface waits for, after the wait the reply names, else until its rate-limit headers say a spent allowance resets, else one growing from 1 second to 30. A spent quota, a wait over 2 minutes, and a server that can't be reached fail at once | Gemini CLI tries 10 times, up to 30 seconds apart [Gemini CLI retries]; OpenAI's Python library doesn't retry past 2 minutes [OpenAI retries] |
| waiting | a reply is waited for until it ends or the user stops it, however long its server says nothing, as while it loads a model, reads a long prompt, or thinks | |
| storing | a vendor is asked not to store replies in the Responses format, which OpenAI and xAI otherwise do for 30 days; the Grok plan's own endpoint isn't asked | [OpenAI data controls], [xAI Responses] |

## 2. Structured answers

A translation answers in a shape a schema describes, kept to it the way each
vendor keeps to one:

| Vendor | Credential | Keeps to a schema by |
|---|---|---|
| `koharu` | none | a grammar compiled from the schema, which sampling can't leave, so even a small model writes an answer translation can read |
| `openai`, `anthropic`, `gcp.gemini`, `xai` | API key | the provider's own structured outputs [OpenAI structured outputs] |
| `grok` | sign-in | xAI's own, as `xai` |
| `openrouter` | API key | the provider's own, routed only to hosts that keep to it |
| `chatgpt` | sign-in | asking in the prompt |
| `deepseek`, `minimax` | API key | asking in the prompt |
| `openai-compatible` | optional key | asking in the prompt, as the server may take no schema |
| `ollama`, `lmstudio` | optional key | the server's own |

rig checks an answer against its schema whichever way it was kept. One that
doesn't fit is asked for once more and then fails; one the token limit cut off
fails at once, as the cut answer would leave the next less room, and so does one
its provider filtered.

## 3. Local models

| Part | Model | Cite |
|---|---|---|
| catalog | each model pinned to a Hugging Face commit, with its quantizations, its vision projector if it reads images, how its template writes reasoning and tool calls, its sampling defaults, and the languages it translates into | |
| translation models | only translate, and say so: they take no tools, whatever their template writes | |
| loading | llama.cpp's fit measures free memory and shrinks the context, down to 4,096 tokens a sequence, then moves layers to system memory, leaving 1 GiB and the projector free; a projector loads only when images are asked for | llama.cpp's own fit [llama.cpp fit] |
| prompts | the model's own chat template from its GGUF, rendered as llama.cpp renders it with its Jinja engine | |
| decoding | one thread owns the model, and its sequences decode together | llama.cpp's parallel decoding [llama.cpp parallel] |
| repetition | a reply that ends in one pattern over and over, for 128 tokens and four times, is cut off as at its token limit | vLLM's repetition detection, whose sizes the caller sets [vLLM repetition] |
| sliding windows | a layer that attends to a sliding window keeps that window, not the whole context | llama.cpp's own tools, where its library keeps the whole context by default [llama.cpp SWA] |
| reuse | a request reuses the longest prefix any sequence holds, at least 64 tokens; a recurrent model keeps a checkpoint of its state short of each prompt's end to go back to, with the same answer as decoding the whole prompt again, and one with a sliding window keeps the window there, for a prompt the window has moved past | llama-server's context checkpoints [llama.cpp checkpoints] |
| reasoning and tool calls | read from the reply as it streams, by markers its template writes; a reply held to a schema may reason first, where its prompt left reasoning open or by opening it at its start, and the grammar holds the rest, so markers in the answer are text; a call the token limit cut off never runs | llama.cpp's Gemma 4 grammar: an optional thought, then the schema [llama.cpp Gemma 4] |

## 4. Context

A conversation keeps inside its model's window with rig's compaction, the pi
coding agent's [pi compaction]:

- **Fitting** summarizes older messages before a request that wouldn't leave the
  model's reserve free; a summary that fails leaves the conversation as it was.
  **Compacting** summarizes now, because the model refused a request as too
  long. **Halving** drops the older half, for a model that filled its window,
  which a summary might not fit either.
- **A translation model** can't summarize, so its oldest turns are dropped,
  each reply with its turn.
- **Counting** starts from the tokens a provider last reported, because rig's
  estimate of four characters a token undercounts Japanese and Chinese. What a
  compaction keeps is scaled by them too, or it would keep all of a Japanese
  conversation.
- **Summaries** are written for a translator by its own prompts and for the
  agent by rig's, and the room they keep is capped at a quarter of the model's
  window, which rig's defaults, made for large windows, would overrun.

## Sources

Read in November 2026.

- [rig], [Ollama API], [Codex config], [OpenAI structured outputs], [LM Studio list], [Zed providers]
- Hosted requests: [Gemini CLI retries], [OpenAI retries], [OpenAI data controls], [xAI Responses]
- xAI's sign-in: [Hermes xAI], [OpenClaw xAI]
- Context: [pi compaction]
- llama.cpp: [llama.cpp fit], [llama.cpp parallel], [llama.cpp SWA], [llama.cpp checkpoints], [llama.cpp Gemma 4]
- Repetition: [vLLM repetition]

[rig]: https://github.com/0xPlaygrounds/rig
[Ollama API]: https://github.com/ollama/ollama/blob/main/docs/api.md
[Codex config]: https://developers.openai.com/codex/config-reference
[OpenAI structured outputs]: https://developers.openai.com/api/docs/guides/structured-outputs
[LM Studio list]: https://lmstudio.ai/docs/developer/rest/list
[Zed providers]: https://zed.dev/docs/ai/llm-providers#openai-api-compatible
[Gemini CLI retries]: https://github.com/google-gemini/gemini-cli/blob/60f8082b14ffa7f10b608270232f3f57b5abbec8/packages/core/src/utils/retry.ts#L42-L46
[OpenAI retries]: https://github.com/openai/openai-python/blob/804a9af5a5199f36d28dfdf84c7981f0253a3a3f/src/openai/_base_client.py#L922-L930
[OpenAI data controls]: https://platform.openai.com/docs/guides/your-data
[xAI Responses]: https://docs.x.ai/developers/rest-api-reference/inference/responses
[Hermes xAI]: https://github.com/NousResearch/hermes-agent/blob/main/hermes_cli/auth_xai.py
[OpenClaw xAI]: https://github.com/openclaw/openclaw/blob/main/extensions/xai/xai-oauth.ts
[pi compaction]: https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/compaction.md
[llama.cpp fit]: https://github.com/ggml-org/llama.cpp/blob/785322d17f2d/common/fit.h
[llama.cpp parallel]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476
[llama.cpp SWA]: https://github.com/ggml-org/llama.cpp/pull/13194
[llama.cpp checkpoints]: https://github.com/ggml-org/llama.cpp/pull/15293
[vLLM repetition]: https://github.com/vllm-project/vllm/blob/85a084c38c2c714c6fbdf0bdcd195b7f0301dece/vllm/v1/core/sched/utils.py#L28-L62
[llama.cpp Gemma 4]: https://github.com/ggml-org/llama.cpp/blob/d7d1b632da766f1247456b4f42406a96a0609d93/common/parsers/gemma4.cpp#L344-L351
