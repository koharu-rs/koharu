use std::{collections::BTreeMap, sync::Arc};

use revision::revisioned;

use crate::*;

const EVOLVING_COMPONENT_KIND: &str = "dev.koharu.test.evolving";

#[revisioned(revision = 1)]
#[derive(Clone)]
struct EvolvingComponentV1 {
    title: String,
    legacy_count: u32,
}

impl Component for EvolvingComponentV1 {
    const KIND: &'static str = EVOLVING_COMPONENT_KIND;
}

#[revisioned(revision = 2)]
#[derive(Clone, Debug, PartialEq)]
struct EvolvingComponentV2 {
    title: String,
    #[revision(end = 2, convert_fn = "migrate_legacy_count")]
    legacy_count: u32,
    #[revision(start = 2)]
    count: u64,
    #[revision(start = 2, default_fn = "default_label")]
    label: String,
}

impl EvolvingComponentV2 {
    fn migrate_legacy_count(
        &mut self,
        _revision: u16,
        value: u32,
    ) -> std::result::Result<(), revision::Error> {
        self.count = u64::from(value);
        Ok(())
    }

    fn default_label(_revision: u16) -> std::result::Result<String, revision::Error> {
        Ok("migrated".to_owned())
    }
}

impl Component for EvolvingComponentV2 {
    const KIND: &'static str = EVOLVING_COMPONENT_KIND;
}

fn page() -> PageDraft {
    PageDraft::new("page", 1200.0, 1800.0)
}

fn assert_send_sync<T: Send + Sync>() {}

fn source(text: &str) -> SourceText {
    SourceText {
        text: Authored::user(text.to_owned()),
        language: Some(LanguageTag::new("ja").unwrap()),
    }
}

#[tokio::test]
async fn fresh_scene_has_an_empty_project_hierarchy() {
    assert_send_sync::<Snapshot>();
    assert_send_sync::<Patch>();
    let session = Session::memory().await.unwrap();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.revision(), Revision::ZERO);
    assert_eq!(snapshot.pages().len(), 0);
}

#[tokio::test]
async fn components_on_new_entities_do_not_observe_missing_base_state() {
    let mut session = Session::memory().await.unwrap();
    let mut entity = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let id = edit.add_entity(page, At::End)?;
            edit.set(
                id,
                &Visibility {
                    origin: Origin::User,
                    visible: true,
                    opacity: 1.0,
                },
            )?;
            entity = Some(id);
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    assert!(
        snapshot
            .component::<Visibility>(entity.unwrap())
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn built_in_component_schemas_remain_revision_one() {
    fn schema<T: Component>() -> u16 {
        <T as revision::Revisioned>::revision()
    }

    assert_eq!(
        [
            schema::<Project>(),
            schema::<Page>(),
            schema::<RasterLayer>(),
            schema::<Geometry>(),
            schema::<Visibility>(),
            schema::<SourceText>(),
            schema::<TextLayout>(),
            schema::<Translation>(),
            schema::<TextRole>(),
            schema::<OcrAnalysis>(),
            schema::<Group>(),
            schema::<TextGroup>(),
            schema::<Typography>(),
            schema::<Region>(),
            schema::<DetectionAnalysis>(),
            schema::<EntityOrigin>(),
            schema::<TextContent>(),
            schema::<crate::components::Assets>(),
            schema::<Relation>(),
        ],
        [1; 19]
    );
}

#[tokio::test]
async fn built_in_component_kinds_express_domain_ownership() {
    assert_eq!(
        [
            Project::KIND,
            Page::KIND,
            EntityOrigin::KIND,
            Relation::KIND,
            Geometry::KIND,
            Visibility::KIND,
            RasterLayer::KIND,
            TextLayout::KIND,
            TextContent::KIND,
            SourceText::KIND,
            Translation::KIND,
            TextRole::KIND,
            Group::KIND,
            TextGroup::KIND,
            Typography::KIND,
            Region::KIND,
            DetectionAnalysis::KIND,
            OcrAnalysis::KIND,
            crate::components::Assets::KIND,
        ],
        [
            "dev.koharu.project",
            "dev.koharu.page",
            "dev.koharu.entity.origin",
            "dev.koharu.relation",
            "dev.koharu.geometry",
            "dev.koharu.visibility",
            "dev.koharu.layer.raster",
            "dev.koharu.layer.text",
            "dev.koharu.text.content",
            "dev.koharu.text.source",
            "dev.koharu.text.translation",
            "dev.koharu.text.role",
            "dev.koharu.group",
            "dev.koharu.text.group",
            "dev.koharu.text.typography",
            "dev.koharu.analysis.region",
            "dev.koharu.analysis.detection",
            "dev.koharu.analysis.ocr",
            "dev.koharu.assets",
        ]
    );
}

#[tokio::test]
async fn revision_one_project_components_upgrade_when_their_schema_evolves() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("evolving.khrproj");
    let entity;
    {
        let mut session = Session::create(&path).await.unwrap();
        let mut created = None;
        let patch = session
            .snapshot()
            .patch(|edit| {
                let page = edit.add_page(page(), At::End)?;
                let id = edit.add_entity(page, At::End)?;
                edit.set(
                    id,
                    &EvolvingComponentV1 {
                        title: "revision one".to_owned(),
                        legacy_count: 7,
                    },
                )?;
                created = Some(id);
                Ok(())
            })
            .unwrap();
        session.commit(patch).await.unwrap();
        entity = created.unwrap();
    }

    let mut session = Session::open(&path).await.unwrap();
    let upgraded = session
        .snapshot()
        .component::<EvolvingComponentV2>(entity)
        .unwrap()
        .unwrap();
    assert_eq!(
        upgraded,
        EvolvingComponentV2 {
            title: "revision one".to_owned(),
            count: 7,
            label: "migrated".to_owned(),
        }
    );

    let patch = session
        .snapshot()
        .patch(|edit| edit.set(entity, &upgraded))
        .unwrap();
    session.commit(patch).await.unwrap();
    drop(session);

    let reopened = Session::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .snapshot()
            .component::<EvolvingComponentV2>(entity)
            .unwrap(),
        Some(upgraded)
    );
}

