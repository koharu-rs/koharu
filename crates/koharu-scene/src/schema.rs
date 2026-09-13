//! Koharu document schema and cross-component invariants.
//!
//! The state kernel owns storage, hierarchy, indexing, and relations as data.
//! This module owns the meaning of Koharu's component combinations and typed
//! relation endpoints.

use crate::{
    BubbleRegion, DetectionAnalysis, EntityId, EntityOrigin, Error, Geometry, Group, OcrAnalysis,
    Page, Project, RasterLayer, Region, RegionSpec, Relation, Result, SourceText, TextContent,
    TextGroup, TextLayout, TextRegion, TextRole, Translation, Typography, Visibility,
    component::{Component, ComponentRecord, ValidationContext, decode, key},
    components::Assets,
    state::{Components, State},
};

macro_rules! component_schema {
    ($($mask:ident = $bit:literal => $component:ty),+ $(,)?) => {
        $(const $mask: u32 = 1 << $bit;)+

        fn component_mask(kind: &str) -> u32 {
            $(if kind == <$component as Component>::KIND {
                return $mask;
            })+
            0
        }

        fn validate_component(
            kind: &str,
            raw: &ComponentRecord,
            context: &ValidationContext<'_>,
        ) -> Result<()> {
            $(if kind == <$component as Component>::KIND {
                decode::<$component>(raw, context)?;
                return Ok(());
            })+
            // Extension components remain open-ended. Their owning crate is
            // responsible for decoding them; the kernel still preserves their
            // revision, references, and fingerprints.
            Ok(())
        }
    };
}

component_schema! {
    GLOSSARY = 18 => crate::ProjectGlossary,
    PROJECT = 0 => Project,
    PAGE = 1 => Page,
    GROUP = 2 => Group,
    TEXT_GROUP = 3 => TextGroup,
    GEOMETRY = 4 => Geometry,
    RASTER_LAYER = 5 => RasterLayer,
    VISIBILITY = 6 => Visibility,
    SOURCE_TEXT = 7 => SourceText,
    TEXT_CONTENT = 8 => TextContent,
    TEXT_LAYOUT = 9 => TextLayout,
    OCR_ANALYSIS = 10 => OcrAnalysis,
    TRANSLATION = 11 => Translation,
    TEXT_ROLE = 12 => TextRole,
    TYPOGRAPHY = 13 => Typography,
    REGION = 14 => Region,
    DETECTION_ANALYSIS = 15 => DetectionAnalysis,
    ASSETS = 16 => Assets,
    ENTITY_ORIGIN = 17 => EntityOrigin,
}

pub(crate) fn validate_components(
    components: &Components,
    context: &ValidationContext<'_>,
) -> Result<()> {
    for (key, raw) in components {
        validate_component(&key.kind, raw, context)?;
    }
    Ok(())
}

