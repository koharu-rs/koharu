//! The bundled catalog, from the `mayocream/fonts` dataset: which faces it
//! has, and each face downloaded on first use.

use std::{collections::HashMap, ops::RangeInclusive, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use fontique::{
    Attributes, Blob, Collection, CollectionOptions, FontInfoOverride, QueryFamily, QueryStatus,
    SourceCache,
};
use jsonptr::{Pointer, PointerBuf};
use koharu_runtime::Repository;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Font, FontRequest, Fonts};
use crate::types::{FontFace, FontFamily, FontMetadata, FontSource, FontStyle as PublicFontStyle};

// The catalog and every referenced face or preview must come from one immutable snapshot.
pub(super) const FONTS: Repository = Repository::dataset(
    "mayocream/fonts",
    "304901a868bebd5f31f95664a462e5839873d939",
);

impl Fonts {
    /// Downloads the bundled faces `requests` name. A family the catalog
    /// doesn't have is remembered, so it isn't requested again.
    pub(crate) async fn load(&self, requests: &[FontRequest]) -> Result<()> {
        if requests.is_empty() {
            return Ok(());
        }
        let catalog = self.catalog().await?;
        for request in requests {
            match catalog.select(&request.family, request.attributes()) {
                Some(face) => {
                    self.load_face(face).await?;
                }
                None => {
                    self.unavailable.lock().insert(normalize(&request.family));
                }
            }
        }
        Ok(())
    }

    pub(super) async fn catalog(&self) -> Result<&Arc<FontCatalog>> {
        self.catalog
            .get_or_try_init(|| async {
                let [path] = FONTS.resolve(["index.json"]).await?;
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("failed to read {}", path.display()))?;
                tokio::task::spawn_blocking(move || {
                    let index: Value =
                        serde_json::from_slice(&bytes).context("invalid bundled font index")?;
                    FontCatalog::from_index(&index).map(Arc::new)
                })
                .await?
            })
            .await
    }

    /// Loads a face once: a second caller waits for the first.
    async fn load_face(&self, face: Arc<BundledFace>) -> Result<Font> {
        self.bundled
            .get_or_insert_async(face.post_script_name.as_str(), async {
                let [path] = FONTS.resolve([face.file_name.as_str()]).await?;
                let data = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let face = face.clone();
                tokio::task::spawn_blocking(move || font_from_blob(Blob::from(data), &face)).await?
            })
            .await
    }
}

fn font_from_blob(blob: Blob<u8>, face: &BundledFace) -> Result<Font> {
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
    query.set_attributes(Attributes {
        weight: fontique::FontWeight::new(f32::from(face.weight)),
        style: face.style.into(),
        ..Attributes::default()
    });
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

pub(super) struct FontCatalog {
    /// The families as the font list shows them.
    pub(super) families: Vec<FontFamily>,
    /// Each family's faces, by its normalized name.
    faces: HashMap<String, Vec<Arc<BundledFace>>>,
}

impl FontCatalog {
    /// The catalog an index of schema 3 describes.
    fn from_index(index: &Value) -> Result<Self> {
        let index = Field {
            value: index,
            at: PointerBuf::root(),
        };
        let schema: u32 = index.required("/schema_version")?;
        ensure!(
            schema == 3,
            "unsupported bundled font index schema {schema}"
        );
        let mut catalog = Self {
            families: Vec::new(),
            faces: HashMap::new(),
        };
        for family in index.field("/families")?.items()? {
            let (listed, faces) = Self::family(&family)?;
            catalog.faces.insert(normalize(&listed.name), faces);
            catalog.families.push(listed);
        }
        Ok(catalog)
    }

    /// A family of the index as the font list shows it, and its faces.
    fn family(family: &Field) -> Result<(FontFamily, Vec<Arc<BundledFace>>)> {
        let family_name: String = family.required("/family_name")?;
        let mut faces = Vec::new();
        for weight in family.field("/weights")?.items()? {
            let value: u16 = weight.required("/weight")?;
            for font in weight.field("/fonts")?.items()? {
                let width: f32 = font.required("/width")?;
                faces.push(Arc::new(BundledFace {
                    family_name: family_name.clone(),
                    file_name: font.required("/file_name")?,
                    post_script_name: font.required("/postscript_name")?,
                    weight: value,
                    weight_range: font.range("/weight_range", 1.0)?,
                    stretch: (width * 100.0).round().clamp(1.0, 1000.0) as u16,
                    stretch_range: font.range("/width_range", 100.0)?,
                    style: font.required("/style")?,
                    preview_file: font.required("/preview/file")?,
                }));
            }
        }
        let use_cases = match family.get("/use_cases")? {
            Some(list) => list
                .items()?
                .iter()
                .map(|use_case| use_case.required("/name"))
                .collect::<Result<_>>()?,
            None => Vec::new(),
        };
        let listed = FontFamily {
            name: family_name,
            metadata: FontMetadata {
                primary_script: family.optional("/primary_script")?,
                scripts: family.optional("/scripts")?.unwrap_or_default(),
                languages: family.optional("/languages")?.unwrap_or_default(),
                category: family.optional("/category")?,
                classifications: family.optional("/classifications")?.unwrap_or_default(),
                use_cases,
            },
            sources: vec![FontSource::Bundled],
            faces: faces
                .iter()
                .map(|face| FontFace {
                    post_script_name: face.post_script_name.clone(),
                    weight: face.weight,
                    weight_range: face.weight_range.clone(),
                    style: face.style,
                })
                .collect(),
        };
        Ok((listed, faces))
    }

    pub(super) fn select(&self, name: &str, attributes: Attributes) -> Option<Arc<BundledFace>> {
        self.faces
            .get(&normalize(name))?
            .iter()
            .min_by_key(|face| {
                let style = if fontique::FontStyle::from(face.style) == attributes.style {
                    0
                } else {
                    2_000
                };
                let weight = axis_distance(
                    attributes.weight.value(),
                    face.weight,
                    face.weight_range.as_ref(),
                );
                let stretch = axis_distance(
                    attributes.width.percentage(),
                    face.stretch,
                    face.stretch_range.as_ref(),
                );
                style + weight + stretch
            })
            .cloned()
    }
}

/// A value in the index and the pointer to it, which every error names.
struct Field<'a> {
    value: &'a Value,
    at: PointerBuf,
}

