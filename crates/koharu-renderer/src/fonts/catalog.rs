//! The bundled catalog, the `mayocream/fonts` dataset: its families and their
//! faces, each face downloaded when a text first asks for it and registered
//! into the library, where it resolves as an installed face does.

use std::{collections::HashMap, ops::RangeInclusive, sync::LazyLock};

use anyhow::{Context, Result, anyhow};
use fontique::{Attributes, FontStyle};
use koharu_runtime::Repository;
use serde::Deserialize;

use super::{FontFace, FontFamily, library, normalize};

// The catalog and every referenced face or preview must come from one immutable snapshot.
const DATASET: Repository = Repository::dataset(
    "mayocream/fonts",
    "304901a868bebd5f31f95664a462e5839873d939",
);

/// Each family by its normalized name, from the dataset's index, downloaded
/// the first time. One that fails to load stays failed until Koharu starts
/// again.
static CATALOG: LazyLock<Result<HashMap<String, Family>>> =
    LazyLock::new(|| parse(&download("index.json")?));

/// The faces registered into the library, by PostScript name. A second
/// caller waits for the first's download, and nothing is ever evicted, so a
/// face registers once.
static REGISTERED: LazyLock<quick_cache::sync::Cache<String, ()>> =
    LazyLock::new(|| quick_cache::sync::Cache::new(1 << 16));

/// A family of the catalog, with at least one face.
pub(super) struct Family {
    name: String,
    faces: Vec<Face>,
}

struct Face {
    post_script_name: String,
    weight: u16,
    /// A variable face's weights.
    weight_range: Option<RangeInclusive<u16>>,
    style: FontStyle,
    /// A percentage of normal width.
    stretch: u16,
    stretch_range: Option<RangeInclusive<u16>>,
    /// The dataset's file of the face.
    file: String,
    /// The dataset's file of a WebP of the family's name set in the face.
    preview: String,
}

/// The catalog's families by normalized name, loaded on this thread unless
/// the preload has: an async caller reaches it through a blocking task.
pub(super) fn families() -> Result<&'static HashMap<String, Family>> {
    CATALOG
        .as_ref()
        .map_err(|error| anyhow!("the bundled font catalog didn't load: {error:#}"))
}

/// The family named `name`, in any case; `None` when the catalog lacks it.
pub(super) fn family(name: &str) -> Result<Option<&'static Family>> {
    Ok(families()?.get(&normalize(name)))
}

/// Registers `family`'s face nearest `attributes` into the library,
/// downloaded on this thread the first time.
pub(super) fn register(family: &Family, attributes: Attributes) -> Result<()> {
    let face = nearest(family, attributes);
    REGISTERED.get_or_insert_with(face.post_script_name.as_str(), || {
        let data = download(&face.file)?;
        library::with(|library| library.register(data, &family.name))
    })
}

/// A WebP of `family`'s name set in its regular face, downloaded on this
/// thread the first time.
pub(super) fn preview(family: &Family) -> Result<Vec<u8>> {
    download(&nearest(family, Attributes::default()).preview)
}

/// `family` as the font list shows it.
pub(super) fn listed(family: &Family) -> FontFamily {
    FontFamily {
        name: family.name.clone(),
        faces: family
            .faces
            .iter()
            .map(|face| FontFace {
                post_script_name: face.post_script_name.clone(),
                weight: face.weight,
                weight_range: face.weight_range.clone(),
                italic: face.style != FontStyle::Normal,
            })
            .collect(),
    }
}

/// `family`'s face nearest `attributes`, by CSS font matching's distances on
/// each axis, its style first.
fn nearest(family: &Family, attributes: Attributes) -> &Face {
    let axis = |value: f32, default: u16, range: Option<&RangeInclusive<u16>>| {
        let distance = match range {
            Some(range) if value < f32::from(*range.start()) => f32::from(*range.start()) - value,
            Some(range) if value > f32::from(*range.end()) => value - f32::from(*range.end()),
            Some(_) => 0.0,
            None => (f32::from(default) - value).abs(),
        };
        distance.round() as u32
    };
    family
        .faces
        .iter()
        .min_by_key(|face| {
            let style = match face.style == attributes.style {
                true => 0,
                false => 2_000,
            };
            style
                + axis(
                    attributes.weight.value(),
                    face.weight,
                    face.weight_range.as_ref(),
                )
                + axis(
                    attributes.width.percentage(),
                    face.stretch,
                    face.stretch_range.as_ref(),
                )
        })
        .expect("a catalog family has a face")
}

/// The families the dataset's `index` describes, by normalized name. A
/// family with no faces is left out.
fn parse(index: &[u8]) -> Result<HashMap<String, Family>> {
    // The index as far as the catalog reads it.
    #[derive(Deserialize)]
    struct Index {
        families: Vec<IndexFamily>,
    }
    #[derive(Deserialize)]
    struct IndexFamily {
        family_name: String,
        weights: Vec<IndexWeight>,
    }
    #[derive(Deserialize)]
    struct IndexWeight {
        weight: u16,
        fonts: Vec<IndexFace>,
    }
    #[derive(Deserialize)]
    struct IndexFace {
        file_name: String,
        postscript_name: String,
        /// As CSS's `font-style` spells it.
        style: String,
        /// A fraction of normal width.
        width: f32,
        weight_range: Option<IndexRange>,
        /// Fractions of normal width.
        width_range: Option<IndexRange>,
        preview: IndexFile,
    }
    #[derive(Deserialize)]
    struct IndexFile {
        file: String,
    }
    #[derive(Deserialize)]
    struct IndexRange {
        minimum: f32,
        maximum: f32,
    }
    // A CSS weight or width: a whole number from 1 to 1000.
    let axis = |value: f32| value.round().clamp(1.0, 1000.0) as u16;
    let scaled =
        |range: IndexRange, scale: f32| axis(range.minimum * scale)..=axis(range.maximum * scale);

    let index: Index =
        serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(index))
            .context("invalid bundled font index")?;
    let mut families = HashMap::new();
    for family in index.families {
        let mut faces = Vec::new();
        for weight in family.weights {
            for face in weight.fonts {
                faces.push(Face {
                    post_script_name: face.postscript_name,
                    weight: weight.weight,
                    weight_range: face.weight_range.map(|range| scaled(range, 1.0)),
                    style: FontStyle::parse_css(&face.style)
                        .with_context(|| format!("invalid font style {:?}", face.style))?,
                    stretch: axis(face.width * 100.0),
                    stretch_range: face.width_range.map(|range| scaled(range, 100.0)),
                    file: face.file_name,
                    preview: face.preview.file,
                });
            }
        }
        if !faces.is_empty() {
            families.insert(
                normalize(&family.family_name),
                Family {
                    name: family.family_name,
                    faces,
                },
            );
        }
    }
    Ok(families)
}

/// The runtime the dataset's downloads run on, for as long as the process.
/// The Hub's client is one for the process, and each connection it pools
/// is driven from the runtime that opened it, so a runtime made for one
/// download and dropped after it leaves the next download a dead connection.
/// Downloads come from render threads with no runtime of their own.
static DOWNLOADS: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("koharu-fonts")
        .enable_all()
        .build()
        .expect("the font downloads' runtime starts")
});

/// A file of the dataset, downloaded the first time.
fn download(file: &str) -> Result<Vec<u8>> {
    let [path] = DOWNLOADS.block_on(DATASET.resolve([file]))?;
    Ok(fs_err::read(path)?)
}
