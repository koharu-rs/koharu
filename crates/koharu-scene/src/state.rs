use std::{collections::BTreeSet, sync::Arc};

use hashbrown::{HashMap, HashSet};
use imbl::HashMap as PersistentHashMap;
use revision::revisioned;
use slotmap::{DenseSlotMap, new_key_type};
use smallvec::SmallVec;

use crate::{
    BlobId, ComponentOwner, EntityId, EntityOrigin, Error, Page, Relation, RelationId, Result,
    component::{ComponentKey, ComponentRecord, StoredComponent, ValidationContext},
    schema,
};

const MAX_HIERARCHY_DEPTH: usize = 256;

new_key_type! {
    pub(crate) struct LocalEntityKey;
}

pub(crate) type Components = SmallVec<[(ComponentKey, ComponentRecord); 8]>;

#[derive(Clone, Debug)]
struct DetachedEntity {
    id: EntityId,
    parent: Option<EntityId>,
    components: Components,
}

#[derive(Clone, Debug)]
struct DetachedSubtree {
    entities: Vec<DetachedEntity>,
}

#[derive(Clone, Debug)]
pub(crate) struct EntityNode {
    pub(crate) id: EntityId,
    pub(crate) parent: Option<LocalEntityKey>,
    pub(crate) children: Vec<LocalEntityKey>,
    pub(crate) components: Components,
}

impl EntityNode {
    fn component(&self, key: &ComponentKey) -> Option<&ComponentRecord> {
        self.components
            .binary_search_by(|(candidate, _)| candidate.cmp(key))
            .ok()
            .map(|index| &self.components[index].1)
    }

    fn set_component(
        &mut self,
        key: ComponentKey,
        value: ComponentRecord,
    ) -> Option<ComponentRecord> {
        match self
            .components
            .binary_search_by(|(candidate, _)| candidate.cmp(&key))
        {
            Ok(index) => Some(std::mem::replace(&mut self.components[index].1, value)),
            Err(index) => {
                self.components.insert(index, (key, value));
                None
            }
        }
    }

