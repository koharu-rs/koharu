//! A conversation kept inside a model's context window, with rig-memory's
//! compaction: nothing in rig's agent runs it, so this does.

use std::{slice, sync::Arc};

use anyhow::{Context as _, Result};
use indoc::formatdoc;
use rig_core::{
    catalog::ModelSpec,
    completion::{CompletionRequest, Message, UnsupportedOption, Usage},
    conversation::Conversation,
    driver::DynModel,
    operation::Completion,
};
use rig_memory::{
    CompactionPreparation, CompactionSettings, SummaryState, apply_summary, estimate_tokens,
    find_cut_point, prepare_compaction, serialize_conversation, summarization_request,
    summary_author, summary_request, summary_text,
};

use crate::TRANSLATION;

/// What a model is asked for a summary in place of rig's prompts, which are
/// written for a coding agent.
#[derive(Clone, Copy, Debug)]
pub struct SummaryPrompts {
    pub system: &'static str,
    /// The request for a first summary, after the conversation.
    pub initial: &'static str,
    /// The request to fold the conversation into `<previous-summary>`.
    pub update: &'static str,
    /// The summary's format, after either request.
    pub format: &'static str,
}

/// A conversation that fits the model it's sent to.
///
/// It's kept in memory; where a conversation is saved isn't decided.
#[derive(Clone, Debug, Default)]
pub struct Context {
    conversation: Conversation,
    /// rig's own when unset.
    prompts: Option<SummaryPrompts>,
    /// The tokens a provider last reported, and how many messages they cover.
    /// rig's estimate of 4 characters a token undercounts Japanese and Chinese,
    /// so it only stands in for what comes after them.
    count: Option<(u64, usize)>,
}