#[tokio::test]
async fn stale_disjoint_patches_can_rebase_without_hiding_conflicts() {
    let mut session = Session::memory().await.unwrap();
    let mut entities = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            entities = Some((
                edit.add_entity(page, At::End)?,
                edit.add_entity(page, At::End)?,
            ));
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let (left, right) = entities.unwrap();
    let left_patch = base
        .patch(|edit| edit.set(left, &Geometry::rectangle(0.0, 0.0, 10.0, 10.0)))
        .unwrap();
    let right_patch = base
        .patch(|edit| edit.set(right, &Geometry::rectangle(1.0, 1.0, 10.0, 10.0)))
        .unwrap();
    let conflicting = base
        .patch(|edit| edit.set(left, &Geometry::rectangle(2.0, 2.0, 10.0, 10.0)))
        .unwrap();

    let current = session.commit(left_patch).await.unwrap().snapshot;
    let current = session
        .commit(right_patch.rebase_on(&current).unwrap())
        .await
        .unwrap()
        .snapshot;
    assert!(conflicting.rebase_on(&current).is_err());
}

#[tokio::test]
async fn hierarchy_insertion_rebases_across_component_edits_but_not_sibling_changes() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let content = edit.add_text_content(page, At::End)?;
            ids = Some((page, content));
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let (page, content) = ids.unwrap();

    let insert = base
        .patch(|edit| {
            edit.add_entity(page, At::Start)?;
            Ok(())
        })
        .unwrap();
    let component_edit = base
        .patch(|edit| edit.set(content, &source("translated")))
        .unwrap();
    let current = base.preview([&component_edit]).unwrap();
    assert!(insert.rebase_on(&current).is_ok());

    let sibling_insert = base
        .patch(|edit| {
            edit.add_entity(page, At::End)?;
            Ok(())
        })
        .unwrap();
    let current = base.preview([&sibling_insert]).unwrap();
    assert!(insert.rebase_on(&current).is_err());
}

#[tokio::test]
async fn observed_page_subtree_guards_pipeline_rebase_inputs() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let left_page = edit.add_page(PageDraft::new("left", 100.0, 100.0), At::End)?;
            let left_entity = edit.add_text_content(left_page, At::End)?;
            let right_page = edit.add_page(PageDraft::new("right", 100.0, 100.0), At::End)?;
            ids = Some((left_page, left_entity, right_page));
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let (left_page, left_entity, right_page) = ids.unwrap();

    let observed = base
        .patch(|edit| {
            edit.observe_subtree(left_page)?;
            edit.set(left_entity, &source("derived"))
        })
        .unwrap();
    let unrelated = base
        .patch(|edit| edit.set_page(right_page, PageDraft::new("right changed", 100.0, 100.0)))
        .unwrap();
    let current = session.commit(unrelated).await.unwrap().snapshot;
    assert!(observed.rebase_on(&current).is_ok());

    let observed = current
        .patch(|edit| {
            edit.observe_subtree(left_page)?;
            edit.set(left_entity, &source("stale"))
        })
        .unwrap();
    let changed_input = current
        .patch(|edit| edit.set_page(left_page, PageDraft::new("left changed", 100.0, 100.0)))
        .unwrap();
    let current = session.commit(changed_input).await.unwrap().snapshot;
    assert!(observed.rebase_on(&current).is_err());
}

