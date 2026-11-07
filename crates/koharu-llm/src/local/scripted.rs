//! A local model for tests: the real wire and transport over a thread that
//! answers from a script, so no weights or device are needed.

use std::sync::{Arc, Mutex, PoisonError, mpsc};

use indoc::indoc;
use rig_core::{catalog::ModelFacts, driver::Model as RigModel, error::ProviderError};
use serde_json::Map;

use super::{
    Engine, Local,
    engine::{Job, Step},
    markup::Markup,
    spec,
    template::Template,
};
use crate::{Model, Provider};

/// Renders every role, so tests can read what a request sent.
const TEMPLATE: &str = indoc! {"
    {% if tools %}<tools>{{ tools | tojson }}</tools>{% endif %}
    {% for m in messages %}<{{ m.role }}>{{ m.content }}{% if m.tool_calls %}{{ m.tool_calls | tojson }}{% endif %}</{{ m.role }}>
    {% endfor %}{% if enable_thinking %}<think>{% else %}<think></think>{% endif %}"
};

pub(crate) struct Scripted {
    pub(crate) model: Model,
    /// The prompt of each request so far.
    pub(crate) prompts: Arc<Mutex<Vec<String>>>,
}

/// A model that answers its requests with `replies` in order.
pub(crate) fn scripted(
    markup: Option<&'static Markup>,
    window: u32,
    replies: Vec<Result<&'static str, ProviderError>>,
) -> Scripted {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let (jobs, queue) = mpsc::channel::<Job>();
    let heard = Arc::clone(&prompts);
    std::thread::spawn(move || {
        let mut replies = replies.into_iter();
        while let Ok(job) = queue.recv() {
            heard
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(job.prompt.clone());
            let reply = replies.next().expect("the script ran out");
            match reply {
                Ok(text) => {
                    let _ = job.frames.send(Ok(Step::Text(text.to_owned())));
                    let _ = job.frames.send(Ok(Step::End {
                        length: false,
                        prompt_tokens: job.prompt.len() as u64 / 4,
                        output_tokens: text.len() as u64 / 4,
                    }));
                }
                Err(error) => {
                    let _ = job.frames.send(Err(error));
                }
            }
        }
    });
    let wire = Local {
        id: "scripted".to_owned(),
        template: Arc::new(Template::from_source(TEMPLATE)),
        marker: "<image>".to_owned(),
        vision: false,
        defaults: Map::new(),
        markup,
        facts: ModelFacts::new(
            spec("scripted", markup, false, window).expect("the scripted model's name is valid"),
        ),
    };
    Scripted {
        model: Model {
            completion: RigModel::new(wire, Engine(jobs)).erase(),
            provider: Provider::Local,
        },
        prompts,
    }
}
