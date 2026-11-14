# koharu-agent

Koharu's agent and the tools it works with.

The tools work on one open project, over the scene's queries and edits, the
pipeline's runs, and the renderer. The agent in the app uses them, and an MCP
server offers the same tools to agents in other programs. Where it follows an
established product, it cites it; the links are under [Sources](#sources).

## Goal

An agent changes a project the way the user does, through the same edits and
runs, so its work is undoable and shows as it happens. The agent in the app and
one in another program have the same tools, so neither can do what the other
can't. The scene and the pipeline know nothing of tools.

## 1. Tools

| Tool | Does | Only reads |
|---|---|---|
| `get_project` | the pages in order, each with its size and scan | yes |
| `get_page` | a page's layers nested in reading order: panels, balloons, and blocks, each block with its source and translation | yes |
| `get_layer` | one layer with everything inside it | yes |
| `get_history` | the undo steps, oldest first, and how many are done | yes |
| `edit` | the scene's ops, applied as one undo step under a label, or not at all | no |
| `run` | starts a pipeline run on the project, some pages, some blocks, or a painted area, and answers its id at once | no |
| `stop` | stops every run, running and queued; what they finished stays | no |
| `history` | what a run told so far, waiting up to 30 seconds for it to end | yes |
| `view_page` | the page as the editor draws it, as a JPEG at most 1,280 pixels on its longer side | yes |

- **Schemas** inline every subschema, because some providers refuse one that
  refers to definitions. `edit`'s schema leaves out what an agent never sends,
  such as file bytes and raw pixels.
- **Read-only tools say so**, with MCP's tool annotations [MCP schema].
- **Errors reach the model word for word**, because the message is what lets it
  correct itself.
- **No project, no call.** A tool finds the open project when it runs, not when
  it's made, so the tools can be listed with none open, and a call then fails
  with "no project is open".
- **A long run** is started by `run`, which answers at once, so the model can
  do something else meanwhile, and followed by `history`, which waits up to 30
  seconds, as Codex's command tool waits for a running command before it
  answers [Codex unified exec].

## 2. Runs

| Part | Model | Cite |
|---|---|---|
| request | the system prompt, then the project's outline as a document ahead of the history, kept out of it, so the model sees only the project's latest state | |
| turns | at most `max_turns` model calls a run | |
| arguments | every call is held to its tool's schema before it runs, and a call that doesn't fit is skipped with the reason, since a local model has no grammar holding it to the schema | |
| window | a run starts by fitting its conversation into the model's window; a turn over the threshold is sent with older tool outputs cleared, which needs no model call | Claude's context editing clears the oldest tool results once a conversation passes a threshold [Claude context editing] |
| too long | a request refused as too long before any tool ran compacts the conversation and starts over once; after a tool ran, the project may have changed, so the run fails | |
| stopping | dropping a run stops it; an edit already applying finishes whole | |
| after | the conversation keeps the run's messages, so the next request continues it | |

## 3. MCP

| Part | Model | Cite |
|---|---|---|
| transport | Streamable HTTP at `/mcp`, on this machine only | MCP's Streamable HTTP transport [MCP transports] |
| browsers | a request with an `Origin` is refused, since a browser sends one and an MCP client doesn't | MCP asks a server to validate `Origin` and a local one to bind to localhost only [MCP transports] |
| calls | checked against the tool's schema before they run, and run through the same tools as the app's agent on whichever project is open | |
| answers | text and JSON as text, `view_page` as an image | |

## 4. Settings

| `[agent]` | Default | Does |
|---|---|---|
| `model` | none | the agent's model, a `vendor/model` reference |
| `reasoning` | the model's own | how much the model reasons |
| `prompt` | Koharu Agent's instructions | the system prompt |
| `max_turns` | 20 | model calls a run makes at most |
| `corrections` | 2 | how many times a call the agent can't take goes back to the model before the run fails |

## Sources

Read in November 2026.

- Model Context Protocol: [MCP schema], [MCP transports]
- Anthropic and OpenAI: [Claude context editing], [Codex unified exec]

[MCP schema]: https://modelcontextprotocol.io/specification/2025-06-18/schema#toolannotations
[MCP transports]: https://modelcontextprotocol.io/specification/2025-06-18/basic/transports
[Claude context editing]: https://docs.claude.com/en/docs/build-with-claude/context-editing
[Codex unified exec]: https://github.com/openai/codex/blob/71a4c070369fb761e51fd71009107f398d9f0f81/codex-rs/core/src/unified_exec/mod.rs#L66-L70
