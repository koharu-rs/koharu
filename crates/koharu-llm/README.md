# koharu-llm

`koharu-llm` runs Koharu's language models, on the user's machine or at a
hosted provider the user chose, and the in-process agent that edits a project
through tools. It's built on `rig` [rig]: hosted models are rig's providers,
and Koharu's local llama.cpp engine is one more rig model, so page
translation, PaddleOCR-VL OCR, and the agent send rig's requests and read rig's
streams whichever model answers.

## What it owns

| From | Here | Gone |
|---|---|---|
| `koharu-translator`'s hosted providers, model listing, and GGUF catalog | rig's providers, Koharu's model listing, and the local catalog | DeepL, Google Cloud Translation, and Caiyun; five copies of one hand-written client; the `translate` binary that read keys from environment variables |
| `koharu-ml`'s `llm` module | the local engine, on `koharu-llama`, as a rig model | a llama.cpp context per call |
| `koharu-agent` | the agent, on rig's agent loop, with rig's ChatGPT sign-in | its own loop, and its own Codex sign-in and token storage |

Translation's prompt, schema, and answer check belong to the pipeline's
translation step, and PaddleOCR-VL's task prompt to its model in `koharu-ml`,
which runs on the local engine here. `koharu-translator` and `koharu-agent`
are removed.

## Models

```rust,ignore
let model = koharu_llm::model(&selection, size).await?;  // local or hosted, as a rig model
let request = model.structured(request, schema);         // JSON the way this model keeps to it
let models = koharu_llm::models().await;                 // what the user can pick, with capabilities
```

| Rule | Evidence |
|---|---|
| Hosted models come from rig's providers: OpenAI's Responses API, Claude, Gemini, xAI, DeepSeek, MiniMax, and OpenRouter have their own clients, and LM Studio and OpenAI-compatible servers use its OpenAI dialect at their base URL. | rig's providers [rig OpenAI dialect]; `AGENTS.md`: a well-known crate before code of its own |
| The local engine is a rig model: Koharu implements rig's completion wire and transport for it, so translation and OCR send it the same request as a hosted model. Its option mapping names every option rig has, so a new one fails the build until the engine says what it does with it. | rig's `Wire` [rig wire] and replay target [rig replay target] are public for other engines to implement |
| A provider's key comes from `koharu-config`'s secrets, never an environment variable, and requests run on Koharu's HTTP client, so proxies and timeouts are the app's. | rig connects through a `reqwest::Client` it's given [rig connect] [rig reqwest] |
| A 429 or 5xx is retried up to three times, after the wait rig reads from the provider's headers, or one, two, then four seconds when they name none; then the request fails with the provider's message. The HTTP client Koharu gives rig retries, before any of the reply is read. | rig reads `retry-after` but doesn't retry [rig retry]; Anthropic [Claude rate limits] and OpenAI [OpenAI rate limits] direct waiting |
| Cancelling is dropping the stream: a local sequence is freed at the next step, and a hosted request's connection closes. | rig's streams stop when dropped [rig agent readme] |

`models()` lists every model the user can pick, with what it can do: images,
tools, reasoning, and how it keeps to a JSON schema. rig lists each provider's
models, and as its lists carry no capabilities [rig listing], they come from
rig's catalog, built from models.dev [rig catalog]. A listed model the catalog
doesn't know can do none of the three, and one it knows as neither calling
tools nor keeping to a schema, such as an embedding model, isn't listed. The
pipeline and the agent offer models by them.

| Provider | rig client | How Koharu asks for JSON | Listing |
|---|---|---|---|
| local | Koharu's engine | a JSON schema, enforced by an llguidance grammar | the catalog |
| OpenAI | Responses | the schema, strict [OpenAI structured outputs] | rig's list |
| ChatGPT | ChatGPT, Codex's route | the schema in the instructions; the route takes no structured output [rig codex limits] | Codex's model list |
| Claude | Anthropic | the schema, as `output_config` [Claude structured outputs] | rig's list |
| Gemini | Gemini | the schema, as `responseJsonSchema` [Gemini structured output] | rig's list |
| xAI | xAI Responses | the schema [xAI structured outputs] | rig's list |
| DeepSeek | DeepSeek | JSON mode, with the schema in the instructions [DeepSeek JSON mode] | rig's list |
| MiniMax | MiniMax | the schema in the instructions; MiniMax documents no JSON setting | rig's list |
| OpenRouter | OpenRouter | the schema, with `provider.require_parameters` [OpenRouter structured outputs] | rig's list |
| LM Studio | OpenAI dialect | the schema, enforced by a grammar [LM Studio structured output] | `/api/v1/models` with capabilities [LM Studio models], as rig neither lists nor catalogs it |
| OpenAI-compatible | OpenAI dialect | the schema | rig's list, with no capabilities |

