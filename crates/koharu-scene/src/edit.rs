use std::collections::{BTreeSet, HashSet};

use smallvec::SmallVec;

use crate::{
    Asset, AssetInput, AssetRole, BlobId, ComponentOwner, EntityId, EntityOrigin, Error,
    Generation, Group, Origin, Page, PageDraft, Patch, Relation, RelationId, RelationKind,
    RelationSpec, Result, Snapshot, TextGroup, Visibility,
    component::{Component, ComponentKey, ComponentRecord, ValidationContext, decode, encode, key},
    components::Assets,
    patch::{Observation, Operation},
    schema,
    state::{Components, State, store_components},
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum At {
    Start,
    End,
    Before(EntityId),
    After(EntityId),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RemovePolicy {
    RejectNonEmpty,
    Cascade,
    PromoteChildren,
}

pub struct Edit {
    base: Snapshot,
    state: State,
    observations: BTreeSet<Observation>,
    operations: Vec<Operation>,
    attachments: Vec<(BlobId, bytes::Bytes)>,
    validate_entities: HashSet<EntityId>,
    generation: Option<Generation>,
}

impl Edit {
    pub(crate) fn new(base: Snapshot, generation: Option<Generation>) -> Self {
        Self {
            state: (*base.state).clone(),
            base,
            observations: BTreeSet::new(),
            operations: Vec::new(),
            attachments: Vec::new(),
            validate_entities: HashSet::new(),
            generation,
        }
    }

    /// Observes the page that owns this subtree. Page epochs include hierarchy,
    /// component, and incident-relation changes, making the check constant-time.
    pub fn observe_subtree(&mut self, root: EntityId) -> Result<()> {
        let page = self.state.page_for(root)?;
        self.observations.insert(Observation::Page {
            page,
            epoch: self.state.pages.get(&page).map(|page| page.epoch),
        });
        Ok(())
    }

    pub fn observe_project<T: Component>(&mut self) -> Result<()> {
        self.observe_component(ComponentOwner::Project, key::<T>()?)
    }

    pub fn observe<T: Component>(&mut self, entity: EntityId) -> Result<()> {
        self.observe_component(ComponentOwner::Entity(entity), key::<T>()?)
    }

    /// Observes the complete typed asset collection owned by an entity.
    pub fn observe_assets(&mut self, entity: EntityId) -> Result<()> {
        self.observe_component(ComponentOwner::Entity(entity), key::<Assets>()?)
    }

    pub fn add_page(&mut self, page: PageDraft, at: At) -> Result<EntityId> {
        self.observe_page_order_write();
        let id = EntityId::new();
        let position = resolve_position(&self.state.page_order, at, None)?;
        let components = self.entity_components([
            self.encoded(&Page::from(page))?,
            self.encoded(&EntityOrigin {
                origin: self.lifecycle_origin(),
            })?,
        ]);
        let stored = store_components(&components);
        self.state.insert_page(id, position, components)?;
        self.operations.push(Operation::InsertPage {
            id,
            position: position as u32,
            components: stored,
        });
        Ok(id)
    }

    pub fn add_entity(&mut self, parent: EntityId, at: At) -> Result<EntityId> {
        let page = self.state.page_for(parent)?;
        self.observe_children_write(parent)?;
        let siblings = self
            .state
            .page(page)?
            .entity(parent)?
            .children
            .iter()
            .map(|key| self.state.pages[&page].entities[*key].id)
            .collect::<Vec<_>>();
        let position = resolve_position(&siblings, at, None)?;
        let id = EntityId::new();
        let components = self.entity_components([self.encoded(&EntityOrigin {
            origin: self.lifecycle_origin(),
        })?]);
        let stored = store_components(&components);
        self.state
            .insert_entity(page, id, parent, position, components)?;
        self.operations.push(Operation::InsertEntity {
            page,
            id,
            parent,
            position: position as u32,
            components: stored,
        });
        Ok(id)
    }

    pub fn set_page(&mut self, entity: EntityId, page: PageDraft) -> Result<()> {
        if !self.state.pages.contains_key(&entity) {
            return Err(Error::EntityNotFound(entity));
        }
        self.replace_component(
            ComponentOwner::Entity(entity),
            key::<Page>()?,
            Some(self.encode_value(&Page::from(page))?),
        )
    }

    pub fn move_entity(
        &mut self,
        entity: EntityId,
        parent: Option<EntityId>,
        at: At,
    ) -> Result<()> {
        let page = self.state.page_for(entity)?;
        if entity == page {
            if parent.is_some() {
                return Err(Error::invalid("pages must remain at the project root"));
            }
            let before = self.state.parent_and_position(entity)?.1;
            let position = resolve_position(&self.state.page_order, at, Some(entity))?;
            if before == position {
                return Ok(());
            }
            self.observe_page_order_write();
            self.state.move_page(entity, position)?;
            self.operations.push(Operation::MovePage {
                id: entity,
                before: before as u32,
                after: position as u32,
            });
            return Ok(());
        }

        let parent = parent.ok_or_else(|| Error::invalid("only pages may use the project root"))?;
        let target_page = self.state.page_for(parent)?;
        if target_page == page
            && self
                .state
                .page(page)?
                .descendants(entity)?
                .contains(&parent)
        {
            return Err(Error::HierarchyCycle);
        }
        let (before_parent, before_position) = self.state.parent_and_position(entity)?;
        let before_parent = before_parent.expect("non-page entity has a parent");
        let siblings = self
            .state
            .page(target_page)?
            .entity(parent)?
            .children
            .iter()
            .map(|key| self.state.pages[&target_page].entities[*key].id)
            .collect::<Vec<_>>();
        let position = resolve_position(&siblings, at, Some(entity))?;
        if before_parent == parent && before_position == position {
            return Ok(());
        }
        self.observe_children_write(before_parent)?;
        self.observe_children_write(parent)?;
        self.state.move_entity(entity, parent, position)?;
        self.validate_entities
            .extend([entity, before_parent, parent]);
        self.operations.push(Operation::MoveEntity {
            id: entity,
            before_page: page,
            before_parent,
            before_position: before_position as u32,
            after_page: target_page,
            after_parent: parent,
            after_position: position as u32,
        });
        Ok(())
    }

    pub fn remove_entity(&mut self, entity: EntityId, policy: RemovePolicy) -> Result<()> {
        let page = self.state.page_for(entity)?;
        if entity != page
            && self
                .state
                .component(entity, &key::<TextGroup>()?)?
                .is_some()
        {
            return Err(Error::invalid("the page text group cannot be removed"));
        }
        self.observe_page_write(page);
        if entity == page {
            self.observe_page_order_write();
        }
        let subtree = self.state.page(page)?.descendants(entity)?;
        for id in &subtree {
            self.validate_lifecycle_removal(*id)?;
        }
        let children = self
            .state
            .page(page)?
            .entity(entity)?
            .children
            .iter()
            .map(|key| self.state.pages[&page].entities[*key].id)
            .collect::<Vec<_>>();
        if policy == RemovePolicy::RejectNonEmpty && !children.is_empty() {
            return Err(Error::NonEmptyEntity(entity));
        }
        if policy == RemovePolicy::PromoteChildren && entity == page && !children.is_empty() {
            return Err(Error::invalid("page children cannot be promoted"));
        }
        let incident = subtree
            .iter()
            .flat_map(|id| self.incident_relations(*id))
            .collect::<BTreeSet<_>>();
        if policy != RemovePolicy::Cascade && !incident.is_empty() {
            return Err(Error::IncidentRelations(entity));
        }
        if policy == RemovePolicy::Cascade {
            for id in &subtree {
                self.validate_entities.remove(id);
            }
        } else {
            self.validate_entities.remove(&entity);
        }

        match policy {
            RemovePolicy::RejectNonEmpty => self.remove_leaf(entity),
            RemovePolicy::PromoteChildren => {
                let (parent, position) = self.state.parent_and_position(entity)?;
                let parent = parent.ok_or_else(|| Error::invalid("page cannot be promoted"))?;
                for (offset, child) in children.into_iter().enumerate() {
                    self.move_to_position(child, parent, position + offset)?;
                }
                self.remove_leaf(entity)
            }
            RemovePolicy::Cascade => {
                for relation in incident {
                    self.remove_relation(relation)?;
                }
                for id in subtree.into_iter().rev() {
                    self.remove_leaf(id)?;
                }
                Ok(())
            }
        }
    }

    pub fn promote_entity_to_user(&mut self, entity: EntityId) -> Result<()> {
        if self.generation.is_some() {
            return Err(Error::Authorship(
                "pipeline edits cannot claim user entity ownership".to_owned(),
            ));
        }
        self.replace_component(
            ComponentOwner::Entity(entity),
            key::<EntityOrigin>()?,
            Some(self.encode_value(&EntityOrigin {
                origin: Origin::User,
            })?),
        )
    }

    pub fn set<T: Component>(&mut self, entity: EntityId, value: &T) -> Result<()> {
        if !self.state.contains_entity(entity) {
            return Err(Error::EntityNotFound(entity));
        }
        if matches!(
            T::KIND,
            Relation::KIND | Page::KIND | EntityOrigin::KIND | TextGroup::KIND
        ) {
            return Err(Error::invalid(
                "structural components use dedicated scene methods",
            ));
        }
        let value = self.prepare_value(ComponentOwner::Entity(entity), value)?;
        self.replace_component(
            ComponentOwner::Entity(entity),
            key::<T>()?,
            Some(self.encode_value(&value)?),
        )?;
        self.validate_entities.insert(entity);
        Ok(())
    }

    pub fn set_project<T: Component>(&mut self, value: &T) -> Result<()> {
        if matches!(
            T::KIND,
            Relation::KIND | Page::KIND | EntityOrigin::KIND | Group::KIND | TextGroup::KIND
        ) {
            return Err(Error::invalid("project cannot carry structural components"));
        }
        let value = self.prepare_value(ComponentOwner::Project, value)?;
        self.replace_component(
            ComponentOwner::Project,
            key::<T>()?,
            Some(self.encode_value(&value)?),
        )
    }

    pub fn remove_project<T: Component>(&mut self) -> Result<()> {
        self.validate_removal_authorship::<T>(ComponentOwner::Project)?;
        self.replace_component(ComponentOwner::Project, key::<T>()?, None)
    }

    pub fn remove<T: Component>(&mut self, entity: EntityId) -> Result<()> {
        if matches!(
            T::KIND,
            Relation::KIND | Page::KIND | EntityOrigin::KIND | TextGroup::KIND
        ) {
            return Err(Error::invalid(
                "required structural component cannot be removed",
            ));
        }
        self.validate_removal_authorship::<T>(ComponentOwner::Entity(entity))?;
        self.replace_component(ComponentOwner::Entity(entity), key::<T>()?, None)?;
        self.validate_entities.insert(entity);
        Ok(())
    }

    pub fn set_asset(
        &mut self,
        entity: EntityId,
        role: &AssetRole,
        value: AssetInput,
    ) -> Result<()> {
        let bytes = bytes::Bytes::from_owner(value.bytes);
        let blob = BlobId::for_bytes(&bytes);
        self.attachments.push((blob, bytes));
        let owner = ComponentOwner::Entity(entity);
        let key = key::<Assets>()?;
        let previous = self.component(owner, &key)?;
        let mut assets = previous
            .map(|value| {
                let record_exists = |id| self.state.contains_entity(id);
                let blob_exists = |_id| true;
                decode::<Assets>(value, &ValidationContext::new(&record_exists, &blob_exists))
            })
            .transpose()?
            .unwrap_or_default();
        if let Some(existing) = assets.values.get(role) {
            self.validate_origin_removal(Some(&existing.origin))?;
        }
        self.observe_component(owner, key.clone())?;
        assets.values.insert(
            role.clone(),
            Asset {
                origin: self.lifecycle_origin(),
                blob,
                media_type: value.media_type,
                metadata: value.metadata,
            },
        );
        let record = self.encode_value(&assets)?;
        self.replace_component(owner, key, Some(record))
    }

    pub fn remove_asset(&mut self, entity: EntityId, role: &AssetRole) -> Result<()> {
        let owner = ComponentOwner::Entity(entity);
        let key = key::<Assets>()?;
        let previous = self.component(owner, &key)?;
        let Some(mut assets) = previous
            .map(|value| {
                let record_exists = |id| self.state.contains_entity(id);
                let blob_exists = |_id| true;
                decode::<Assets>(value, &ValidationContext::new(&record_exists, &blob_exists))
            })
            .transpose()?
        else {
            return Ok(());
        };
        let Some(existing) = assets.values.get(role) else {
            return Ok(());
        };
        self.validate_origin_removal(Some(&existing.origin))?;
        self.observe_component(owner, key.clone())?;
        assets.values.remove(role);
        if assets.values.is_empty() {
            self.replace_component(owner, key, None)
        } else {
            let record = self.encode_value(&assets)?;
            self.replace_component(owner, key, Some(record))
        }
    }

    pub fn add_relation(
        &mut self,
        kind: RelationKind,
        source: EntityId,
        target: EntityId,
    ) -> Result<RelationId> {
        if !self.state.contains_entity(source) {
            return Err(Error::EntityNotFound(source));
        }
        if !self.state.contains_entity(target) {
            return Err(Error::EntityNotFound(target));
        }
        let id = RelationId::new();
        let value = Relation {
            origin: self.lifecycle_origin(),
            kind,
            source,
            target,
        };
        let record_exists = |id| self.state.contains_entity(id);
        let blob_exists = |id| {
            self.attachments.iter().any(|(blob, _)| *blob == id)
                || self.base.storage.blobs().contains(id)
        };
        schema::validate_new_relation(
            &self.state,
            &value,
            &ValidationContext::new(&record_exists, &blob_exists),
        )?;
        self.state.insert_relation(id, value.clone())?;
        self.operations.push(Operation::InsertRelation {
            id,
            value,
            components: Vec::new(),
        });
        Ok(id)
    }

    pub fn relate<R: RelationSpec>(
        &mut self,
        source: EntityId,
        target: EntityId,
    ) -> Result<RelationId> {
        self.add_relation(R::kind(), source, target)
    }

    pub fn remove_relation(&mut self, id: RelationId) -> Result<()> {
        let relation = self
            .state
            .relations
            .get(&id)
            .cloned()
            .ok_or(Error::RelationNotFound(id))?;
        self.validate_origin_removal(Some(&relation.value.origin))?;
        let components = store_components(&relation.components);
        let value = relation.value.clone();
        self.state.remove_relation(id)?;
        self.operations.push(Operation::RemoveRelation {
            id,
            value,
            components,
        });
        Ok(())
    }

    pub fn promote_relation_to_user(&mut self, id: RelationId) -> Result<()> {
        if self.generation.is_some() {
            return Err(Error::Authorship(
                "pipeline edits cannot claim user relation ownership".to_owned(),
            ));
        }
        let before = self
            .state
            .relations
            .get(&id)
            .ok_or(Error::RelationNotFound(id))?
            .value
            .clone();
        let after = Relation {
            origin: Origin::User,
            ..before.clone()
        };
        self.state.set_relation_value(id, after.clone())?;
        self.operations
            .push(Operation::ReplaceRelation { id, before, after });
        Ok(())
    }

    pub fn set_relation<T: Component>(&mut self, relation: RelationId, value: &T) -> Result<()> {
        if !self.state.relations.contains_key(&relation) {
            return Err(Error::RelationNotFound(relation));
        }
        if matches!(
            T::KIND,
            Relation::KIND | Page::KIND | EntityOrigin::KIND | Group::KIND | TextGroup::KIND
        ) {
            return Err(Error::invalid(
                "relation structural component is managed directly",
            ));
        }
        let owner = ComponentOwner::Relation(relation);
        let value = self.prepare_value(owner, value)?;
        self.replace_component(owner, key::<T>()?, Some(self.encode_value(&value)?))
    }

    pub fn remove_relation_component<T: Component>(&mut self, relation: RelationId) -> Result<()> {
        let owner = ComponentOwner::Relation(relation);
        self.validate_removal_authorship::<T>(owner)?;
        self.replace_component(owner, key::<T>()?, None)
    }

    pub fn finish(self) -> Result<Patch> {
        for entity in &self.validate_entities {
            schema::validate_entity(&self.state, *entity)?;
        }
        let relations = self
            .validate_entities
            .iter()
            .flat_map(|entity| {
                self.state
                    .outgoing
                    .get(entity)
                    .into_iter()
                    .chain(self.state.incoming.get(entity))
                    .flat_map(|relations| relations.iter().copied())
            })
            .collect::<HashSet<_>>();
        let record_exists = |id| self.state.contains_entity(id);
        let blob_exists = |id| {
            self.attachments.iter().any(|(blob, _)| *blob == id)
                || self.base.storage.blobs().contains(id)
        };
        let context = ValidationContext::new(&record_exists, &blob_exists);
        for relation in relations {
            schema::validate_relation(
                &self.state,
                &self
                    .state
                    .relations
                    .get(&relation)
                    .ok_or(Error::RelationNotFound(relation))?
                    .value,
                &context,
            )?;
        }
        let patch = Patch::new(
            &self.base,
            self.state,
            self.observations.into_iter().collect(),
            self.operations,
            self.attachments,
            None,
        )?;
        Ok(patch)
    }

    fn move_to_position(
        &mut self,
        entity: EntityId,
        parent: EntityId,
        position: usize,
    ) -> Result<()> {
        let (before_parent, before_position) = self.state.parent_and_position(entity)?;
        let before_parent = before_parent.expect("promoted child is not a page");
        self.state.move_entity(entity, parent, position)?;
        self.operations.push(Operation::MoveEntity {
            id: entity,
            before_page: self.state.page_for(parent)?,
            before_parent,
            before_position: before_position as u32,
            after_page: self.state.page_for(parent)?,
            after_parent: parent,
            after_position: position as u32,
        });
        Ok(())
    }

    fn remove_leaf(&mut self, entity: EntityId) -> Result<()> {
        let page = self.state.page_for(entity)?;
        let (parent, position) = self.state.parent_and_position(entity)?;
        if entity == page {
            let page_state = self.state.page(page)?;
            let components = store_components(&page_state.entities[page_state.root].components);
            self.state.remove_page(entity)?;
            self.operations.push(Operation::RemovePage {
                id: entity,
                position: position as u32,
                components,
            });
        } else {
            let parent = parent.expect("non-page entity has a parent");
            let components = store_components(&self.state.entity(entity)?.components);
            self.state.remove_leaf(entity)?;
            self.operations.push(Operation::RemoveEntity {
                page,
                id: entity,
                parent,
                position: position as u32,
                components,
            });
        }
        Ok(())
    }

    fn replace_component(
        &mut self,
        owner: ComponentOwner,
        key: ComponentKey,
        after: Option<ComponentRecord>,
    ) -> Result<()> {
        let before = self.component(owner, &key)?.cloned();
        if before == after {
            return Ok(());
        }
        match (owner, after.clone()) {
            (ComponentOwner::Project, Some(value)) => {
                self.state.set_project_component(key.clone(), value);
            }
            (ComponentOwner::Project, None) => {
                self.state.remove_project_component(&key);
            }
            (ComponentOwner::Entity(id), Some(value)) => {
                self.state.set_entity_component(id, key.clone(), value)?;
            }
            (ComponentOwner::Entity(id), None) => {
                self.state.remove_entity_component(id, &key)?;
            }
            (ComponentOwner::Relation(id), Some(value)) => {
                self.state.set_relation_component(id, key.clone(), value)?;
            }
            (ComponentOwner::Relation(id), None) => {
                self.state.remove_relation_component(id, &key)?;
            }
        }
        self.operations.push(Operation::ReplaceComponent {
            owner,
            key,
            before: before.map(|value| value.to_stored()),
            after: after.map(|value| value.to_stored()),
        });
        Ok(())
    }

    fn component(
        &self,
        owner: ComponentOwner,
        key: &ComponentKey,
    ) -> Result<Option<&ComponentRecord>> {
        match owner {
            ComponentOwner::Project => Ok(self.state.project_component(key)),
            ComponentOwner::Entity(id) => self.state.component(id, key),
            ComponentOwner::Relation(id) => self.state.relation_component(id, key),
        }
    }

    /// Observations describe dependencies on the edit's base snapshot. Changes made earlier in
    /// this edit are already ordered by `operations` and must not become competing observations.
    fn observe_component(&mut self, owner: ComponentOwner, key: ComponentKey) -> Result<()> {
        let fingerprint = match owner {
            ComponentOwner::Project => self
                .base
                .state
                .project_component(&key)
                .map(ComponentRecord::fingerprint),
            ComponentOwner::Entity(id) if self.base.state.contains_entity(id) => self
                .base
                .state
                .component(id, &key)?
                .map(ComponentRecord::fingerprint),
            ComponentOwner::Relation(id) if self.base.state.relations.contains_key(&id) => self
                .base
                .state
                .relation_component(id, &key)?
                .map(ComponentRecord::fingerprint),
            ComponentOwner::Entity(_) | ComponentOwner::Relation(_) => return Ok(()),
        };
        self.observations.insert(Observation::Component {
            owner,
            key,
            fingerprint,
        });
        Ok(())
    }

    fn encoded<T: Component>(&self, value: &T) -> Result<(ComponentKey, ComponentRecord)> {
        Ok((key::<T>()?, self.encode_value(value)?))
    }

    pub(crate) fn ensure_text_group(&mut self, page: EntityId) -> Result<EntityId> {
        self.state.page(page)?;
        let marker = key::<TextGroup>()?;
        let children = self
            .state
            .page(page)?
            .entity(page)?
            .children
            .iter()
            .map(|child| self.state.pages[&page].entities[*child].id)
            .collect::<Vec<_>>();
        let mut groups = Vec::new();
        for child in children {
            if self.state.component(child, &marker)?.is_some() {
                groups.push(child);
            }
        }
        match groups.as_slice() {
            [group] => {
                if self.generation.is_none() {
                    self.promote_entity_to_user(*group)?;
                }
                return Ok(*group);
            }
            [] => {}
            _ => return Err(Error::invalid("page contains multiple text groups")),
        }

        let group = self.add_entity(page, At::End)?;
        self.set(
            group,
            &Group {
                origin: self.lifecycle_origin(),
                name: "Text".to_owned(),
            },
        )?;
        self.set(
            group,
            &Visibility {
                origin: self.lifecycle_origin(),
                visible: true,
                opacity: 1.0,
            },
        )?;
        self.replace_component(
            ComponentOwner::Entity(group),
            marker,
            Some(self.encode_value(&TextGroup {})?),
        )?;
        self.validate_entities.insert(group);
        Ok(group)
    }

    fn encode_value<T: Component>(&self, value: &T) -> Result<ComponentRecord> {
        let record_exists = |id| self.state.contains_entity(id);
        let blob_exists = |id| {
            self.attachments.iter().any(|(blob, _)| *blob == id)
                || self.base.storage.blobs().contains(id)
        };
        encode(value, &ValidationContext::new(&record_exists, &blob_exists))
    }

    fn entity_components(
        &self,
        values: impl IntoIterator<Item = (ComponentKey, ComponentRecord)>,
    ) -> Components {
        let mut result = values.into_iter().collect::<SmallVec<[_; 8]>>();
        result.sort_by(|left, right| left.0.cmp(&right.0));
        result
    }

    fn prepare_value<T: Component>(&mut self, owner: ComponentOwner, value: &T) -> Result<T> {
        self.validate_removal_authorship::<T>(owner)?;
        let mut value = value.clone();
        match &self.generation {
            Some(generation) => {
                if !value.set_origin(Origin::Generated(generation.clone())) {
                    return Err(Error::Authorship(format!(
                        "pipeline cannot write unmanaged component {}",
                        T::KIND
                    )));
                }
            }
            None if value.origin().is_some() && !value.set_origin(Origin::User) => {
                return Err(Error::Authorship(format!(
                    "component {} reports ownership but cannot be stamped",
                    T::KIND
                )));
            }
            None => {}
        }
        Ok(value)
    }

    fn validate_removal_authorship<T: Component>(&mut self, owner: ComponentOwner) -> Result<()> {
        let key = key::<T>()?;
        let existing = self
            .component(owner, &key)?
            .map(|value| {
                let record_exists = |id| self.state.contains_entity(id);
                let blob_exists = |_id| true;
                decode::<T>(value, &ValidationContext::new(&record_exists, &blob_exists))
            })
            .transpose()?;
        self.observe_component(owner, key)?;
        match existing {
            None => Ok(()),
            Some(value) => {
                if self.generation.is_some() && value.origin().is_none() {
                    return Err(Error::Authorship(format!(
                        "pipeline cannot remove unmanaged component {}",
                        T::KIND
                    )));
                }
                self.validate_origin_removal(value.origin())
            }
        }
    }

    fn validate_origin_removal(&self, existing: Option<&Origin>) -> Result<()> {
        let Some(expected) = &self.generation else {
            return Ok(());
        };
        let Some(existing) = existing else {
            return Ok(());
        };
        match existing {
            Origin::User => Err(Error::Authorship(
                "pipeline cannot overwrite or remove a user-owned component".to_owned(),
            )),
            Origin::Generated(actual) if expected.producer != actual.producer => {
                Err(Error::Authorship(format!(
                    "producer {} owns this component, not {}",
                    actual.producer, expected.producer
                )))
            }
            Origin::Generated(_) => Ok(()),
        }
    }

    fn lifecycle_origin(&self) -> Origin {
        self.generation
            .clone()
            .map_or(Origin::User, Origin::Generated)
    }

    fn validate_lifecycle_removal(&self, entity: EntityId) -> Result<()> {
        let Some(producer) = self
            .generation
            .as_ref()
            .map(|generation| generation.producer.clone())
        else {
            return Ok(());
        };
        let key = key::<EntityOrigin>()?;
        let value = self
            .state
            .component(entity, &key)?
            .ok_or_else(|| Error::invalid("entity lifecycle origin is missing"))?;
        let record_exists = |id| self.state.contains_entity(id);
        let blob_exists = |_id| true;
        let lifecycle: EntityOrigin =
            decode(value, &ValidationContext::new(&record_exists, &blob_exists))?;
        match lifecycle.origin {
            Origin::Generated(owner) if owner.producer == producer => Ok(()),
            Origin::Generated(owner) => Err(Error::Authorship(format!(
                "producer {} owns entity {entity}, not {}",
                owner.producer, producer
            ))),
            Origin::User => Err(Error::Authorship(format!(
                "pipeline cannot remove user-owned entity {entity}"
            ))),
        }
    }

    fn incident_relations(&self, entity: EntityId) -> Vec<RelationId> {
        self.state
            .outgoing
            .get(&entity)
            .into_iter()
            .chain(self.state.incoming.get(&entity))
            .flat_map(|ids| ids.iter().copied())
            .collect()
    }

    fn observe_page_order_write(&mut self) {
        self.observations.insert(Observation::ProjectHierarchy {
            epoch: self.base.state.page_order_epoch,
        });
    }

    fn observe_page_write(&mut self, page: EntityId) {
        if let Some(page) = self.base.state.pages.get(&page) {
            self.observations.insert(Observation::Page {
                page: page.entities[page.root].id,
                epoch: Some(page.epoch),
            });
        }
    }

    fn observe_children_write(&mut self, parent: EntityId) -> Result<()> {
        if self.base.state.contains_entity(parent) {
            self.observations.insert(Observation::Children {
                parent,
                children: self.base.state.child_ids(parent)?,
            });
        }
        Ok(())
    }
}

fn resolve_position(values: &[EntityId], at: At, moving: Option<EntityId>) -> Result<usize> {
    let filtered = values
        .iter()
        .copied()
        .filter(|value| Some(*value) != moving)
        .collect::<Vec<_>>();
    match at {
        At::Start => Ok(0),
        At::End => Ok(filtered.len()),
        At::Before(anchor) => filtered
            .iter()
            .position(|id| *id == anchor)
            .ok_or_else(|| Error::invalid("before anchor is not a sibling")),
        At::After(anchor) => filtered
            .iter()
            .position(|id| *id == anchor)
            .map(|position| position + 1)
            .ok_or_else(|| Error::invalid("after anchor is not a sibling")),
    }
}