    fn remove_component(&mut self, key: &ComponentKey) -> Option<ComponentRecord> {
        self.components
            .binary_search_by(|(candidate, _)| candidate.cmp(key))
            .ok()
            .map(|index| self.components.remove(index).1)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PageState {
    pub(crate) root: LocalEntityKey,
    pub(crate) epoch: u64,
    pub(crate) entities: DenseSlotMap<LocalEntityKey, EntityNode>,
    pub(crate) keys: HashMap<EntityId, LocalEntityKey>,
    component_index: HashMap<ComponentKey, HashSet<LocalEntityKey>>,
}

impl PageState {
    fn new(id: EntityId, components: Components) -> Self {
        let mut entities = DenseSlotMap::with_key();
        let root = entities.insert(EntityNode {
            id,
            parent: None,
            children: Vec::new(),
            components,
        });
        let mut page = Self {
            root,
            epoch: 0,
            entities,
            keys: HashMap::from([(id, root)]),
            component_index: HashMap::new(),
        };
        page.index_entity(root);
        page
    }

    pub(crate) fn key(&self, id: EntityId) -> Result<LocalEntityKey> {
        self.keys.get(&id).copied().ok_or(Error::EntityNotFound(id))
    }

    pub(crate) fn entity(&self, id: EntityId) -> Result<&EntityNode> {
        Ok(&self.entities[self.key(id)?])
    }

    pub(crate) fn component(
        &self,
        id: EntityId,
        key: &ComponentKey,
    ) -> Result<Option<&ComponentRecord>> {
        Ok(self.entity(id)?.component(key))
    }

    pub(crate) fn insert_entity(
        &mut self,
        id: EntityId,
        parent: EntityId,
        position: usize,
        components: Components,
    ) -> Result<()> {
        if self.keys.contains_key(&id) {
            return Err(Error::MultipleParents(id));
        }
        let parent_key = self.key(parent)?;
        if position > self.entities[parent_key].children.len() {
            return Err(Error::invalid("entity insertion position is out of range"));
        }
        let key = self.entities.insert(EntityNode {
            id,
            parent: Some(parent_key),
            children: Vec::new(),
            components,
        });
        self.keys.insert(id, key);
        self.entities[parent_key].children.insert(position, key);
        self.index_entity(key);
        self.bump_epoch();
        Ok(())
    }

    pub(crate) fn remove_leaf(&mut self, id: EntityId) -> Result<EntityNode> {
        let key = self.key(id)?;
        if key == self.root {
            return Err(Error::invalid("a page root cannot be removed as a leaf"));
        }
        if !self.entities[key].children.is_empty() {
            return Err(Error::NonEmptyEntity(id));
        }
        let parent = self.entities[key]
            .parent
            .expect("non-root entity has a parent");
        self.entities[parent].children.retain(|child| *child != key);
        self.unindex_entity(key);
        self.keys.remove(&id);
        let entity = self.entities.remove(key).ok_or(Error::EntityNotFound(id))?;
        self.bump_epoch();
        Ok(entity)
    }

    pub(crate) fn move_entity(
        &mut self,
        id: EntityId,
        parent: EntityId,
        position: usize,
    ) -> Result<()> {
        let key = self.key(id)?;
        let new_parent = self.key(parent)?;
        if key == self.root || key == new_parent {
            return Err(Error::invalid("invalid hierarchy move"));
        }
        let mut cursor = Some(new_parent);
        while let Some(candidate) = cursor {
            if candidate == key {
                return Err(Error::invalid("hierarchy move would create a cycle"));
            }
            cursor = self.entities[candidate].parent;
        }
        let old_parent = self.entities[key]
            .parent
            .expect("non-root entity has a parent");
        let old_position = self.entities[old_parent]
            .children
            .iter()
            .position(|child| *child == key)
            .expect("parent and child indexes agree");
        self.entities[old_parent].children.remove(old_position);
        if position > self.entities[new_parent].children.len() {
            return Err(Error::invalid("entity move position is out of range"));
        }
        self.entities[new_parent].children.insert(position, key);
        self.entities[key].parent = Some(new_parent);
        self.bump_epoch();
        Ok(())
    }

    fn detach_subtree(&mut self, id: EntityId) -> Result<DetachedSubtree> {
        let root = self.key(id)?;
        if root == self.root {
            return Err(Error::invalid("a page root cannot move into another page"));
        }

        let parent = self.entities[root]
            .parent
            .expect("non-root entity has a parent");
        let position = self.entities[parent]
            .children
            .iter()
            .position(|child| *child == root)
            .expect("parent and child indexes agree");
        self.entities[parent].children.remove(position);

        let mut keys = Vec::new();
        let mut stack = vec![root];
        while let Some(key) = stack.pop() {
            keys.push(key);
            stack.extend(self.entities[key].children.iter().rev().copied());
        }
        let entities = keys
            .iter()
            .map(|key| {
                let entity = &self.entities[*key];
                DetachedEntity {
                    id: entity.id,
                    parent: (*key != root).then(|| {
                        self.entities[entity.parent.expect("subtree child has a parent")].id
                    }),
                    components: entity.components.clone(),
                }
            })
            .collect();

        for key in keys.into_iter().rev() {
            let entity = self.entities[key].id;
            self.unindex_entity(key);
            self.keys.remove(&entity);
            self.entities
                .remove(key)
                .expect("collected subtree entity exists");
        }
        self.bump_epoch();
        Ok(DetachedSubtree { entities })
    }

    fn attach_subtree(
        &mut self,
        parent: EntityId,
        position: usize,
        subtree: DetachedSubtree,
    ) -> Result<()> {
        let parent = self.key(parent)?;
        if position > self.entities[parent].children.len() {
            return Err(Error::invalid("entity move position is out of range"));
        }
        if subtree.entities.is_empty()
            || subtree
                .entities
                .iter()
                .any(|entity| self.keys.contains_key(&entity.id))
        {
            return Err(Error::invalid("invalid detached subtree"));
        }

        let mut inserted = HashMap::with_capacity(subtree.entities.len());
        let mut root = None;
        for entity in subtree.entities {
            let parent_key = match entity.parent {
                Some(parent) => Some(
                    *inserted
                        .get(&parent)
                        .ok_or_else(|| Error::invalid("detached subtree is not in preorder"))?,
                ),
                None if root.is_none() => {
                    root = Some(entity.id);
                    Some(parent)
                }
                None => return Err(Error::invalid("detached subtree has multiple roots")),
            };
            let key = self.entities.insert(EntityNode {
                id: entity.id,
                parent: parent_key,
                children: Vec::new(),
                components: entity.components,
            });
            self.keys.insert(entity.id, key);
            inserted.insert(entity.id, key);
            if let Some(parent_key) = parent_key
                && parent_key != parent
            {
                self.entities[parent_key].children.push(key);
            }
            self.index_entity(key);
        }
        let root = root.expect("non-empty subtree has a root");
        self.entities[parent]
            .children
            .insert(position, inserted[&root]);
        self.bump_epoch();
        Ok(())
    }

    pub(crate) fn set_component(
        &mut self,
        id: EntityId,
        key: ComponentKey,
        value: ComponentRecord,
    ) -> Result<Option<ComponentRecord>> {
        let entity = self.key(id)?;
        let previous = self.entities[entity].set_component(key.clone(), value);
        if previous.is_none() {
            self.component_index.entry(key).or_default().insert(entity);
        }
        self.bump_epoch();
        Ok(previous)
    }

    pub(crate) fn remove_component(
        &mut self,
        id: EntityId,
        key: &ComponentKey,
    ) -> Result<Option<ComponentRecord>> {
        let entity = self.key(id)?;
        let previous = self.entities[entity].remove_component(key);
        if previous.is_some()
            && let Some(values) = self.component_index.get_mut(key)
        {
            values.remove(&entity);
            if values.is_empty() {
                self.component_index.remove(key);
            }
        }
        if previous.is_some() {
            self.bump_epoch();
        }
        Ok(previous)
    }

    pub(crate) fn ordered_ids(&self) -> Vec<EntityId> {
        let mut result = Vec::with_capacity(self.entities.len());
        let mut stack = vec![self.root];
        while let Some(key) = stack.pop() {
            let entity = &self.entities[key];
            result.push(entity.id);
            stack.extend(entity.children.iter().rev().copied());
        }
        result
    }

    pub(crate) fn descendants(&self, id: EntityId) -> Result<Vec<EntityId>> {
        let root = self.key(id)?;
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(key) = stack.pop() {
            let entity = &self.entities[key];
            result.push(entity.id);
            stack.extend(entity.children.iter().rev().copied());
        }
        Ok(result)
    }

    pub(crate) fn entities_with(&self, key: &ComponentKey) -> Vec<EntityId> {
        let Some(matches) = self.component_index.get(key) else {
            return Vec::new();
        };
        self.ordered_ids()
            .into_iter()
            .filter(|id| self.keys.get(id).is_some_and(|key| matches.contains(key)))
            .collect()
    }

    fn index_entity(&mut self, key: LocalEntityKey) {
        for (component, _) in &self.entities[key].components {
            self.component_index
                .entry(component.clone())
                .or_default()
                .insert(key);
        }
    }

    fn unindex_entity(&mut self, key: LocalEntityKey) {
        let components = self.entities[key]
            .components
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for component in components {
            if let Some(values) = self.component_index.get_mut(&component) {
                values.remove(&key);
                if values.is_empty() {
                    self.component_index.remove(&component);
                }
            }
        }
    }

    fn bump_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RelationState {
    pub(crate) value: Relation,
    pub(crate) components: Components,
}

impl RelationState {
    fn component(&self, key: &ComponentKey) -> Option<&ComponentRecord> {
        self.components
            .binary_search_by(|(candidate, _)| candidate.cmp(key))
            .ok()
            .map(|index| &self.components[index].1)
    }

    fn set_component(
        &mut self,
        key: ComponentKey,
        value: ComponentRecord,
    ) -> Option<ComponentRecord> {
        match self
            .components
            .binary_search_by(|(candidate, _)| candidate.cmp(&key))
        {
            Ok(index) => Some(std::mem::replace(&mut self.components[index].1, value)),
            Err(index) => {
                self.components.insert(index, (key, value));
                None
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct State {
    pub(crate) document: koharu_storage::DocumentId,
    pub(crate) revision: koharu_storage::Revision,
    pub(crate) page_order: Arc<Vec<EntityId>>,
    pub(crate) page_order_epoch: u64,
    pub(crate) pages: PersistentHashMap<EntityId, Arc<PageState>>,
    pub(crate) entity_pages: PersistentHashMap<EntityId, EntityId>,
    pub(crate) project_components: Arc<Components>,
    pub(crate) relations: PersistentHashMap<RelationId, Arc<RelationState>>,
    pub(crate) outgoing: PersistentHashMap<EntityId, Arc<Vec<RelationId>>>,
    pub(crate) incoming: PersistentHashMap<EntityId, Arc<Vec<RelationId>>>,
}

impl State {
    pub(crate) fn empty(document: koharu_storage::DocumentId) -> Self {
        Self {
            document,
            revision: koharu_storage::Revision::ZERO,
            page_order: Arc::new(Vec::new()),
            page_order_epoch: 0,
            pages: PersistentHashMap::new(),
            entity_pages: PersistentHashMap::new(),
            project_components: Arc::new(SmallVec::new()),
            relations: PersistentHashMap::new(),
            outgoing: PersistentHashMap::new(),
            incoming: PersistentHashMap::new(),
        }
    }

    pub(crate) fn contains_entity(&self, id: EntityId) -> bool {
        self.entity_pages.contains_key(&id)
    }

    pub(crate) fn page_for(&self, id: EntityId) -> Result<EntityId> {
        self.entity_pages
            .get(&id)
            .copied()
            .ok_or(Error::EntityNotFound(id))
    }

    pub(crate) fn page(&self, id: EntityId) -> Result<&PageState> {
        self.pages
            .get(&id)
            .map(AsRef::as_ref)
            .ok_or(Error::EntityNotFound(id))
    }

    pub(crate) fn page_containing(&self, id: EntityId) -> Result<&PageState> {
        self.page(self.page_for(id)?)
    }

    pub(crate) fn page_mut(&mut self, page: EntityId) -> Result<&mut PageState> {
        self.pages
            .get_mut(&page)
            .map(Arc::make_mut)
            .ok_or(Error::EntityNotFound(page))
    }

    pub(crate) fn entity(&self, id: EntityId) -> Result<&EntityNode> {
        self.page_containing(id)?.entity(id)
    }

    pub(crate) fn child_ids(&self, parent: EntityId) -> Result<Vec<EntityId>> {
        let page = self.page_containing(parent)?;
        Ok(page
            .entity(parent)?
            .children
            .iter()
            .map(|child| page.entities[*child].id)
            .collect())
    }

    pub(crate) fn component(
        &self,
        id: EntityId,
        key: &ComponentKey,
    ) -> Result<Option<&ComponentRecord>> {
        self.page_containing(id)?.component(id, key)
    }

    pub(crate) fn relation_component(
        &self,
        id: RelationId,
        key: &ComponentKey,
    ) -> Result<Option<&ComponentRecord>> {
        Ok(self
            .relations
            .get(&id)
            .ok_or(Error::RelationNotFound(id))?
            .component(key))
    }

    pub(crate) fn project_component(&self, key: &ComponentKey) -> Option<&ComponentRecord> {
        self.project_components
            .binary_search_by(|(candidate, _)| candidate.cmp(key))
            .ok()
            .map(|index| &self.project_components[index].1)
    }

    pub(crate) fn set_project_component(
        &mut self,
        key: ComponentKey,
        value: ComponentRecord,
    ) -> Option<ComponentRecord> {
        let components = Arc::make_mut(&mut self.project_components);
        match components.binary_search_by(|(candidate, _)| candidate.cmp(&key)) {
            Ok(index) => Some(std::mem::replace(&mut components[index].1, value)),
            Err(index) => {
                components.insert(index, (key, value));
                None
            }
        }
    }

    pub(crate) fn remove_project_component(
        &mut self,
        key: &ComponentKey,
    ) -> Option<ComponentRecord> {
        let components = Arc::make_mut(&mut self.project_components);
        components
            .binary_search_by(|(candidate, _)| candidate.cmp(key))
            .ok()
            .map(|index| components.remove(index).1)
    }

    pub(crate) fn insert_page(
        &mut self,
        id: EntityId,
        position: usize,
        components: Components,
    ) -> Result<()> {
        if self.contains_entity(id) || position > self.page_order.len() {
            return Err(Error::invalid("invalid page insertion"));
        }
        Arc::make_mut(&mut self.page_order).insert(position, id);
        self.page_order_epoch = self.page_order_epoch.wrapping_add(1);
        self.pages
            .insert(id, Arc::new(PageState::new(id, components)));
        self.entity_pages.insert(id, id);
        Ok(())
    }

    pub(crate) fn remove_page(&mut self, id: EntityId) -> Result<PageState> {
        let page = self.pages.get(&id).ok_or(Error::EntityNotFound(id))?;
        if page.entities.len() != 1 {
            return Err(Error::NonEmptyEntity(id));
        }
        let page = self.pages.remove(&id).expect("page checked above");
        Arc::make_mut(&mut self.page_order).retain(|candidate| *candidate != id);
        self.page_order_epoch = self.page_order_epoch.wrapping_add(1);
        self.entity_pages.remove(&id);
        Ok(Arc::unwrap_or_clone(page))
    }

    pub(crate) fn move_page(&mut self, id: EntityId, position: usize) -> Result<()> {
        let pages = Arc::make_mut(&mut self.page_order);
        let old = pages
            .iter()
            .position(|candidate| *candidate == id)
            .ok_or(Error::EntityNotFound(id))?;
        pages.remove(old);
        if position > pages.len() {
            return Err(Error::invalid("page move position is out of range"));
        }
        pages.insert(position, id);
        self.page_order_epoch = self.page_order_epoch.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn insert_entity(
        &mut self,
        page: EntityId,
        id: EntityId,
        parent: EntityId,
        position: usize,
        components: Components,
    ) -> Result<()> {
        if self.contains_entity(id) {
            return Err(Error::MultipleParents(id));
        }
        self.page_mut(page)?
            .insert_entity(id, parent, position, components)?;
        self.entity_pages.insert(id, page);
        Ok(())
    }

    pub(crate) fn remove_leaf(&mut self, id: EntityId) -> Result<EntityNode> {
        let page = self.page_for(id)?;
        let entity = self.page_mut(page)?.remove_leaf(id)?;
        self.entity_pages.remove(&id);
        Ok(entity)
    }

    pub(crate) fn move_entity(
        &mut self,
        id: EntityId,
        parent: EntityId,
        position: usize,
    ) -> Result<()> {
        let source_page = self.page_for(id)?;
        let target_page = self.page_for(parent)?;
        if source_page == target_page {
            return self
                .page_mut(source_page)?
                .move_entity(id, parent, position);
        }

        if id == source_page {
            return Err(Error::invalid("a page root cannot move into another page"));
        }
        let target = self.page(target_page)?;
        let target_parent = target.key(parent)?;
        if position > target.entities[target_parent].children.len() {
            return Err(Error::invalid("entity move position is out of range"));
        }

        let subtree = self.page_mut(source_page)?.detach_subtree(id)?;
        let moved = subtree
            .entities
            .iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>();
        self.page_mut(target_page)?
            .attach_subtree(parent, position, subtree)?;
        for entity in moved {
            self.entity_pages.insert(entity, target_page);
        }
        Ok(())
    }

    pub(crate) fn parent_and_position(&self, id: EntityId) -> Result<(Option<EntityId>, usize)> {
        let page_id = self.page_for(id)?;
        if id == page_id {
            let position = self
                .page_order
                .iter()
                .position(|candidate| *candidate == id)
                .ok_or(Error::EntityNotFound(id))?;
            return Ok((None, position));
        }
        let page = self.page(page_id)?;
        let key = page.key(id)?;
        let parent = page.entities[key]
            .parent
            .expect("non-page entity has a parent");
        let position = page.entities[parent]
            .children
            .iter()
            .position(|candidate| *candidate == key)
            .expect("parent and child indexes agree");
        Ok((Some(page.entities[parent].id), position))
    }

    pub(crate) fn set_entity_component(
        &mut self,
        id: EntityId,
        key: ComponentKey,
        value: ComponentRecord,
    ) -> Result<Option<ComponentRecord>> {
        let page = self.page_for(id)?;
        self.page_mut(page)?.set_component(id, key, value)
    }

    pub(crate) fn remove_entity_component(
        &mut self,
        id: EntityId,
        key: &ComponentKey,
    ) -> Result<Option<ComponentRecord>> {
        let page = self.page_for(id)?;
        self.page_mut(page)?.remove_component(id, key)
    }

    pub(crate) fn insert_relation(&mut self, id: RelationId, value: Relation) -> Result<()> {
        if self.relations.contains_key(&id) {
            return Err(Error::invalid("relation already exists"));
        }
        if !self.contains_entity(value.source) || !self.contains_entity(value.target) {
            return Err(Error::invalid("relation endpoint is missing"));
        }
        self.relations.insert(
            id,
            Arc::new(RelationState {
                value: value.clone(),
                components: SmallVec::new(),
            }),
        );
        append_relation(&mut self.outgoing, value.source, id);
        append_relation(&mut self.incoming, value.target, id);
        self.bump_incident_pages(&value);
        Ok(())
    }

    pub(crate) fn remove_relation(&mut self, id: RelationId) -> Result<RelationState> {
        let relation = self
            .relations
            .remove(&id)
            .ok_or(Error::RelationNotFound(id))?;
        remove_relation_id(&mut self.outgoing, relation.value.source, id);
        remove_relation_id(&mut self.incoming, relation.value.target, id);
        self.bump_incident_pages(&relation.value);
        Ok(Arc::unwrap_or_clone(relation))
    }

    pub(crate) fn set_relation_value(
        &mut self,
        id: RelationId,
        value: Relation,
    ) -> Result<Relation> {
        let previous = self
            .relations
            .get(&id)
            .ok_or(Error::RelationNotFound(id))?
            .value
            .clone();
        if previous.source != value.source || previous.target != value.target {
            remove_relation_id(&mut self.outgoing, previous.source, id);
            remove_relation_id(&mut self.incoming, previous.target, id);
            append_relation(&mut self.outgoing, value.source, id);
            append_relation(&mut self.incoming, value.target, id);
        }
        Arc::make_mut(self.relations.get_mut(&id).expect("relation checked above")).value = value;
        self.bump_incident_pages(&previous);
        Ok(previous)
    }

    pub(crate) fn set_relation_component(
        &mut self,
        id: RelationId,
        key: ComponentKey,
        value: ComponentRecord,
    ) -> Result<Option<ComponentRecord>> {
        let relation = self
            .relations
            .get_mut(&id)
            .map(Arc::make_mut)
            .ok_or(Error::RelationNotFound(id))?;
        let endpoints = relation.value.clone();
        let previous = relation.set_component(key, value);
        self.bump_incident_pages(&endpoints);
        Ok(previous)
    }

    pub(crate) fn remove_relation_component(
        &mut self,
        id: RelationId,
        key: &ComponentKey,
    ) -> Result<Option<ComponentRecord>> {
        let relation = self
            .relations
            .get_mut(&id)
            .map(Arc::make_mut)
            .ok_or(Error::RelationNotFound(id))?;
        let endpoints = relation.value.clone();
        let result = relation
            .components
            .binary_search_by(|(candidate, _)| candidate.cmp(key))
            .ok()
            .map(|index| relation.components.remove(index).1);
        if result.is_some() {
            self.bump_incident_pages(&endpoints);
        }
        Ok(result)
    }

    pub(crate) fn referenced_blobs(&self) -> BTreeSet<BlobId> {
        self.project_components
            .iter()
            .flat_map(|(_, value)| value.blob_refs.iter().copied())
            .chain(self.pages.values().flat_map(|page| {
                page.entities.values().flat_map(|entity| {
                    entity
                        .components
                        .iter()
                        .flat_map(|(_, value)| value.blob_refs.iter().copied())
                })
            }))
            .chain(self.relations.values().flat_map(|relation| {
                relation
                    .components
                    .iter()
                    .flat_map(|(_, value)| value.blob_refs.iter().copied())
            }))
            .collect()
    }

    pub(crate) fn to_checkpoint(&self) -> StoredState {
        StoredState {
            page_order_epoch: self.page_order_epoch,
            project_components: store_components(&self.project_components),
            pages: self
                .page_order
                .iter()
                .map(|page_id| {
                    let page = &self.pages[page_id];
                    StoredPage {
                        id: *page_id,
                        epoch: page.epoch,
                        entities: page
                            .ordered_ids()
                            .into_iter()
                            .map(|id| {
                                let entity = page.entity(id).expect("ordered entity exists");
                                StoredEntity {
                                    id,
                                    parent: entity.parent.map(|parent| page.entities[parent].id),
                                    components: store_components(&entity.components),
                                }
                            })
                            .collect(),
                    }
                })
                .collect(),
            relations: self
                .relations
                .iter()
                .map(|(id, relation)| StoredRelation {
                    id: *id,
                    value: relation.value.clone(),
                    components: store_components(&relation.components),
                })
                .collect(),
        }
    }

    pub(crate) fn from_checkpoint(
        document: koharu_storage::DocumentId,
        revision: koharu_storage::Revision,
        stored: StoredState,
    ) -> Result<Self> {
        let mut state = Self::empty(document);
        state.revision = revision;
        let page_order_epoch = stored.page_order_epoch;
        state.project_components = Arc::new(load_components(stored.project_components)?);
        for page in stored.pages {
            let mut entities = page.entities.into_iter();
            let root = entities
                .next()
                .ok_or_else(|| Error::invalid("stored page has no root entity"))?;
            if root.id != page.id || root.parent.is_some() {
                return Err(Error::invalid("stored page root is invalid"));
            }
            state.insert_page(
                page.id,
                state.page_order.len(),
                load_components(root.components)?,
            )?;
            for entity in entities {
                let parent = entity
                    .parent
                    .ok_or_else(|| Error::invalid("stored child has no parent"))?;
                let position = state.page(page.id)?.entity(parent)?.children.len();
                state.insert_entity(
                    page.id,
                    entity.id,
                    parent,
                    position,
                    load_components(entity.components)?,
                )?;
            }
            state.page_mut(page.id)?.epoch = page.epoch;
        }
        state.page_order_epoch = page_order_epoch;
        for relation in stored.relations {
            state.insert_relation(relation.id, relation.value)?;
            Arc::make_mut(
                state
                    .relations
                    .get_mut(&relation.id)
                    .expect("inserted relation exists"),
            )
            .components = load_components(relation.components)?;
        }
        state.validate()?;
        Ok(state)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let record_exists = |id| self.contains_entity(id);
        let blob_exists = |_id| true;
        let context = ValidationContext::new(&record_exists, &blob_exists);
        schema::validate_components(&self.project_components, ComponentOwner::Project, &context)?;
        for page_id in self.page_order.iter().copied() {
            let page = self.page(page_id)?;
            if page
                .component(page_id, &crate::component::key::<Page>()?)?
                .is_none()
            {
                return Err(Error::invalid("page marker is missing"));
            }
            for id in page.ordered_ids() {
                let entity = page.entity(id)?;
                if entity
                    .component(&crate::component::key::<EntityOrigin>()?)
                    .is_none()
                {
                    return Err(Error::invalid("entity origin is missing"));
                }
                if id != page_id
                    && entity
                        .component(&crate::component::key::<Page>()?)
                        .is_some()
                {
                    return Err(Error::invalid("nested entity carries a page marker"));
                }
                schema::validate_components(
                    &entity.components,
                    ComponentOwner::Entity(id),
                    &context,
                )?;
                schema::validate_entity(self, id)?;
            }
            validate_depth(page)?;
        }
        for (id, relation) in &self.relations {
            schema::validate_relation(self, &relation.value, &context)?;
            schema::validate_components(
                &relation.components,
                ComponentOwner::Relation(*id),
                &context,
            )?;
        }
        Ok(())
    }

    fn bump_incident_pages(&mut self, relation: &Relation) {
        let pages = [relation.source, relation.target]
            .into_iter()
            .filter_map(|entity| self.entity_pages.get(&entity).copied())
            .collect::<HashSet<_>>();
        for page in pages {
            if let Some(page) = self.pages.get_mut(&page) {
                Arc::make_mut(page).bump_epoch();
            }
        }
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredComponentEntry {
    pub(crate) key: ComponentKey,
    pub(crate) value: StoredComponent,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug)]
pub(crate) struct StoredEntity {
    pub(crate) id: EntityId,
    pub(crate) parent: Option<EntityId>,
    pub(crate) components: Vec<StoredComponentEntry>,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug)]
pub(crate) struct StoredPage {
    pub(crate) id: EntityId,
    pub(crate) epoch: u64,
    pub(crate) entities: Vec<StoredEntity>,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug)]
pub(crate) struct StoredRelation {
    pub(crate) id: RelationId,
    pub(crate) value: Relation,
    pub(crate) components: Vec<StoredComponentEntry>,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug)]
pub(crate) struct StoredState {
    pub(crate) page_order_epoch: u64,
    pub(crate) project_components: Vec<StoredComponentEntry>,
    pub(crate) pages: Vec<StoredPage>,
    pub(crate) relations: Vec<StoredRelation>,
}

pub(crate) fn store_components(components: &Components) -> Vec<StoredComponentEntry> {
    components
        .iter()
        .map(|(key, value)| StoredComponentEntry {
            key: key.clone(),
            value: value.to_stored(),
        })
        .collect()
}

pub(crate) fn load_components(values: Vec<StoredComponentEntry>) -> Result<Components> {
    let mut components = values
        .into_iter()
        .map(|entry| Ok((entry.key, ComponentRecord::from_stored(entry.value)?)))
        .collect::<Result<Components>>()?;
    components.sort_by(|left, right| left.0.cmp(&right.0));
    if !components.windows(2).all(|pair| pair[0].0 != pair[1].0) {
        return Err(Error::invalid("stored entity has duplicate components"));
    }
    Ok(components)
}

fn validate_depth(page: &PageState) -> Result<()> {
    let mut stack = vec![(page.root, 1_usize)];
    while let Some((key, depth)) = stack.pop() {
        if depth > MAX_HIERARCHY_DEPTH {
            return Err(Error::invalid("project hierarchy exceeds maximum depth"));
        }
        stack.extend(
            page.entities[key]
                .children
                .iter()
                .map(|child| (*child, depth + 1)),
        );
    }
    Ok(())
}

fn append_relation(
    map: &mut PersistentHashMap<EntityId, Arc<Vec<RelationId>>>,
    entity: EntityId,
    relation: RelationId,
) {
    let values = map.entry(entity).or_insert_with(|| Arc::new(Vec::new()));
    let values = Arc::make_mut(values);
    values.push(relation);
    values.sort_unstable();
}

fn remove_relation_id(
    map: &mut PersistentHashMap<EntityId, Arc<Vec<RelationId>>>,
    entity: EntityId,
    relation: RelationId,
) {
    let Some(values) = map.get_mut(&entity) else {
        return;
    };
    Arc::make_mut(values).retain(|candidate| *candidate != relation);
    if values.is_empty() {
        map.remove(&entity);
    }
}
