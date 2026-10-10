//! System discovery, lazy bundled-font loading, fallback, and bounded byte caching.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt,
    sync::{Arc, LazyLock, OnceLock},
};

use anyhow::{Context, Result, bail};
use fontique::{
    Attributes, Blob, Collection, CollectionOptions, FallbackKey, FontInfoOverride, FontStyle,
    GenericFamily, Language, QueryFamily, QueryFont, QueryStatus, Script, SourceCache,
};
use harfrust::{
    ShapeOptions, ShapePlan, ShapePlanKey, ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};
use koharu_runtime::HuggingFaceFile;
use parking_lot::Mutex;
use serde::Deserialize;
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
    string::StringId,
};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::{
    script::fontique_scripts,
    types::{
        FontFace, FontFamily, FontMetadata, FontRange, FontSource, FontStyle as PublicFontStyle,
    },
};

const FONT_REPOSITORY: &str = "mayocream/fonts";
// The catalog and every referenced face or preview must come from one immutable snapshot.
const FONT_REVISION: &str = "304901a868bebd5f31f95664a462e5839873d939";
const FONT_INDEX: &str = "index.json";
const FONT_INDEX_SCHEMA: u32 = 3;
const DEFAULT_FONT_CACHE_BYTES: usize = 64 * 1024 * 1024;
const SHAPE_PLAN_CACHE_CAPACITY: usize = 8;
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

fn symbol_fallback_families() -> &'static [String] {
    static FAMILIES: LazyLock<Vec<String>> = LazyLock::new(|| {
        SYMBOL_FALLBACK_FAMILIES
            .iter()
            .map(|family| (*family).to_owned())
            .collect()
    });
    &FAMILIES
}

/// A resolved face and variable-font instance used by shaping, metrics, and drawing.
#[derive(Clone)]
pub struct Font {
    data: Blob<u8>,
    bytes: Arc<OnceLock<Arc<[u8]>>>,
    index: u32,
    family_name: String,
    font_name: String,
    post_script_name: String,
    synthesis: fontique::Synthesis,
    shaper_data: Arc<ShaperData>,
    shaper_instance: ShaperInstance,
    shape_plans: Arc<Mutex<VecDeque<Arc<ShapePlan>>>>,
    normalized_coords: Vec<skrifa::instance::NormalizedCoord>,
    vello_coords: Vec<vello::NormalizedCoord>,
}

impl fmt::Debug for Font {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Font")
            .field("index", &self.index)
            .field("family_name", &self.family_name)
            .field("font_name", &self.font_name)
            .field("post_script_name", &self.post_script_name)
            .field("synthesis", &self.synthesis)
            .finish_non_exhaustive()
    }
}

impl Font {
    fn from_query(font: QueryFont, family_name: String) -> Result<Self> {
        let variations = font
            .synthesis
            .variation_settings()
            .iter()
            .map(|(tag, value)| Variation {
                tag: harfrust::Tag::new(&tag.to_be_bytes()),
                value: *value,
            })
            .collect::<Vec<_>>();
        let harfrust_font = harfrust::FontRef::from_index(font.blob.as_ref(), font.index)
            .context("failed to create HarfRust font reference")?;
        let shaper_data = Arc::new(ShaperData::new(&harfrust_font));
        let shaper_instance = ShaperInstance::from_variations(&harfrust_font, &variations);
        let skrifa_font = skrifa::FontRef::from_index(font.blob.as_ref(), font.index)
            .context("failed to create Skrifa font reference")?;
        let location = skrifa_font
            .axes()
            .location(variations.iter().map(|variation| {
                (
                    skrifa::Tag::new(&variation.tag.into_bytes()),
                    variation.value,
                )
            }));
        let normalized_coords = location.coords().to_vec();
        let vello_coords = normalized_coords
            .iter()
            .map(|coord| coord.to_bits())
            .collect();
        let font_name = skrifa_font
            .localized_strings(StringId::new(4))
            .english_or_first()
            .map(|name| name.to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| family_name.clone());
        let post_script_name = skrifa_font
            .localized_strings(StringId::new(6))
            .english_or_first()
            .map(|name| name.to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| family_name.clone());

        Ok(Self {
            data: font.blob,
            bytes: Arc::new(OnceLock::new()),
            index: font.index,
            family_name,
            font_name,
            post_script_name,
            synthesis: font.synthesis,
            shaper_data,
            shaper_instance,
            shape_plans: Arc::new(Mutex::new(VecDeque::new())),
            normalized_coords,
            vello_coords,
        })
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        self.bytes
            .get_or_init(|| Arc::from(self.data.as_ref()))
            .as_ref()
    }