## JSON output

rig doesn't fail closed on structured output, so `koharu-llm` decides what
each request carries, and the caller's check after decoding, as the
pipeline's, stays the guarantee.

- **Dropped schemas.** rig drops a DeepSeek schema with only a log line [rig
  dialects], and drops a schema silently from a chat-format request that also
  offers tools [rig chat schema]. Koharu asks DeepSeek for JSON mode with the
  schema in the instructions, and translation never offers tools.
- **Rewritten schemas.** For OpenAI and Claude, rig rewrites a schema toward
  strict mode: every property required, `oneOf` turned into `anyOf`, what sits
  beside a `$ref` dropped, and for Claude, numeric bounds dropped [rig schema].
  Koharu's schemas are written that way already, so the rewrite changes
  nothing; a test holds that, as a rig release may change the rewrite.
- **Refusals.** rig hands back an OpenAI refusal as answer text [rig refusal],
  and a Claude refusal as a content filter [Claude refusals]. Translation's
  check fails a refusal either way, as it isn't the schema's JSON.

## Local models

The catalog lists GGUF models pinned to a Hugging Face revision, each with its
quantizations, an optional vision projector, sampling defaults, and the
languages it translates, as now; Gemma 4 12B at Q4_K_XL is the default. A
model is downloaded through `koharu-runtime` when it's first picked and loaded
on the runtime's device.

- **One engine per loaded model.** A thread owns the model, one llama.cpp
  context, and its samplers, which can't move between threads, and takes
  requests from a queue.
- **Requests decode together.** Each request is a sequence in the context; at
  every step the engine decodes one token for every running sequence, and a
  new request joins when a sequence finishes. Orca schedules a batch "at the
  granularity of iteration" this way [Orca], and llama.cpp's `parallel` example
  shows it on the C API [llama.cpp parallel]. Each sequence has its own KV
  cache (`kv_unified` off) [llama.h kv_unified] and its own sampler, with an
  llguidance grammar for a JSON schema.
- **Images go one at a time.** A request's image is encoded and decoded alone,
  with the attention the projector needs set for the whole context [mtmd
  decode], before the sequence joins the others.
- **Shared instructions.** A prompt that starts with the instructions another
  sequence already decoded copies them (`seq_cp`) instead of decoding them
  again [llama.cpp parallel].
- **Size.** The number of sequences and the context each holds are what the
  model loads with, as the KV cache for all of them is allocated with the
  context; the pipeline chooses them from its memory budget. One more
  sequence holds the shared instructions, as `parallel`'s sequence 0 does.
- **Stopping.** A dropped request frees its sequence at the next step. A long
  prompt is decoded in chunks, so a cancel waits at most one chunk; llama.cpp's
  abort callback isn't used, as it works "only with CPU execution" [llama.h
  abort].

The engine runs on `koharu-llama`, upstream's `llama-cpp-2` [llama-cpp-2]
unchanged but for its bindings: `koharu-llama-sys`, which loads the runtime's
llama.cpp.

## ChatGPT sign-in

The agent signs in with the user's ChatGPT plan through rig's ChatGPT
provider, the route Codex uses: Codex's client at `auth.openai.com`, and
requests to `chatgpt.com/backend-api/codex` [rig chatgpt] [rig chatgpt auth].

- **Sign-in.** rig's `Authenticator` signs in through the system browser with
  PKCE and a loopback redirect, or, where no browser opens, with a device code
  the agent panel shows; `sign_out` revokes the tokens [rig sign-in].
- **Tokens.** rig keeps the sign-in through a `TokenStore`, which Koharu
  implements over `koharu-config`'s secrets, so the tokens live in Koharu's
  encrypted secrets under `chatgpt` and never in rig's file store; rig refreshes
  them, one refresh at a time [rig token store].
- **Requests.** The route takes no structured output, temperature, or token
  limit, and rig leaves them out [rig codex limits], so a ChatGPT model gets a
  schema in its instructions, as MiniMax does.
- **Models.** rig lists no ChatGPT models, so Koharu reads Codex's model list,
  as the agent does now.

