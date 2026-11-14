//! Fonts: the installed ones through fontique, the bundled catalog's
//! downloaded on first use, and the font chain a span resolves to.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, OnceLock},
};

use anyhow::Result;
use fontique::{Attributes, FontStyle, Language};
use koharu_scene::{CharStyle, LanguageIdentifier};
use parking_lot::Mutex;
use tokio::sync::OnceCell;

use crate::types::{FontFamily, FontMetadata, FontSource};

mod catalog;
mod face;
mod system;

use catalog::{FONTS, FontBytes, FontCatalog, normalize};
pub(crate) use face::Font;
pub(crate) use system::FontSystem;

const SYMBOL_FALLBACK_FAMILIES: &[&str] = &[
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

pub(crate) struct Fonts {
    system: OnceLock<Arc<Mutex<FontSystem>>>,
    catalog: OnceCell<Arc<FontCatalog>>,
    /// Loaded bundled faces by PostScript name, bounded by their bytes.
    bundled: quick_cache::sync::Cache<String, Font, FontBytes>,
    /// Families a load found in neither the system nor the catalog, which
    /// are no longer reported missing.
    unavailable: Mutex<HashSet<String>>,
}

/// A face a text asked for that isn't installed or loaded yet.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FontRequest {
    pub(crate) family: String,
    pub(crate) weight: u16,
    pub(crate) italic: bool,
}

impl FontRequest {
    fn attributes(&self) -> Attributes {
        Attributes {
            weight: fontique::FontWeight::new(f32::from(self.weight)),
            style: if self.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            ..Attributes::default()
        }
    }
}

pub(crate) enum FontPreview {
    Webp(Vec<u8>),
    System(Box<Font>),
}

impl Fonts {
    pub(crate) fn new() -> Self {
        Self {
            system: OnceLock::new(),
            catalog: OnceCell::new(),
            // A shard keeps nothing heavier than its share of the bytes, so
            // one shard holds them all and the largest font stays.
            bundled: quick_cache::sync::Cache::with_options(
                quick_cache::OptionsBuilder::new()
                    .estimated_items_capacity(16)
                    .weight_capacity(64 << 20)
                    .shards(1)
                    .build()
                    .expect("both capacities are set"),
                FontBytes,
                Default::default(),
                Default::default(),
            ),
            unavailable: Mutex::new(HashSet::new()),
        }
    }

    /// The installed fonts, scanned on first use.
    fn system(&self) -> &Arc<Mutex<FontSystem>> {
        self.system
            .get_or_init(|| Arc::new(Mutex::new(FontSystem::new())))
    }

    pub(crate) async fn families(&self) -> Result<Vec<FontFamily>> {
        let system = self.system().clone();
        let catalog = self.catalog().await?;
        let mut families = BTreeMap::<String, FontFamily>::new();

        let faces = tokio::task::spawn_blocking(move || system.lock().faces()).await?;
        for (name, face) in faces {
            let key = normalize(&name);
            let family = families.entry(key).or_insert_with(|| FontFamily {
                name,
                metadata: FontMetadata::default(),
                sources: vec![FontSource::System],
                faces: Vec::new(),
            });
            family.faces.push(face);
        }

        for bundled in &catalog.families {
            let key = normalize(&bundled.name);
            let family = families.entry(key).or_insert_with(|| bundled.clone());
            if family.metadata.primary_script.is_none() {
                family.metadata.clone_from(&bundled.metadata);
            }
            if !family.sources.contains(&FontSource::Bundled) {
                family.sources.push(FontSource::Bundled);
            }
            let mut known = family
                .faces
                .iter()
                .map(|face| normalize(&face.post_script_name))
                .collect::<HashSet<_>>();
            family.faces.extend(
                bundled
                    .faces
                    .iter()
                    .filter(|face| known.insert(normalize(&face.post_script_name)))
                    .cloned(),
            );
        }

        let mut families = families.into_values().collect::<Vec<_>>();
        for family in &mut families {
            family.faces.sort_by(|left, right| {
                left.style
                    .cmp(&right.style)
                    .then_with(|| left.weight.cmp(&right.weight))
                    .then_with(|| left.post_script_name.cmp(&right.post_script_name))
            });
            family.sources.sort_unstable();
        }
        Ok(families)
    }

    /// The font chain for `text` in `style`: its family, then the
    /// renderer's families, then Arial, then the platform's fallback for the
    /// text. A family that is neither installed nor loaded comes back as a
    /// request, and the chain falls back until `load` brings it in.
    pub(crate) fn resolve(
        &self,
        style: &CharStyle,
        renderer_families: &[String],
        text: &str,
        language: &LanguageIdentifier,
    ) -> Result<(Vec<Font>, Option<FontRequest>)> {
        let preferred = style
            .family
            .as_deref()
            .filter(|family| !family.trim().is_empty());
        let mut families: Vec<String> = Vec::with_capacity(renderer_families.len() + 2);
        for family in preferred
            .into_iter()
            .chain(renderer_families.iter().map(String::as_str))
            .chain(["Arial"])
        {
            if !families
                .iter()
                .any(|known| known.eq_ignore_ascii_case(family))
            {
                families.push(family.to_owned());
            }
        }
        let request = FontRequest {
            family: families[0].clone(),
            weight: style.weight.unwrap_or(400),
            italic: style.italic.unwrap_or(false),
        };
        let attributes = request.attributes();
        let (bundled, missing) = self.bundled_font(request);
        let mut resolved = self.system().lock().chain(
            &families,
            SYMBOL_FALLBACK_FAMILIES,
            attributes,
            text,
            (*language != LanguageIdentifier::UNKNOWN)
                .then(|| Language::parse(&language.to_string()).ok())
                .flatten(),
        )?;
        if let Some(font) = bundled {
            resolved.retain(|candidate| !candidate.same(&font));
            resolved.insert(0, font);
        }
        Ok((resolved, missing))
    }

    /// For a family that isn't installed, its bundled face when it's loaded,
    /// else `request` to load it, unless the catalog lacks it.
    fn bundled_font(&self, request: FontRequest) -> (Option<Font>, Option<FontRequest>) {
        let family = &request.family;
        if self
            .system()
            .lock()
            .collection
            .family_by_name(family)
            .is_some()
        {
            return (None, None);
        }
        let font = self
            .catalog
            .get()
            .and_then(|catalog| catalog.select(family, request.attributes()))
            .and_then(|face| self.bundled.get(face.post_script_name.as_str()));
        let unavailable = self.unavailable.lock().contains(&normalize(family));
        match font {
            None if !unavailable => (None, Some(request)),
            font => (font, None),
        }
    }

    pub(crate) async fn preview(&self, family_name: &str) -> Result<FontPreview> {
        let system = self.system().clone();
        let catalog = self.catalog().await?;
        if let Some(face) = catalog.select(family_name, Attributes::default()) {
            let [path] = FONTS.resolve([face.preview_file.as_str()]).await?;
            return Ok(FontPreview::Webp(fs_err::tokio::read(&path).await?));
        }
        tokio::task::spawn_blocking({
            let family_name = family_name.to_owned();
            move || {
                system
                    .lock()
                    .query_family(&family_name)
                    .map(Box::new)
                    .map(FontPreview::System)
            }
        })
        .await?
    }
}
