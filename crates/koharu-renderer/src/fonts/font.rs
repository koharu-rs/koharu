//! A font: one face as a variable-font instance, with its character map,
//! metrics, HarfRust font and shape plans, and variation coordinates.

use std::{collections::HashMap, fmt, ops::Range, sync::Arc};

use anyhow::{Context, Result, ensure};
use fontique::{CharmapIndex, QueryFont, Synthesis};
use harfrust::{
    Buffer, Direction, Feature, Language, Script, ShapeOptions, ShapePlan, ShaperFont, Tag,
    font::{Blob, Variation},
};
use icu_properties::{CodePointSetData, props::DefaultIgnorableCodePoint};
use skrifa::{
    GlyphId, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
    string::StringId,
};
use smallvec::SmallVec;
use vello_gpu::peniko::FontData;

/// A face at the instance fontique matched, as text shapes, measures, and
/// draws it.
#[derive(Clone)]
pub(crate) struct Font {
    /// The face's file and its index in it.
    pub data: FontData,
    pub family_name: String,
    pub post_script_name: String,
    /// The instance of a variable face, and the slant fontique fakes for a
    /// face without one.
    pub synthesis: Synthesis,
    /// Drawn bolder than its face, for a text that asks for bold of one that
    /// isn't.
    pub faux_bold: bool,
    charmap: CharmapIndex,
    pub shaper: harfrust::Font,
    /// Shape plans, shared by the font's clones.
    plans: Arc<quick_cache::sync::Cache<PlanKey, Arc<ShapePlan>>>,
    /// Whether it has a vertical form for a character, as it was asked.
    vertical_forms: Arc<quick_cache::sync::Cache<char, bool>>,
    /// The instance's normalized variation coordinates, as glyph runs take
    /// them.
    pub coords: Vec<glifo::NormalizedCoord>,
    units_per_em: f32,
    /// Above and below the baseline, both positive, in font units at the
    /// instance.
    ascent: f32,
    descent: f32,
    /// How far above the baseline the centre of the ideographic em box is,
    /// in font units: OpenType has a CJK face for vertical text set the box
    /// as its typo ascender and descender, which a face's ascent and descent,
    /// spaced for lines, can lie far outside.
    em_centre: f32,
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

/// A table of a face: its range of the face's file.
struct Table(Arc<dyn AsRef<[u8]> + Send + Sync>, Range<usize>);

impl AsRef<[u8]> for Table {
    fn as_ref(&self) -> &[u8] {
        &(*self.0).as_ref()[self.1.clone()]
    }
}

impl fmt::Debug for Font {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Font")
            .field("index", &self.data.index)
            .field("family_name", &self.family_name)
            .field("post_script_name", &self.post_script_name)
            .field("synthesis", &self.synthesis)
            .field("faux_bold", &self.faux_bold)
            .finish_non_exhaustive()
    }
}