pub(crate) fn validate_entity(state: &State, id: EntityId) -> Result<()> {
    let entity = state.entity(id)?;
    let kinds = entity
        .components
        .iter()
        .fold(0, |kinds, (key, _)| kinds | component_mask(&key.kind));
    let has = |component| kinds & component != 0;
    let has_source = has(SOURCE_TEXT);
    let has_content = has(TEXT_CONTENT);
    let has_translation = has(TRANSLATION);
    let has_region = has(REGION);
    let has_geometry = has(GEOMETRY);
    let has_raster = has(RASTER_LAYER);
    let has_assets = has(ASSETS);
    let has_layout = has(TEXT_LAYOUT);
    let has_typography = has(TYPOGRAPHY);
    let has_detection = has(DETECTION_ANALYSIS);
    let has_ocr = has(OCR_ANALYSIS);
    let has_group = has(GROUP);
    let has_text_group = has(TEXT_GROUP);
    let parent = state.parent_and_position(id)?.0;
    let parent_is_text_group = parent.is_some_and(|parent| {
        state.entity(parent).is_ok_and(|entity| {
            entity
                .components
                .iter()
                .any(|(key, _)| component_mask(&key.kind) == TEXT_GROUP)
        })
    });

    if (has_source || has_translation || has(TEXT_ROLE)) && !has_content {
        Err(Error::invalid(format!(
            "entity {id} carries text content data but is not text content"
        )))
    } else if has_translation && !has_source {
        Err(Error::invalid(format!(
            "entity {id} has a translation but no source text"
        )))
    } else if has_text_group && !has_group {
        Err(Error::invalid(format!(
            "text group {id} does not carry the group component"
        )))
    } else if has_group
        && (has(PAGE)
            || has_content
            || has_region
            || has_geometry
            || has_raster
            || has_assets
            || has_layout
            || has_typography
            || has_detection
            || has_ocr)
    {
        Err(Error::invalid(format!(
            "group {id} also carries content, analysis, or layer components"
        )))
    } else if has_text_group {
        let page = state.page_for(id)?;
        if parent != Some(page) {
            return Err(Error::invalid(format!(
                "text group {id} is not a direct child of its page"
            )));
        }
        let groups = state.page(page)?.entities_with(&key::<TextGroup>()?);
        if groups.len() != 1 {
            return Err(Error::invalid(format!(
                "page {page} must not contain multiple text groups"
            )));
        }
        Ok(())
    } else if parent_is_text_group && !has_layout {
        Err(Error::invalid(format!(
            "text group child {id} is not a text layer"
        )))
    } else if has_layout && !parent_is_text_group {
        Err(Error::invalid(format!(
            "text layer {id} is not contained by the page text group"
        )))
    } else if has_content
        && (has_region || has_geometry || has_layout || has_typography || has_detection || has_ocr)
    {
        Err(Error::invalid(format!(
            "text content entity {id} also carries analysis or presentation components"
        )))
    } else if has_region && (has_layout || has_typography || has_source || has_translation) {
        Err(Error::invalid(format!(
            "analysis region {id} also carries text presentation or content components"
        )))
    } else if has_region && !has_geometry {
        Err(Error::invalid(format!(
            "analysis region {id} has no geometry"
        )))
    } else if has_layout
        && (has_source || has_translation || has_region || has_detection || has_ocr)
    {
        Err(Error::invalid(format!(
            "text layer {id} also carries content or analysis components"
        )))
    } else if has_typography && !has_layout {
        Err(Error::invalid(format!(
            "entity {id} has typography but is not a text layer"
        )))
    } else if (has_detection || has_ocr) && !has_region {
        Err(Error::invalid(format!(
            "entity {id} has analysis results but is not a region"
        )))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_relation(
    state: &State,
    relation: &Relation,
    context: &ValidationContext<'_>,
) -> Result<()> {
    validate_relation_endpoints(state, relation, context)?;
    validate_automatic_placement(state, relation)?;
    if is_functional(&relation.kind)
        && state
            .outgoing
            .get(&relation.source)
            .into_iter()
            .flat_map(|relations| relations.iter())
            .filter(|relation_id| state.relations[*relation_id].value.kind == relation.kind)
            .count()
            > 1
    {
        return Err(Error::invalid(format!(
            "entity {} has multiple {} relations",
            relation.source,
            relation.kind.as_str()
        )));
    }
    Ok(())
}

pub(crate) fn validate_new_relation(
    state: &State,
    relation: &Relation,
    context: &ValidationContext<'_>,
) -> Result<()> {
    validate_relation_endpoints(state, relation, context)?;
    validate_automatic_placement(state, relation)?;
    if is_functional(&relation.kind)
        && state
            .outgoing
            .get(&relation.source)
            .into_iter()
            .flat_map(|relations| relations.iter())
            .any(|relation_id| state.relations[relation_id].value.kind == relation.kind)
    {
        return Err(Error::invalid(format!(
            "entity {} already has a {} relation",
            relation.source,
            relation.kind.as_str()
        )));
    }
    Ok(())
}

fn validate_relation_endpoints(
    state: &State,
    relation: &Relation,
    context: &ValidationContext<'_>,
) -> Result<()> {
    relation.validate(context)?;
    let has = |entity, kind: &str| {
        state
            .entity(entity)
            .is_ok_and(|entity| entity.components.iter().any(|(key, _)| key.kind == kind))
    };
    let region_kind = |entity| -> Result<Option<_>> {
        let raw = state
            .entity(entity)?
            .components
            .iter()
            .find_map(|(key, raw)| (key.kind == Region::KIND).then_some(raw));
        raw.map(|raw| decode::<Region>(raw, context).map(|region| region.kind))
            .transpose()
    };
    let valid = match relation.kind.as_str() {
        <crate::Presents as crate::RelationSpec>::KIND => {
            has(relation.source, TextLayout::KIND) && has(relation.target, TextContent::KIND)
        }
        <crate::RecognizedFrom as crate::RelationSpec>::KIND => {
            has(relation.source, TextContent::KIND) && has(relation.target, Region::KIND)
        }
        <crate::FitsTo as crate::RelationSpec>::KIND => {
            has(relation.source, TextLayout::KIND)
                && has(relation.target, Region::KIND)
                && has(relation.target, Geometry::KIND)
                && region_kind(relation.target)?.is_some_and(|kind| kind == TextRegion::kind())
        }
        <crate::FlowsIn as crate::RelationSpec>::KIND => {
            has(relation.source, TextLayout::KIND)
                && has(relation.target, Region::KIND)
                && has(relation.target, Geometry::KIND)
                && region_kind(relation.target)?.is_some_and(|kind| kind == BubbleRegion::kind())
        }
        <crate::Inside as crate::RelationSpec>::KIND => {
            has(relation.source, Region::KIND)
                && has(relation.target, Region::KIND)
                && has(relation.source, Geometry::KIND)
                && has(relation.target, Geometry::KIND)
        }
        _ => true,
    };
    if !valid {
        Err(Error::invalid(format!(
            "relation {} has incompatible endpoints",
            relation.kind.as_str()
        )))
    } else {
        Ok(())
    }
}

fn is_functional(kind: &crate::RelationKind) -> bool {
    matches!(
        kind.as_str(),
        <crate::Presents as crate::RelationSpec>::KIND
            | <crate::RecognizedFrom as crate::RelationSpec>::KIND
            | <crate::FitsTo as crate::RelationSpec>::KIND
            | <crate::FlowsIn as crate::RelationSpec>::KIND
    )
}

fn validate_automatic_placement(state: &State, relation: &Relation) -> Result<()> {
    let other = match relation.kind.as_str() {
        <crate::FitsTo as crate::RelationSpec>::KIND => {
            Some(<crate::FlowsIn as crate::RelationSpec>::KIND)
        }
        <crate::FlowsIn as crate::RelationSpec>::KIND => {
            Some(<crate::FitsTo as crate::RelationSpec>::KIND)
        }
        _ => None,
    };
    let Some(other) = other else {
        return Ok(());
    };
    let conflicts = state
        .outgoing
        .get(&relation.source)
        .into_iter()
        .flat_map(|relations| relations.iter())
        .any(|id| state.relations[id].value.kind.as_str() == other);
    if conflicts {
        Err(Error::invalid(format!(
            "text layer {} has conflicting automatic placement relations",
            relation.source
        )))
    } else {
        Ok(())
    }
}