impl<'a> Field<'a> {
    /// The value at `pointer` below this one; `None` when it's missing or
    /// null.
    fn get(&self, pointer: &'static str) -> Result<Option<Self>> {
        let pointer = Pointer::from_static(pointer);
        match pointer.resolve(self.value) {
            Ok(Value::Null) => Ok(None),
            Ok(value) => Ok(Some(Self {
                value,
                at: self.at.concat(pointer),
            })),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("can't read {} in the bundled font index", self.at)),
        }
    }

    fn field(&self, pointer: &'static str) -> Result<Self> {
        self.get(pointer)?
            .with_context(|| format!("the bundled font index has no {}{pointer}", self.at))
    }

    fn parse<T: DeserializeOwned>(&self) -> Result<T> {
        T::deserialize(self.value)
            .with_context(|| format!("invalid {} in the bundled font index", self.at))
    }

    fn required<T: DeserializeOwned>(&self, pointer: &'static str) -> Result<T> {
        self.field(pointer)?.parse()
    }

    fn optional<T: DeserializeOwned>(&self, pointer: &'static str) -> Result<Option<T>> {
        self.get(pointer)?.map(|field| field.parse()).transpose()
    }

    /// The items of this list.
    fn items(&self) -> Result<Vec<Self>> {
        let Value::Array(items) = self.value else {
            bail!("{} in the bundled font index isn't a list", self.at);
        };
        Ok(items
            .iter()
            .enumerate()
            .map(|(index, value)| Self {
                value,
                at: self.at.with_trailing_token(index),
            })
            .collect())
    }

    /// The `minimum` to `maximum` range at `pointer`, its ends times `scale`
    /// as whole numbers from 1 to 1000.
    fn range(&self, pointer: &'static str, scale: f32) -> Result<Option<RangeInclusive<u16>>> {
        let Some(range) = self.get(pointer)? else {
            return Ok(None);
        };
        let end = |pointer| -> Result<u16> {
            let value: f32 = range.required(pointer)?;
            Ok((value * scale).round().clamp(1.0, 1000.0) as u16)
        };
        Ok(Some(end("/minimum")?..=end("/maximum")?))
    }
}

pub(super) struct BundledFace {
    family_name: String,
    file_name: String,
    pub(super) post_script_name: String,
    weight: u16,
    weight_range: Option<RangeInclusive<u16>>,
    stretch: u16,
    stretch_range: Option<RangeInclusive<u16>>,
    style: PublicFontStyle,
    pub(super) preview_file: String,
}

#[derive(Clone)]
pub(super) struct FontBytes;

impl quick_cache::Weighter<String, Font> for FontBytes {
    fn weight(&self, _: &String, font: &Font) -> u64 {
        font.data().len() as u64
    }
}

impl From<PublicFontStyle> for fontique::FontStyle {
    fn from(value: PublicFontStyle) -> Self {
        match value {
            PublicFontStyle::Normal => Self::Normal,
            PublicFontStyle::Italic => Self::Italic,
            PublicFontStyle::Oblique => Self::Oblique(None),
        }
    }
}

pub(super) fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn axis_distance(value: f32, default: u16, range: Option<&RangeInclusive<u16>>) -> u32 {
    let distance = match range {
        Some(range) if value < f32::from(*range.start()) => f32::from(*range.start()) - value,
        Some(range) if value > f32::from(*range.end()) => value - f32::from(*range.end()),
        Some(_) => 0.0,
        None => (f32::from(default) - value).abs(),
    };
    distance.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_filter_metadata_preview_and_weight_range() -> Result<()> {
        let index = serde_json::from_str::<Value>(
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
        )?;

        let catalog = FontCatalog::from_index(&index)?;
        let family = &catalog.families[0];
        assert_eq!(family.name, "Example Sans");
        assert_eq!(family.metadata.primary_script.as_deref(), Some("latn"));
        assert_eq!(family.metadata.scripts, ["latn"]);
        assert_eq!(family.metadata.category.as_deref(), Some("SANS_SERIF"));
        assert_eq!(family.metadata.classifications, ["sans-serif", "comic"]);
        assert_eq!(family.metadata.use_cases, ["comic-dialogue"]);
        let face = &catalog.faces["example sans"][0];
        assert_eq!(face.preview_file, "previews/ExampleSans.ttf.webp");
        assert_eq!(face.weight_range, Some(300..=700));
        assert_eq!(face.stretch_range, Some(75..=100));
        Ok(())
    }
}