This is Codex's own route; for other apps, OpenAI documents Sign in with
ChatGPT and asks them not to use ChatGPT's `backend-api` [SIWC inference].

## Agent

The agent edits the open project through the tools the app's host gives it, on
rig's agent loop, with any hosted model that takes tools.

```rust,ignore
pub trait Host: Send + Sync + 'static {
    fn context(&self) -> serde_json::Value;
    fn tools(&self) -> Vec<Tool>;
    fn invoke(&self, tool: &str, arguments: Value) -> impl Future<Output = Invocation> + Send;
}
```

| Rule | Evidence |
|---|---|
| Each host tool becomes a rig tool defined at run time from its JSON schema; rig checks a call's arguments against the schema before the tool runs and sends a mismatch back to the model. | rig's `DynamicTool` [rig dynamic tools] and argument checking [rig tool validation] |
| A run asks the model, runs the tools it calls one at a time and in order, returns their results, and asks again, until the model answers without tools. | rig runs tools in call order by default [rig runner]; the same loop as OpenAI's Agents SDK [OpenAI agents loop] |
| A run stops after 20 model calls, set with `max_turns(20)` as rig's default is 1, and reports the limit as a failure. | rig ends the run with `MaxTurns` [rig max turns]; Vercel's `ToolLoopAgent` defaults to 20 steps [Vercel loop control] |
| A tool that fails returns its error to the model, marked as an error; the run goes on. | rig's tool errors [rig tool result]; Claude's `is_error` [Claude tool calls]; MCP's `isError` [MCP tools] |
| Edits run without asking, as each is one undo step. | the Claude Agent SDK's `acceptEdits` mode [Claude permissions]; MCP asks for a human able to deny [MCP tools], which undo gives |
| A model's reasoning state goes back unchanged on the next call: rig replays a model's own turns as they came. A run keeps its instructions and tools fixed, as Claude's replay drops thinking signatures when either changes; the project's state goes with each request as a context document, ahead of the history and never in it. | rig's history replay [rig history], its Claude rule [rig claude replay], and its run documents [rig run document] [rig documents] |
| Cancelling drops rig's stream, which drops a running tool with it, so each edit runs detached and finishes: an edit is whole or not at all. | rig's cancellation is dropping the stream [rig agent readme] |

The agent panel's events come from rig's stream (text and reasoning deltas,
the final answer) and, as rig's stream has no live "tool started" item [rig
agent streaming], from each tool's own callback, which runs when the tool
does. A run's history stays in
memory for the open project. Local models don't take tools, as their chat
templates' tool syntax isn't parsed, so the agent runs on hosted models.

## Version

rig breaks its API in every release, 0.40 to 0.45 between July and October
2026 [rig changelog], so `koharu-llm` pins an exact version and moves to a new
one as a change of its own, with the schema test above. It needs Rust 1.97.1;
Koharu builds with a newer toolchain.

## Configuration and credentials

Both come from `koharu-config`, read when they're needed.

- `[providers]` holds each provider's connection settings, such as LM Studio's
  and an OpenAI-compatible server's base URLs; a provider without an entry
  uses its defaults.
- `[agent]` names the agent's model and reasoning effort.
- Koharu's secrets hold each API key under its provider's id, and under
  `chatgpt` the record rig's ChatGPT sign-in keeps: its tokens, their expiry,
  and the account id [rig chatgpt auth].

## Sources

- rig: [rig], [rig wire], [rig replay target], [rig connect], [rig reqwest],
  [rig retry], [rig listing], [rig catalog], [rig OpenAI dialect], [rig
  dialects], [rig chat schema], [rig schema], [rig refusal], [rig chatgpt], [rig
  chatgpt auth], [rig sign-in], [rig token store], [rig codex limits], [rig
  dynamic tools], [rig tool validation], [rig runner], [rig max turns], [rig
  tool result], [rig history], [rig claude replay], [rig run document], [rig
  documents], [rig agent streaming], [rig agent readme], [rig changelog]
- Providers: [OpenAI structured outputs], [OpenAI rate limits], [Claude
  structured outputs], [Claude refusals], [Claude rate limits], [Gemini
  structured output], [xAI structured outputs], [DeepSeek JSON mode],
  [OpenRouter structured outputs], [LM Studio structured output], [LM Studio
  models]
- ChatGPT: [SIWC inference]
- Local inference: [Orca], [llama.cpp parallel], [llama.h kv_unified], [mtmd
  decode], [llama.h abort], [llama-cpp-2]