#[tokio::test]
async fn project_and_relation_metadata_use_typed_components() {
    let mut session = Session::memory().await.unwrap();
    let settings = Project {
        source_locale: Some(LanguageTag::new("ja").unwrap()),
        target_locales: vec![LanguageTag::new("en").unwrap()],
    };
    let kind = RelationKind::new("dev.koharu.test.link").unwrap();
    let mut relation = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            edit.set_project(&settings)?;
            let page = edit.add_page(page(), At::End)?;
            let left = edit.add_entity(page, At::End)?;
            let right = edit.add_entity(page, At::End)?;
            let id = edit.add_relation(kind, left, right)?;
            edit.set_relation(
                id,
                &Visibility {
                    origin: Origin::User,
                    visible: true,
                    opacity: 0.5,
                },
            )?;
            relation = Some(id);
            Ok(())
        })
        .unwrap();
    let commit = session.commit(patch).await.unwrap();
    let snapshot = commit.snapshot;
    assert!(
        commit.changes.components.iter().any(|change| {
            change.owner == ComponentOwner::Project && change.kind == Project::KIND
        })
    );
    assert!(commit.changes.components.iter().any(|change| {
        change.owner == ComponentOwner::Relation(relation.unwrap())
            && change.kind == Visibility::KIND
    }));
    assert_eq!(
        snapshot.project_component::<Project>().unwrap(),
        Some(settings)
    );
    assert_eq!(
        snapshot
            .relation(relation.unwrap())
            .unwrap()
            .component::<Visibility>()
            .unwrap()
            .unwrap()
            .opacity,
        0.5
    );
}

#[tokio::test]
async fn producer_reruns_respect_component_ownership() {
    let mut session = Session::memory().await.unwrap();
    let mut entities = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let user = edit.add_text_content(page, At::End)?;
            edit.set(user, &source("user"))?;
            let generated = edit.add_text_content(page, At::End)?;
            entities = Some((user, generated));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let (user, generated) = entities.unwrap();
    let producer = ProducerId::new("dev.koharu.pipeline.ocr").unwrap();
    let generation = Generation::new(producer.clone());
    let mut edit = snapshot.edit_as(generation.clone());
    assert!(matches!(
        edit.set(user, &source("overwrite")),
        Err(Error::Authorship(_))
    ));
    edit.set(generated, &source("generated")).unwrap();
    let snapshot = session
        .commit(edit.finish().unwrap())
        .await
        .unwrap()
        .snapshot;

    let mut rerun = snapshot.edit_as(generation);
    rerun.set(generated, &source("generated again")).unwrap();
    let snapshot = session
        .commit(rerun.finish().unwrap())
        .await
        .unwrap()
        .snapshot;
    let other = Generation::new(ProducerId::new("dev.koharu.pipeline.other-ocr").unwrap());
    let mut other_edit = snapshot.edit_as(other);
    assert!(matches!(
        other_edit.set(generated, &source("wrong owner")),
        Err(Error::Authorship(_))
    ));
}

#[tokio::test]
async fn pipeline_removal_respects_entity_lifecycle_owner() {
    let mut session = Session::memory().await.unwrap();
    let mut page_id = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            page_id = Some(edit.add_page(page(), At::End)?);
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let page = page_id.unwrap();
    let owner = Generation::new(ProducerId::new("dev.koharu.pipeline.detector").unwrap());
    let mut edit = snapshot.edit_as(owner.clone());
    let generated = edit.add_entity(page, At::End).unwrap();
    let snapshot = session
        .commit(edit.finish().unwrap())
        .await
        .unwrap()
        .snapshot;

    let mut other = snapshot.edit_as(Generation::new(
        ProducerId::new("dev.koharu.pipeline.other-detector").unwrap(),
    ));
    assert!(matches!(
        other.remove_entity(generated, RemovePolicy::Cascade),
        Err(Error::Authorship(_))
    ));

    let mut owner_edit = snapshot.edit_as(owner);
    owner_edit
        .remove_entity(generated, RemovePolicy::Cascade)
        .unwrap();
    let snapshot = session
        .commit(owner_edit.finish().unwrap())
        .await
        .unwrap()
        .snapshot;
    assert!(snapshot.entity(generated).is_err());
}

#[tokio::test]
async fn typed_page_entity_and_components_round_trip() {
    let mut session = Session::memory().await.unwrap();
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let text = edit.add_text_content(page, At::End)?;
            edit.set(text, &source("こんにちは"))?;
            Ok(())
        })
        .unwrap();
    let commit = session.commit(patch).await.unwrap();
    let page = commit.snapshot.pages().next().unwrap();
    assert_eq!(page.page().unwrap().label, "page");
    let text = commit.snapshot.children(page.id()).unwrap().next().unwrap();
    assert_eq!(
        commit
            .snapshot
            .component::<SourceText>(text)
            .unwrap()
            .unwrap()
            .text
            .value,
        "こんにちは"
    );
}

