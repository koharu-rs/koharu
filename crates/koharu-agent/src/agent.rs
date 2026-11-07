use std::{fmt, sync::Arc};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    Codex, CodexModel, Config, Control, Host, ToolCall,
    bulk_review::{BulkReviewer, page_job_tool},
    codex::{Delta, Request, ReviewClient, function_output, message, project_context},
};

const INSTRUCTIONS: &str = r#"You are Koharu Agent, operating a manga translation project inside Koharu.
An up-to-date project summary and page index are supplied in a koharu_project_context block on every user turn. Page contents and images are omitted.
Call inspect_pages with the relevant page IDs to read their semantic contents. Its detailed results are available for the current turn but omitted from later conversation history, so inspect the pages again in a later turn when needed.
Call view_page only when visual inspection is needed, and only for relevant pages.
For a task that targets more than five pages, or whose page contents may be large, call process_pages_job once with the exact ordered page IDs and the user's complete objective. This runs every page automatically in small sequential batches; never ask the user to submit later batches.
Use the provided tools whenever the user asks to inspect or change the project. Never claim a change succeeded unless its tool result says it succeeded.
All project changes are revisioned and reversible. Do not ask for permission. Do not produce a plan or expose internal steps; continue using tools until the request is complete or cannot be completed.
Do not invent entity identifiers. Preserve artwork and existing authored content unless the user asks to change them."#;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, Type)]
#[serde(transparent)]
pub struct RunId(Uuid);

impl RunId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started {
        run: RunId,
    },
    TextDelta {
        run: RunId,
        delta: String,
    },
    ReasoningDelta {
        run: RunId,
        delta: String,
    },
    ToolStarted {
        run: RunId,
        call_id: String,
        name: String,
    },
    ToolFinished {
        run: RunId,
        call_id: String,
        name: String,
        changed: bool,
        output: String,
    },
    Completed {
        run: RunId,
        message: String,
    },
    Failed {
        run: RunId,
        message: String,
    },
    Cancelled {
        run: RunId,
    },
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct RunResult {
    pub run: RunId,
    pub message: String,
}

pub struct Agent<H> {
    codex: Codex,
    review: Box<dyn ReviewClient>,
    host: Arc<H>,
    config: koharu_config::Config<Config>,
    history: Mutex<Vec<Value>>,
    serial: Mutex<()>,
}

