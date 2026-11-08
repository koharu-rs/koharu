//! One face as a variable-font instance: its character map, metrics, HarfRust
//! font and shape plans, and its variation coordinates.

use std::{fmt, sync::Arc};

use anyhow::{Context, Result, ensure};
use fontique::{Blob, CharmapIndex, QueryFont, Synthesis};
use harfrust::{Direction, Feature, Language, Script, ShapePlan, Tag, font::Variation};
use skrifa::{
    GlyphId, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
    string::StringId,
};
use smallvec::SmallVec;

/// A resolved face and variable-font instance used by shaping, metrics, and drawing.
#[derive(Clone)]
pub(crate) struct Font {
    data: Blob<u8>,
    index: u32,
    family_name: String,
    post_script_name: String,
    synthesis: Synthesis,
    charmap: CharmapIndex,
    shaper: harfrust::Font,
    /// Shape plans, shared by the font's clones.
    plans: Arc<quick_cache::sync::Cache<PlanKey, Arc<ShapePlan>>>,
    /// The instance's normalized variation coordinates, as vello takes them.
    coords: Vec<vello::NormalizedCoord>,
    units_per_em: f32,
    /// Above and below the baseline, both positive, in font units at the
    /// instance.
    ascent: f32,
    descent: f32,
}

/// What HarfRust's `ShapePlanKey::matches` compares but the font's own
/// variations, which are fixed for a `Font`. `Feature` can't be compared
/// whole, so each is its tag, its value, and whether it covers all the text,
/// which is all `matches` compares of it. Text asks for two features at most,
/// so a key stays off the heap.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PlanKey {
    direction: Direction,
    script: Option<Script>,
    language: Option<Language>,
    features: SmallVec<[(Tag, u32, bool); 2]>,
}

impl fmt::Debug for Font {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Font")
            .field("index", &self.index)
            .field("family_name", &self.family_name)
            .field("post_script_name", &self.post_script_name)
            .field("synthesis", &self.synthesis)
            .finish_non_exhaustive()
    }
}

impl Font {
    pub(super) fn from_query(font: QueryFont, family_name: String) -> Result<Self> {
        let variations = font
            .synthesis
            .variation_settings()
            .iter()
            .map(|(tag, value)| Variation {
                tag: Tag::new(&tag.to_be_bytes()),
                value: *value,
            })
            .collect::<Vec<_>>();
        let shaper = harfrust::Font::new(font.blob.clone().into_raw_parts().0, font.index)
            .context("failed to create HarfRust font")?
            .instance_builder()
            .variations(&variations)
            .build();
        let face = skrifa::FontRef::from_index(font.blob.as_ref(), font.index)
            .context("failed to create Skrifa font reference")?;
        let location = face.axes().location(variations.iter().map(|variation| {
            (
                skrifa::Tag::new(&variation.tag.into_bytes()),
                variation.value,
            )
        }));
        // In font units: scaled by the size over units per em when used, as
        // skrifa scales them.
        let metrics = face.metrics(Size::unscaled(), &location);
        ensure!(metrics.units_per_em > 0, "the font has no units per em");
        let post_script_name = post_script_name(&face).unwrap_or_else(|| family_name.clone());
        Ok(Self {
            data: font.blob,
            index: font.index,
            family_name,
            post_script_name,
            synthesis: font.synthesis,
            charmap: font.charmap_index,
            shaper,
            plans: Arc::new(quick_cache::sync::Cache::new(64)),
            coords: location
                .coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect(),
            units_per_em: f32::from(metrics.units_per_em),
            ascent: metrics.ascent,
            descent: -metrics.descent,
        })
    }

