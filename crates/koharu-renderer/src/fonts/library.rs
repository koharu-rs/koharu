//! The library: fontique's collection of the installed fonts, and of the
//! catalog's as they're registered, which the process shares. Each thread
//! works on its own clone, which sees what any clone registers, so text laid
//! out across rayon takes no lock.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
    sync::LazyLock,
};

use anyhow::{Context, Result, bail, ensure};
use fontique::{
    Attributes, Blob, CollectionOptions, FallbackKey, FontInfo, FontInfoOverride, FontStyle,
    FontWeight, GenericFamily, Language, QueryFamily, QueryFont, QueryStatus, Script, SourceCache,
};
use parking_lot::Mutex;

use super::{FontFace, FontFamily, font, font::Font, normalize};
use crate::script::{fontique_scripts, implied_language};

/// The library every thread clones.
static SHARED: LazyLock<Mutex<Library>> = LazyLock::new(|| Mutex::new(Library::new()));

/// The installed families by normalized name, as the system had them when
/// the library was made, before any catalog face joined it.
static INSTALLED: LazyLock<HashSet<String>> = LazyLock::new(|| {
    SHARED
        .lock()
        .collection
        .family_names()
        .map(normalize)
        .collect()
});

thread_local! {
    static LOCAL: RefCell<Library> = RefCell::new(SHARED.lock().clone());
}

/// Runs `run` with this thread's clone of the process's library.
pub(super) fn with<T>(run: impl FnOnce(&mut Library) -> T) -> T {
    LOCAL.with_borrow_mut(run)
}

/// Whether `family` was installed when the library was made.
pub(super) fn installed(family: &str) -> bool {
    INSTALLED.contains(&normalize(family))
}

/// Lists the installed families, which `installed` would otherwise wait for.
pub(super) fn preload() {
    LazyLock::force(&INSTALLED);
}

/// fontique's collection, the sources its queries load, and each font built
/// from them, once, which keeps its shape plans.
#[derive(Clone)]
pub(crate) struct Library {
    collection: fontique::Collection,
    sources: SourceCache,
    /// Built fonts by file and index, one for each instance and each bold and
    /// slant faked.
    fonts: HashMap<(u64, u32), Vec<Font>>,
}

/// What a chain is asked for.
#[derive(Clone, Copy)]
pub(super) struct Request<'a> {
    /// The families to set the text in, in order.
    pub families: &'a [String],
    /// The last resort for symbols and emoji, after the platform's fallback.
    pub symbols: &'a [&'a str],
    pub attributes: Attributes,
    pub text: &'a str,
    /// The text's language; `None` to take the one its scripts imply.
    pub language: Option<Language>,
}

impl Library {
    /// A library of the installed fonts, apart from the process's.
    pub(crate) fn new() -> Self {
        Self {
            collection: fontique::Collection::new(CollectionOptions {
                shared: true,
                ..CollectionOptions::default()
            }),
            sources: SourceCache::new_shared(),
            fonts: HashMap::new(),
        }
    }

