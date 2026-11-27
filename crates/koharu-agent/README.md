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
| `get_project` | the pages in order, each with how many blocks it has and how many are translated | yes |
| `get_page` | a page's size, its scan, and its layers nested in reading order: panels, balloons, and blocks, each block with its source and translation | yes |
| `get_layer` | one layer with all its values, and what's inside it as `get_page` answers it | yes |
| `edit` | the scene's ops, applied as one undo step under a label, or not at all | no |
| `run` | starts a pipeline run on the project, some pages, some blocks, a painted area, or metadata (with optional `priority`), and answers its id at once | no |
| `wait` | waits up to 30 seconds for a run to end, and answers what it told so far | yes |
| `stop` | stops one run by id, or every run (running, queued, and waiting) when left out; what they finished stays | no |
| `view_page` | the page as the editor draws it, or its scan alone, whole or a region of it, as a JPEG 1,280 pixels on its longer side | yes |

- **Schemas** inline every subschema, because some providers refuse one that
  refers to definitions. `edit`'s schema leaves out what an agent never sends,
  such as file bytes and raw pixels, and gives a new layer's schema in its
  description, where it names each type once: inlined, they repeat until it
  takes more than a small model's context. An edit that doesn't fit it fails
  with what's wrong.
- **Part of a text** is styled, or given ruby, by its spans, the whole text in order
  as `get_page` answers it, so a model counts no characters.
- **What's sent whole is small.** The outline that goes with every request has
  each page's id and progress and no more, and `get_page` gives a path's bounds
  for the path and leaves out attributes, masks, and clean-ups, as Figma's
  `get_metadata` answers "just basic properties such as the layer IDs, names,
  types, position and sizes" for a closer call to fill in [Figma MCP].
- **Read-only tools say so**, with MCP's tool annotations [MCP schema].
- **No undo history.** The scene's history query isn't a tool: an agent has no
  undo to act on it with, and each `edit` answers the history with what it
  changed.
- **A region fills the view**, enlarged, as the zoom of Claude's computer use
  scales one to fit its screenshots, so a model reads text too small for it on
  the whole page [Claude computer use].
- **The scan alone** shows what a block's source says after its clean-up covers
  it, as the canvas shows it to the user.
- **Errors reach the model word for word**, because the message is what lets it
  correct itself.
- **No project, no call.** A tool finds the open project when it runs, not when
  it's made, so the tools can be listed with none open, and a call then fails
  with "no project is open".
- **A long run** is started by `run`, which answers at once, so the model can
  do something else meanwhile, and followed by `wait`, which waits up to 30
  seconds, as Codex's command tool waits for a running command before it
  answers [Codex unified exec].

## 2. Runs

| Part | Model | Cite |
|---|---|---|
| request | the system prompt, Koharu's own and not a setting, since it describes Koharu's tools and project document, then the project's outline as a document ahead of the history, kept out of it, so the model sees only the project's latest state | |
| turns | as many model calls as the request takes: a run ends when the model answers without a tool call, or is stopped | |
| arguments | every call is held to its tool's schema before it runs, and a call that doesn't fit is skipped with the reason, since a local model has no grammar holding it to the schema | |
| corrections | a call whose arguments aren't a JSON object goes back to the model with what's wrong, and the third turn in a row with one fails the run; a call to a tool that isn't there goes back with the tools' names, and a run's third fails it | |
| window | before each of a run's model calls, older messages are summarized if the call wouldn't fit the model's window, a tool call kept with its result | pi compacts before a prompt and before a request after tool results [pi compaction] |
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
| `reasoning` | the model's own | how much the model reasons: `off` or an effort level, one word |

## Sources

Read in November 2026.

- Model Context Protocol: [MCP schema], [MCP transports]
- Agents: [pi compaction], [Codex unified exec], [Claude computer use], [Figma MCP]

[MCP schema]: https://modelcontextprotocol.io/specification/2025-06-18/schema#toolannotations
[MCP transports]: https://modelcontextprotocol.io/specification/2025-06-18/basic/transports
[pi compaction]: https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/compaction.md
[Codex unified exec]: https://github.com/openai/codex/blob/71a4c070369fb761e51fd71009107f398d9f0f81/codex-rs/core/src/unified_exec/mod.rs#L66-L70
[Claude computer use]: https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool
[Figma MCP]: https://developers.figma.com/docs/figma-mcp-server/tools-and-prompts