#[tokio::test]
async fn text_analysis_content_and_presentation_have_distinct_ownership() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let region = edit.add_analysis_region::<TextRegion>(
                page,
                At::End,
                &Geometry::rectangle(10.0, 20.0, 100.0, 40.0),
                None,
            )?;
            let content = edit.add_text_content(page, At::End)?;
            edit.set(content, &source("source"))?;
            let layer = edit.add_text_layer(
                page,
                At::End,
                content,
                &TextLayout {
                    origin: Origin::User,
                    kind: TextLayoutKind::Paragraph,
                },
            )?;
            edit.relate::<RecognizedFrom>(content, region)?;
            edit.relate::<FitsTo>(layer, region)?;
            ids = Some((content, layer));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let (content, layer) = ids.unwrap();
    assert_eq!(
        snapshot
            .relation_from::<Presents>(layer)
            .unwrap()
            .unwrap()
            .value()
            .target,
        content
    );
    assert!(matches!(
        snapshot.patch(|edit| edit.set(content, &Geometry::rectangle(0.0, 0.0, 10.0, 10.0))),
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn text_group_owns_the_canonical_text_order() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let first_content = edit.add_text_content(page, At::End)?;
            let first = edit.add_text_layer(
                page,
                At::End,
                first_content,
                &TextLayout {
                    origin: Origin::User,
                    kind: TextLayoutKind::Paragraph,
                },
            )?;
            let second_content = edit.add_text_content(page, At::End)?;
            let second = edit.add_text_layer(
                page,
                At::End,
                second_content,
                &TextLayout {
                    origin: Origin::User,
                    kind: TextLayoutKind::Paragraph,
                },
            )?;
            ids = Some((page, first, second));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let (page, first, second) = ids.unwrap();
    let group = snapshot.page(page).unwrap().text_group().unwrap().unwrap();
    assert_eq!(
        group
            .text_layers()
            .unwrap()
            .map(TextLayerRef::id)
            .collect::<Vec<_>>(),
        vec![first, second]
    );

    let patch = snapshot
        .patch(|edit| edit.move_entity(second, Some(group.id()), At::Start))
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    assert_eq!(
        snapshot
            .page(page)
            .unwrap()
            .text_group()
            .unwrap()
            .unwrap()
            .text_layers()
            .unwrap()
            .map(TextLayerRef::id)
            .collect::<Vec<_>>(),
        vec![second, first]
    );
}

#[tokio::test]
async fn typed_relations_enforce_endpoints_and_functional_cardinality() {
    let session = Session::memory().await.unwrap();
    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let region = edit.add_analysis_region::<TextRegion>(
            page,
            At::End,
            &Geometry::rectangle(0.0, 0.0, 10.0, 10.0),
            None,
        )?;
        let layer = edit.add_entity(page, At::End)?;
        edit.set(
            layer,
            &TextLayout {
                origin: Origin::User,
                kind: TextLayoutKind::Paragraph,
            },
        )?;
        edit.relate::<Presents>(layer, region)?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Invalid(_))));

    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let first = edit.add_text_content(page, At::End)?;
        let second = edit.add_text_content(page, At::End)?;
        let layer = edit.add_text_layer(
            page,
            At::End,
            first,
            &TextLayout {
                origin: Origin::User,
                kind: TextLayoutKind::Paragraph,
            },
        )?;
        edit.relate::<Presents>(layer, second)?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Invalid(_))));

    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let bubble = edit.add_analysis_region::<BubbleRegion>(
            page,
            At::End,
            &Geometry::rectangle(0.0, 0.0, 20.0, 20.0),
            None,
        )?;
        let content = edit.add_text_content(page, At::End)?;
        let layer = edit.add_text_layer(
            page,
            At::End,
            content,
            &TextLayout {
                origin: Origin::User,
                kind: TextLayoutKind::Paragraph,
            },
        )?;
        edit.relate::<FitsTo>(layer, bubble)?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Invalid(_))));

    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let text_region = edit.add_analysis_region::<TextRegion>(
            page,
            At::End,
            &Geometry::rectangle(0.0, 0.0, 10.0, 10.0),
            None,
        )?;
        let bubble = edit.add_analysis_region::<BubbleRegion>(
            page,
            At::End,
            &Geometry::rectangle(0.0, 0.0, 20.0, 20.0),
            None,
        )?;
        let content = edit.add_text_content(page, At::End)?;
        let layer = edit.add_text_layer(
            page,
            At::End,
            content,
            &TextLayout {
                origin: Origin::User,
                kind: TextLayoutKind::Paragraph,
            },
        )?;
        edit.relate::<FitsTo>(layer, text_region)?;
        edit.relate::<FlowsIn>(layer, bubble)?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Invalid(_))));

    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let text_region = edit.add_analysis_region::<TextRegion>(
            page,
            At::End,
            &Geometry::rectangle(0.0, 0.0, 10.0, 10.0),
            None,
        )?;
        let content = edit.add_text_content(page, At::End)?;
        let layer = edit.add_text_layer(
            page,
            At::End,
            content,
            &TextLayout {
                origin: Origin::User,
                kind: TextLayoutKind::Paragraph,
            },
        )?;
        edit.relate::<FlowsIn>(layer, text_region)?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn component_changes_cannot_invalidate_incident_typed_relations() {
    let mut session = Session::memory().await.unwrap();
    let mut layer = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let content = edit.add_text_content(page, At::End)?;
            layer = Some(edit.add_text_layer(
                page,
                At::End,
                content,
                &TextLayout {
                    origin: Origin::User,
                    kind: TextLayoutKind::Paragraph,
                },
            )?);
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let result = snapshot.patch(|edit| edit.remove::<TextLayout>(layer.unwrap()));
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn hierarchy_move_and_promote_are_ordered() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let group = edit.add_entity(page, At::End)?;
            let child = edit.add_text_content(group, At::End)?;
            let sibling = edit.add_entity(page, At::End)?;
            ids = Some((page, group, child, sibling));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let (page, group, child, sibling) = ids.unwrap();
    let patch = snapshot
        .patch(|edit| {
            edit.move_entity(sibling, Some(group), At::Start)?;
            edit.remove_entity(group, RemovePolicy::PromoteChildren)
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    assert_eq!(
        snapshot.children(page).unwrap().collect::<Vec<_>>(),
        vec![sibling, child]
    );
    assert_eq!(snapshot.parent(child).unwrap(), Some(page));
    assert!(matches!(
        snapshot.entity(group),
        Err(Error::EntityNotFound(_))
    ));
}

#[tokio::test]
async fn cross_page_subtree_move_preserves_identity_components_relations_and_undo() {
    let mut session = Session::memory().await.unwrap();
    let kind = RelationKind::new("dev.koharu.test.cross-page").unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let source_page = edit.add_page(page(), At::End)?;
            let target_page = edit.add_page(page(), At::End)?;
            let group = edit.add_entity(source_page, At::End)?;
            let child = edit.add_text_content(group, At::End)?;
            edit.set(child, &source("stable"))?;
            let relation = edit.add_relation(kind.clone(), child, target_page)?;
            ids = Some((source_page, target_page, group, child, relation));
            Ok(())
        })
        .unwrap();
    session.commit(create).await.unwrap();
    let (source_page, target_page, group, child, relation) = ids.unwrap();

    let moved = session
        .snapshot()
        .patch(|edit| edit.move_entity(group, Some(target_page), At::End))
        .unwrap();
    let commit = session.commit(moved).await.unwrap();
    assert!(
        commit
            .snapshot
            .children(source_page)
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(commit.snapshot.parent(group).unwrap(), Some(target_page));
    assert_eq!(commit.snapshot.parent(child).unwrap(), Some(group));
    assert_eq!(
        commit
            .snapshot
            .component::<SourceText>(child)
            .unwrap()
            .unwrap()
            .text
            .value,
        "stable"
    );
    assert_eq!(
        commit.snapshot.relation(relation).unwrap().value().source,
        child
    );

    let undone = session.undo(commit.revision).await.unwrap().snapshot;
    assert_eq!(undone.parent(group).unwrap(), Some(source_page));
    assert_eq!(undone.parent(child).unwrap(), Some(group));
    assert_eq!(undone.relation(relation).unwrap().value().source, child);
}

#[tokio::test]
async fn relations_are_records_with_typed_adjacency() {
    let mut session = Session::memory().await.unwrap();
    let kind = RelationKind::new("dev.koharu.test.reading-order").unwrap();
    let mut ids = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let first = edit.add_entity(page, At::End)?;
            let second = edit.add_entity(page, At::End)?;
            let relation = edit.add_relation(kind.clone(), first, second)?;
            ids = Some((first, second, relation));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let (first, second, relation) = ids.unwrap();
    assert_eq!(
        snapshot
            .relations_from(first, Some(&kind))
            .next()
            .unwrap()
            .id(),
        relation
    );
    assert_eq!(
        snapshot
            .relations_to(second, None)
            .next()
            .unwrap()
            .value()
            .source,
        first
    );
    assert!(matches!(
        snapshot.patch(|edit| edit.remove_entity(first, RemovePolicy::RejectNonEmpty)),
        Err(Error::IncidentRelations(id)) if id == first
    ));
    let patch = snapshot
        .patch(|edit| edit.remove_entity(first, RemovePolicy::Cascade))
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    assert!(snapshot.relation(relation).is_err());
}

#[tokio::test]
async fn translation_requires_source_text() {
    let session = Session::memory().await.unwrap();
    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let entity = edit.add_entity(page, At::End)?;
        edit.set(
            entity,
            &Translation {
                text: Authored::user("hello".to_owned()),
                language: Some(LanguageTag::new("en").unwrap()),
            },
        )
    });
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn text_components_have_one_value_per_entity() {
    let session = Session::memory().await.unwrap();
    let result = session.snapshot().patch(|edit| {
        let page = edit.add_page(page(), At::End)?;
        let entity = edit.add_text_content(page, At::End)?;
        edit.set(entity, &source("source"))?;
        edit.set(
            entity,
            &Translation {
                text: Authored::user("translation".to_owned()),
                language: Some(LanguageTag::new("en").unwrap()),
            },
        )
    });
    assert!(result.is_ok());
}

