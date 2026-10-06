use std::{collections::HashMap, fmt, sync::Arc};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    Codex, CodexModel, Config, Control, Host, ToolCall,
    bulk_review::{BulkReviewer, JobContext, review_job_tool},
    codex::{
        Delta, Request, ReviewClient, function_output, is_context_limit, message, project_context,
    },
};

const INSTRUCTIONS: &str = r#"You are Koharu Agent, operating a manga translation project inside Koharu.
An up-to-date project summary and page index are supplied in a koharu_project_context block on every user turn. Page contents and images are omitted.
Any review or revision of translations goes to review_translations_job — one page or a hundred. Call it once with the exact ordered page IDs and the user's complete objective. It reads the pages itself, revises them in small sequential batches, and applies its own changes, so do not call inspect_pages or view_page first to look at that text, and never ask the user to submit later batches. Report what it flags: text it judged should not have been translated, and any translation it removed.
Use inspect_pages for layout and presentation — geometry, typography, layers, visibility — and view_page only when rendered artwork must be read. Their detailed results stay in the current turn; read them again in a later turn if you still need them.
Any other bulk edit still uses the per-element tools.
Use the provided tools whenever the user asks to inspect or change the project. Never claim a change succeeded unless its tool result says it succeeded.
A write result names the pages it changed and whether the screen shows them. Only the page the user is looking at can be repainted: bring the edited page on screen with select_page when the result is what the user asked to see, and otherwise say which page to open instead of implying the screen changed.
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
    /// Cost and latency of one model request within this run. Emitted whether or
    /// not the backend reported usage, so a missing usage report is visible.
    /// Counts are `u32` because Specta refuses to export `usize` to TypeScript.
    Usage {
        run: RunId,
        request: u32,
        input_tokens: u32,
        cached_input_tokens: u32,
        output_tokens: u32,
        reasoning_tokens: u32,
        payload_bytes: u32,
        first_delta_ms: u32,
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

/// Reported counts are `u32` because Specta refuses to export `usize` to TypeScript.
fn count(value: usize) -> u32 {
    value.try_into().unwrap_or(u32::MAX)
}

pub struct Agent<H> {
    codex: Codex,
    review: Box<dyn ReviewClient>,
    host: Arc<H>,
    config: koharu_config::Config<Config>,
    history: Mutex<Vec<Value>>,
    /// Identifies the stretch of conversation currently held in `history`. The
    /// prompt cache key is derived from it, so repeated turns reuse the cached
    /// prefix instead of re-billing the whole transcript every message.
    conversation: Mutex<Uuid>,
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
            conversation: Mutex::new(Uuid::new_v4()),
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
        // The next conversation must not inherit this one's cached prefix, which
        // may describe a different project.
        *self.conversation.lock().await = Uuid::new_v4();
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
                message: if is_context_limit(error) {
                    // The provider says the input is too long; only the user can make
                    // the conversation shorter, so say what actually resets it.
                    format!(
                        "{error:#}\nThe conversation outgrew the model's context window. Start a new chat to keep working on this project."
                    )
                } else {
                    format!("{error:#}")
                },
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
        tools.push(review_job_tool());
        let conversation = *self.conversation.lock().await;
        let cache_key = format!("chat:{conversation}");

        let mut request_index = 0_usize;
        // Image-bearing tool output, remembered so it can be replaced by the tool's own
        // image-free form once a response has answered on top of it. A page render is
        // evidence for the next answer, not for every later one: left in the live turn it
        // is re-uploaded on each request of the run, growing the body quadratically and
        // showing the model pictures of pages it already looked at. Only images are
        // trimmed; tool text stays, because a later step of the same turn may still be
        // acting on the coordinates or IDs it reported.
        let mut spent: HashMap<String, Value> = HashMap::new();
        loop {
            control.ensure_running()?;
            if !spent.is_empty() {
                for item in &mut input {
                    let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(image_free) = spent.get(call_id) {
                        *item = image_free.clone();
                    }
                }
            }
            let request = Request::new(
                model.id.clone(),
                INSTRUCTIONS.to_owned(),
                input.clone(),
                tools.clone(),
                reasoning,
                cache_key.clone(),
            )
            .with_reasoning_continuity();
            let turn = self
                .codex
                .respond(&request, control, |event| match event {
                    Delta::Text(delta) => publish(Event::TextDelta { run, delta }),
                    Delta::Reasoning(delta) => publish(Event::ReasoningDelta { run, delta }),
                })
                .await?;
            request_index = request_index.saturating_add(1);
            let usage = turn.usage.unwrap_or_default();
            publish(Event::Usage {
                run,
                request: count(request_index),
                input_tokens: count(usage.input_tokens),
                cached_input_tokens: count(usage.cached_input_tokens),
                output_tokens: count(usage.output_tokens),
                reasoning_tokens: count(usage.reasoning_tokens),
                payload_bytes: count(turn.payload_bytes),
                first_delta_ms: turn.first_delta_ms,
            });
            input.extend(turn.output.iter().cloned());
            persisted.extend(turn.history_items().cloned());

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
                // The batch reviewer owns its tool, so this loop never names it:
                // `None` means the call belongs to the host.
                let reviewed = BulkReviewer::execute(
                    &*self.review,
                    &*self.host,
                    JobContext {
                        run,
                        conversation,
                        call_id: call.call_id.clone(),
                    },
                    &call,
                    model,
                    control,
                    publish,
                )
                .await;
                let invocation = match reviewed {
                    Some(invocation) => invocation,
                    None => {
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
                    }
                };
                let (output, historical_output, changed, images) = match invocation {
                    Ok(invocation) => (
                        json!({ "ok": true, "value": invocation.value }),
                        json!({ "ok": true, "value": invocation.history }),
                        invocation.changed,
                        invocation.images,
                    ),
                    Err(error) => {
                        let output = json!({ "ok": false, "error": format!("{error:#}") });
                        (output.clone(), output, false, Vec::new())
                    }
                };
                publish(Event::ToolFinished {
                    run,
                    call_id: call.call_id.clone(),
                    name: call.name,
                    changed,
                    output: output.to_string(),
                });
                input.push(function_output(&call.call_id, &output, &images)?);
                let image_free = function_output(&call.call_id, &historical_output, &[])?;
                if !images.is_empty() {
                    spent.insert(call.call_id.clone(), image_free.clone());
                }
                persisted.push(image_free);
            }
        }
    }
}
