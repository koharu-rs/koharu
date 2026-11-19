//! Human review operations. Apply accepted edits and close their suggestions atomically.
use std::sync::Arc;

use super::Run;
use crate::{AliasSuggestion, Error, GlossaryEntry, GlossaryKind, Result, TextSuggestion};

impl Run<'_, '_> {
    pub(super) fn suggest_aliases(
        &mut self,
        index: usize,
        suggestions: &[AliasSuggestion],
    ) -> Result<()> {
        for item in suggestions {
            if !self.project.check_alias_evidence(item) {
                return Err(Error::invalid(
                    index,
                    "the alias evidence is missing or stale",
                ));
            }
            let reviews = Arc::make_mut(&mut self.project.suggestion_reviews);
            if reviews
                .closed_aliases
                .contains(&(item.source.clone(), item.canonical.clone()))
            {
                continue;
            }
            if let Some(row) = reviews
                .aliases
                .iter_mut()
                .find(|s| s.source == item.source && s.canonical == item.canonical)
            {
                *row = item.clone();
            } else {
                reviews.aliases.push(item.clone());
            }
        }
        Ok(())
    }

    pub(super) fn resolve_alias(
        &mut self,
        index: usize,
        suggestion: &AliasSuggestion,
        translation: Option<&str>,
    ) -> Result<()> {
        if !self.project.alias_suggestions().contains(suggestion) {
            return Err(Error::invalid(
                index,
                "the suggestion changed or was already resolved",
            ));
        }
        if let Some(translation) = translation {
            if translation.trim().is_empty() || !self.project.check_alias_evidence(suggestion) {
                return Err(Error::invalid(
                    index,
                    "the alias translation is empty or its evidence changed",
                ));
            }
            let entries = Arc::make_mut(&mut self.project.glossary);
            let canonical = entries.iter().find(|e| e.source == suggestion.canonical);
            let wiki_url = canonical.and_then(|e| e.wiki_url.clone());
            if let Some(alias) = entries.iter_mut().find(|e| e.source == suggestion.source) {
                alias.kind = GlossaryKind::Person;
                alias.translation = translation.trim().into();
                alias.wiki_url = alias.wiki_url.take().or(wiki_url);
            } else {
                entries.push(GlossaryEntry {
                    kind: GlossaryKind::Person,
                    source: suggestion.source.clone(),
                    translation: translation.trim().into(),
                    note: suggestion.canonical.clone(),
                    wiki_url,
                    excluded_layers: Default::default(),
                });
            }
        }
        let reviews = Arc::make_mut(&mut self.project.suggestion_reviews);
        reviews.aliases.retain(|row| {
            let close = row.source == suggestion.source
                && (translation.is_some() || row.canonical == suggestion.canonical);
            if close {
                reviews
                    .closed_aliases
                    .insert((row.source.clone(), row.canonical.clone()));
            }
            !close
        });
        Ok(())
    }

    pub(super) fn suggest_translation(
        &mut self,
        index: usize,
        suggestion: &TextSuggestion,
    ) -> Result<()> {
        if suggestion.reason.trim().is_empty()
            || suggestion.translation.trim().is_empty()
            || suggestion.previous == suggestion.translation
            || !self.project.check_translation_evidence(suggestion)
        {
            return Err(Error::invalid(
                index,
                "invalid correction or its source or translation changed",
            ));
        }
        let reviews = Arc::make_mut(&mut self.project.suggestion_reviews);
        if !reviews.closed_corrections.contains(&suggestion.key()) {
            reviews.corrections.retain(|old| {
                old.location.layer != suggestion.location.layer
                    || old.translation != suggestion.translation
            });
            reviews.corrections.push(suggestion.clone());
        }
        Ok(())
    }

    pub(super) fn resolve_translation(
        &mut self,
        index: usize,
        suggestion: &TextSuggestion,
        translation: Option<&str>,
    ) -> Result<()> {
        if !self.project.text_suggestions().contains(suggestion) {
            return Err(Error::invalid(
                index,
                "the correction changed or was already resolved",
            ));
        }
        if let Some(text) = translation {
            if text.trim().is_empty() || !self.project.check_translation_evidence(suggestion) {
                return Err(Error::invalid(
                    index,
                    "the correction is empty or its source or translation changed",
                ));
            }
            self.set_text(index, &suggestion.location.layer.into(), text)?;
        }
        let reviews = Arc::make_mut(&mut self.project.suggestion_reviews);
        reviews.corrections.retain(|s| {
            let close = s == suggestion
                || (translation.is_some()
                    && s.location.layer == suggestion.location.layer
                    && s.previous == suggestion.previous);
            if close {
                reviews.closed_corrections.insert(s.key());
            }
            !close
        });
        Ok(())
    }
}
