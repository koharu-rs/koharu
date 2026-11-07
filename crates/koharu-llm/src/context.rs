//! A conversation kept inside a model's context window, with rig-memory's
//! compaction: nothing in rig's agent runs it, so this does.

use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use rig_core::{
    catalog::ModelSpec,
    completion::{Message, Usage},
    conversation::Conversation,
    driver::DynModel,
    operation::Completion,
};
use rig_memory::{
    CompactReason, CompactionPolicy, NearWindow, Summarizer, SummaryPrompts, apply_summary,
    estimate_tokens, summary_author, summary_state,
};

use crate::Model;

/// A conversation that fits the model it's sent to.
///
/// It's kept in memory; where a conversation is saved isn't decided.
#[derive(Debug, Default)]
pub struct Context {
    conversation: Conversation,
    prompts: SummaryPrompts,
    /// The tokens a provider last reported, and how many messages they cover.
    /// rig's estimate of 4 bytes a token undercounts Japanese and Chinese, so
    /// it only stands in for what comes after them.
    count: Option<(u64, usize)>,
}

impl Context {
    /// An empty conversation, summarized as `prompts` ask. rig's own, the
    /// default, are written for a coding agent.
    #[must_use]
    pub fn new(prompts: SummaryPrompts) -> Self {
        Self {
            prompts,
            ..Self::default()
        }
    }

    /// What the next request sends as its history.
    #[must_use]
    pub fn messages(&self) -> &[Arc<Message>] {
        self.conversation.messages()
    }

    /// Adds replies to one history, in order, each with the messages of its
    /// run and the usage of its last model call. Each usage covers that history
    /// and its own reply, so only the first reply's sets the count, and the
    /// others are estimated.
    pub fn record(&mut self, replies: impl IntoIterator<Item = (Vec<Message>, Usage)>) {
        for (index, (messages, usage)) in replies.into_iter().enumerate() {
            self.conversation.extend(messages);
            if index == 0 {
                let through = self.conversation.messages().len();
                self.count = usage.context_tokens().map(|tokens| (tokens, through));
            }
        }
    }

    /// Makes room if the next request wouldn't leave the model's reserve free:
    /// clears old tool outputs, then summarizes older messages if that isn't
    /// enough. A model with no known limit is never over; `compact` with
    /// [`CompactReason::Overflow`] answers its refusal.
    pub async fn fit(&mut self, model: &Model) -> Result<()> {
        let spec = spec(model)?.clone();
        self.fit_to(&model.completion, &spec).await
    }

    /// Summarizes now, because the user asked or the model refused the
    /// request.
    pub async fn compact(&mut self, model: &Model, reason: CompactReason) -> Result<()> {
        let spec = spec(model)?.clone();
        self.summarize(&model.completion, &spec, &reason).await
    }

    async fn fit_to(&mut self, model: &DynModel<Completion>, spec: &ModelSpec) -> Result<()> {
        let policy = policy(&self.prompts, spec);
        if !policy.over_threshold(self.tokens(), spec) {
            return Ok(());
        }
        let cleared = policy
            .clearing
            .clear_conversation(&mut self.conversation, summary_author());
        if cleared.results > 0 {
            self.count = None;
        }
        if !policy.over_threshold(self.tokens(), spec) {
            return Ok(());
        }
        self.summarize(model, spec, &CompactReason::Threshold).await
    }

    async fn summarize(
        &mut self,
        model: &DynModel<Completion>,
        spec: &ModelSpec,
        reason: &CompactReason,
    ) -> Result<()> {
        let policy = policy(&self.prompts, spec);
        let state = summary_state(&self.conversation);
        let plan = policy
            .plan_summary(&self.conversation, &state, reason, spec)
            .map_err(|error| anyhow!("{error}"))?;
        let Some(plan) = plan else {
            return Ok(());
        };
        let response = model
            .call(plan.request)
            .await
            .context("couldn't summarize the conversation")?;
        let mut next = plan.state;
        next.summary =
            Summarizer::summary_text(&response).context("couldn't summarize the conversation")?;
        if apply_summary(&mut self.conversation, plan.keep_from, &next).is_some() {
            self.count = None;
        }
        Ok(())
    }

    /// About how many tokens the next request's history takes.
    fn tokens(&self) -> u64 {
        let messages = self.conversation.messages();
        match self.count {
            Some((tokens, through)) => {
                tokens.saturating_add(estimate_tokens(messages.get(through..).unwrap_or_default()))
            }
            None => estimate_tokens(messages),
        }
    }

    pub(crate) fn policy(&self, spec: &ModelSpec) -> CompactionPolicy {
        policy(&self.prompts, spec)
    }
}

/// What every [`Model`] carries.
pub(crate) fn spec(model: &Model) -> Result<&ModelSpec> {
    model
        .completion
        .spec()
        .context("the model says nothing of its limits")
}