    /// Whether both are the same face of the same file.
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.data.id() == other.data.id() && self.index == other.index
    }

    /// Whether this is the instance `query` asks for.
    pub(super) fn is(&self, query: &QueryFont) -> bool {
        self.data.id() == query.blob.id()
            && self.index == query.index
            && self.synthesis == query.synthesis
    }

    pub(crate) fn font_data(&self) -> vello::peniko::FontData {
        vello::peniko::FontData::new(self.data.clone(), self.index)
    }

    pub(crate) fn normalized_coords(&self) -> &[vello::NormalizedCoord] {
        &self.coords
    }

    pub(crate) fn shaper(&self) -> &harfrust::Font {
        &self.shaper
    }

    /// From font units to pixels at `size`.
    pub(crate) fn scale(&self, size: f32) -> f32 {
        size / self.units_per_em
    }

    /// The ascent and descent at `size`, both positive.
    pub(crate) fn extent(&self, size: f32) -> (f32, f32) {
        let scale = self.scale(size);
        (self.ascent * scale, self.descent * scale)
    }

    /// The plan HarfRust shapes with these settings by, made the first time
    /// they're asked for.
    pub(crate) fn shape_plan(
        &self,
        direction: Direction,
        script: Option<Script>,
        language: Option<&Language>,
        features: &[Feature],
    ) -> Arc<ShapePlan> {
        let key = PlanKey {
            direction,
            script,
            language: language.cloned(),
            features: features
                .iter()
                .map(|feature| {
                    let global = feature.start == Feature::GLOBAL_START
                        && feature.end == Feature::GLOBAL_END;
                    (feature.tag, feature.value, global)
                })
                .collect(),
        };
        if let Some(plan) = self.plans.get(&key) {
            return plan;
        }
        let plan = Arc::new(ShapePlan::new(
            &self.shaper,
            direction,
            script,
            language,
            features,
        ));
        self.plans.insert(key, plan.clone());
        plan
    }

    pub(crate) fn synthetic_bold(&self) -> bool {
        self.synthesis.embolden()
    }

    pub(crate) fn synthetic_skew(&self) -> Option<f32> {
        self.synthesis.skew()
    }

    #[cfg(test)]
    pub(crate) fn has_glyph(&self, character: char) -> bool {
        self.covers(character.encode_utf8(&mut [0; 4]))
    }

    /// Whether the face has a glyph for every visible character of `text`.
    pub(crate) fn covers(&self, text: &str) -> bool {
        self.charmap
            .charmap(self.data.as_ref())
            .is_some_and(|charmap| {
                text.chars()
                    .filter(|character| visible(*character))
                    .all(|character| charmap.map(character).is_some_and(|glyph| glyph != 0))
            })
    }

    /// Whether the face draws `text` itself, for a preview of its name: it
    /// isn't a symbol font, and each visible character maps to a glyph with
    /// an outline.
    pub(crate) fn renders(&self, text: &str) -> bool {
        let Ok(face) = skrifa::FontRef::from_index(self.data.as_ref(), self.index) else {
            return false;
        };
        let charmap = face.charmap();
        let coords: Vec<NormalizedCoord> = self
            .coords
            .iter()
            .map(|bits| NormalizedCoord::from_bits(*bits))
            .collect();
        let metrics = face.glyph_metrics(Size::unscaled(), LocationRef::new(&coords));
        let mut characters = text
            .chars()
            .filter(|character| visible(*character))
            .peekable();
        !charmap.is_symbol()
            && characters.peek().is_some()
            && characters.all(|character| {
                charmap
                    .map(character)
                    .filter(|glyph| *glyph != GlyphId::NOTDEF)
                    .and_then(|glyph| metrics.bounds(glyph))
                    .is_some_and(|bounds| {
                        bounds.x_min < bounds.x_max && bounds.y_min < bounds.y_max
                    })
            })
    }

    pub(crate) fn family_name(&self) -> &str {
        &self.family_name
    }

    pub(crate) fn post_script_name(&self) -> &str {
        &self.post_script_name
    }

    pub(crate) fn data(&self) -> &[u8] {
        self.data.as_ref()
    }
}

/// Whether a character draws: not a control, white space, or a default
/// ignorable such as a joiner or variation selector.
fn visible(character: char) -> bool {
    !character.is_control()
        && !character.is_whitespace()
        && !matches!(
            character as u32,
            0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0020..=0xE007F | 0xE0100..=0xE01EF
        )
}

/// The face's PostScript name, when it has one.
pub(super) fn post_script_name(font: &skrifa::FontRef<'_>) -> Option<String> {
    font.localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
}