    pub(crate) fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(self.bytes.get_or_init(|| Arc::from(self.data.as_ref())))
    }

    pub(crate) const fn index(&self) -> u32 {
        self.index
    }

    pub(crate) fn normalized_coords(&self) -> &[vello::NormalizedCoord] {
        &self.vello_coords
    }

    pub(crate) fn location(&self) -> LocationRef<'_> {
        LocationRef::new(&self.normalized_coords)
    }

    pub(crate) fn shaper_instance(&self) -> &ShaperInstance {
        &self.shaper_instance
    }

    pub(crate) fn shaper_data(&self) -> &ShaperData {
        &self.shaper_data
    }

    pub(crate) fn shape_plan(
        &self,
        font: &harfrust::FontRef<'_>,
        direction: harfrust::Direction,
        script: Option<harfrust::Script>,
        language: Option<&harfrust::Language>,
        features: &[harfrust::Feature],
    ) -> Arc<ShapePlan> {
        let key = ShapePlanKey::new(script, direction)
            .language(language)
            .instance(Some(&self.shaper_instance))
            .features(features);
        let mut plans = self.shape_plans.lock();
        if let Some(index) = plans.iter().position(|plan| key.matches(plan)) {
            let plan = plans.remove(index).expect("shape plan index must exist");
            plans.push_back(plan.clone());
            return plan;
        }

        let shaper = self
            .shaper_data
            .shaper(font)
            .instance(Some(&self.shaper_instance))
            .build();
        let plan = Arc::new(ShapePlan::new(
            &shaper, direction, script, language, features,
        ));
        if plans.len() == SHAPE_PLAN_CACHE_CAPACITY {
            plans.pop_front();
        }
        plans.push_back(plan.clone());
        plan
    }

    pub(crate) fn synthetic_bold(&self) -> bool {
        self.synthesis.embolden()
    }

    pub(crate) fn synthetic_skew(&self) -> Option<f32> {
        self.synthesis.skew()
    }

    pub(crate) fn skrifa_ref(&self) -> Result<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(self.data.as_ref(), self.index)
            .context("failed to create Skrifa font reference")
    }

    pub(crate) fn harfrust_ref(&self) -> Result<harfrust::FontRef<'_>> {
        harfrust::FontRef::from_index(self.data.as_ref(), self.index)
            .context("failed to create HarfRust font reference")
    }

    #[cfg(test)]
    pub fn has_glyph(&self, character: char) -> bool {
        self.skrifa_ref()
            .is_ok_and(|font| font.charmap().map(character).is_some())
    }

    pub(crate) fn covers(&self, text: &str) -> bool {
        self.skrifa_ref().is_ok_and(|font| {
            let charmap = font.charmap();
            text.chars()
                .filter(|character| {
                    !character.is_control()
                        && !character.is_whitespace()
                        && !is_default_ignorable(*character)
                })
                .all(|character| charmap.map(character).is_some())
        })
    }

    pub(crate) fn renders(&self, text: &str, font_size: f32) -> bool {
        let Ok(font) = self.skrifa_ref() else {
            return false;
        };
        if font.charmap().is_symbol() {
            return false;
        }
        let Ok(harfrust_font) = self.harfrust_ref() else {
            return false;
        };
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = self
            .shaper_data
            .shaper(&harfrust_font)
            .instance(Some(&self.shaper_instance))
            .build()
            .shape(buffer, ShapeOptions::new());
        let outlines = font.outline_glyphs();
        let size = Size::new(font_size);
        let metrics = font.glyph_metrics(size, self.location());
        let mut rendered = false;

        for glyph in shaped.glyph_infos() {
            let character = text
                .get(glyph.cluster as usize..)
                .and_then(|tail| tail.chars().next());
            if character.is_some_and(|character| {
                character.is_control()
                    || character.is_whitespace()
                    || is_default_ignorable(character)
            }) {
                continue;
            }
            let glyph_id = skrifa::GlyphId::new(glyph.glyph_id);
            let Some(bounds) = metrics.bounds(glyph_id) else {
                return false;
            };
            let Some(outline) = outlines.get(glyph_id) else {
                return false;
            };
            let mut pen = OutlinePresence::default();
            if glyph.glyph_id == 0
                || bounds.x_min >= bounds.x_max
                || bounds.y_min >= bounds.y_max
                || outline
                    .draw(DrawSettings::unhinted(size, self.location()), &mut pen)
                    .is_err()
                || !pen.has_ink
            {
                return false;
            }
            rendered = true;
        }

        rendered
    }

    pub fn family_name(&self) -> &str {
        &self.family_name
    }

    pub fn post_script_name(&self) -> &str {
        &self.post_script_name
    }

    /// Original font bytes for consumers that need a rasterizer other than Vello.
    pub fn data(&self) -> &[u8] {
        self.data.as_ref()
    }
}