/// rig's defaults, which assume a large window (16,384 tokens reserved and
/// 20,000 of tool output kept), capped at a quarter of this model's, so a small
/// local model still works. rig caps the messages it keeps the same way.
fn policy(prompts: &SummaryPrompts, spec: &ModelSpec) -> CompactionPolicy {
    let mut policy = CompactionPolicy {
        near_window: NearWindow::Clear,
        ..CompactionPolicy::default()
    };
    policy.summarizer.prompts = prompts.clone();
    if let Some(limit) = spec.input_limit() {
        let quarter = u64::from(limit) / 4;
        policy.summarizer.limits.reserve = policy.summarizer.limits.reserve.min(quarter);
        policy.clearing.keep_tokens = policy.clearing.keep_tokens.min(quarter as usize);
    }
    policy
}

#[cfg(test)]
mod tests {
    use rig_core::message::{CallId, LocalCallId, ToolName};

    use super::*;
    use crate::local::{markup::FUNCTION_TAGS, scripted::scripted};

    fn usage(tokens: u64) -> Usage {
        let mut usage = Usage::default();
        usage.input_tokens = Some(tokens);
        usage
    }

    fn long(bytes: usize) -> String {
        "x".repeat(bytes)
    }

    #[test]
    fn only_the_first_reply_in_a_batch_sets_the_count() {
        let mut context = Context::default();
        let first = vec![Message::user("One."), Message::assistant("A.")];
        let second = vec![Message::user(long(4000)), Message::assistant("B.")];
        let estimate = estimate_tokens(&second);
        context.record([(first, usage(100)), (second, usage(100))]);
        assert_eq!(context.tokens(), 100 + estimate);
    }

    #[test]
    fn with_no_usage_it_estimates() {
        let mut context = Context::default();
        let messages = vec![Message::user(long(4000))];
        let estimate = estimate_tokens(&messages);
        context.record([(messages, Usage::default())]);
        assert_eq!(context.tokens(), estimate);
    }

    #[tokio::test]
    async fn a_conversation_that_fits_is_left_alone() {
        // An empty script: asking the model anything panics its thread.
        let script = scripted(Some(&FUNCTION_TAGS), 8192, Vec::new());
        let mut context = Context::default();
        context.record([(vec![Message::user("Hi.")], usage(10))]);
        context.fit(&script.model).await.unwrap();
        assert_eq!(context.messages().len(), 1);
    }

    #[tokio::test]
    async fn old_tool_outputs_go_before_anything_is_summarized() {
        let script = scripted(Some(&FUNCTION_TAGS), 4000, Vec::new());
        let call = CallId::Local(LocalCallId::new());
        let name = ToolName::new("get_page").unwrap();
        let mut context = Context::default();
        context.record([(
            vec![
                Message::user("Read it."),
                Message::tool_result(call, name, long(16_000)),
                Message::assistant("Read."),
                Message::user("Thanks."),
            ],
            Usage::default(),
        )]);
        context.fit(&script.model).await.unwrap();
        assert_eq!(context.messages().len(), 4);
        assert!(context.tokens() < 3000);
    }

    #[tokio::test]
    async fn older_messages_are_summarized_when_clearing_is_not_enough() {
        let script = scripted(Some(&FUNCTION_TAGS), 4000, vec![Ok("The story so far.")]);
        let mut context = Context::default();
        let messages = (0..40)
            .flat_map(|_| [Message::user(long(1000)), Message::assistant(long(1000))])
            .collect();
        context.record([(messages, Usage::default())]);
        context.fit(&script.model).await.unwrap();
        assert!(context.messages().len() < 80);
        let Message::User { content } = &*context.messages()[0] else {
            panic!("a summary");
        };
        assert!(format!("{content:?}").contains("The story so far."));
        assert!(context.tokens() < 3000);
    }

    #[tokio::test]
    async fn a_model_that_refused_gets_a_shorter_conversation() {
        let script = scripted(Some(&FUNCTION_TAGS), 400_000, vec![Ok("The story so far.")]);
        let mut context = Context::default();
        let messages = (0..4)
            .flat_map(|_| [Message::user("Page."), Message::assistant("Answer.")])
            .collect();
        context.record([(messages, usage(10))]);
        context
            .compact(&script.model, CompactReason::Overflow)
            .await
            .unwrap();
        assert!(context.messages().len() < 8);
    }

    #[test]
    fn a_small_window_caps_the_defaults() {
        let window = |tokens| {
            let vendor = rig_core::providers::registry::ProviderId::declare("test").unwrap();
            ModelSpec::new(vendor, "test").with_context_window(tokens)
        };
        let small = policy(&SummaryPrompts::default(), &window(4000));
        assert_eq!(small.summarizer.limits.reserve, 1000);
        assert_eq!(small.clearing.keep_tokens, 1000);
        let large = policy(&SummaryPrompts::default(), &window(1_000_000));
        assert_eq!(large.summarizer.limits.reserve, 16_384);
    }
}
