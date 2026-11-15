//! Fonts. They belong to the machine, so they're the process's, shared by
//! every renderer: the installed ones and the bundled catalog's in one
//! library, the font list the editor shows, each family's preview, and the
//! chain of fonts a span of text is set in.

use std::{collections::BTreeMap, ops::RangeInclusive, sync::LazyLock};

use anyhow::Result;
use fontique::{Attributes, FontStyle, FontWeight, Language};
use koharu_scene::{CharStyle, LanguageIdentifier};
use serde::Serialize;
use specta::Type;

mod catalog;
mod font;
mod library;
mod preview;

use self::library::Request;
pub use self::preview::preview;
pub(crate) use self::{font::Font, library::Library};

/// Families a text falls back on after the platform's fallback for its
/// scripts, for the symbols and emoji that one lacks.
const SYMBOL_FAMILIES: &[&str] = &[
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Noto Color Emoji",
    "Apple Symbols",
    "Apple Color Emoji",
    "Symbola",
    "Arial Unicode MS",
];

/// A family as the font list shows it, installed or bundled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct FontFamily {
    pub name: String,
    pub faces: Vec<FontFace>,
}

/// One of a family's faces, by the weight and slant a text asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct FontFace {
    pub post_script_name: String,
    pub weight: u16,
    /// A variable face's weights.
    pub weight_range: Option<RangeInclusive<u16>>,
    /// Italic or oblique, as a text's `italic` asks for.
    pub italic: bool,
}

/// Every family installed or bundled, by name, upright faces first and then
/// by weight. An installed family and a bundled one of the same name are
/// one, and so are an installed face and a bundled one of the same
/// PostScript name.
pub async fn list() -> Result<Vec<FontFamily>> {
    tokio::task::spawn_blocking(|| {
        // Reading every installed face's names loads its file, so it's done
        // once.
        static INSTALLED: LazyLock<Vec<FontFamily>> =
            LazyLock::new(|| library::with(Library::families));
        let bundled = catalog::families()?.values().map(catalog::listed);
        let mut families = BTreeMap::<String, FontFamily>::new();
        for family in INSTALLED.iter().cloned().chain(bundled) {
            let listed = families
                .entry(normalize(&family.name))
                .or_insert_with(|| FontFamily {
                    name: family.name,
                    faces: Vec::new(),
                });
            for face in family.faces {
                let name = normalize(&face.post_script_name);
                if !listed
                    .faces
                    .iter()
                    .any(|known| normalize(&known.post_script_name) == name)
                {
                    listed.faces.push(face);
                }
            }
        }
        Ok(families
            .into_values()
            .map(|mut family| {
                family.faces.sort_by(|left, right| {
                    (left.italic, left.weight, &left.post_script_name).cmp(&(
                        right.italic,
                        right.weight,
                        &right.post_script_name,
                    ))
                });
                family
            })
            .collect())
    })
    .await?
}

/// The fonts a span in `style` sets `text` in, in order: its family, then
/// `fallbacks`, then Arial, then the platform's fallback for the text's
/// scripts in `language`, then symbol and emoji families. A family that
/// isn't installed is registered from the bundled catalog, downloaded here the
/// first time it's asked for; one the catalog lacks too, or that fails to
/// download, is left out.
pub(crate) fn chain(
    style: &CharStyle,
    fallbacks: &[String],
    text: &str,
    language: &LanguageIdentifier,
) -> Result<Vec<Font>> {
    let mut families: Vec<String> = Vec::with_capacity(fallbacks.len() + 2);
    let asked = style
        .family
        .as_deref()
        .filter(|family| !family.trim().is_empty());
    for family in asked
        .into_iter()
        .chain(fallbacks.iter().map(String::as_str))
        .chain(["Arial"])
    {
        if !families
            .iter()
            .any(|known| known.eq_ignore_ascii_case(family))
        {
            families.push(family.to_owned());
        }
    }
    let attributes = Attributes {
        weight: FontWeight::new(f32::from(style.weight.unwrap_or(400))),
        style: match style.italic.unwrap_or(false) {
            true => FontStyle::Italic,
            false => FontStyle::Normal,
        },
        ..Attributes::default()
    };
    for family in families.iter().filter(|family| !library::installed(family)) {
        let registered = catalog::family(family).and_then(|bundled| match bundled {
            Some(bundled) => catalog::register(bundled, attributes),
            None => Ok(()),
        });
        if let Err(error) = registered {
            tracing::warn!("couldn't download the font {family:?}: {error:#}");
        }
    }
    let language = (*language != LanguageIdentifier::UNKNOWN)
        .then(|| Language::parse(&language.to_string()).ok())
        .flatten();
    library::with(|library| {
        library.chain(Request {
            families: &families,
            symbols: SYMBOL_FAMILIES,
            attributes,
            text,
            language,
        })
    })
}

/// Lists the installed families and loads the bundled catalog, which the
/// first text would otherwise wait for.
pub(crate) fn preload() {
    library::preload();
    let _ = catalog::families();
}

/// A family or face name as Koharu matches it: trimmed, in any case.
fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}