#[derive(Default)]
struct OutlinePresence {
    has_ink: bool,
}

impl OutlinePen for OutlinePresence {
    fn move_to(&mut self, _x: f32, _y: f32) {}

    fn line_to(&mut self, _x: f32, _y: f32) {
        self.has_ink = true;
    }

    fn quad_to(&mut self, _cx0: f32, _cy0: f32, _x: f32, _y: f32) {
        self.has_ink = true;
    }

    fn curve_to(&mut self, _cx0: f32, _cy0: f32, _cx1: f32, _cy1: f32, _x: f32, _y: f32) {
        self.has_ink = true;
    }

    fn close(&mut self) {}
}

fn is_default_ignorable(character: char) -> bool {
    matches!(
        character as u32,
        0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0020..=0xE007F | 0xE0100..=0xE01EF
    )
}

pub(crate) fn font_key(font: &Font) -> usize {
    font as *const Font as usize
}

#[derive(Clone, Debug)]
struct SystemFace {
    pub(crate) family_name: String,
    pub(crate) post_script_name: String,
    pub(crate) weight: u16,
    pub(crate) style: PublicFontStyle,
}

/// Font discovery, matching, fallback, and source caching.
pub struct FontSystem {
    collection: Collection,
    sources: SourceCache,
    system_families: Vec<String>,
    system_faces: Option<Vec<SystemFace>>,
    resolved: HashMap<ResolveKey, Vec<Font>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ResolveKey {
    families: Vec<String>,
    fallback_families: Vec<String>,
    width: u32,
    weight: u32,
    style: (u8, u32),
    scripts: Vec<[u8; 4]>,
    language: Option<String>,
}

impl FontSystem {
    #[must_use]
    pub fn new() -> Self {
        let mut collection = Collection::new(CollectionOptions::default());
        let system_families = collection.family_names().map(str::to_owned).collect();
        Self {
            collection,
            sources: SourceCache::new_shared(),
            system_families,
            system_faces: None,
            resolved: HashMap::new(),
        }
    }

    /// Resolves an ordered font chain for the requested families, attributes, and scripts.
    pub fn resolve(
        &mut self,
        families: &[String],
        attributes: Attributes,
        scripts: &[Script],
        language: Option<&str>,
    ) -> Result<Vec<Font>> {
        self.resolve_with_fallback_families(families, &[], attributes, scripts, language)
    }