#[tokio::test]
async fn independent_pipeline_components_rebase() {
    let mut session = Session::memory().await.unwrap();
    let mut entities = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let content = edit.add_text_content(page, At::End)?;
            edit.set(content, &source("source"))?;
            let layer = edit.add_text_layer(
                page,
                At::End,
                content,
                &TextLayout {
                    origin: Origin::User,
                    kind: TextLayoutKind::Paragraph,
                },
            )?;
            entities = Some((content, layer));
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let (content, layer) = entities.unwrap();
    let translation = base
        .patch(|edit| {
            edit.set(
                content,
                &Translation {
                    text: Authored::user("translation".to_owned()),
                    language: Some(LanguageTag::new("en").unwrap()),
                },
            )
        })
        .unwrap();
    let typography = base
        .patch(|edit| {
            edit.set(
                layer,
                &Typography {
                    origin: Origin::User,
                    preferred_font: Some("Inter".to_owned()),
                    font_weight: Some(500),
                    font_style: Some(FontStyle::Italic),
                    size: Some(24.0),
                    auto_fit: false,
                    color: Some([1, 2, 3, 255]),
                    stroke_color: None,
                    stroke_width: None,
                    alignment: Some(TextAlignment::Center),
                    writing_mode: None,
                    extensions: BTreeMap::new(),
                },
            )
        })
        .unwrap();
    let current = session.commit(translation).await.unwrap().snapshot;
    let typography = typography.rebase_on(&current).unwrap();
    let snapshot = session.commit(typography).await.unwrap().snapshot;
    assert!(
        snapshot
            .component::<Translation>(content)
            .unwrap()
            .is_some()
    );
    assert!(snapshot.component::<Typography>(layer).unwrap().is_some());
}

