# koharu-llm

Koharu's language models, local on llama.cpp or at a hosted provider the user chose, and
the in-process agent that edits a project through tools. Everything is built on rig: the
local engine is one more rig model, so translation and the agent work the same whichever
model answers.

## API

| Item | What it does |
|---|---|
| `connect(&selection, size).await` | The `Model` a `Selection` names, its files fetched first for a local one. Only a local one uses `size` |
| `models().await` | Every model the user can pick, with what each can do. A local one says which quantizations are downloaded |
| `Model::agent()` | rig's agent builder on this model |
| `Model::answers(schema)` | The same, asking for answers `schema` describes in the way this provider keeps to one |
| `Agent::new(host, model, context)` | The agent's conversation about one open project |
| `Agent::run(prompt, events).await` | One request, until the model answers without tools |
| `Context` | A conversation kept inside a model's context window, in memory |
| `sign_in`, `sign_out` | ChatGPT's sign-in, in the system browser or with a device code |
| `AgentConfig`, `Providers` | The `[agent]` and `[providers]` config sections |

`rig` re-exports rig itself, since every model takes and gives its requests and streams.
`Selection` has no default: a caller names the model.

`prompt_typed` checks an answer against its schema. A caller of plain `prompt` checks the
answer itself, because rig fails open on structured output.

## Providers

| Provider | Credential | Keeps to a schema by |
|---|---|---|
| `local` | None | An llguidance grammar compiled from the schema |
| `openai`, `claude`, `gemini`, `xai` | API key | The provider's own structured output |
| `chatgpt` | Sign-in | Asking in the prompt |
| `deepseek` | API key | Asking in the prompt, with JSON mode, because rig's DeepSeek dialect drops the schema |
| `minimax` | API key | Asking in the prompt, over its OpenAI dialect |
| `openrouter` | API key | The provider's own structured output, routed only to hosts that keep to the schema |
| `lm-studio` | Optional key | The server's own, at `http://localhost:1234` unless `[providers]` says otherwise |
| `openai-compatible` | Optional key | The server's own, at `http://localhost:11434/v1` unless `[providers]` says otherwise |

Keys and the ChatGPT sign-in live in `koharu-config`'s encrypted secrets, so rig reads no
environment variables. A provider that needs a key and has none lists no models, and a
provider whose list fails is left out and logged.

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
translates, so `models()` says it neither calls tools nor reasons, whatever markup its
template writes.

- **Loading.** `Size` asks for a number of sequences and the tokens each may hold, and
  `Size::AGENT` is one sequence of 65,536. llama.cpp's fit then measures free memory,
  leaving 1 GiB plus the projector free. It shrinks the context, down to 4,096 tokens a
  sequence, and then moves layers to system memory. The model's spec says what it got.
- **Prompts.** The GGUF's own chat template renders with minijinja, as llama.cpp renders it
  with its Jinja engine, stripping whitespace around blocks as it does. A conversation
  merges two messages in a row with the same role, because most Qwen templates raise an
  error on a second leading system message.
- **Decoding.** One thread owns the model, and its sequences decode together. A sequence
  keeps what it decoded after its request ends, and a new request reuses the longest common
  prefix any sequence holds, if it's at least 64 tokens.
- **Sampling.** A request starts from the catalog's `Sampling` and can override it through
  rig's options and `additional_params`. An unset value leaves its sampler out.

Markup is how a model's template writes reasoning and tool calls in its reply:

| Markup | Models | Reasoning | Tool call |
|---|---|---|---|
| `FUNCTION_TAGS` | LFM3, Qwen, Index-Translate | `<think>`, `</think>` | `<tool_call>`, `</tool_call>` |
| `GEMMA4` | Gemma 4 | `<\|channel>thought`, `<channel\|>` | `<\|tool_call>`, `<tool_call\|>` |
| `GEMMA4_NO_TOOLS` | Gemma 4 templates that render a tool result as a `tool` turn | The same | None |

The parser reads a reply as it streams, holding back what could be the start of a marker.
A reply starts in reasoning if the template left a reasoning marker open at the prompt's
end. It looks for calls only when the request offered tools. A call's arguments that don't
parse stay as written, so rig can send the model back to correct them.

## The agent

The app implements `Host`: the project's state, its tools, and how to run one. Each request
sends that state as a document ahead of the history, so the model sees only the latest and
the history doesn't keep old copies.

- **Limits.** A run stops after 20 model calls, and lets the model correct a bad call twice.
- **Tools.** Arguments are checked against the tool's schema first, and a mismatch goes back
  to the model. A tool runs detached, so dropping the run's future cancels the run but lets
  a running tool finish, as each edit is whole or not at all.
- **Local models.** The catalog's settings are for translation, so a run turns the repeat
  penalty off and allows a quarter of the window for each answer.
- **Events.** `events` hears `Text`, `Reasoning`, `ToolStarted`, and `ToolFinished`.

## Context

`Context::fit` makes room before a request: it clears old tool outputs, then summarizes
older messages if that isn't enough. `Context::compact` summarizes now, because the user
asked or the model refused the request. `Agent::run` fits first, and if the model refuses a
request as too long before any tool has run, it compacts and tries once more. During a run,
a turn over the threshold is sent with older tool outputs cleared on a copy of its history.

rig's compaction defaults assume a large window, so the reserve and the tool output kept are
capped at a quarter of the model's input limit. A count starts from the tokens a provider
last reported, because rig's estimate of 4 bytes a token undercounts Japanese and Chinese.
rig's summary prompts are written for a coding agent, so `Context::new` takes Koharu's own.