impl Context {
    /// An empty conversation, summarized as `prompts` ask.
    #[must_use]
    pub fn new(prompts: SummaryPrompts) -> Self {
        Self {
            prompts: Some(prompts),
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

    /// Compacts if a request that goes on with `next` wouldn't leave the model's
    /// reserve free. A model with no known limit is never over; [`Self::compact`]
    /// answers its refusal. As in rig's own agent, a summary that fails leaves
    /// the conversation as it was.
    pub async fn fit(&mut self, model: &DynModel<Completion>, next: &Message) {
        let Some(spec) = model.spec() else {
            return;
        };
        let tokens = self.tokens() + estimate_tokens(slice::from_ref(next));
        let over = spec
            .input_limit()
            .is_some_and(|limit| settings(spec).should_compact(tokens, limit.into()));
        if over && let Err(error) = self.compact_with(model, spec, false).await {
            tracing::warn!("{error:#}");
        }
    }

    /// Compacts now, because the model refused the request as too long.
    pub async fn compact(&mut self, model: &DynModel<Completion>) -> Result<()> {
        let spec = model
            .spec()
            .context("the model says nothing of its limits")?;
        self.compact_with(model, spec, true).await
    }

    /// Drops the older half, for a model that filled its window, which it may
    /// not say, and a summary might not fit either.
    pub fn halve(&mut self) {
        self.drop_turns(estimate_tokens(self.messages()) / 2, true);
    }

    /// Summarizes older messages, or, for a model whose spec says it only
    /// translates, which can't summarize, drops them, keeping the newest.
    async fn compact_with(
        &mut self,
        model: &DynModel<Completion>,
        spec: &ModelSpec,
        refused: bool,
    ) -> Result<()> {
        let keep = self.keep(spec, refused);
        if spec.details.kind.as_deref() == Some(TRANSLATION) {
            self.drop_turns(keep, refused);
            return Ok(());
        }
        let Some(plan) = prepare_compaction(&self.conversation, keep, refused) else {
            return Ok(());
        };
        let summary = summarize(self.prompts.as_ref(), &plan, model, spec)
            .await
            .context("couldn't summarize the conversation")?;
        if apply_summary(&mut self.conversation, &plan.replaces, &summary).is_some() {
            self.count = None;
        }
        Ok(())
    }

    /// Drops the oldest turns, keeping about `keep` tokens of the newest.
    fn drop_turns(&mut self, keep: u64, refused: bool) {
        let messages = self.conversation.messages();
        let cut = find_cut_point(messages, (0, messages.len()), keep, refused);
        // The cut may be at a reply, but a reply stays with the turn it answers.
        let turn = messages
            .iter()
            .take(cut + 1)
            .rposition(|message| matches!(**message, Message::User { .. }));
        if let Some(turn @ 1..) = turn {
            self.conversation
                .splice(..turn, std::iter::empty(), summary_author());
            self.count = None;
        }
    }

    /// How many tokens of the newest messages a compaction keeps, as rig
    /// estimates them, which can make a quarter as many of Japanese and Chinese
    /// as the provider counted.
    fn keep(&self, spec: &ModelSpec, refused: bool) -> u64 {
        let estimate = estimate_tokens(self.messages());
        let keep = settings(spec).keep_recent_tokens;
        let keep = (keep * estimate / self.tokens().max(1)).clamp(keep / 4, keep);
        // What the model refused gets shorter, even if it would all be kept.
        if refused {
            keep.min(estimate / 2)
        } else {
            keep
        }
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
}

/// rig's defaults, which assume a large window (16,384 tokens reserved and
/// 20,000 of the newest kept), capped at a quarter of this model's, so a small
/// local model still works.
fn settings(spec: &ModelSpec) -> CompactionSettings {
    let mut settings = CompactionSettings::default();
    if let Some(limit) = spec.input_limit() {
        let quarter = u64::from(limit) / 4;
        settings.reserve_tokens = settings.reserve_tokens.min(quarter);
        settings.keep_recent_tokens = settings.keep_recent_tokens.min(quarter);
    }
    settings
}

/// What `plan` summarizes, in one call, the earlier part of a turn it splits
/// included.
async fn summarize(
    prompts: Option<&SummaryPrompts>,
    plan: &CompactionPreparation,
    model: &DynModel<Completion>,
    spec: &ModelSpec,
) -> Result<SummaryState> {
    let older: Vec<_> = plan
        .to_summarize
        .iter()
        .chain(&plan.turn_prefix)
        .cloned()
        .collect();
    let previous = plan.previous.as_ref();
    let request = match prompts {
        Some(prompts) => prompted_request(prompts, &older, previous, spec)?,
        None => summary_request(&older, (previous, ""), spec, None)?,
    };
    let response = model.stream(request)?.finish().await?;
    Ok(SummaryState {
        summary: summary_text(&response, "the summary")?,
        ..SummaryState::default()
    })
}

/// rig's request for a summary of `older`, asked as `prompts` ask.
fn prompted_request(
    prompts: &SummaryPrompts,
    older: &[Arc<Message>],
    previous: Option<&SummaryState>,
    spec: &ModelSpec,
) -> Result<CompletionRequest, UnsupportedOption> {
    let conversation = serialize_conversation(older);
    let ask = match previous.and_then(SummaryState::previous_summary) {
        Some(previous) => formatdoc! {"
            <previous-summary>
            {previous}
            </previous-summary>

            {}", prompts.update},
        None => prompts.initial.to_owned(),
    };
    let prompt = formatdoc! {"
        <conversation>
        {conversation}
        </conversation>

        {ask}

        {}", prompts.format};
    summarization_request((prompts.system, prompt), spec, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        markup::FUNCTION_TAGS,
        scripted::{scripted, scripted_kind},
    };

    fn usage(tokens: u64) -> Usage {
        let mut usage = Usage::default();
        usage.input_tokens = Some(tokens);
        usage
    }

    fn turns(count: usize, text: &str) -> Vec<Message> {
        (0..count)
            .flat_map(|_| [Message::user(text), Message::assistant(text)])
            .collect()
    }

    #[test]
    fn only_the_first_reply_in_a_batch_sets_the_count() {
        let mut context = Context::default();
        let first = vec![Message::user("One."), Message::assistant("A.")];
        let second = vec![Message::user("x".repeat(4000)), Message::assistant("B.")];
        let estimate = estimate_tokens(&second);
        context.record([(first, usage(100)), (second, usage(100))]);
        assert_eq!(context.tokens(), 100 + estimate);
    }

    #[test]
    fn with_no_usage_it_estimates() {
        let mut context = Context::default();
        let messages = vec![Message::user("x".repeat(4000))];
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
        context.fit(&script.model, &Message::user("Go on.")).await;
        assert_eq!(context.messages().len(), 1);
    }

    #[tokio::test]
    async fn older_messages_are_summarized() {
        let script = scripted(Some(&FUNCTION_TAGS), 4000, vec![Ok("The story so far.")]);
        let mut context = Context::default();
        context.record([(turns(40, &"x".repeat(1000)), Usage::default())]);
        context.fit(&script.model, &Message::user("Go on.")).await;
        assert!(context.messages().len() < 80);
        let Message::User { content } = &*context.messages()[0] else {
            panic!("a summary");
        };
        assert!(format!("{content:?}").contains("The story so far."));
        assert!(context.tokens() < 3000);
    }

    /// rig's estimate of these messages is a quarter of what they take, and
    /// within it nothing would be cut.
    #[tokio::test]
    async fn japanese_is_kept_by_what_the_provider_counted() {
        let script = scripted(Some(&FUNCTION_TAGS), 32_768, vec![Ok("これまでの話。")]);
        let mut context = Context::default();
        context.record([(turns(42, &"あ".repeat(300)), usage(25_200))]);
        context.fit(&script.model, &Message::user("次。")).await;
        assert!(
            context.messages().len() < 40,
            "{}",
            context.messages().len()
        );
    }

    #[tokio::test]
    async fn a_summary_that_fails_leaves_the_conversation_as_it_was() {
        let script = scripted(Some(&FUNCTION_TAGS), 4000, vec![Ok(" ")]);
        let mut context = Context::default();
        context.record([(turns(40, &"x".repeat(1000)), Usage::default())]);
        context.fit(&script.model, &Message::user("Go on.")).await;
        assert_eq!(context.messages().len(), 80);
    }

    #[tokio::test]
    async fn a_translation_model_drops_its_oldest_turns_unsummarized() {
        // An empty script: asking the model for a summary panics its thread.
        let script = scripted_kind(Some(&FUNCTION_TAGS), Some(TRANSLATION), 4000, Vec::new());
        let mut context = Context::default();
        context.record([(turns(40, &"x".repeat(1000)), Usage::default())]);
        context.fit(&script.model, &Message::user("Go on.")).await;
        // The newest pages stay whole, each turn with its reply.
        let kept = context.messages();
        assert!(kept.len() < 80 && kept.len() % 2 == 0, "{}", kept.len());
        assert!(matches!(*kept[0], Message::User { .. }));
        assert!(context.tokens() < 3000);
    }

    #[tokio::test]
    async fn a_model_that_refused_gets_a_shorter_conversation() {
        let script = scripted(Some(&FUNCTION_TAGS), 400_000, vec![Ok("The story so far.")]);
        let mut context = Context::default();
        context.record([(turns(4, "Page."), usage(10))]);
        context.compact(&script.model).await.unwrap();
        assert!(context.messages().len() < 8);
    }

    #[test]
    fn a_small_window_caps_the_defaults() {
        let window = |tokens| {
            let vendor = rig_core::providers::registry::ProviderId::declare("test").unwrap();
            ModelSpec::new(vendor, "test").with_context_window(tokens)
        };
        let small = settings(&window(4000));
        assert_eq!(small.reserve_tokens, 1000);
        assert_eq!(small.keep_recent_tokens, 1000);
        let large = settings(&window(1_000_000));
        assert_eq!(large.reserve_tokens, 16_384);
    }
}