impl Font {
    /// The face fontique `found`, of family `family_name`, drawn bolder when
    /// `faux_bold` says so.
    pub(super) fn new(found: QueryFont, family_name: String, faux_bold: bool) -> Result<Self> {
        let variations = found
            .synthesis
            .variation_settings()
            .iter()
            .map(|(tag, value)| Variation {
                tag: Tag::new(&tag.to_be_bytes()),
                value: *value,
            })
            .collect::<Vec<_>>();
        let face = skrifa::FontRef::from_index(found.blob.as_ref(), found.index)
            .context("failed to create Skrifa font reference")?;
        // Shaped with OpenType layout alone, as Photoshop and DirectWrite
        // shape: a face's Apple `morx` or `mort` is left out. A Chinese face's
        // one `morx` subtable, horizontal and on by default, swapped in the
        // vertical forms of `…` and `《` in horizontal text.
        let apple = [b"morx", b"mort"].map(skrifa::Tag::new);
        let bytes = found.blob.clone().into_raw_parts().0;
        let source: harfrust::font::Source =
            match apple.iter().any(|tag| face.table_data(*tag).is_some()) {
                false => bytes.into(),
                true => {
                    let tables: HashMap<Tag, Range<usize>> = (face.table_directory.table_records())
                        .iter()
                        .filter(|record| !apple.contains(&record.tag()))
                        .map(|record| {
                            let start = record.offset() as usize;
                            let tag = Tag::new(&record.tag().into_bytes());
                            (tag, start..start + record.length() as usize)
                        })
                        .collect();
                    let table: Arc<dyn Fn(Tag) -> Option<Blob> + Send + Sync> =
                        Arc::new(move |tag| {
                            let range = tables.get(&tag)?.clone();
                            let table: Arc<dyn AsRef<[u8]> + Send + Sync> =
                                Arc::new(Table(bytes.clone(), range));
                            Some(Blob::from(table))
                        });
                    table.into()
                }
            };
        let shaper = harfrust::Font::new(source, found.index)
            .context("failed to create HarfRust font")?
            .instance_builder()
            .variations(&variations)
            .build();
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
        let em_centre = face
            .os2()
            .ok()
            .map(|os2| (os2.s_typo_ascender(), os2.s_typo_descender()))
            .filter(|&typo| typo != (0, 0))
            .map_or(metrics.ascent + metrics.descent, |(ascent, descent)| {
                f32::from(ascent) + f32::from(descent)
            })
            * 0.5;
        Ok(Self {
            data: FontData::new(found.blob, found.index),
            family_name,
            post_script_name,
            synthesis: found.synthesis,
            faux_bold,
            charmap: found.charmap_index,
            shaper,
            plans: Arc::new(quick_cache::sync::Cache::new(64)),
            vertical_forms: Arc::new(quick_cache::sync::Cache::new(256)),
            coords: location
                .coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect(),
            units_per_em: f32::from(metrics.units_per_em),
            ascent: metrics.ascent,
            descent: -metrics.descent,
            em_centre,
        })
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

    /// How far above the baseline the centre of the ideographic em box is
    /// at `size`.
    pub(crate) fn em_centre(&self, size: f32) -> f32 {
        self.em_centre * self.scale(size)
    }

    /// How far a faux bold grows each side of a glyph at `size`: CC Wild
    /// Words' own bold is twice this thicker and as much wider.
    pub(crate) fn growth(&self, size: f32) -> f32 {
        if self.faux_bold { size * 0.02 } else { 0.0 }
    }

    /// How far right a faux italic leans for each pixel up from the baseline.
    pub(crate) fn lean(&self) -> f32 {
        self.synthesis
            .skew()
            .map_or(0.0, |angle| angle.to_radians().tan())
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

    /// Whether the face sets `ch` in vertical text in a form of its own: its
    /// `vert` feature, or Unicode's vertical presentation forms
    /// without them, substitute another glyph for it, one that draws: a
    /// Chinese face draws `︽` and `︾` as nothing.
    pub(crate) fn has_vertical_form(&self, ch: char) -> bool {
        if let Some(known) = self.vertical_forms.get(&ch) {
            return known;
        }
        let horizontal = self
            .charmap
            .charmap(self.data.data.as_ref())
            .and_then(|charmap| charmap.map(ch));
        let features = [Feature::new(Tag::new(b"vert"), 1, ..)];
        let mut buffer = Buffer::new();
        buffer.push_str(ch.encode_utf8(&mut [0; 4]));
        buffer.guess_segment_properties();
        buffer.set_direction(Direction::TopToBottom);
        let plan = self.shape_plan(
            buffer.direction(),
            buffer.script(),
            buffer.language(),
            &features,
        );
        let shaped = harfrust::shape(
            &ShaperFont::new(&self.shaper),
            &mut buffer,
            ShapeOptions::new().plan(Some(&plan)).features(&features),
        );
        let vertical = shaped.is_ok()
            && buffer.glyph_infos().first().is_some_and(|info| {
                Some(info.glyph_id) != horizontal && self.ink(info.glyph_id, 1.0).is_some()
            });
        self.vertical_forms.insert(ch, vertical);
        vertical
    }

    /// Where `glyph` inks at `size`, from its outline's points, with y up
    /// from the baseline as the font has it; `None` for one that inks
    /// nothing.
    pub(crate) fn ink(&self, glyph: u32, size: f32) -> Option<kurbo::Rect> {
        /// The rectangle around every point an outline draws through.
        struct Ink(Option<kurbo::Rect>);
        impl Ink {
            fn take(&mut self, x: f32, y: f32) {
                let point = kurbo::Point::new(f64::from(x), f64::from(y));
                self.0 = Some(match self.0 {
                    Some(rect) => rect.union_pt(point),
                    None => kurbo::Rect::from_points(point, point),
                });
            }
        }
        impl OutlinePen for Ink {
            fn move_to(&mut self, x: f32, y: f32) {
                self.take(x, y);
            }
            fn line_to(&mut self, x: f32, y: f32) {
                self.take(x, y);
            }
            fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
                self.take(cx, cy);
                self.take(x, y);
            }
            fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
                self.take(cx0, cy0);
                self.take(cx1, cy1);
                self.take(x, y);
            }
            fn close(&mut self) {}
        }
        let face = skrifa::FontRef::from_index(self.data.data.as_ref(), self.data.index).ok()?;
        let coords: Vec<NormalizedCoord> = self
            .coords
            .iter()
            .map(|bits| NormalizedCoord::from_bits(*bits))
            .collect();
        let outline = face.outline_glyphs().get(GlyphId::new(glyph))?;
        let mut ink = Ink(None);
        outline
            .draw(
                DrawSettings::unhinted(Size::new(size), LocationRef::new(&coords)),
                &mut ink,
            )
            .ok()?;
        ink.0
            .filter(|rect| rect.width() > 0.0 || rect.height() > 0.0)
    }

    /// Whether the face has a glyph for every visible character of `text`.
    pub(crate) fn covers(&self, text: &str) -> bool {
        self.charmap
            .charmap(self.data.data.as_ref())
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
        let Ok(face) = skrifa::FontRef::from_index(self.data.data.as_ref(), self.data.index) else {
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
}

/// Whether a character draws: not a control, white space, or a default
/// ignorable such as a joiner or variation selector.
fn visible(character: char) -> bool {
    !character.is_control()
        && !character.is_whitespace()
        && !CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(character)
}

/// The PostScript name of the face `file` holds, when it has one.
pub(super) fn post_script_name(file: &skrifa::FontRef<'_>) -> Option<String> {
    file.localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
}
