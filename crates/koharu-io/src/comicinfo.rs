//! ComicInfo.xml 2.1 schema, `LocalizedField` model, and XML serialization.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use encoding_rs::Encoding;
use quick_xml::{de::from_str as from_xml_str, se::Serializer};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, IntoDeserializer},
};
use specta::Type;

/// Project metadata key storing [`ComicInfo`] in `.khr` `metadata(key, value)`.
pub const COMIC_INFO_KEY: &str = "comic_info";

/// A translatable prose field storing both original and optional translated text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct LocalizedField {
    pub original: String,
    pub translated: Option<String>,
}

impl LocalizedField {
    #[must_use]
    pub fn new(original: impl Into<String>) -> Self {
        Self {
            original: original.into(),
            translated: None,
        }
    }

    /// Returns `translated` when non-blank, otherwise `original` when non-blank.
    #[must_use]
    pub fn resolved(&self) -> Option<&str> {
        self.translated
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let orig = self.original.trim();
                (!orig.is_empty()).then_some(orig)
            })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.original.trim().is_empty()
            && self
                .translated
                .as_deref()
                .is_none_or(|t| t.trim().is_empty())
    }

    /// Clears stale `translated` when `original` changed and caller did not also
    /// supply a new `translated`.
    pub fn reconcile_with(&mut self, previous: &Self) {
        if self.original != previous.original && self.translated == previous.translated {
            self.translated = None;
        }
        if self
            .translated
            .as_deref()
            .is_some_and(|t| t.trim().is_empty())
        {
            self.translated = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum YesNo {
    #[default]
    Unknown,
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Manga {
    #[default]
    Unknown,
    No,
    Yes,
    YesAndRightToLeft,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum AgeRating {
    #[default]
    Unknown,
    #[serde(rename = "Adults Only 18+")]
    AdultsOnly18Plus,
    #[serde(rename = "Early Childhood")]
    EarlyChildhood,
    Everyone,
    #[serde(rename = "Everyone 10+")]
    Everyone10Plus,
    G,
    #[serde(rename = "Kids to Adults")]
    KidsToAdults,
    M,
    #[serde(rename = "MA15+")]
    Ma15Plus,
    #[serde(rename = "Mature 17+")]
    Mature17Plus,
    PG,
    #[serde(rename = "R18+")]
    R18Plus,
    #[serde(rename = "Rating Pending")]
    RatingPending,
    Teen,
    #[serde(rename = "X18+")]
    X18Plus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum ComicPageType {
    FrontCover,
    InnerCover,
    Roundup,
    #[default]
    Story,
    Advertisement,
    Editorial,
    Letters,
    Preview,
    BackCover,
    Other,
    Deleted,
}

/// One `<Page .../>` entry inside `<Pages>` in ComicInfo.xml 2.1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ComicPageInfo {
    pub image: i32,
    pub page_type: Option<ComicPageType>,
    pub double_page: Option<bool>,
    pub image_size: Option<i64>,
    pub key: Option<String>,
    pub bookmark: Option<String>,
    pub image_width: Option<i32>,
    pub image_height: Option<i32>,
}

/// Full ComicInfo.xml 2.1 metadata (all 44 XSD elements).
///
/// The 14 prose elements use [`LocalizedField`] while structural elements are
/// stored raw.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ComicInfo {
    pub title: Option<LocalizedField>,
    pub series: Option<LocalizedField>,
    pub number: Option<String>,
    pub count: Option<i32>,
    pub volume: Option<i32>,
    pub alternate_series: Option<LocalizedField>,
    pub alternate_number: Option<String>,
    pub alternate_count: Option<i32>,
    pub summary: Option<LocalizedField>,
    pub notes: Option<LocalizedField>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
    pub writer: Option<String>,
    pub penciller: Option<String>,
    pub inker: Option<String>,
    pub colorist: Option<String>,
    pub letterer: Option<String>,
    pub cover_artist: Option<String>,
    pub editor: Option<String>,
    pub translator: Option<String>,
    pub publisher: Option<String>,
    pub imprint: Option<String>,
    pub genre: Option<LocalizedField>,
    pub tags: Option<LocalizedField>,
    pub web: Option<String>,
    pub page_count: Option<i32>,
    pub language_iso: Option<String>,
    pub format: Option<String>,
    pub black_and_white: Option<YesNo>,
    pub manga: Option<Manga>,
    pub characters: Option<LocalizedField>,
    pub teams: Option<LocalizedField>,
    pub locations: Option<LocalizedField>,
    pub scan_information: Option<String>,
    pub story_arc: Option<LocalizedField>,
    pub story_arc_number: Option<String>,
    pub series_group: Option<LocalizedField>,
    pub age_rating: Option<AgeRating>,
    pub pages: Vec<ComicPageInfo>,
    pub community_rating: Option<f64>,
    pub main_character_or_team: Option<LocalizedField>,
    pub review: Option<LocalizedField>,
    pub gtin: Option<String>,
}

impl ComicInfo {
    /// Returns all 14 prose fields paired with their ComicInfo XML element name.
    #[must_use]
    pub fn prose_fields(&self) -> [(&'static str, Option<&LocalizedField>); 14] {
        [
            ("Title", self.title.as_ref()),
            ("Series", self.series.as_ref()),
            ("AlternateSeries", self.alternate_series.as_ref()),
            ("Summary", self.summary.as_ref()),
            ("Notes", self.notes.as_ref()),
            ("Genre", self.genre.as_ref()),
            ("Tags", self.tags.as_ref()),
            ("Characters", self.characters.as_ref()),
            ("Teams", self.teams.as_ref()),
            ("Locations", self.locations.as_ref()),
            ("StoryArc", self.story_arc.as_ref()),
            ("SeriesGroup", self.series_group.as_ref()),
            ("MainCharacterOrTeam", self.main_character_or_team.as_ref()),
            ("Review", self.review.as_ref()),
        ]
    }

    fn prose_fields_mut(&mut self) -> [(&'static str, &mut Option<LocalizedField>); 14] {
        [
            ("Title", &mut self.title),
            ("Series", &mut self.series),
            ("AlternateSeries", &mut self.alternate_series),
            ("Summary", &mut self.summary),
            ("Notes", &mut self.notes),
            ("Genre", &mut self.genre),
            ("Tags", &mut self.tags),
            ("Characters", &mut self.characters),
            ("Teams", &mut self.teams),
            ("Locations", &mut self.locations),
            ("StoryArc", &mut self.story_arc),
            ("SeriesGroup", &mut self.series_group),
            ("MainCharacterOrTeam", &mut self.main_character_or_team),
            ("Review", &mut self.review),
        ]
    }

    /// Returns `(field_name, original_text)` for each prose field with non-blank original text.
    #[must_use]
    pub fn translatable_fields(&self) -> Vec<(&'static str, &str)> {
        self.prose_fields()
            .into_iter()
            .filter_map(|(name, field)| {
                let field = field?;
                (!field.original.trim().is_empty()).then_some((name, field.original.as_str()))
            })
            .collect()
    }

    #[must_use]
    pub fn has_translatable_fields(&self) -> bool {
        !self.translatable_fields().is_empty()
    }

    /// Applies translated prose values keyed by ComicInfo field name.
    pub fn apply_translations(&mut self, translations: &BTreeMap<String, String>) {
        for (name, slot) in self.prose_fields_mut() {
            let Some(field) = slot.as_mut() else {
                continue;
            };
            if let Some(translated) = translations.get(name) {
                let trimmed = translated.trim();
                field.translated = (!trimmed.is_empty()).then(|| translated.clone());
            }
        }
    }

    /// Sets `translated` on `field_name` only when `original` still matches `expected_original`.
    pub fn set_translated_if_unchanged(
        &mut self,
        field_name: &str,
        expected_original: &str,
        translated: String,
    ) -> bool {
        for (name, slot) in self.prose_fields_mut() {
            if name == field_name {
                let Some(field) = slot.as_mut() else {
                    return false;
                };
                if field.original != expected_original {
                    return false;
                }
                let trimmed = translated.trim();
                field.translated = (!trimmed.is_empty()).then_some(translated);
                return true;
            }
        }
        false
    }

    /// Reconciles edited prose fields against `previous` (clearing stale `translated`
    /// when `original` changed without a matching `translated` edit) and normalizes
    /// blank fields to `None`.
    pub fn reconcile_with(&mut self, previous: &Self) {
        let prev_fields = previous.prose_fields();
        for ((_, current), (_, prev)) in self.prose_fields_mut().into_iter().zip(prev_fields) {
            if let Some(field) = current.as_mut() {
                if let Some(prev) = prev {
                    field.reconcile_with(prev);
                } else if field
                    .translated
                    .as_deref()
                    .is_some_and(|t| t.trim().is_empty())
                {
                    field.translated = None;
                }
                if field.is_empty() {
                    *current = None;
                }
            }
        }

        for opt in [
            &mut self.number,
            &mut self.alternate_number,
            &mut self.writer,
            &mut self.penciller,
            &mut self.inker,
            &mut self.colorist,
            &mut self.letterer,
            &mut self.cover_artist,
            &mut self.editor,
            &mut self.translator,
            &mut self.publisher,
            &mut self.imprint,
            &mut self.web,
            &mut self.language_iso,
            &mut self.format,
            &mut self.scan_information,
            &mut self.story_arc_number,
            &mut self.gtin,
        ] {
            if opt.as_deref().is_some_and(|s| s.trim().is_empty()) {
                *opt = None;
            }
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prose_fields()
            .into_iter()
            .all(|(_, field)| field.is_none_or(LocalizedField::is_empty))
            && [
                self.number.as_deref(),
                self.alternate_number.as_deref(),
                self.writer.as_deref(),
                self.penciller.as_deref(),
                self.inker.as_deref(),
                self.colorist.as_deref(),
                self.letterer.as_deref(),
                self.cover_artist.as_deref(),
                self.editor.as_deref(),
                self.translator.as_deref(),
                self.publisher.as_deref(),
                self.imprint.as_deref(),
                self.web.as_deref(),
                self.language_iso.as_deref(),
                self.format.as_deref(),
                self.scan_information.as_deref(),
                self.story_arc_number.as_deref(),
                self.gtin.as_deref(),
            ]
            .into_iter()
            .all(|s| s.is_none_or(|v| v.trim().is_empty()))
            && self.count.is_none()
            && self.volume.is_none()
            && self.alternate_count.is_none()
            && self.year.is_none()
            && self.month.is_none()
            && self.day.is_none()
            && self.page_count.is_none()
            && self.black_and_white.is_none()
            && self.manga.is_none()
            && self.age_rating.is_none()
            && self.pages.is_empty()
            && self.community_rating.is_none()
    }

    /// Parses `ComicInfo.xml` bytes (supporting UTF-8 and UTF-16 BOMs).
    ///
    /// Returns `Ok(None)` when the document contains no non-empty fields.
    pub fn from_xml_bytes(bytes: &[u8]) -> Result<Option<Self>> {
        let text = decode_xml_bytes(bytes)?;
        let xml: XmlComicInfo = from_xml_str(&text).context("invalid ComicInfo.xml")?;
        let info = xml.into_comic_info();
        if info.is_empty() {
            Ok(None)
        } else {
            Ok(Some(info))
        }
    }

    /// Serializes this metadata to a UTF-8 `ComicInfo.xml` 2.1 document, using
    /// `translated` falling back to `original` for prose fields.
    pub fn to_xml_string(&self) -> Result<String> {
        let xml = XmlComicInfo::from_comic_info(self);
        let mut body = String::new();
        let mut serializer = Serializer::with_root(&mut body, Some("ComicInfo"))?;
        serializer.indent(' ', 2);
        xml.serialize(serializer)?;
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{body}\n"
        ))
    }
}

fn decode_xml_bytes(bytes: &[u8]) -> Result<String> {
    if let Some((encoding, bom_len)) = Encoding::for_bom(bytes) {
        let (decoded, _, had_errors) = encoding.decode(&bytes[bom_len..]);
        if had_errors {
            bail!("ComicInfo.xml contains invalid {} bytes", encoding.name());
        }
        return Ok(decoded.into_owned());
    }
    Ok(std::str::from_utf8(bytes)
        .context("ComicInfo.xml is not valid UTF-8")?
        .to_owned())
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn opt_localized(value: Option<String>) -> Option<LocalizedField> {
    non_empty_string(value).map(LocalizedField::new)
}

fn de_opt_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(non_empty_string(Option::<String>::deserialize(
        deserializer,
    )?))
}

fn de_opt_i32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i32>, D::Error> {
    let Some(raw) = non_empty_string(Option::<String>::deserialize(deserializer)?) else {
        return Ok(None);
    };
    if raw == "-1" {
        return Ok(None);
    }
    raw.parse::<i32>().map(Some).map_err(de::Error::custom)
}

fn de_opt_i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    let Some(raw) = non_empty_string(Option::<String>::deserialize(deserializer)?) else {
        return Ok(None);
    };
    if raw == "-1" {
        return Ok(None);
    }
    raw.parse::<i64>().map(Some).map_err(de::Error::custom)
}

fn de_opt_f64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    let Some(raw) = non_empty_string(Option::<String>::deserialize(deserializer)?) else {
        return Ok(None);
    };
    let parsed: f64 = raw.parse().map_err(de::Error::custom)?;
    if !parsed.is_finite() {
        return Err(de::Error::custom("CommunityRating must be finite"));
    }
    Ok(Some(parsed))
}

fn de_opt_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    let Some(raw) = non_empty_string(Option::<String>::deserialize(deserializer)?) else {
        return Ok(None);
    };
    match raw.to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(Some(true)),
        "false" | "0" => Ok(Some(false)),
        _ => Err(de::Error::custom(format!("invalid XML boolean `{raw}`"))),
    }
}