    pub(crate) fn resolve_with_fallback_families(
        &mut self,
        families: &[String],
        fallback_families: &[String],
        attributes: Attributes,
        scripts: &[Script],
        language: Option<&str>,
    ) -> Result<Vec<Font>> {
        let language = language.and_then(|tag| Language::parse(tag).ok());
        let scripts = if scripts.is_empty() {
            &[Script::from_bytes(*b"Latn")][..]
        } else {
            scripts
        };
        let style = match attributes.style {
            FontStyle::Normal => (0, 0),
            FontStyle::Italic => (1, 0),
            FontStyle::Oblique(angle) => (2, angle.unwrap_or(14.0).to_bits()),
        };
        let key = ResolveKey {
            families: families.to_vec(),
            fallback_families: fallback_families.to_vec(),
            width: attributes.width.percentage().to_bits(),
            weight: attributes.weight.value().to_bits(),
            style,
            scripts: scripts.iter().map(|script| script.to_bytes()).collect(),
            language: language
                .as_ref()
                .map(|language| language.as_str().to_owned()),
        };
        if let Some(fonts) = self.resolved.get(&key) {
            return Ok(fonts.clone());
        }
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();

        for &script in scripts {
            let mut candidates = Vec::new();
            {
                let mut query = self.collection.query(&mut self.sources);
                if families.is_empty() {
                    query.set_families([QueryFamily::Generic(GenericFamily::SansSerif)]);
                } else {
                    query.set_families(
                        families
                            .iter()
                            .map(|family| QueryFamily::Named(family.as_str())),
                    );
                }
                query.set_attributes(attributes);
                query.set_fallbacks(FallbackKey::new(script, language.as_ref()));
                query.matches_with(|font| {
                    candidates.push(font.clone());
                    QueryStatus::Continue
                });
            }
            // Explicit symbol families are a final safety net, not body-font
            // candidates. Query them separately so a missing requested family
            // still falls back to the platform's script-appropriate text font.
            if fallback_families.iter().any(|fallback| {
                !families
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(fallback))
            }) {
                let mut query = self.collection.query(&mut self.sources);
                query.set_families(
                    fallback_families
                        .iter()
                        .filter(|fallback| {
                            !families
                                .iter()
                                .any(|family| family.eq_ignore_ascii_case(fallback))
                        })
                        .map(|family| QueryFamily::Named(family.as_str())),
                );
                query.set_attributes(attributes);
                query.matches_with(|font| {
                    candidates.push(font.clone());
                    QueryStatus::Continue
                });
            }

            for candidate in candidates {
                if !seen.insert((candidate.blob.id(), candidate.index)) {
                    continue;
                }
                let family_name = self
                    .collection
                    .family_name(candidate.family.0)
                    .unwrap_or("Unknown")
                    .to_owned();
                resolved.push(Font::from_query(candidate, family_name)?);
            }
        }

        if resolved.is_empty() && !families.is_empty() {
            resolved = self.resolve_with_fallback_families(
                &[],
                fallback_families,
                attributes,
                scripts,
                language.as_ref().map(Language::as_str),
            )?;
        }
        if resolved.is_empty() {
            bail!("no usable fonts are installed");
        }
        if self.resolved.len() >= 256 {
            self.resolved.clear();
        }
        self.resolved.insert(key, resolved.clone());
        Ok(resolved)
    }

    pub fn query_family(&mut self, family: &str) -> Result<Font> {
        self.resolve(
            &[family.to_owned()],
            Attributes::default(),
            &[Script::from_bytes(*b"Latn")],
            None,
        )?
        .into_iter()
        .find(|font| font.family_name().eq_ignore_ascii_case(family))
        .with_context(|| format!("no font found for family {family:?}"))
    }

    #[cfg(test)]
    pub fn first_font(&mut self) -> Result<Font> {
        self.resolve(
            &[],
            Attributes::default(),
            &[Script::from_bytes(*b"Latn")],
            None,
        )?
        .into_iter()
        .next()
        .context("no usable fonts are installed")
    }

    fn system_faces(&mut self) -> Vec<SystemFace> {
        if let Some(faces) = &self.system_faces {
            return faces.clone();
        }
        let faces = self.faces_for_families(self.system_families.clone());
        self.system_faces = Some(faces.clone());
        faces
    }

