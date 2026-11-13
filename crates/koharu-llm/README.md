# koharu-llm

Koharu's language models, local on llama.cpp or at a hosted provider the user chose, and
the in-process agent that edits a project through tools. Everything is built on rig: the
local engine is one more rig model, so translation and the agent work the same whichever
model answers.

## API

Models are named, described, and connected with rig's own types: a `vendor/model` reference,
as rig's `ModelSpec::reference` writes it, the model's `ModelSpec`, and a `DynModel`.

| Item | What it does |
|---|---|
| `connect(reference, size).await` | The `DynModel` a reference such as `anthropic/claude-opus-5-5` or `koharu/index-translate-2b` names, its files fetched first for one of Koharu's own. Only those use `size` |
| `models().await` | Every model the user can pick, as a `ModelSpec` |
| `local(&spec)` | Koharu's catalog's facts on one of its models: its quantizations, which are downloaded, and the languages it translates into |
| `answers(&model, schema)` | rig's agent builder asking for answers `schema` describes in the way the model's vendor keeps to one |
| `Agent::new(host, model, context)` | The agent's conversation about one open project |
| `Agent::run(prompt, events).await` | One request, until the model answers without tools |
| `Context` | A conversation kept inside a model's context window, in memory |
| `sign_in`, `sign_out` | ChatGPT's sign-in, in the system browser or with a device code |
| `AgentConfig`, `Providers` | The `[agent]` and `[providers]` config sections |

`rig` re-exports rig itself, since every model takes and gives its requests and streams.
`AgentConfig` holds a reference and has no default model: a caller names it.

`prompt_typed` checks an answer against its schema. A caller of plain `prompt` checks the
answer itself, because rig fails open on structured output.

## Vendors

Every vendor is a rig `ProviderId`, by its rig name.

| Vendor | Credential | Keeps to a schema by |
|---|---|---|
| `koharu` | None | An llguidance grammar compiled from the schema |
| `openai`, `anthropic`, `gcp.gemini`, `xai` | API key | The provider's own structured output |
| `chatgpt` | Sign-in | Asking in the prompt |
| `deepseek` | API key | Asking in the prompt, with JSON mode, because rig's DeepSeek dialect drops the schema |
| `minimax` | API key | Asking in the prompt, over its OpenAI dialect |
| `openrouter` | API key | The provider's own structured output, routed only to hosts that keep to the schema |
| `ollama` | Optional key | The server's own, at rig's default URL unless `[providers]` says otherwise |
| `lmstudio` | Optional key | The server's own, at `http://localhost:1234` unless `[providers]` says otherwise |

`koharu`, the in-process engine, and `lmstudio`, which rig has no dialect for, are declared
at run time. One of Koharu's own models takes a quantization after a colon, as Ollama's tags
do (`koharu/index-translate-2b:Q8_0`), and otherwise loads its first.

`[providers]` holds a vendor's `base_url` by its rig name, as Codex's
`[model_providers.<id>]` does: `[providers.ollama] base_url = "http://gpu:11434/v1"`.

Keys and the ChatGPT sign-in live in `koharu-config`'s encrypted secrets under the vendor's
rig name, so rig reads no environment variables. A vendor that needs a key and has none lists
no models, and a vendor whose list fails is left out and logged.

rig's catalog says what a hosted model can do. A listed model it knows as neither calling
tools nor keeping to a schema, such as an embedding model, is dropped. For a model it
doesn't know, rig refuses nothing and the limits stay unknown. LM Studio's list says what
its own models can do, and ChatGPT's comes from Codex's model list for the signed-in plan.

Hosted requests share one HTTP client. A 429 or 5xx reply is sent again up to 3 times,
after the wait the reply names, or after 1, 2, then 4 seconds. rig itself retries nothing.

## Local models

The catalog pins each model to a Hugging Face commit and holds its quantizations, its
projector if it reads images, its markup, its sampling defaults, and the languages it can
translate into. It offers LFM3, Gemma 4, Qwen 3.5, 3.6, and 3.8, and the translation model
Index-Translate, which decodes greedily as its card does. A translation model only
translates: its spec's `details.kind` is `translation`, the field rig keeps models.dev's
kinds in, spelled as Hugging Face's pipeline tag, and it says the model calls no tools,
whatever markup its template writes. It reasons when its template does: Index-Translate's
opens a think block unless reasoning is off.

