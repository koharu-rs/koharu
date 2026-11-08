//! The installed fonts through fontique, and the font chain a text resolves
//! to among them.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use fontique::{
    Attributes, Collection, CollectionOptions, FallbackKey, FontStyle, GenericFamily, Language,
    QueryFamily, QueryFont, QueryStatus, Script, SourceCache,
};

use super::face::{self, Font};
use crate::{
    script::{fontique_scripts, implied_language},
    types::{FontFace, FontStyle as PublicFontStyle},
};

/// fontique's collection, the sources its queries load, and each face built
/// from them once, which keeps its shape plans.
pub(crate) struct FontSystem {
    pub(super) collection: Collection,
    sources: SourceCache,
    built: Vec<Font>,
    /// Every installed face with its family's name, listed on first use.
    faces: Option<Vec<(String, FontFace)>>,
}

impl FontSystem {
    pub(crate) fn new() -> Self {
        let mut collection = Collection::new(CollectionOptions::default());
        // fontique 0.12 keeps Han's fallback for no locale and for zh-CN,
        // Han's default locale, in one slot (`canonical_locale` in
        // `fallback.rs`), and every query also asks for Han with no locale
        // (the `HACK` in `Query::set_fallbacks`), which the platform answers
        // with the system locale's font. The first to ask keeps the slot, so
        // on a Japanese system Chinese was set in Yu Gothic after any other
        // text. Asking for zh-CN first gives the slot the Chinese font.
        let chinese = Language::parse("zh-CN").ok();
        let _ = collection.fallback_families(FallbackKey::new(
            Script::from_bytes(*b"Hani"),
            chinese.as_ref(),
        ));
        Self {
            collection,
            sources: SourceCache::new_shared(),
            built: Vec::new(),
            faces: None,
        }
    }

    /// The fonts of `families`, then the platform's fallback for each of
    /// `text`'s scripts in `language`, or the one its scripts imply, then
    /// `symbols`; the platform's sans-serif when none of `families` is
    /// installed.
    pub(super) fn chain(
        &mut self,
        families: &[String],
        symbols: &[&str],
        attributes: Attributes,
        text: &str,
        language: Option<Language>,
    ) -> Result<Vec<Font>> {
        let language =
            language.or_else(|| implied_language(text).and_then(|tag| Language::parse(tag).ok()));
        // Explicit symbol families are a final safety net, not body-font
        // candidates. Query them separately so a missing requested family
        // still falls back to the platform's script-appropriate text font.
        let symbols: Vec<&str> = symbols
            .iter()
            .copied()
            .filter(|symbol| {
                !families
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(symbol))
            })
            .collect();
        let mut candidates = Vec::new();
        for script in fontique_scripts(text) {
            {
                let mut query = self.collection.query(&mut self.sources);
                if families.is_empty() {
                    query.set_families([QueryFamily::Generic(GenericFamily::SansSerif)]);
                } else {
                    query.set_families(families.iter().map(|family| QueryFamily::Named(family)));
                }
                query.set_attributes(attributes);
                query.set_fallbacks(FallbackKey::new(script, language.as_ref()));
                query.matches_with(|font| {
                    candidates.push(font.clone());
                    QueryStatus::Continue
                });
            }
            if !symbols.is_empty() {
                let mut query = self.collection.query(&mut self.sources);
                query.set_families(symbols.iter().map(|family| QueryFamily::Named(family)));
                query.set_attributes(attributes);
                query.matches_with(|font| {
                    candidates.push(font.clone());
                    QueryStatus::Continue
                });
            }
        }

        let mut seen = HashSet::new();
        let mut resolved = Vec::new();
        for candidate in candidates {
            // A face that can't be read, such as one with no units per em,
            // is left out, as it couldn't draw the text.
            if seen.insert((candidate.blob.id(), candidate.index))
                && let Ok(font) = self.face(candidate)
            {
                resolved.push(font);
            }
        }
        if resolved.is_empty() && !families.is_empty() {
            resolved = self.chain(&[], &symbols, attributes, text, language)?;
        }
        if resolved.is_empty() {
            bail!("no usable fonts are installed");
        }
        Ok(resolved)
    }

    /// The font `query` found, built the first time it's asked for.
    fn face(&mut self, query: QueryFont) -> Result<Font> {
        if let Some(font) = self.built.iter().find(|font| font.is(&query)) {
            return Ok(font.clone());
        }
        let family_name = self
            .collection
            .family_name(query.family.0)
            .unwrap_or("Unknown")
            .to_owned();
        let font = Font::from_query(query, family_name)?;
        self.built.push(font.clone());
        Ok(font)
    }

    /// The installed family's regular face.
    pub(crate) fn query_family(&mut self, family: &str) -> Result<Font> {
        self.chain(
            &[family.to_owned()],
            &[],
            Attributes::default(),
            family,
            None,
        )?
        .into_iter()
        .find(|font| font.family_name().eq_ignore_ascii_case(family))
        .with_context(|| format!("no font found for family {family:?}"))
    }

    pub(super) fn faces(&mut self) -> Vec<(String, FontFace)> {
        if let Some(faces) = &self.faces {
            return faces.clone();
        }
        let mut faces = Vec::new();
        let families: Vec<String> = self.collection.family_names().map(str::to_owned).collect();
        for family_name in families {
            let Some(family) = self.collection.family_by_name(&family_name) else {
                continue;
            };
            for info in family.fonts() {
                let Some(blob) = info.load(Some(&mut self.sources)) else {
                    continue;
                };
                let Ok(font) = skrifa::FontRef::from_index(blob.as_ref(), info.index()) else {
                    continue;
                };
                let face = FontFace {
                    post_script_name: face::post_script_name(&font)
                        .unwrap_or_else(|| family_name.clone()),
                    weight: info.weight().value().round().clamp(1.0, 1000.0) as u16,
                    weight_range: None,
                    style: match info.style() {
                        FontStyle::Normal => PublicFontStyle::Normal,
                        FontStyle::Italic => PublicFontStyle::Italic,
                        FontStyle::Oblique(_) => PublicFontStyle::Oblique,
                    },
                };
                faces.push((family_name.clone(), face));
            }
        }
        self.faces = Some(faces.clone());
        faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers_simplified(system: &mut FontSystem, language: &str) -> bool {
        system
            .chain(
                &["Arial".to_owned()],
                &[],
                Attributes::default(),
                "创长说",
                Language::parse(language).ok(),
            )
            .expect("a chain")
            .iter()
            .any(|font| font.covers("创长说"))
    }

    #[test]
    fn simplified_chinese_falls_back_alike_after_japanese() {
        let fresh = covers_simplified(&mut FontSystem::new(), "zh");
        let mut system = FontSystem::new();
        covers_simplified(&mut system, "ja");
        assert_eq!(covers_simplified(&mut system, "zh"), fresh);
    }

    #[test]
    fn untagged_kana_falls_back_like_japanese() {
        let mut system = FontSystem::new();
        let mut families = |language| {
            system
                .chain(&[], &[], Attributes::default(), "ひらがなと漢字", language)
                .expect("a chain")
                .iter()
                .map(|font| font.family_name().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(families(None), families(Language::parse("ja").ok()));
    }
}
