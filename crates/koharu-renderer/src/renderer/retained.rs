//! Retained content: pixels, shapes and text built from the scene, kept by
//! what changes them, so a render after an edit rebuilds only what it
//! touched.

use std::{io::Cursor, sync::Arc};

use anyhow::{Context as _, Result, anyhow};
use image::ImageReader;
use koharu_scene::{FillRule, Hash, LayerId, Offset, Session, Shape, Text};
use kurbo::{BezPath, Cap, Join, Shape as _, Stroke};
use vello_gpu::peniko::{Color, Fill};

use super::Renderer;
use crate::{
    fonts,
    frame::{Content, ShapeVector},
    pixels::{Pixels, assemble, check_size},
    text,
};

/// Each source's content, by the hash of what it's built from: an image's
/// file, a picture's tiles, or a text's or shape's canonical JSON (RFC
/// 8785), which stays the same for equal values whatever order a map's keys
/// come in. No two kinds hash the same bytes: a file is an image, the tiles
/// hash binary, a text's JSON is an array, and a shape's an object. `None`
/// for a source that draws nothing.
pub(super) type Cache = quick_cache::sync::Cache<Hash, Option<Content>, Weight>;

/// What retained content may take, in bytes: pixels count with their levels;
/// a text's layout or a shape counts as a kilobyte.
const CACHE_BYTES: u64 = 2 << 30;

#[derive(Clone)]
pub(super) struct Weight;

impl quick_cache::Weighter<Hash, Option<Content>> for Weight {
    fn weight(&self, _: &Hash, content: &Option<Content>) -> u64 {
        match content {
            Some(Content::Pixels(pixels)) => pixels.bytes(),
            _ => 1024,
        }
    }
}

pub(super) fn cache() -> Cache {
    // A shard keeps nothing heavier than its share of the bytes, so one
    // shard holds them all and a large page's pixels stay.
    quick_cache::sync::Cache::with_options(
        quick_cache::OptionsBuilder::new()
            .estimated_items_capacity(1024)
            .weight_capacity(CACHE_BYTES)
            .shards(1)
            .build()
            .expect("both capacities are set"),
        Weight,
        Default::default(),
        Default::default(),
    )
}

fn canonical(value: &impl serde::Serialize) -> Result<Hash> {
    let bytes = serde_json_canonicalizer::to_vec(value)
        .map_err(|error| anyhow!("a value has no canonical JSON: {error}"))?;
    Ok(Hash::of(&bytes))
}

impl Renderer {
    /// What `key` holds, `build` building it when nothing does. Two renders
    /// may both build it: one waiting on the other's build inside rayon could
    /// deadlock.
    fn retain(
        &self,
        key: Hash,
        build: impl FnOnce() -> Result<Option<Content>>,
    ) -> Result<Option<Content>> {
        if let Some(retained) = self.0.cache.get(&key) {
            return Ok(retained);
        }
        let retained = build()?;
        self.0.cache.insert(key, retained.clone());
        Ok(retained)
    }

    /// Text layer `id`'s text laid out, falling back on the renderer's
    /// families; `None` when it has no characters.
    pub(super) fn text(&self, id: LayerId, layer: &Text) -> Result<Option<Content>> {
        // The transform places the layout when it's drawn, so moving a text
        // keeps its layout. Naming every field fails to build when one is
        // added, until the key says whether it counts.
        let Text {
            content,
            style,
            frame,
            direction,
            language,
            transform: _,
        } = layer;
        let families = self.0.config.read().font_families.clone();
        let key = canonical(&(content, style, frame, direction, language, &families))?;
        self.retain(key, || {
            let layout = text::layout(layer, |style, characters| {
                fonts::chain(style, &families, characters, language)
            })
            .with_context(|| format!("text layer {id} couldn't be laid out"))?;
            Ok(layout.map(|layout| Content::Text(Arc::new(layout))))
        })
    }

    /// A shape layer's path, fill, and stroke: centred on the path, with
    /// miter joins and butt caps.
    pub(super) fn shape(&self, shape: &Shape) -> Result<Option<Content>> {
        self.retain(canonical(shape)?, || {
            let path = BezPath::from(&shape.path);
            let mut bounds = path.bounding_box();
            let stroke = shape.stroke.as_ref().map(|stroke| {
                bounds = bounds.inflate(stroke.width * 2.0, stroke.width * 2.0);
                (
                    Stroke::new(stroke.width)
                        .with_join(Join::Miter)
                        .with_caps(Cap::Butt),
                    Color::from(stroke.color),
                )
            });
            Ok(Some(Content::Shape(Arc::new(ShapeVector {
                path,
                fill_rule: match shape.fill_rule {
                    FillRule::NonZero => Fill::NonZero,
                    FillRule::EvenOdd => Fill::EvenOdd,
                },
                fill: shape.fill.map(Color::from),
                stroke,
                bounds,
            }))))
        })
    }

    /// Imported file `file` decoded, its size checked before any pixel is.
    pub(super) fn image(&self, session: &Session, file: Hash) -> Result<Option<Content>> {
        self.retain(file, || {
            let bytes = session.file(&file)?;
            let unsupported = || format!("file {file} is not a supported image");
            let reader = || {
                ImageReader::new(Cursor::new(&bytes))
                    .with_guessed_format()
                    .with_context(unsupported)
            };
            let (width, height) = reader()?.into_dimensions().with_context(unsupported)?;
            check_size(width, height)?;
            let image = reader()?.decode().with_context(unsupported)?.into_rgba8();
            Ok(Some(Content::Pixels(Arc::new(Pixels::new(
                Offset::default(),
                image,
            )))))
        })
    }

    /// The picture of the layer or block part `layer`, cropped to what it
    /// paints; `None` when it paints nothing.
    pub(super) fn tiles(&self, session: &Session, layer: LayerId) -> Result<Option<Content>> {
        self.retain(session.tiles_hash(layer)?, || {
            Ok(assemble(session.tiles(layer, None)?)?
                .map(|pixels| Content::Pixels(Arc::new(pixels))))
        })
    }
}