fn de_opt_enum<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let Some(raw) = non_empty_string(Option::<String>::deserialize(deserializer)?) else {
        return Ok(None);
    };
    T::deserialize(raw.into_deserializer()).map(Some)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct XmlPage {
    #[serde(rename = "@Image")]
    image: i32,
    #[serde(
        rename = "@Type",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_enum"
    )]
    page_type: Option<ComicPageType>,
    #[serde(
        rename = "@DoublePage",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_bool"
    )]
    double_page: Option<bool>,
    #[serde(
        rename = "@ImageSize",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i64"
    )]
    image_size: Option<i64>,
    #[serde(
        rename = "@Key",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    key: Option<String>,
    #[serde(
        rename = "@Bookmark",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    bookmark: Option<String>,
    #[serde(
        rename = "@ImageWidth",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    image_width: Option<i32>,
    #[serde(
        rename = "@ImageHeight",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    image_height: Option<i32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct XmlPages {
    #[serde(rename = "Page", default)]
    pages: Vec<XmlPage>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct XmlComicInfo {
    #[serde(
        rename = "Title",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    title: Option<String>,
    #[serde(
        rename = "Series",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    series: Option<String>,
    #[serde(
        rename = "Number",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    number: Option<String>,
    #[serde(
        rename = "Count",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    count: Option<i32>,
    #[serde(
        rename = "Volume",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    volume: Option<i32>,
    #[serde(
        rename = "AlternateSeries",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    alternate_series: Option<String>,
    #[serde(
        rename = "AlternateNumber",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    alternate_number: Option<String>,
    #[serde(
        rename = "AlternateCount",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    alternate_count: Option<i32>,
    #[serde(
        rename = "Summary",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    summary: Option<String>,
    #[serde(
        rename = "Notes",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    notes: Option<String>,
    #[serde(
        rename = "Year",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    year: Option<i32>,
    #[serde(
        rename = "Month",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    month: Option<i32>,
    #[serde(
        rename = "Day",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    day: Option<i32>,
    #[serde(
        rename = "Writer",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    writer: Option<String>,
    #[serde(
        rename = "Penciller",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    penciller: Option<String>,
    #[serde(
        rename = "Inker",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    inker: Option<String>,
    #[serde(
        rename = "Colorist",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    colorist: Option<String>,
    #[serde(
        rename = "Letterer",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    letterer: Option<String>,
    #[serde(
        rename = "CoverArtist",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    cover_artist: Option<String>,
    #[serde(
        rename = "Editor",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    editor: Option<String>,
    #[serde(
        rename = "Translator",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    translator: Option<String>,
    #[serde(
        rename = "Publisher",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    publisher: Option<String>,
    #[serde(
        rename = "Imprint",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    imprint: Option<String>,
    #[serde(
        rename = "Genre",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    genre: Option<String>,
    #[serde(
        rename = "Tags",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    tags: Option<String>,
    #[serde(
        rename = "Web",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    web: Option<String>,
    #[serde(
        rename = "PageCount",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_i32"
    )]
    page_count: Option<i32>,
    #[serde(
        rename = "LanguageISO",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    language_iso: Option<String>,
    #[serde(
        rename = "Format",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    format: Option<String>,
    #[serde(
        rename = "BlackAndWhite",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_enum"
    )]
    black_and_white: Option<YesNo>,
    #[serde(
        rename = "Manga",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_enum"
    )]
    manga: Option<Manga>,
    #[serde(
        rename = "Characters",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    characters: Option<String>,
    #[serde(
        rename = "Teams",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    teams: Option<String>,
    #[serde(
        rename = "Locations",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    locations: Option<String>,
    #[serde(
        rename = "ScanInformation",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    scan_information: Option<String>,
    #[serde(
        rename = "StoryArc",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    story_arc: Option<String>,
    #[serde(
        rename = "StoryArcNumber",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    story_arc_number: Option<String>,
    #[serde(
        rename = "SeriesGroup",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    series_group: Option<String>,
    #[serde(
        rename = "AgeRating",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_enum"
    )]
    age_rating: Option<AgeRating>,
    #[serde(rename = "Pages", default, skip_serializing_if = "Option::is_none")]
    pages: Option<XmlPages>,
    #[serde(
        rename = "CommunityRating",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_f64"
    )]
    community_rating: Option<f64>,
    #[serde(
        rename = "MainCharacterOrTeam",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    main_character_or_team: Option<String>,
    #[serde(
        rename = "Review",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    review: Option<String>,
    #[serde(
        rename = "GTIN",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_opt_string"
    )]
    gtin: Option<String>,
}