#[tokio::test]
async fn descendant_scene_patch_rebases_after_ancestor_commit() {
    let mut session = Session::memory().await.unwrap();
    let base = session.snapshot();
    let mut edit = base.edit();
    let page = edit.add_page(page(), At::End).unwrap();
    let ancestor = edit.finish().unwrap();
    let preview = base.preview([&ancestor]).unwrap();
    let descendant = preview
        .patch(|edit| {
            let entity = edit.add_text_content(page, At::End)?;
            edit.set(entity, &source("late"))
        })
        .unwrap();
    assert!(base.preview([&ancestor, &descendant]).is_ok());
    let current = session.commit(ancestor).await.unwrap().snapshot;
    let descendant = descendant.rebase_on(&current).unwrap();
    let snapshot = session.commit(descendant).await.unwrap().snapshot;
    assert_eq!(snapshot.children(page).unwrap().len(), 1);
}

#[tokio::test]
async fn assets_attach_bytes_without_decoding() {
    let mut session = Session::memory().await.unwrap();
    let role = AssetRole::new("source").unwrap();
    let mut entity = None;
    let patch = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            entity = Some(page);
            edit.set_asset(
                page,
                &role,
                AssetInput::new(
                    Arc::<[u8]>::from(&b"encoded image"[..]),
                    "image/test",
                    AssetMetadata {
                        width: Some(10),
                        height: Some(20),
                        attributes: BTreeMap::new(),
                    },
                ),
            )
        })
        .unwrap();
    let snapshot = session.commit(patch).await.unwrap().snapshot;
    let asset = snapshot.asset(entity.unwrap(), &role).unwrap().unwrap();
    assert_eq!(
        snapshot.read_blob(asset.blob).await.unwrap().as_ref(),
        b"encoded image"
    );
}

#[tokio::test]
async fn repeated_asset_writes_rebase_over_an_unrelated_page_edit() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let first_page = edit.add_page(page(), At::End)?;
            let second_page = edit.add_page(page(), At::End)?;
            ids = Some((first_page, second_page));
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let (first_page, second_page) = ids.unwrap();

    let assets = base
        .patch(|edit| {
            for role in ["text-mask", "bubble-mask"] {
                edit.set_asset(
                    second_page,
                    &AssetRole::new(role)?,
                    AssetInput::new(
                        Arc::<[u8]>::from(role.as_bytes()),
                        "image/test",
                        AssetMetadata {
                            width: None,
                            height: None,
                            attributes: BTreeMap::new(),
                        },
                    ),
                )?;
            }
            Ok(())
        })
        .unwrap();
    let unrelated = base
        .patch(|edit| {
            edit.set_asset(
                first_page,
                &AssetRole::new("source")?,
                AssetInput::new(
                    Arc::<[u8]>::from(&b"source"[..]),
                    "image/test",
                    AssetMetadata {
                        width: None,
                        height: None,
                        attributes: BTreeMap::new(),
                    },
                ),
            )
        })
        .unwrap();
    let current = session.commit(unrelated).await.unwrap().snapshot;

    let assets = assets.rebase_on(&current).unwrap();
    let snapshot = session.commit(assets).await.unwrap().snapshot;
    for role in ["text-mask", "bubble-mask"] {
        assert!(
            snapshot
                .asset(second_page, &AssetRole::new(role).unwrap())
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn disk_scene_reopens_and_validates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.khrproj");
    {
        let mut session = Session::create(&path).await.unwrap();
        let patch = session
            .snapshot()
            .patch(|edit| edit.add_page(page(), At::End).map(|_| ()))
            .unwrap();
        session.commit(patch).await.unwrap();
    }
    let session = Session::open(&path).await.unwrap();
    assert_eq!(session.snapshot().pages().len(), 1);
}

#[tokio::test]
async fn reopen_updates_page_local_component_queries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("refresh.khrproj");
    let mut writer = Session::create(&path).await.unwrap();
    let mut entity = None;
    let create = writer
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            entity = Some(edit.add_text_content(page, At::End)?);
            Ok(())
        })
        .unwrap();
    writer.commit(create).await.unwrap();
    let entity = entity.unwrap();

    let update = writer
        .snapshot()
        .patch(|edit| edit.set(entity, &source("refreshed")))
        .unwrap();
    writer.commit(update).await.unwrap();
    drop(writer);

    let reader = Session::open(&path).await.unwrap();
    let snapshot = reader.snapshot();
    assert_eq!(
        snapshot
            .component::<SourceText>(entity)
            .unwrap()
            .unwrap()
            .text
            .value,
        "refreshed"
    );
}

