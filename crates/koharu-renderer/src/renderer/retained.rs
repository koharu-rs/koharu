//! Retained content: pixels, shapes and text built from the scene, kept by
//! what changes them, so a render after an edit rebuilds only what it
//! touched.

use std::sync::{Arc, atomic::Ordering};

use koharu_scene::{
    Content as SceneContent, FillRule, Hash, LayerId, PageId, Project, Session, Shape, Text,
};
use vello::{
    Scene,
    kurbo::{Affine, Cap, Join, Shape as _, Stroke},
    peniko::{Color, Fill},
};

use super::Renderer;
use crate::{
    Error, Result,
    frame::{Retained, Source, Vector, parse},
    images, text,
    tiles::{Tiled, assemble},
};

pub(super) type Cache = quick_cache::sync::Cache<Key, Retained, Weight>;

/// What retained content may take, in bytes: pixels count with their tiles;
/// a vector scene counts as a kilobyte.
const CACHE_BYTES: u64 = 2 << 30;

/// What retained content is keyed by: only what changes it. Texts and shapes
/// key on the hash of their canonical JSON (RFC 8785), so the key stays the
/// same for equal values whatever order a map's keys come in.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum Key {
    /// A text's hash and the font generation.
    Text(Hash, u64),
    Shape(Hash),
    Image(Hash),
    Tiles(Hash),
}

#[derive(Clone)]
pub(super) struct Weight;

impl quick_cache::Weighter<Key, Retained> for Weight {
    fn weight(&self, _: &Key, retained: &Retained) -> u64 {
        match retained {
            Retained::Pixels(Some(tiled)) => tiled.bytes(),
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
        .map_err(|error| Error::invalid(format!("a value has no canonical JSON: {error}")))?;
    Ok(Hash::of(&bytes))
}

impl Renderer {
    /// Content, retained by what it depends on.
    pub(super) fn content(&self, session: &Session, source: Source) -> Result<Retained> {
        let key = match source {
            Source::Text(layer, text) => return self.text_content(layer, text),
            Source::Shape(shape) => Key::Shape(canonical(shape)?),
            Source::Image(file) => Key::Image(file),
            Source::Tiles(layer) => Key::Tiles(session.tiles_hash(layer)?),
        };
        if let Some(retained) = self.0.cache.get(&key) {
            return Ok(retained);
        }
        let retained = match source {
            Source::Tiles(layer) => {
                Retained::Pixels(assemble(session.tiles(layer, None)?)?.map(Arc::new))
            }
            Source::Image(file) => {
                let decoded = images::decode(&file, &session.file(&file)?)?;
                Retained::Pixels(Some(Arc::new(Tiled::new((0, 0), decoded))))
            }
            Source::Shape(shape) => shape_vector(shape)?,
            Source::Text(..) => unreachable!("texts return above"),
        };
        self.0.cache.insert(key, retained.clone());
        Ok(retained)
    }

    pub(super) fn text_content(&self, id: LayerId, layer: &Text) -> Result<Retained> {
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
        let laid_out = canonical(&(content, style, frame, direction, language))?;
        let key = Key::Text(laid_out, self.0.generation.load(Ordering::Relaxed));
        if let Some(retained) = self.0.cache.get(&key) {
            return Ok(retained);
        }
        let retained = self.text(id, layer)?;
        self.0.cache.insert(key, retained.clone());
        Ok(retained)
    }

    fn text(&self, id: LayerId, layer: &Text) -> Result<Retained> {
        let families = self.0.config.read().font_families.clone();
        let mut missing = Vec::new();
        let layout = text::layout(layer, |style, characters| {
            let (fonts, request) =
                self.0
                    .fonts
                    .resolve(style, &families, characters, &layer.language)?;
            missing.extend(request);
            Ok(fonts)
        })
        .map_err(|source| Error::Text { layer: id, source })?;
        Ok(match layout {
            Some(layout) => Retained::Vector {
                vector: Arc::new(text::paint(&layout)),
                text: Some(Arc::new(layout.resolved)),
                missing,
            },
            None => Retained::Vector {
                vector: Arc::new(Vector {
                    scene: Scene::new(),
                    strokes: None,
                    bounds: vello::kurbo::Rect::ZERO,
                }),
                text: None,
                missing,
            },
        })
    }
}

/// Everything a page's content is built from: its background, and each
/// layer's pixels, image, shape, or text.
pub(super) fn sources(project: &Project, page: PageId) -> Result<Vec<Source<'_>>> {
    let mut out: Vec<Source> = project
        .page(page)?
        .background
        .map(|background| Source::Image(background.file))
        .into_iter()
        .collect();
    for layer in project.layers_of(page)? {
        match &layer.content {
            SceneContent::Raster(_) | SceneContent::Mask(_) => {
                out.push(Source::Tiles(layer.id));
            }
            SceneContent::Image(image) => out.push(Source::Image(image.file)),
            SceneContent::Shape(shape) => out.push(Source::Shape(shape)),
            SceneContent::Text(text) => out.push(Source::Text(layer.id, text)),
            SceneContent::Block(block) => {
                out.push(Source::Tiles(block.cleanup.id));
                out.push(Source::Tiles(block.mask.id));
                if let Some(text) = &block.translation {
                    out.push(Source::Text(layer.id, text));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A shape layer's fill and stroke.
fn shape_vector(shape: &Shape) -> Result<Retained> {
    let path = parse(shape.path.as_str())?;
    let mut scene = Scene::new();
    let fill = match shape.fill_rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    };
    if let Some(color) = shape.fill {
        scene.fill(fill, Affine::IDENTITY, Color::from(color), None, &path);
    }
    let mut bounds = path.bounding_box();
    if let Some(stroke) = &shape.stroke {
        // Centred on the path, with miter joins and butt caps.
        let style = Stroke::new(stroke.width)
            .with_join(Join::Miter)
            .with_caps(Cap::Butt);
        scene.stroke(
            &style,
            Affine::IDENTITY,
            Color::from(stroke.color),
            None,
            &path,
        );
        bounds = bounds.inflate(stroke.width * 2.0, stroke.width * 2.0);
    }
    Ok(Retained::Vector {
        vector: Arc::new(Vector {
            scene,
            strokes: None,
            bounds,
        }),
        text: None,
        missing: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RendererConfig;

    #[test]
    fn a_layer_as_heavy_as_the_whole_budget_fits_one_shard() {
        let renderer = Renderer::new(RendererConfig::default());
        assert_eq!(renderer.0.cache.shard_capacity(), CACHE_BYTES);
    }
}