- **Loading.** `Size` asks for a number of sequences, the tokens each may hold, and whether
  the model reads images, and `Size::AGENT` is one sequence of 65,536 with text only. A
  model's projector is fetched and loaded only for images, so without them its memory goes
  to the model, and the spec says it reads none. llama.cpp's fit then measures free memory,
  leaving 1 GiB plus the projector free. It shrinks the context, down to 4,096 tokens a
  sequence, and then moves layers to system memory. The model's spec says what it got.
- **Prompts.** The GGUF's own chat template renders with minijinja, as llama.cpp renders it
  with its Jinja engine, stripping whitespace around blocks as it does. A conversation
  merges two messages in a row with the same role, because most Qwen templates raise an
  error on a second leading system message.
- **Decoding.** One thread owns the model, and its sequences decode together. A sequence
  keeps what it decoded after its request ends, and a new request reuses the longest common
  prefix any sequence holds, if it's at least 64 tokens. A recurrent or hybrid model's state
  can't be cut back to that prefix, so its sequence keeps a checkpoint of its recurrent state
  at a micro-batch boundary short of each prompt's end, and goes back to it; the answer is the
  same as decoding the whole prompt again. A sequence holds a prompt only up to its first
  image, and the checkpoint lies before it, so a turn after one that showed an image reuses
  the history they share.
- **Sampling.** A request starts from the catalog's `Sampling` and can override it through
  rig's options and `additional_params`. An unset value leaves its sampler out. An answer,
  reasoning included, may take a quarter of the context, the room a conversation's `Context`
  keeps free for it.

Markup is how a model's template writes reasoning and tool calls in its reply:

| Markup | Models | Reasoning | Tool call |
|---|---|---|---|
| `FUNCTION_TAGS` | LFM3, Qwen, Index-Translate | `<think>`, `</think>` | `<tool_call>`, `</tool_call>` |
| `GEMMA4` | Gemma 4 | `<\|channel>thought`, `<channel\|>` | `<\|tool_call>`, `<tool_call\|>` |
| `GEMMA4_NO_TOOLS` | Gemma 4 templates that render a tool result as a `tool` turn | The same | None |

The parser reads a reply as it streams, holding back what could be the start of a marker.
A reply starts in reasoning if the template left a reasoning marker open at the prompt's
end. It looks for calls only when the request offered tools. A call's arguments that don't
parse stay as written, so rig can send the model back to correct them. A reply held to a
schema while the model reasons reasons freely, and the grammar holds it from the reasoning's
close marker on, as llama.cpp's lazy grammar starts at a trigger. A schema and tools in one
request fail.

## The agent

The app implements `Host`: the project's state, its tools, and how to run one. Each request
sends that state as a document ahead of the history, so the model sees only the latest and
the history doesn't keep old copies.

- **Limits.** A run stops after 20 model calls, and lets the model correct a bad call twice.
- **Tools.** Arguments are checked against the tool's schema first, and a mismatch goes back
  to the model. A tool runs detached, so dropping the run's future cancels the run but lets
  a running tool finish, as each edit is whole or not at all.
- **Local models.** The catalog's settings are for translation, so a run turns the repeat
  penalty off.
- **Events.** `events` hears `Text`, `Reasoning`, `ToolStarted`, and `ToolFinished`.

## Context

`Context::fit` makes room before a request: it clears old tool outputs, then summarizes
older messages if that isn't enough. `Context::compact` summarizes now, because the user
asked or the model refused the request. A model whose spec says it only translates can't
summarize, so its oldest turns are dropped at the cut rig-memory picks, each reply kept with
its turn. `Agent::run` fits first, and if the model refuses a
request as too long before any tool has run, it compacts and tries once more. During a run,
a turn over the threshold is sent with older tool outputs cleared on a copy of its history.

rig's compaction defaults assume a large window, so the reserve and the tool output kept are
capped at a quarter of the model's input limit. A count starts from the tokens a provider
last reported, because rig's estimate of 4 bytes a token undercounts Japanese and Chinese.
rig's summary prompts are written for a coding agent, so `Context::new` takes Koharu's own.