#[tokio::test]
async fn pipeline_component_removal_and_relation_lifecycle_respect_ownership() {
    let mut session = Session::memory().await.unwrap();
    let role = AssetRole::new("source").unwrap();
    let relation_kind = RelationKind::new("dev.koharu.test.association").unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let left = edit.add_text_content(page, At::End)?;
            let right = edit.add_entity(page, At::End)?;
            let analysis = edit.add_entity(page, At::End)?;
            edit.set(left, &source("user text"))?;
            edit.set(analysis, &Geometry::rectangle(0.0, 0.0, 10.0, 10.0))?;
            edit.set(
                analysis,
                &Region {
                    origin: Origin::User,
                    kind: TextRegion::kind(),
                    label: None,
                },
            )?;
            edit.set_asset(
                left,
                &role,
                AssetInput::new(
                    Arc::<[u8]>::from(&b"user asset"[..]),
                    "image/test",
                    AssetMetadata {
                        width: Some(1),
                        height: Some(1),
                        attributes: BTreeMap::new(),
                    },
                ),
            )?;
            let relation = edit.add_relation(relation_kind.clone(), left, right)?;
            ids = Some((left, analysis, relation));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(create).await.unwrap().snapshot;
    let (entity, analysis_entity, relation) = ids.unwrap();
    let generation = Generation::new(ProducerId::new("dev.koharu.pipeline.detector").unwrap());
    let mut pipeline = snapshot.edit_as(generation.clone());
    assert!(matches!(
        pipeline.remove::<SourceText>(entity),
        Err(Error::Authorship(_))
    ));
    assert!(matches!(
        pipeline.remove_asset(entity, &role),
        Err(Error::Authorship(_))
    ));
    assert!(matches!(
        pipeline.remove_relation(relation),
        Err(Error::Authorship(_))
    ));

    let analysis = DetectionAnalysis {
        origin: Origin::User,
        labels: vec![DetectionLabel {
            kind: RegionKind::new("dev.koharu.region.text").unwrap(),
            confidence: 0.9,
        }],
    };
    pipeline
        .set(analysis_entity, &analysis)
        .expect("new pipeline analysis is stamped by the edit context");
    let snapshot = session
        .commit(pipeline.finish().unwrap())
        .await
        .unwrap()
        .snapshot;
    let stored = snapshot
        .component::<DetectionAnalysis>(analysis_entity)
        .unwrap()
        .unwrap();
    assert!(matches!(stored.origin, Origin::Generated(_)));

    let mut other = snapshot.edit_as(Generation::new(
        ProducerId::new("dev.koharu.pipeline.other-detector").unwrap(),
    ));
    assert!(matches!(
        other.set(analysis_entity, &analysis),
        Err(Error::Authorship(_))
    ));
    assert!(matches!(
        other.remove::<DetectionAnalysis>(analysis_entity),
        Err(Error::Authorship(_))
    ));
}

#[tokio::test]
async fn pipeline_queries_and_analysis_components_are_semantic_and_ordered() {
    let mut session = Session::memory().await.unwrap();
    let mut ids = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let first = edit.add_text_content(page, At::End)?;
            let nested = edit.add_text_content(first, At::End)?;
            let second = edit.add_entity(page, At::End)?;
            edit.set(first, &source("first"))?;
            edit.set(nested, &source("nested"))?;
            edit.set(second, &Geometry::rectangle(0.0, 0.0, 10.0, 10.0))?;
            edit.set(
                second,
                &Region {
                    origin: Origin::User,
                    kind: TextRegion::kind(),
                    label: None,
                },
            )?;
            ids = Some((page, first, nested, second));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(create).await.unwrap().snapshot;
    let (page, first, nested, second) = ids.unwrap();
    assert_eq!(
        snapshot.entities().map(EntityRef::id).collect::<Vec<_>>(),
        vec![page, first, nested, second]
    );
    assert_eq!(
        snapshot
            .entities_with::<SourceText>()
            .unwrap()
            .map(EntityRef::id)
            .collect::<Vec<_>>(),
        vec![first, nested]
    );
    assert_eq!(
        snapshot
            .subtree(first)
            .unwrap()
            .map(EntityRef::id)
            .collect::<Vec<_>>(),
        vec![first, nested]
    );
    assert_eq!(
        snapshot
            .descendants(first)
            .unwrap()
            .map(EntityRef::id)
            .collect::<Vec<_>>(),
        vec![nested]
    );

    let generation = Generation::new(ProducerId::new("dev.koharu.pipeline.ocr").unwrap());
    let mut edit = snapshot.edit_as(generation);
    edit.set(
        second,
        &OcrAnalysis {
            origin: Origin::User,
            direction: TextDirection::Vertical,
            confidence: Some(0.95),
            line_boundaries: vec![[
                Point { x: 0.0, y: 0.0 },
                Point { x: 1.0, y: 0.0 },
                Point { x: 1.0, y: 1.0 },
                Point { x: 0.0, y: 1.0 },
            ]],
        },
    )
    .unwrap();
    let snapshot = session
        .commit(edit.finish().unwrap())
        .await
        .unwrap()
        .snapshot;
    assert!(matches!(
        snapshot
            .component::<OcrAnalysis>(second)
            .unwrap()
            .unwrap()
            .origin,
        Origin::Generated(_)
    ));
}

