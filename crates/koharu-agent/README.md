# koharu-agent

The agent that works on an open Koharu project, and the tools it works with. The tools are
written here as rig's own tools over the scene's queries and edits, the pipeline's runs, and
the renderer. The in-app agent registers them with rig directly. The MCP server serves the same
tool set to outside agents, on this machine only, so an agent in the app and one in another
program change a project the same way. The scene and the pipeline know nothing of tools.

## API

| Item | What it does |
|---|---|
| `Tools::new(session, pipeline, renderer)` | What the tools work on: one open project, the pipeline that runs its stages, and the renderer that draws its pages |
| `Tools::toolset()` | Every tool, as rig's `ToolSet`, the same whichever project is open |
| `Tools::tool_context()` | The rig tool context a call on this project runs with |
| `Tools::context()` | The project's outline, which goes with each of the agent's requests |
| `Agent::new(tools, model, context)` | The agent's conversation about one project, on any model that takes tools, starting from a koharu-llm `Context` |
| `Agent::run(prompt, events).await` | Runs a request until the model answers without tools, and returns its last text |
| `Agent::set_config(config)` | Follows a changed `[agent]` from the next run on |
| `Event` | What the agent panel shows during a run: text and reasoning as they stream, and each tool call as it starts and finishes |
| `McpServer::start(port).await` | Serves MCP on 127.0.0.1, at a free port for 0, with no project open |
| `McpServer::open(tools)` | The project calls go to from now on, or none |
| `AgentConfig` | The `[agent]` section |

## Tools

Each tool is a struct implementing rig's `Tool`. It has typed arguments, a JSON Schema with
every subschema inlined, because some providers refuse a schema that refers to definitions,
and annotations, which use MCP's fields and say which tools only read. A tool finds the project
in the tool context it runs with, not in itself. So the tool set can be listed before a project
is open, and a call with none fails with "no project is open".

| Tool | Arguments | Answers | Only reads |
|---|---|---|---|
| `get_project` | None | The pages in order, each with its size and scan | Yes |
| `get_page` | `page` | The page's layers nested in reading order: panels, balloons, and blocks, each block with its source and translation | Yes |
| `get_layer` | `layer` | One layer with everything inside it | Yes |
| `get_history` | None | The undo steps, oldest first, and how many are done | Yes |
| `edit` | `label`, `ops` | What changed. All the ops apply as one undo step under the label, or none do | No |
| `run` | `target`, `stages` | The run's id, at once. The run goes on in the background | No |
| `stop` | None | That every run, running and queued, is stopped. What they finished stays | No |
| `history` | `run` | What the run told so far, in order, the last saying how it ended. It waits up to 30 seconds for the run to end, as Codex waits on a long command | Yes |
| `view_page` | `page` | The page as the editor draws it, translations and clean-ups included, flattened over white, as a JPEG at most 1,280 pixels on its longer side | Yes |

The queries answer the scene's own values, `edit` the scene's change, and `history` the
pipeline's events. `run` and `stop` answer plain text. `edit` takes the scene's ops, whose
schema leaves out what an agent never sends, such as file bytes and raw pixels. A `run` target
is the project, some pages, some blocks, or a painted area, with any of the stages detect,
recognize, inpaint, and translate, as the pipeline takes a request.

A tool's error reaches the model word for word. rig shows a model only the kind of an error it
wasn't handed as a tool error, and the message is what lets the model correct itself.

## Runs

- **Each request.** The system prompt comes from `[agent]`. The project's outline goes with
  each of a run's requests as a document ahead of the history, kept out of the history, so
  the model sees only the project's latest state. A run makes at most `max_turns` model calls.
  A request to one of Koharu's own models sends no repeat penalty, which would penalize the
  markers that repeat in every call.
- **Arguments.** A hook holds every call to its tool's schema before it runs, because
  rig-agent doesn't and a local model has no grammar holding it to the schema. A call that
  doesn't fit is skipped, and the model sees why. rig itself sends back a call it can't take,
  one naming no tool or with arguments that aren't a JSON object, at most `corrections` times.
- **The window.** A run starts by fitting its conversation into the model's window with
  koharu-llm's `Context::fit`. During a run, a turn over the threshold is sent with older tool
  outputs cleared on a copy of its history. That needs no model call, and clearing any sooner
  would drop outputs for nothing and invalidate prompt caches.
- **A refused request.** If the model refuses a request as too long before any call has
  reached its tool, the conversation is compacted and the run starts over once. After a call
  has reached its tool, the project may have changed, so the run fails instead.
- **Cancelling.** Dropping a run's future cancels it. An edit already applying finishes on
  its own thread, whole.
- **After a run.** The conversation keeps the run's messages and the tokens the provider last
  reported, so the next request continues it.

## MCP

`McpServer` serves Streamable HTTP at `/mcp` on 127.0.0.1, with rmcp and axum. A browser sends
an `Origin` and an MCP client doesn't, so a request with one is refused, as the MCP spec asks
of a local server; rmcp's `Host` check keeps it to loopback. The server lists rig's tool
definitions as MCP tools, annotations included, and runs a call through the same `ToolSet` on
whichever project is open. rmcp doesn't hold a call to the tool's schema, and an op the edit
schema leaves out would still parse, so the server checks each call against its schema first.
A tool's text and JSON answers go as text, and `view_page` as an image.

The `serve` example serves a project with no app. It opens a project file, or makes one from
image files as its pages, starts the pipeline and the renderer with their settings, and serves
until Ctrl+C. Its `--port` defaults to 7878.

## Settings

| `[agent]` | Default | What it does |
|---|---|---|
| `model` | None | The agent's model, a `vendor/model` reference as koharu-llm connects one |
| `reasoning` | The model's own | How much the model reasons, by rig's levels after OpenAI's `reasoning_effort` |
| `prompt` | Koharu Agent's instructions | The system prompt |
| `max_turns` | 20 | How many model calls a run makes at most. rig's default of one would end a run at its first tool call |
| `corrections` | 2 | How many times rig sends back a call it can't take before the run fails |