    fn faces_for_families(
        &mut self,
        families: impl IntoIterator<Item = String>,
    ) -> Vec<SystemFace> {
        let mut faces = Vec::new();
        for family_name in families {
            let Some(family) = self.collection.family_by_name(&family_name) else {
                continue;
            };
            for info in family.fonts() {
                let Some(blob) = info.load(Some(&mut self.sources)) else {
                    continue;
                };
                let query = QueryFont {
                    family: (family.id(), 0),
                    blob,
                    index: info.index(),
                    synthesis: fontique::Synthesis::default(),
                    charmap_index: info.charmap_index(),
                };
                let Ok(font) = Font::from_query(query, family_name.clone()) else {
                    continue;
                };
                faces.push(SystemFace {
                    family_name: family_name.clone(),
                    post_script_name: font.post_script_name,
                    weight: info.weight().value().round().clamp(1.0, 1000.0) as u16,
                    style: match info.style() {
                        FontStyle::Normal => PublicFontStyle::Normal,
                        FontStyle::Italic => PublicFontStyle::Italic,
                        FontStyle::Oblique(_) => PublicFontStyle::Oblique,
                    },
                });
            }
        }
        faces
    }
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct Fonts {
    system: OnceCell<Arc<Mutex<FontSystem>>>,
    catalog: OnceCell<Arc<FontCatalog>>,
    cache: Mutex<FontCache>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FontRequest {
    pub(crate) family: String,
    pub(crate) weight: Option<u16>,
    pub(crate) style: Option<PublicFontStyle>,
}

pub(crate) enum FontPreview {
    Webp(Vec<u8>),
    System(Box<Font>),
}

impl Fonts {
    pub(crate) fn new() -> Self {
        Self {
            system: OnceCell::new(),
            catalog: OnceCell::new(),
            cache: Mutex::new(FontCache::new(DEFAULT_FONT_CACHE_BYTES)),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_system_initialized(&self) -> bool {
        self.system.get().is_some()
    }

    pub(crate) async fn families(&self) -> Result<Vec<FontFamily>> {
        let system = self.ensure_system().await;
        let catalog = self.catalog().await?;
        let mut families = BTreeMap::<String, FontFamily>::new();

        let faces = tokio_rayon::spawn(move || system.lock().system_faces()).await;
        for face in faces {
            let key = normalize(&face.family_name);
            let family = families.entry(key).or_insert_with(|| FontFamily {
                name: face.family_name.clone(),
                metadata: FontMetadata::default(),
                sources: vec![FontSource::System],
                faces: Vec::new(),
            });
            family.faces.push(FontFace {
                post_script_name: face.post_script_name,
                weight: face.weight,
                weight_range: None,
                style: face.style,
            });
        }

        for bundled in &catalog.families {
            let key = normalize(&bundled.family_name);
            let family = families.entry(key).or_insert_with(|| bundled.info());
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
                    .map(|face| face.info()),
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

    pub(crate) fn resolve(
        &self,
        preferred_font: Option<&str>,
        font_weight: Option<u16>,
        font_style: Option<PublicFontStyle>,
        theme_families: &[String],
        text: &str,
        language: Option<&str>,
    ) -> Result<Vec<Font>> {
        let mut families = Vec::with_capacity(theme_families.len() + 2);
        if let Some(preferred) = preferred_font.filter(|value| !value.trim().is_empty()) {
            families.push(preferred.to_owned());
        }
        for family in theme_families {
            if !families
                .iter()
                .any(|value| value.eq_ignore_ascii_case(family))
            {
                families.push(family.clone());
            }
        }
        if !families
            .iter()
            .any(|family| family.eq_ignore_ascii_case("Arial"))
        {
            families.push("Arial".to_owned());
        }

        let mut attributes = Attributes::default();
        if let Some(weight) = font_weight {
            attributes.weight = fontique::FontWeight::new(f32::from(weight));
        }
        if let Some(style) = font_style {
            attributes.style = match style {
                PublicFontStyle::Normal => FontStyle::Normal,
                PublicFontStyle::Italic => FontStyle::Italic,
                PublicFontStyle::Oblique => FontStyle::Oblique(None),
            };
        }

        let system = self
            .system
            .get()
            .context("font system was not prepared before shaping")?;
        let bundled = if let Some(family) = families
            .first()
            .filter(|family| system.lock().collection.family_by_name(family).is_none())
            && let Some(catalog) = self.catalog.get()
        {
            if let Some(face) = catalog.select(family, attributes) {
                self.cache.lock().get(&face.post_script_name)
            } else {
                None
            }
        } else {
            None
        };

        let mut resolved = system.lock().resolve_with_fallback_families(
            &families,
            symbol_fallback_families(),
            attributes,
            &fontique_scripts(text),
            language,
        )?;
        if let Some(font) = bundled {
            resolved.retain(|candidate| {
                candidate.data.id() != font.data.id() || candidate.index != font.index
            });
            resolved.insert(0, font);
        }
        Ok(resolved)
    }

    pub(crate) async fn preview(&self, family_name: &str) -> Result<FontPreview> {
        let system = self.ensure_system().await;
        let catalog = self.catalog().await?;
        if let Some(face) = catalog.select(family_name, Attributes::default()) {
            let path =
                HuggingFaceFile::pinned_dataset(FONT_REPOSITORY, FONT_REVISION, &face.preview_file)
                    .resolve()
                    .await?;
            return tokio::fs::read(&path)
                .await
                .with_context(|| format!("failed to read {}", path.display()))
                .map(FontPreview::Webp);
        }
        tokio_rayon::spawn({
            let family_name = family_name.to_owned();
            move || {
                system
                    .lock()
                    .query_family(&family_name)
                    .map(Box::new)
                    .map(FontPreview::System)
            }
        })
        .await
    }

    pub(crate) async fn prepare(&self, requests: &[FontRequest]) -> Result<()> {
        if requests.is_empty() {
            return Ok(());
        }
        let system = self.ensure_system().await;
        let missing = {
            let mut system = system.lock();
            requests
                .iter()
                .filter(|request| system.collection.family_by_name(&request.family).is_none())
                .cloned()
                .collect::<Vec<_>>()
        };
        if missing.is_empty() {
            return Ok(());
        }
        let catalog = self.catalog().await?;
        for request in missing {
            let style = match request.style.unwrap_or(PublicFontStyle::Normal) {
                PublicFontStyle::Normal => FontStyle::Normal,
                PublicFontStyle::Italic => FontStyle::Italic,
                PublicFontStyle::Oblique => FontStyle::Oblique(None),
            };
            let attributes = Attributes {
                weight: fontique::FontWeight::new(f32::from(request.weight.unwrap_or(400))),
                style,
                ..Attributes::default()
            };
            if let Some(face) = catalog.select(&request.family, attributes) {
                self.load(face).await?;
            }
        }
        Ok(())
    }

    async fn ensure_system(&self) -> Arc<Mutex<FontSystem>> {
        self.system
            .get_or_init(|| tokio_rayon::spawn(|| Arc::new(Mutex::new(FontSystem::new()))))
            .await
            .clone()
    }

    async fn catalog(&self) -> Result<&Arc<FontCatalog>> {
        self.catalog
            .get_or_try_init(|| async {
                let path =
                    HuggingFaceFile::pinned_dataset(FONT_REPOSITORY, FONT_REVISION, FONT_INDEX)
                        .resolve()
                        .await?;
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("failed to read {}", path.display()))?;
                tokio_rayon::spawn(move || {
                    let index: CatalogIndex =
                        serde_json::from_slice(&bytes).context("invalid bundled font index")?;
                    if index.schema_version != FONT_INDEX_SCHEMA {
                        bail!(
                            "unsupported bundled font index schema {}",
                            index.schema_version
                        );
                    }
                    Ok(Arc::new(FontCatalog::from_index(index)))
                })
                .await
            })
            .await
    }

    async fn load(&self, face: Arc<BundledFace>) -> Result<Font> {
        if let Some(font) = self.cache.lock().get(&face.post_script_name) {
            return Ok(font);
        }
        let _load = face.load.lock().await;
        if let Some(font) = self.cache.lock().get(&face.post_script_name) {
            return Ok(font);
        }
        let path = HuggingFaceFile::pinned_dataset(FONT_REPOSITORY, FONT_REVISION, &face.file_name)
            .resolve()
            .await?;
        let data = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        let face_for_worker = face.clone();
        let font = tokio_rayon::spawn(move || {
            font_from_blob(
                Blob::from(data),
                &face_for_worker,
                Attributes {
                    weight: fontique::FontWeight::new(f32::from(face_for_worker.weight)),
                    style: face_for_worker.style.into(),
                    ..Attributes::default()
                },
            )
        })
        .await?;
        self.cache
            .lock()
            .insert(face.post_script_name.clone(), font.clone());
        Ok(font)
    }
}

fn font_from_blob(blob: Blob<u8>, face: &BundledFace, attributes: Attributes) -> Result<Font> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    // The catalog owns the public family name. CJK fonts often expose a different
    // localized name in their internal records, so register the face under the
    // catalog name before querying it.
    if collection
        .register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(&face.family_name),
                ..FontInfoOverride::default()
            }),
        )
        .is_empty()
    {
        bail!("font file {:?} contained no usable faces", face.file_name);
    }
    let mut sources = SourceCache::new(Default::default());
    let mut candidates = Vec::new();
    let mut query = collection.query(&mut sources);
    query.set_families([QueryFamily::Named(&face.family_name)]);
    query.set_attributes(attributes);
    query.matches_with(|font| {
        candidates.push(font.clone());
        QueryStatus::Continue
    });
    let mut fallback = None;
    for candidate in candidates {
        let font = Font::from_query(candidate, face.family_name.clone())?;
        if font
            .post_script_name()
            .eq_ignore_ascii_case(&face.post_script_name)
        {
            return Ok(font);
        }
        fallback.get_or_insert(font);
    }
    fallback.with_context(|| format!("could not load bundled font {:?}", face.family_name))
}

struct FontCatalog {
    families: Vec<Arc<BundledFamily>>,
    family_names: HashMap<String, Arc<BundledFamily>>,
}

impl FontCatalog {
    fn from_index(index: CatalogIndex) -> Self {
        let mut families = Vec::new();
        let mut family_names = HashMap::new();
        for family in index.families {
            let mut faces = Vec::new();
            for weight in family.weights {
                for font in weight.fonts {
                    let face = Arc::new(BundledFace {
                        family_name: family.family_name.clone(),
                        file_name: font.file_name,
                        post_script_name: font.postscript_name,
                        weight: weight.weight,
                        weight_range: font.weight_range.map(FontRange::weight),
                        stretch: (font.width * 100.0).round().clamp(1.0, 1000.0) as u16,
                        stretch_range: font.width_range.map(FontRange::width),
                        style: font.style,
                        preview_file: font.preview.file,
                        load: AsyncMutex::new(()),
                    });
                    faces.push(face);
                }
            }
            let family = Arc::new(BundledFamily {
                family_name: family.family_name,
                metadata: FontMetadata {
                    primary_script: family.primary_script,
                    scripts: family.scripts,
                    languages: family.languages,
                    category: family.category,
                    classifications: family.classifications,
                    use_cases: family
                        .use_cases
                        .into_iter()
                        .map(|value| value.name)
                        .collect(),
                },
                faces,
            });
            family_names.insert(normalize(&family.family_name), family.clone());
            families.push(family);
        }
        Self {
            families,
            family_names,
        }
    }