    /// The fonts of `request`'s families, then the platform's fallback for
    /// each of its text's scripts, then its symbol families: the platform's
    /// sans-serif instead when none of its families is installed.
    pub(super) fn chain(&mut self, request: Request) -> Result<Vec<Font>> {
        // fontique 0.12 keeps Han's fallback for no locale and for zh-CN,
        // Han's default locale, in one slot (`canonical_locale` in
        // `fallback.rs`), and every query also asks for Han with no locale
        // (the `HACK` in `Query::set_fallbacks`), which the platform answers
        // with the system locale's font. The first to ask keeps the slot, so
        // on a Japanese system Chinese was set in Yu Gothic after any other
        // text. Asking for zh-CN first gives the slot the Chinese font. A
        // clone empties its fallbacks when another clone registers a font
        // (`sync_shared`), so it's asked before every chain, where it's a
        // lookup unless they were emptied.
        let chinese = Language::parse("zh-CN").ok();
        let _ = self.collection.fallback_families(FallbackKey::new(
            Script::from_bytes(*b"Hani"),
            chinese.as_ref(),
        ));
        // A symbol family asked for as a text family is one already.
        let symbols: Vec<&str> = request
            .symbols
            .iter()
            .copied()
            .filter(|symbol| {
                !request
                    .families
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(symbol))
            })
            .collect();
        let request = Request {
            symbols: &symbols,
            language: request.language.or_else(|| {
                implied_language(request.text).and_then(|tag| Language::parse(tag).ok())
            }),
            ..request
        };
        let mut seen = HashSet::new();
        let mut fonts = Vec::new();
        for found in self.matches(request) {
            // A face that can't be read, such as one with no units per em,
            // is left out, as it couldn't draw the text.
            if seen.insert((found.blob.id(), found.index))
                && let Ok(font) = self.font(found, request.attributes.weight)
            {
                fonts.push(font);
            }
        }
        if fonts.is_empty() && !request.families.is_empty() {
            fonts = self.chain(Request {
                families: &[],
                ..request
            })?;
        }
        if fonts.is_empty() {
            bail!("no usable fonts are installed");
        }
        Ok(fonts)
    }

    /// The faces fontique matches for `request`, in order, script by script:
    /// its families' and the platform's fallback, then its symbol families'
    /// apart, so a missing family still falls back to the platform's font
    /// for the script.
    fn matches(&mut self, request: Request) -> Vec<QueryFont> {
        let mut found = Vec::new();
        for script in fontique_scripts(request.text) {
            let mut query = self.collection.query(&mut self.sources);
            if request.families.is_empty() {
                query.set_families([QueryFamily::Generic(GenericFamily::SansSerif)]);
            } else {
                query.set_families(
                    request
                        .families
                        .iter()
                        .map(|family| QueryFamily::Named(family)),
                );
            }
            query.set_attributes(request.attributes);
            query.set_fallbacks(FallbackKey::new(script, request.language.as_ref()));
            query.matches_with(|font| {
                found.push(font.clone());
                QueryStatus::Continue
            });
            drop(query);
            if !request.symbols.is_empty() {
                let mut query = self.collection.query(&mut self.sources);
                query.set_families(
                    request
                        .symbols
                        .iter()
                        .map(|family| QueryFamily::Named(family)),
                );
                query.set_attributes(request.attributes);
                query.matches_with(|font| {
                    found.push(font.clone());
                    QueryStatus::Continue
                });
            }
        }
        found
    }

    /// The font `found` is in a text of `weight`, built the first time it's
    /// asked for.
    fn font(&mut self, found: QueryFont, weight: FontWeight) -> Result<Font> {
        let (id, index) = found.family;
        let family = self.collection.family(id);
        // Bold is 600 and up, as Firefox counts it. fontique fakes any weight
        // more than 200 over the face's, which leaves a medium face medium in
        // bold, and thickens a thin one in regular text.
        let bold = FontWeight::SEMI_BOLD.value();
        let face = family.as_ref().and_then(|family| family.fonts().get(index));
        let faux_bold = weight.value() >= bold
            && face.is_some_and(|face| {
                weights(face).map_or(face.weight().value(), |range| f32::from(*range.end())) < bold
            });
        let built = self
            .fonts
            .entry((found.blob.id(), found.index))
            .or_default();
        if let Some(font) = built
            .iter()
            .find(|font| font.synthesis == found.synthesis && font.faux_bold == faux_bold)
        {
            return Ok(font.clone());
        }
        let name = family.as_ref().map_or("Unknown", |family| family.name());
        let font = Font::new(found, name.to_owned(), faux_bold)?;
        built.push(font.clone());
        Ok(font)
    }

    /// `family`'s regular face, when it's installed.
    pub(crate) fn family(&mut self, family: &str) -> Result<Font> {
        self.chain(Request {
            families: &[family.to_owned()],
            symbols: &[],
            attributes: Attributes::default(),
            text: family,
            language: None,
        })?
        .into_iter()
        .find(|font| font.family_name.eq_ignore_ascii_case(family))
        .with_context(|| format!("no font found for family {family:?}"))
    }

    /// Adds the faces `data` holds as `family`, the catalog's name for them:
    /// CJK fonts often carry a different, localized one.
    pub(super) fn register(&mut self, data: Vec<u8>, family: &str) -> Result<()> {
        let registered = self.collection.register_fonts(
            Blob::from(data),
            Some(FontInfoOverride {
                family_name: Some(family),
                ..FontInfoOverride::default()
            }),
        );
        ensure!(
            !registered.is_empty(),
            "the font file holds no usable faces"
        );
        Ok(())
    }

    /// Every installed family as the font list shows it, with the faces whose
    /// files load: reading a face's PostScript name loads its file.
    pub(super) fn families(&mut self) -> Vec<FontFamily> {
        let names: Vec<String> = self
            .collection
            .family_names()
            .filter(|name| installed(name))
            .map(str::to_owned)
            .collect();
        let mut families = Vec::with_capacity(names.len());
        for name in names {
            let Some(info) = self.collection.family_by_name(&name) else {
                continue;
            };
            let mut family = FontFamily {
                name,
                faces: Vec::new(),
            };
            for face in info.fonts() {
                let Some(blob) = face.load(Some(&mut self.sources)) else {
                    continue;
                };
                let Ok(file) = skrifa::FontRef::from_index(blob.as_ref(), face.index()) else {
                    continue;
                };
                family.faces.push(FontFace {
                    post_script_name: font::post_script_name(&file)
                        .unwrap_or_else(|| family.name.clone()),
                    weight: face.weight().value().round() as u16,
                    weight_range: weights(face),
                    italic: face.style() != FontStyle::Normal,
                });
            }
            if !family.faces.is_empty() {
                families.push(family);
            }
        }
        families
    }
}

/// A variable face's weights.
fn weights(face: &FontInfo) -> Option<RangeInclusive<u16>> {
    let axis = face
        .axes()
        .iter()
        .find(|axis| axis.tag.to_be_bytes() == *b"wght")?;
    Some(axis.min.round() as u16..=axis.max.round() as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers_simplified(library: &mut Library, language: &str) -> bool {
        library
            .chain(Request {
                families: &["Arial".to_owned()],
                symbols: &[],
                attributes: Attributes::default(),
                text: "创长说",
                language: Language::parse(language).ok(),
            })
            .expect("a chain")
            .iter()
            .any(|font| font.covers("创长说"))
    }

    #[test]
    fn simplified_chinese_falls_back_alike_after_japanese() {
        let fresh = covers_simplified(&mut Library::new(), "zh");
        let mut library = Library::new();
        covers_simplified(&mut library, "ja");
        assert_eq!(covers_simplified(&mut library, "zh"), fresh);
    }
}