- Agents: [OpenAI agents loop], [Claude tool calls], [Claude permissions],
  [Vercel loop control], [MCP tools]

[rig]: https://github.com/0xPlaygrounds/rig/tree/98ed06d7f1c911e6e4c7fdb5d690093bc964c514
[rig wire]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/wire.rs#L735-L779
[rig replay target]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/completion/history.rs#L176-L205
[rig connect]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/client/macros.rs#L29-L46
[rig reqwest]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-reqwest/src/lib.rs#L135-L139
[rig retry]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/error/retry.rs#L9-L42
[rig listing]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/model/listing.rs#L14-L49
[rig catalog]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/catalog/mod.rs#L1-L10
[rig OpenAI dialect]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/openai/wire.rs#L576-L613
[rig dialects]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/openai/wire/dialects.rs#L77-L94
[rig chat schema]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/openai/wire/chat.rs#L298-L312
[rig schema]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/internal/schema.rs#L24-L111
[rig refusal]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/openai/wire/chat.rs#L1054-L1072
[rig chatgpt]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/chatgpt/mod.rs#L25-L72
[rig chatgpt auth]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/chatgpt/auth/native.rs#L21-L43
[rig sign-in]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/chatgpt/auth/native.rs#L156-L289
[rig token store]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/internal/auth/store.rs#L9-L53
[rig codex limits]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/openai/responses_api/mod.rs#L410-L413
[rig dynamic tools]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/tool/contextual.rs#L146-L196
[rig tool validation]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/tool/validate.rs#L1-L40
[rig runner]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-agent/src/agent/runner.rs#L97
[rig max turns]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-agent/src/run/mod.rs#L852-L866
[rig tool result]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/tool/result.rs#L154-L170
[rig history]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/completion/history.rs#L1-L4
[rig claude replay]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/providers/anthropic/wire.rs#L679
[rig run document]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-agent/src/agent/runner.rs#L247-L251
[rig documents]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/src/completion/request.rs#L1397-L1421
[rig agent streaming]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-agent/src/agent/streaming.rs#L181
[rig agent readme]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-agent/README.md#L59-L64
[rig changelog]: https://github.com/0xPlaygrounds/rig/blob/98ed06d7f1c911e6e4c7fdb5d690093bc964c514/crates/rig-core/CHANGELOG.md
[OpenAI structured outputs]: https://developers.openai.com/api/docs/guides/structured-outputs
[OpenAI rate limits]: https://developers.openai.com/api/docs/guides/rate-limits
[Claude structured outputs]: https://platform.claude.com/docs/en/build-with-claude/structured-outputs
[Claude refusals]: https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons
[Claude rate limits]: https://platform.claude.com/docs/en/api/rate-limits
[Gemini structured output]: https://ai.google.dev/gemini-api/docs/structured-output
[xAI structured outputs]: https://docs.x.ai/developers/model-capabilities/text/structured-outputs
[DeepSeek JSON mode]: https://api-docs.deepseek.com/guides/json_mode
[OpenRouter structured outputs]: https://openrouter.ai/docs/guides/features/structured-outputs
[LM Studio structured output]: https://lmstudio.ai/docs/developer/openai-compat/structured-output
[LM Studio models]: https://lmstudio.ai/docs/developer/rest/list
[SIWC inference]: https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference
[Orca]: https://www.usenix.org/conference/osdi22/presentation/yu
[llama.cpp parallel]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476
[llama.h kv_unified]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/include/llama.h#L412-L417
[mtmd decode]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/tools/mtmd/mtmd.cpp#L2426-L2447
[llama.h abort]: https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/include/llama.h#L400-L404
[llama-cpp-2]: https://github.com/utilityai/llama-cpp-rs/tree/e7af607ebd5ec2932709ef918c17cca2f9b0fce4/llama-cpp-2
[OpenAI agents loop]: https://github.com/openai/openai-agents-python/blob/a88d2919631dbd93801a79eb0f97011ef9b451b7/docs/running_agents.md
[Claude tool calls]: https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls
[Claude permissions]: https://code.claude.com/docs/en/agent-sdk/permissions
[Vercel loop control]: https://github.com/vercel/ai/blob/e59b6e017aece6cc45c4ee7d94af88829621ed8d/content/docs/03-agents/04-loop-control.mdx
[MCP tools]: https://github.com/modelcontextprotocol/modelcontextprotocol/blob/54b91cdb8f21825ce586c452704f319d49866ef8/docs/specification/2026-07-28/server/tools.mdx
