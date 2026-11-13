use std::{borrow::Cow, collections::HashMap, sync::Arc};

use icu_locale_core::LanguageIdentifier;
use indoc::indoc;
use koharu_llm::{
    Context, SummaryPrompts,
    rig::{driver::DynModel, operation::Completion},
};
use koharu_scene::ProjectId;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::{TranslatorConfig, model};

/// A project's conversation, and what it was written with.
pub(crate) struct Conversation {
    /// The model, the target, and the prompt its history was written with.
    with: Option<(String, LanguageIdentifier, String)>,
    pub(super) context: Context,
    /// A hosted model, connected for its first page.
    hosted: Option<DynModel<Completion>>,
    /// The last page's texts and their translations.
    pub(super) last: Option<(Vec<String>, Vec<String>)>,
}

impl Conversation {
    fn new() -> Self {
        Self {
            with: None,
            context: Context::new(prompts()),
            hosted: None,
            last: None,
        }
    }

    /// Starts again unless its history was written with `config`'s model,
    /// target, and prompt.
    pub(super) fn keep(&mut self, config: &TranslatorConfig) {
        let with = (
            config.model.clone(),
            config.target.clone(),
            config.prompt.clone(),
        );
        if self.with.as_ref() != Some(&with) {
            *self = Self::new();
            self.with = Some(with);
        }
    }

    /// The hosted model `config` names, connected for the first page.
    pub(super) async fn hosted(
        &mut self,
        config: &TranslatorConfig,
    ) -> anyhow::Result<DynModel<Completion>> {
        if let Some(model) = &self.hosted {
            return Ok(model.clone());
        }
        let model = model::load(&config.model, config.vision).await?;
        self.hosted = Some(model.clone());
        Ok(model)
    }
}

/// Each open project's conversation, which a translation holds while it
/// runs, so a project's pages go through it one at a time.
#[derive(Default)]
pub(crate) struct Conversations(HashMap<ProjectId, Arc<Mutex<Conversation>>>);

impl Conversations {
    /// Whether a translation holds `project`'s conversation.
    pub(crate) fn busy(&self, project: ProjectId) -> bool {
        self.0
            .get(&project)
            .is_some_and(|conversation| conversation.try_lock().is_err())
    }

    /// `project`'s conversation for a translation to hold, a new one the
    /// first time; `None` while another holds it.
    pub(crate) fn lock(&mut self, project: ProjectId) -> Option<OwnedMutexGuard<Conversation>> {
        let conversation = self
            .0
            .entry(project)
            .or_insert_with(|| Arc::new(Mutex::new(Conversation::new())));
        Arc::clone(conversation).try_lock_owned().ok()
    }

    /// Forgets `project`'s conversation; a translation holding it drops it
    /// when it ends.
    pub(crate) fn close(&mut self, project: ProjectId) {
        self.0.remove(&project);
    }
}

/// What a general model is asked when the conversation is full. rig's own
/// are written for a coding agent.
fn prompts() -> SummaryPrompts {
    SummaryPrompts {
        system: Cow::Borrowed(indoc! {"
            You summarize a manga's translation so far, so that its translator can go on with the next pages from the summary alone. Read the conversation and write the summary in the exact format asked for.

            Do not translate anything new. Output only the summary."}),
        initial: Cow::Borrowed(indoc! {"
        The conversation above is a manga's pages and their translations so far. Summarize it for the translator of the next pages."}),
        update: Cow::Borrowed(indoc! {"
        The conversation above is the NEW part of a translation whose earlier part is summarized in <previous-summary>. Update that summary with it, keeping everything in it that still holds."}),
        format: Cow::Borrowed(indoc! {"
            Write three sections:
            ## Characters
            Each character's name as translated, and how they speak.
            ## Terms
            Recurring names and terms, and their translations.
            ## Story
            What has happened so far, briefly."}),
        ..SummaryPrompts::DEFAULT
    }
}