    fn select(&self, name: &str, attributes: Attributes) -> Option<Arc<BundledFace>> {
        self.family_names
            .get(&normalize(name))?
            .faces
            .iter()
            .min_by_key(|face| {
                let style = if fontique::FontStyle::from(face.style) == attributes.style {
                    0
                } else {
                    2_000
                };
                let weight =
                    axis_distance(attributes.weight.value(), face.weight, face.weight_range);
                let stretch = axis_distance(
                    attributes.width.percentage(),
                    face.stretch,
                    face.stretch_range,
                );
                style + weight + stretch
            })
            .cloned()
    }
}

struct BundledFamily {
    family_name: String,
    metadata: FontMetadata,
    faces: Vec<Arc<BundledFace>>,
}

impl BundledFamily {
    fn info(&self) -> FontFamily {
        FontFamily {
            name: self.family_name.clone(),
            metadata: self.metadata.clone(),
            sources: vec![FontSource::Bundled],
            faces: self.faces.iter().map(|face| face.info()).collect(),
        }
    }
}

struct BundledFace {
    family_name: String,
    file_name: String,
    post_script_name: String,
    weight: u16,
    weight_range: Option<FontRange>,
    stretch: u16,
    stretch_range: Option<FontRange>,
    style: CatalogStyle,
    preview_file: String,
    load: AsyncMutex<()>,
}

impl BundledFace {
    fn info(&self) -> FontFace {
        FontFace {
            post_script_name: self.post_script_name.clone(),
            weight: self.weight,
            weight_range: self.weight_range,
            style: self.style.into(),
        }
    }
}

struct FontCache {
    entries: HashMap<String, CachedFont>,
    max_bytes: usize,
    bytes: usize,
    clock: u64,
}

impl FontCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_bytes,
            bytes: 0,
            clock: 0,
        }
    }

    fn get(&mut self, key: &str) -> Option<Font> {
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.clock;
        Some(entry.font.clone())
    }

    fn insert(&mut self, key: String, font: Font) {
        let data_bytes = font.data().len();
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        if self.max_bytes == 0 || data_bytes > self.max_bytes {
            return;
        }
        while self.bytes.saturating_add(data_bytes) > self.max_bytes {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.bytes += data_bytes;
        self.entries.insert(
            key,
            CachedFont {
                font,
                bytes: data_bytes,
                last_used: self.clock,
            },
        );
    }
}