impl XmlComicInfo {
    fn into_comic_info(self) -> ComicInfo {
        ComicInfo {
            title: opt_localized(self.title),
            series: opt_localized(self.series),
            number: self.number,
            count: self.count,
            volume: self.volume,
            alternate_series: opt_localized(self.alternate_series),
            alternate_number: self.alternate_number,
            alternate_count: self.alternate_count,
            summary: opt_localized(self.summary),
            notes: opt_localized(self.notes),
            year: self.year,
            month: self.month,
            day: self.day,
            writer: self.writer,
            penciller: self.penciller,
            inker: self.inker,
            colorist: self.colorist,
            letterer: self.letterer,
            cover_artist: self.cover_artist,
            editor: self.editor,
            translator: self.translator,
            publisher: self.publisher,
            imprint: self.imprint,
            genre: opt_localized(self.genre),
            tags: opt_localized(self.tags),
            web: self.web,
            page_count: self.page_count,
            language_iso: self.language_iso,
            format: self.format,
            black_and_white: self.black_and_white,
            manga: self.manga,
            characters: opt_localized(self.characters),
            teams: opt_localized(self.teams),
            locations: opt_localized(self.locations),
            scan_information: self.scan_information,
            story_arc: opt_localized(self.story_arc),
            story_arc_number: self.story_arc_number,
            series_group: opt_localized(self.series_group),
            age_rating: self.age_rating,
            pages: self
                .pages
                .map(|p| {
                    p.pages
                        .into_iter()
                        .map(|page| ComicPageInfo {
                            image: page.image,
                            page_type: page.page_type,
                            double_page: page.double_page,
                            image_size: page.image_size,
                            key: page.key,
                            bookmark: page.bookmark,
                            image_width: page.image_width,
                            image_height: page.image_height,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            community_rating: self.community_rating,
            main_character_or_team: opt_localized(self.main_character_or_team),
            review: opt_localized(self.review),
            gtin: self.gtin,
        }
    }

    fn from_comic_info(info: &ComicInfo) -> Self {
        let resolve = |field: &Option<LocalizedField>| {
            field
                .as_ref()
                .and_then(LocalizedField::resolved)
                .map(str::to_owned)
        };
        let pages = if info.pages.is_empty() {
            None
        } else {
            Some(XmlPages {
                pages: info
                    .pages
                    .iter()
                    .map(|page| XmlPage {
                        image: page.image,
                        page_type: page.page_type,
                        double_page: page.double_page,
                        image_size: page.image_size,
                        key: non_empty_string(page.key.clone()),
                        bookmark: non_empty_string(page.bookmark.clone()),
                        image_width: page.image_width,
                        image_height: page.image_height,
                    })
                    .collect(),
            })
        };
        Self {
            title: resolve(&info.title),
            series: resolve(&info.series),
            number: non_empty_string(info.number.clone()),
            count: info.count,
            volume: info.volume,
            alternate_series: resolve(&info.alternate_series),
            alternate_number: non_empty_string(info.alternate_number.clone()),
            alternate_count: info.alternate_count,
            summary: resolve(&info.summary),
            notes: resolve(&info.notes),
            year: info.year,
            month: info.month,
            day: info.day,
            writer: non_empty_string(info.writer.clone()),
            penciller: non_empty_string(info.penciller.clone()),
            inker: non_empty_string(info.inker.clone()),
            colorist: non_empty_string(info.colorist.clone()),
            letterer: non_empty_string(info.letterer.clone()),
            cover_artist: non_empty_string(info.cover_artist.clone()),
            editor: non_empty_string(info.editor.clone()),
            translator: non_empty_string(info.translator.clone()),
            publisher: non_empty_string(info.publisher.clone()),
            imprint: non_empty_string(info.imprint.clone()),
            genre: resolve(&info.genre),
            tags: resolve(&info.tags),
            web: non_empty_string(info.web.clone()),
            page_count: info.page_count,
            language_iso: non_empty_string(info.language_iso.clone()),
            format: non_empty_string(info.format.clone()),
            black_and_white: info.black_and_white,
            manga: info.manga,
            characters: resolve(&info.characters),
            teams: resolve(&info.teams),
            locations: resolve(&info.locations),
            scan_information: non_empty_string(info.scan_information.clone()),
            story_arc: resolve(&info.story_arc),
            story_arc_number: non_empty_string(info.story_arc_number.clone()),
            series_group: resolve(&info.series_group),
            age_rating: info.age_rating,
            pages,
            community_rating: info.community_rating,
            main_character_or_team: resolve(&info.main_character_or_team),
            review: resolve(&info.review),
            gtin: non_empty_string(info.gtin.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_44_elements_handles_empty_tags_and_round_trips() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <Title>あつい夏</Title>
  <Series>よつばと!</Series>
  <Number>1</Number>
  <Count>15</Count>
  <Volume>1</Volume>
  <AlternateSeries>Yotsubato</AlternateSeries>
  <AlternateNumber>01</AlternateNumber>
  <AlternateCount>-1</AlternateCount>
  <Summary>よつばの日常</Summary>
  <Notes>Scan note</Notes>
  <Year>2003</Year>
  <Month>8</Month>
  <Day/>
  <Writer>あずまきよひこ</Writer>
  <Penciller>Penciller</Penciller>
  <Inker>Inker</Inker>
  <Colorist>Colorist</Colorist>
  <Letterer>Letterer</Letterer>
  <CoverArtist>CoverArtist</CoverArtist>
  <Editor>Editor</Editor>
  <Translator>Translator</Translator>
  <Publisher>KADOKAWA</Publisher>
  <Imprint>Dengeki</Imprint>
  <Genre>Comedy, Slice of Life</Genre>
  <Tags>manga, kids</Tags>
  <Web>https://example.com</Web>
  <PageCount>200</PageCount>
  <LanguageISO>ja</LanguageISO>
  <Format>Tankobon</Format>
  <BlackAndWhite>Yes</BlackAndWhite>
  <Manga>YesAndRightToLeft</Manga>
  <Characters>Yotsuba, Koiwai</Characters>
  <Teams>Neighbors</Teams>
  <Locations>Tokyo</Locations>
  <ScanInformation>300dpi</ScanInformation>
  <StoryArc>Summer</StoryArc>
  <StoryArcNumber>1</StoryArcNumber>
  <SeriesGroup>Dengeki Comics</SeriesGroup>
  <AgeRating>Everyone 10+</AgeRating>
  <Pages>
    <Page Image="0" Type="FrontCover" DoublePage="false" ImageSize="1234" Key="001.png" Bookmark="Cover" ImageWidth="800" ImageHeight="1200"/>
    <Page Image="1" Type="Story" DoublePage="true" ImageSize="2345" Key="002.png" ImageWidth="1600" ImageHeight="1200"/>
  </Pages>
  <CommunityRating>4.5</CommunityRating>
  <MainCharacterOrTeam>Yotsuba</MainCharacterOrTeam>
  <Review>Classic slice of life.</Review>
  <GTIN>9784840224666</GTIN>
</ComicInfo>"#;

        let mut info = ComicInfo::from_xml_bytes(xml.as_bytes())
            .expect("valid xml")
            .expect("non-empty");
        assert_eq!(info.title, Some(LocalizedField::new("あつい夏")));
        assert_eq!(info.series, Some(LocalizedField::new("よつばと!")));
        assert_eq!(info.alternate_count, None);
        assert_eq!(info.day, None);
        assert_eq!(info.black_and_white, Some(YesNo::Yes));
        assert_eq!(info.manga, Some(Manga::YesAndRightToLeft));
        assert_eq!(info.age_rating, Some(AgeRating::Everyone10Plus));
        assert_eq!(info.community_rating, Some(4.5));
        assert_eq!(info.gtin.as_deref(), Some("9784840224666"));
        assert_eq!(info.pages.len(), 2);
        assert_eq!(info.pages[0].page_type, Some(ComicPageType::FrontCover));
        assert_eq!(info.pages[1].double_page, Some(true));

        // Translated text takes precedence on export, falling back to original.
        let mut translations = BTreeMap::new();
        translations.insert("Title".to_owned(), "Hot Summer".to_owned());
        info.apply_translations(&translations);
        let serialized = info.to_xml_string().expect("xml serializes");
        assert!(serialized.contains("<Title>Hot Summer</Title>"));
        assert!(serialized.contains("<Series>よつばと!</Series>"));
        assert!(serialized.contains("AgeRating>Everyone 10+</AgeRating>"));

        let round_tripped = ComicInfo::from_xml_bytes(serialized.as_bytes())
            .expect("round-trip parse")
            .expect("non-empty");
        assert_eq!(round_tripped.title, Some(LocalizedField::new("Hot Summer")));
        assert_eq!(round_tripped.pages, info.pages);
    }

    #[test]
    fn handles_empty_xml_utf16_bom_and_reconcile_with() {
        assert_eq!(
            ComicInfo::from_xml_bytes(
                b"<ComicInfo><Year/><Title></Title><BlackAndWhite/><Pages/></ComicInfo>"
            )
            .unwrap(),
            None
        );

        let xml = "<ComicInfo><Series>Yotsuba</Series></ComicInfo>";
        let mut utf16 = vec![0xFFu8, 0xFE];
        for unit in xml.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let parsed = ComicInfo::from_xml_bytes(&utf16).unwrap().unwrap();
        assert_eq!(parsed.series, Some(LocalizedField::new("Yotsuba")));

        let prev = ComicInfo {
            title: Some(LocalizedField {
                original: "Old".to_owned(),
                translated: Some("Old TL".to_owned()),
            }),
            series: Some(LocalizedField {
                original: "Same".to_owned(),
                translated: Some("Same TL".to_owned()),
            }),
            ..Default::default()
        };
        let mut edited = ComicInfo {
            title: Some(LocalizedField {
                original: "New".to_owned(),
                translated: Some("Old TL".to_owned()),
            }),
            series: Some(LocalizedField {
                original: "Same".to_owned(),
                translated: Some("Same TL".to_owned()),
            }),
            ..Default::default()
        };
        edited.reconcile_with(&prev);
        assert_eq!(edited.title.as_ref().unwrap().translated, None);
        assert_eq!(
            edited.series.as_ref().unwrap().translated.as_deref(),
            Some("Same TL")
        );
    }
}