impl<H> Agent<H>
where
    H: Host,
{
    pub fn new(codex: Codex, host: H) -> Result<Self> {
        Ok(Self {
            review: Box::new(codex.clone()),
            codex,
            host: Arc::new(host),
            config: Config::load()?,
            history: Mutex::new(Vec::new()),
            serial: Mutex::new(()),
        })
    }
    pub fn codex(&self) -> &Codex {
        &self.codex
    }

    pub fn config(&self) -> Result<Config> {
        Ok(self.config.read()?.clone())
    }

    pub async fn models(&self) -> Result<Vec<CodexModel>> {
        self.codex.models().await
    }

    pub fn save_config(&self, config: Config) -> Result<Config> {
        let mut current = self.config.write()?;
        *current = config;
        let saved = current.clone();
        current.save()?;
        Ok(saved)
    }

    pub async fn clear(&self) {
        self.history.lock().await.clear();
    }

    #[tracing::instrument(skip_all)]
    pub async fn run<F>(
        &self,
        run: RunId,
        prompt: String,
        control: Control,
        mut publish: F,
    ) -> Result<RunResult>
    where
        F: FnMut(Event) + Send,
    {
        let _serial = self.serial.lock().await;
        publish(Event::Started { run });
        let result = self.run_inner(run, prompt, &control, &mut publish).await;
        match &result {
            Ok(result) => publish(Event::Completed {
                run,
                message: result.message.clone(),
            }),
            Err(_) if control.is_cancelled() => publish(Event::Cancelled { run }),
            Err(error) => publish(Event::Failed {
                run,
                message: format!("{error:#}"),
            }),
        }
        result
    }

    async fn run_inner<F>(
        &self,
        run: RunId,
        prompt: String,
        control: &Control,
        publish: &mut F,
    ) -> Result<RunResult>
    where
        F: FnMut(Event),
    {
        control.ensure_running()?;
        let context = self.host.context().await?;
        let config = self.config()?;
        let models = self.codex.models().await?;
        let model = match config.model.as_deref() {
            Some(selected) => models
                .iter()
                .find(|model| model.id == selected)
                .with_context(|| format!("configured Codex model {selected} is not available"))?,
            None => models
                .first()
                .context("Codex returned no available model")?,
        };
        let reasoning = if model.reasoning.is_empty() || model.reasoning.contains(&config.reasoning)
        {
            config.reasoning
        } else {
            model.reasoning[0]
        };
        let clean_user = message("user", &prompt);
        let context_message = project_context(&context)?;
        let base = self.history.lock().await.clone();
        let mut input = base.clone();
        input.push(context_message);
        input.push(clean_user.clone());
        let mut persisted = base;
        persisted.push(clean_user);
        let mut tools = self.host.tools();
        tools.push(page_job_tool());
        let session = run.to_string();

        loop {
            control.ensure_running()?;
            let request = Request::new(
                model.id.clone(),
                INSTRUCTIONS.to_owned(),
                input.clone(),
                tools.clone(),
                reasoning,
                session.clone(),
            );
            let turn = self
                .codex
                .respond(&request, control, |event| match event {
                    Delta::Text(delta) => publish(Event::TextDelta { run, delta }),
                    Delta::Reasoning(delta) => publish(Event::ReasoningDelta { run, delta }),
                })
                .await?;
            input.extend(turn.output.iter().cloned());
            persisted.extend(turn.output.iter().cloned());

            if turn.calls.is_empty() {
                *self.history.lock().await = persisted;
                return Ok(RunResult {
                    run,
                    message: turn.text,
                });
            }

            for call in turn.calls {
                control.ensure_running()?;
                publish(Event::ToolStarted {
                    run,
                    call_id: call.call_id.clone(),
                    name: call.name.clone(),
                });
                let invocation = if call.name == "process_pages_job" {
                    BulkReviewer::new(&*self.review, &*self.host)
                        .run_job(run, &call.call_id, &call.arguments, model, control, publish)
                        .await
                } else {
                    self.host
                        .invoke(
                            ToolCall {
                                call_id: call.call_id.clone(),
                                name: call.name.clone(),
                                arguments: call.arguments,
                            },
                            control,
                        )
                        .await
                };
                let (output, changed, images) = match invocation {
                    Ok(invocation) => (
                        json!({ "ok": true, "value": invocation.value }),
                        invocation.changed,
                        invocation.images,
                    ),
                    Err(error) => (
                        json!({ "ok": false, "error": format!("{error:#}") }),
                        false,
                        Vec::new(),
                    ),
                };
                let historical_output = historical_tool_output(&call.name, &output);
                publish(Event::ToolFinished {
                    run,
                    call_id: call.call_id.clone(),
                    name: call.name,
                    changed,
                    output: output.to_string(),
                });
                input.push(function_output(&call.call_id, &output, &images)?);
                persisted.push(function_output(&call.call_id, &historical_output, &[])?);
            }
        }
    }
}

fn historical_tool_output(name: &str, output: &Value) -> Value {
    if output.get("ok").and_then(Value::as_bool) != Some(true) {
        return output.clone();
    }
    match name {
        "inspect_pages" => json!({
            "ok": true,
            "value": {
                "omitted_from_history": true,
                "message": "Page contents were available in the turn that called inspect_pages; call inspect_pages again if they are needed."
            }
        }),
        "process_pages_job" => json!({
            "ok": true,
            "value": {
                "total": output.pointer("/value/total"),
                "completed": output.pointer("/value/completed"),
                "failed_count": output.pointer("/value/failed").and_then(Value::as_array).map_or(0, Vec::len),
                "details_omitted_from_history": true,
            }
        }),
        _ => output.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::historical_tool_output;

    #[test]
    fn omits_successful_page_inspection_from_history() {
        let output = json!({ "ok": true, "value": { "pages": [{ "large": "state" }] } });
        let historical = historical_tool_output("inspect_pages", &output);
        assert_eq!(historical["ok"], true);
        assert_eq!(historical["value"]["omitted_from_history"], true);
        assert!(historical["value"].get("pages").is_none());
    }

    #[test]
    fn retains_other_tool_outputs_and_inspection_errors() {
        let output = json!({ "ok": true, "value": { "project": "summary" } });
        assert_eq!(historical_tool_output("inspect_project", &output), output);
        let error = json!({ "ok": false, "error": "invalid entity ID" });
        assert_eq!(historical_tool_output("inspect_pages", &error), error);
    }

    #[test]
    fn compacts_completed_page_jobs_in_history() {
        let output = json!({
            "ok": true,
            "value": {
                "total": 70,
                "completed": 69,
                "completed_pages": ["large list"],
                "failed": [{ "page": "p42" }],
                "memory": "large memory"
            }
        });
        let historical = historical_tool_output("process_pages_job", &output);
        assert_eq!(historical["value"]["total"], 70);
        assert_eq!(historical["value"]["completed"], 69);
        assert_eq!(historical["value"]["failed_count"], 1);
        assert!(historical["value"].get("memory").is_none());
    }
}