struct CachedFont {
    font: Font,
    bytes: usize,
    last_used: u64,
}

#[derive(Deserialize)]
struct CatalogIndex {
    schema_version: u32,
    families: Vec<CatalogFamily>,
}

#[derive(Deserialize)]
struct CatalogFamily {
    family_name: String,
    primary_script: Option<String>,
    #[serde(default)]
    scripts: Vec<String>,
    #[serde(default)]
    languages: Vec<String>,
    category: Option<String>,
    #[serde(default)]
    classifications: Vec<String>,
    #[serde(default)]
    use_cases: Vec<CatalogUseCase>,
    weights: Vec<CatalogWeight>,
}

#[derive(Deserialize)]
struct CatalogUseCase {
    name: String,
}

#[derive(Deserialize)]
struct CatalogWeight {
    weight: u16,
    fonts: Vec<CatalogFont>,
}

#[derive(Deserialize)]
struct CatalogFont {
    file_name: String,
    postscript_name: String,
    style: CatalogStyle,
    width: f32,
    weight_range: Option<CatalogRange>,
    width_range: Option<CatalogRange>,
    preview: CatalogPreview,
}

#[derive(Deserialize)]
struct CatalogPreview {
    file: String,
}

#[derive(Clone, Copy, Deserialize)]
struct CatalogRange {
    minimum: f32,
    maximum: f32,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum CatalogStyle {
    Normal,
    Italic,
    Oblique,
}

impl From<CatalogStyle> for PublicFontStyle {
    fn from(value: CatalogStyle) -> Self {
        match value {
            CatalogStyle::Normal => Self::Normal,
            CatalogStyle::Italic => Self::Italic,
            CatalogStyle::Oblique => Self::Oblique,
        }
    }
}

impl From<CatalogStyle> for fontique::FontStyle {
    fn from(value: CatalogStyle) -> Self {
        match value {
            CatalogStyle::Normal => Self::Normal,
            CatalogStyle::Italic => Self::Italic,
            CatalogStyle::Oblique => Self::Oblique(None),
        }
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

impl FontRange {
    fn weight(range: CatalogRange) -> Self {
        Self {
            minimum: range.minimum.round().clamp(1.0, 1000.0) as u16,
            maximum: range.maximum.round().clamp(1.0, 1000.0) as u16,
        }
    }

    fn width(range: CatalogRange) -> Self {
        Self {
            minimum: (range.minimum * 100.0).round().clamp(1.0, 1000.0) as u16,
            maximum: (range.maximum * 100.0).round().clamp(1.0, 1000.0) as u16,
        }
    }
}

fn axis_distance(value: f32, default: u16, range: Option<FontRange>) -> u32 {
    let distance = match range {
        Some(range) if value < f32::from(range.minimum) => f32::from(range.minimum) - value,
        Some(range) if value > f32::from(range.maximum) => value - f32::from(range.maximum),
        Some(_) => 0.0,
        None => (f32::from(default) - value).abs(),
    };
    distance.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_filter_metadata_preview_and_weight_range() {
        let index = serde_json::from_str::<CatalogIndex>(
            r#"{
                "schema_version": 3,
                "families": [{
                    "family_name": "Example Sans",
                    "primary_script": "latn",
                    "scripts": ["latn"],
                    "primary_language": "en",
                    "languages": ["en"],
                    "category": "SANS_SERIF",
                    "classifications": ["sans-serif", "comic"],
                    "use_cases": [{ "name": "comic-dialogue" }],
                    "weights": [{
                        "weight": 400,
                        "fonts": [{
                            "file_name": "ExampleSans.ttf",
                            "font_name": "Example Sans Regular",
                            "postscript_name": "ExampleSans-Regular",
                            "style": "normal",
                            "width": 1.0,
                            "weight_range": { "minimum": 300, "maximum": 700 },
                            "width_range": { "minimum": 0.75, "maximum": 1.0 },
                            "preview": { "file": "previews/ExampleSans.ttf.webp" }
                        }]
                    }]
                }]
            }"#,
        )
        .expect("the index fixture must deserialize");

        let catalog = FontCatalog::from_index(index);
        let family = &catalog.families[0];
        assert_eq!(family.family_name, "Example Sans");
        assert_eq!(family.metadata.primary_script.as_deref(), Some("latn"));
        assert_eq!(family.metadata.scripts, ["latn"]);
        assert_eq!(family.metadata.category.as_deref(), Some("SANS_SERIF"));
        assert_eq!(family.metadata.classifications, ["sans-serif", "comic"]);
        assert_eq!(family.metadata.use_cases, ["comic-dialogue"]);
        let face = &family.faces[0];
        assert_eq!(face.preview_file, "previews/ExampleSans.ttf.webp");
        assert_eq!(
            face.weight_range,
            Some(FontRange {
                minimum: 300,
                maximum: 700
            })
        );
        assert_eq!(
            face.stretch_range,
            Some(FontRange {
                minimum: 75,
                maximum: 100
            })
        );
    }
}