#[tokio::test]
async fn scene_patch_exposes_stable_identity_and_base() {
    let mut session = Session::memory().await.unwrap();
    let mut entity = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            entity = Some(edit.add_text_content(page, At::End)?);
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let entity = entity.unwrap();
    let patch = base
        .patch(|edit| edit.set(entity, &source("identity")))
        .unwrap();
    assert_eq!(patch.project_id(), base.project_id());
    assert_eq!(patch.base_revision(), base.revision());
    assert_eq!(patch.fingerprint(), patch.clone().fingerprint());
    assert_ne!(
        patch.fingerprint(),
        patch
            .clone()
            .with_label("different commit label")
            .fingerprint()
    );
}

#[tokio::test]
async fn component_snapshots_clone_only_the_touched_page() {
    let mut session = Session::memory().await.unwrap();
    let mut entity = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            entity = Some(edit.add_text_content(page, At::End)?);
            Ok(())
        })
        .unwrap();
    let base = session.commit(create).await.unwrap().snapshot;
    let entity = entity.unwrap();
    let page = base.state.page_for(entity).unwrap();
    let base_page = base.state.pages[&page].clone();

    let patch = base
        .patch(|edit| edit.set(entity, &source("cached")))
        .unwrap();
    let preview = base.preview([&patch]).unwrap();
    assert!(!Arc::ptr_eq(&preview.state.pages[&page], &base_page));
    assert_eq!(preview.entities_with::<SourceText>().unwrap().len(), 1);
    let committed = session.commit(patch).await.unwrap().snapshot;
    assert!(!Arc::ptr_eq(&committed.state.pages[&page], &base_page));
    let first = session.snapshot();
    let second = session.snapshot();
    assert!(Arc::ptr_eq(&first.state, &second.state));
}

#[tokio::test]
async fn user_promotion_protects_generated_entity_and_relation_lifecycle() {
    let mut session = Session::memory().await.unwrap();
    let mut endpoints = None;
    let create = session
        .snapshot()
        .patch(|edit| {
            let page = edit.add_page(page(), At::End)?;
            let left = edit.add_entity(page, At::End)?;
            let right = edit.add_entity(page, At::End)?;
            endpoints = Some((left, right));
            Ok(())
        })
        .unwrap();
    let snapshot = session.commit(create).await.unwrap().snapshot;
    let (left, right) = endpoints.unwrap();
    let generation = Generation::new(ProducerId::new("dev.koharu.pipeline.detector").unwrap());
    let mut generated = snapshot.edit_as(generation.clone());
    let entity = generated.add_entity(left, At::End).unwrap();
    let relation = generated
        .add_relation(
            RelationKind::new("dev.koharu.test.generated-link").unwrap(),
            entity,
            right,
        )
        .unwrap();
    let snapshot = session
        .commit(generated.finish().unwrap())
        .await
        .unwrap()
        .snapshot;

    let promoted = snapshot
        .patch(|edit| {
            edit.promote_entity_to_user(entity)?;
            edit.promote_relation_to_user(relation)
        })
        .unwrap();
    let snapshot = session.commit(promoted).await.unwrap().snapshot;
    let mut rerun = snapshot.edit_as(generation);
    assert!(matches!(
        rerun.remove_relation(relation),
        Err(Error::Authorship(_))
    ));
    assert!(matches!(
        rerun.remove_entity(entity, RemovePolicy::Cascade),
        Err(Error::Authorship(_))
    ));
}

#[tokio::test]
async fn dropped_history_is_not_undoable() {
    let mut session = Session::memory().await.unwrap();
    let mut revisions = Vec::new();
    for _ in 0..5 {
        let patch = session
            .snapshot()
            .patch(|edit| {
                edit.add_page(page(), At::End)?;
                Ok(())
            })
            .unwrap();
        revisions.push(session.commit(patch).await.unwrap().revision);
    }

    assert_eq!(session.drop_history([revisions[1], revisions[3]]), 2);
    assert_eq!(session.drop_history([revisions[1]]), 0);

    let Err(Error::Invalid(message)) = session.undo(revisions[1]).await else {
        panic!("undoing a dropped revision must fail");
    };
    assert!(message.contains("not undoable"));

    let undone = session.undo(revisions[0]).await.unwrap();
    assert_eq!(undone.snapshot.pages().len(), 4);
}
