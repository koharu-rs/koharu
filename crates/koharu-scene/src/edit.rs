use std::collections::BTreeSet;

use loro::{LoroDoc, LoroMap, LoroTree, TreeID};

use crate::{
    Error, Id, Image, Layer, LayerContent, LayerNode, Origin, Page, PageSummary, PageView, Region,
    Result, Text,
    document::{self, ID, PageMap},
};

/// The writes of one undoable edit.
///
/// Every operation checks its inputs before it writes. Loro has no rollback,
/// so an edit that fails after a write poisons the session instead of
/// committing half of itself.
pub struct Edit<'a> {
    doc: &'a LoroDoc,
    author: Origin,
    pub(crate) wrote: bool,
}

impl<'a> Edit<'a> {
    pub(crate) fn new(doc: &'a LoroDoc, author: Origin) -> Self {
        Self {
            doc,
            author,
            wrote: false,
        }
    }

    #[must_use]
    pub const fn author(&self) -> &Origin {
        &self.author
    }

    /// Fails with [`Error::Stale`] unless `holds`. Work prepared from an
    /// earlier view checks here that what it relied on is unchanged.
    pub fn expect(&self, holds: bool) -> Result<()> {
        holds.then_some(()).ok_or(Error::Stale)
    }

    pub fn pages(&self) -> Result<Vec<PageSummary>> {
        document::summaries(self.doc)
    }

    pub fn page(&self, id: Id) -> Result<PageView> {
        PageMap::find(self.doc, id)?.1.view()
    }

    pub fn image(&self, page: Id, image: Id) -> Result<Image> {
        document::read_image(&PageMap::find(self.doc, page)?.1.images()?, image)
    }

    /// Adds a page whose size and artwork come from `source`, at `index` in
    /// project order (last when `None`).
    pub fn add_page(&mut self, index: Option<usize>, name: String, source: Image) -> Result<Id> {
        check_image(&source)?;
        let pages = document::page_list(self.doc);
        let index = index.map_or(pages.len(), |index| index.min(pages.len()));
        let (id, source_id) = (Id::new(), Id::new());
        let record = Page {
            name,
            width: f64::from(source.width),
            height: f64::from(source.height),
            source: source_id,
            text_mask: None,
            bubble_mask: None,
        };
        self.wrote = true;
        let page = PageMap::insert(&pages, index, id)?;
        document::insert_image(&page.images()?, source_id, &source)?;
        document::write(&page.meta()?, &record)?;
        Ok(id)
    }

    pub fn update_page(&mut self, id: Id, update: impl FnOnce(&mut Page)) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, id)?;
        let meta = page.meta()?;
        let mut record: Page = document::read(&meta, &[])?;
        let before = record.clone();
        update(&mut record);
        if record == before {
            return Ok(());
        }
        let images = page.image_ids()?;
        for image in [Some(record.source), record.text_mask, record.bubble_mask]
            .into_iter()
            .flatten()
        {
            exists("image", image, images.contains(&image))?;
        }
        self.wrote = true;
        document::write(&meta, &record)
    }

    pub fn move_page(&mut self, id: Id, index: usize) -> Result<()> {
        let pages = document::page_list(self.doc);
        let (from, _) = PageMap::find(self.doc, id)?;
        let to = index.min(pages.len() - 1);
        if from != to {
            self.wrote = true;
            pages.mov(from, to)?;
        }
        Ok(())
    }

    pub fn remove_page(&mut self, id: Id) -> Result<()> {
        let (index, _) = PageMap::find(self.doc, id)?;
        self.wrote = true;
        document::page_list(self.doc).delete(index, 1)?;
        Ok(())
    }

    pub fn add_image(&mut self, page: Id, image: Image) -> Result<Id> {
        check_image(&image)?;
        let (_, page) = PageMap::find(self.doc, page)?;
        let id = Id::new();
        self.wrote = true;
        document::insert_image(&page.images()?, id, &image)?;
        Ok(id)
    }

    /// Removes an image nothing refers to any more.
    pub fn remove_image(&mut self, page: Id, image: Id) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        exists("image", image, page.image_ids()?.contains(&image))?;
        let view = page.view()?;
        let in_use = [
            Some(view.page.source),
            view.page.text_mask,
            view.page.bubble_mask,
        ]
        .contains(&Some(image))
            || any_layer(
                &view.layers,
                |content| matches!(content, LayerContent::Raster { image: used, .. } if *used == image),
            );
        unreferenced("image", image, in_use)?;
        self.wrote = true;
        page.images()?.delete(&image.to_string())?;
        Ok(())
    }

    pub fn add_region(&mut self, page: Id, mut region: Region) -> Result<Id> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let regions = page.regions()?;
        for inside in &region.inside {
            exists("region", *inside, has(&regions, *inside))?;
        }
        region.origin = self.author.clone();
        let id = Id::new();
        self.wrote = true;
        let map = regions.insert_container(&id.to_string(), LoroMap::new())?;
        document::write(&map, &region)?;
        Ok(id)
    }

    pub fn update_region(
        &mut self,
        page: Id,
        id: Id,
        update: impl FnOnce(&mut Region),
    ) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let regions = page.regions()?;
        let map = record_map(&regions, "region", id)?;
        let mut region: Region = document::read(&map, &[])?;
        self.owner_check(&region.origin, "region", id)?;
        let before = region.clone();
        update(&mut region);
        if region == before {
            return Ok(());
        }
        for inside in &region.inside {
            exists("region", *inside, *inside != id && has(&regions, *inside))?;
        }
        region.origin = self.author.clone();
        self.wrote = true;
        document::write(&map, &region)
    }

    /// Removes a region no text, layer, or other region refers to.
    pub fn remove_region(&mut self, page: Id, id: Id) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let view = page.view()?;
        let region = view
            .regions
            .get(&id)
            .ok_or(Error::NotFound { kind: "region", id })?;
        self.owner_check(&region.origin, "region", id)?;
        let in_use = view
            .regions
            .values()
            .any(|other| other.inside.contains(&id))
            || view.texts.values().any(|text| text.region == Some(id))
            || any_layer(&view.layers, |content| {
                matches!(
                    content,
                    LayerContent::Text { fit_to, flows_in, .. }
                        if *fit_to == Some(id) || *flows_in == Some(id)
                )
            });
        unreferenced("region", id, in_use)?;
        self.wrote = true;
        page.regions()?.delete(&id.to_string())?;
        Ok(())
    }

    pub fn add_text(&mut self, page: Id, mut text: Text) -> Result<Id> {
        let (_, page) = PageMap::find(self.doc, page)?;
        if let Some(region) = text.region {
            exists("region", region, has(&page.regions()?, region))?;
        }
        text.origin = self.author.clone();
        text.source_origin = self.author.clone();
        text.translation_origin = self.author.clone();
        let id = Id::new();
        self.wrote = true;
        let map = page
            .texts()?
            .insert_container(&id.to_string(), LoroMap::new())?;
        document::write(&map, &text)?;
        Ok(id)
    }

    /// Updates a text. The source and the translation each keep who wrote
    /// them: a generation may fill or replace a generated value, never the
    /// user's. Every other change follows the text's owner, and any change by
    /// the user makes the text the user's.
    pub fn update_text(&mut self, page: Id, id: Id, update: impl FnOnce(&mut Text)) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let map = record_map(&page.texts()?, "text", id)?;
        let before: Text = document::read(&map, &[])?;
        let mut text = before.clone();
        update(&mut text);
        if text == before {
            return Ok(());
        }
        if text.source != before.source {
            self.owner_check(&before.source_origin, "text", id)?;
            text.source_origin = self.author.clone();
        }
        if text.translation != before.translation {
            self.owner_check(&before.translation_origin, "text", id)?;
            text.translation_origin = self.author.clone();
        }
        if (
            &text.region,
            &text.role,
            &text.source_language,
            &text.translation_language,
        ) != (
            &before.region,
            &before.role,
            &before.source_language,
            &before.translation_language,
        ) {
            self.owner_check(&before.origin, "text", id)?;
            if let Some(region) = text.region {
                exists("region", region, has(&page.regions()?, region))?;
            }
        }
        if self.author.is_user() {
            text.origin = Origin::User;
        }
        self.wrote = true;
        document::write(&map, &text)
    }

    /// Removes a text no layer presents.
    pub fn remove_text(&mut self, page: Id, id: Id) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let view = page.view()?;
        let text = view
            .texts
            .get(&id)
            .ok_or(Error::NotFound { kind: "text", id })?;
        self.owner_check(&text.origin, "text", id)?;
        let in_use = any_layer(
            &view.layers,
            |content| matches!(content, LayerContent::Text { text, .. } if *text == id),
        );
        unreferenced("text", id, in_use)?;
        self.wrote = true;
        page.texts()?.delete(&id.to_string())?;
        Ok(())
    }

    /// Adds a layer under `parent` (the page's top level when `None`) at
    /// `index` among its siblings (last when `None`).
    pub fn add_layer(
        &mut self,
        page: Id,
        parent: Option<Id>,
        index: Option<usize>,
        mut layer: Layer,
    ) -> Result<Id> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let view = page.view()?;
        check_content(&layer.content, &view, &page.image_ids()?)?;
        let tree = page.layers()?;
        let parent = parent
            .map(|parent| group_node(&tree, &view.layers, parent))
            .transpose()?;
        let siblings = tree.children(parent).map_or(0, |children| children.len());
        layer.origin = self.author.clone();
        let id = Id::new();
        self.wrote = true;
        let node = tree.create_at(parent, index.map_or(siblings, |index| index.min(siblings)))?;
        let meta = tree.get_meta(node)?;
        meta.insert(ID, id.to_string())?;
        document::write(&meta, &layer)?;
        Ok(id)
    }

    pub fn update_layer(
        &mut self,
        page: Id,
        id: Id,
        update: impl FnOnce(&mut Layer),
    ) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let tree = page.layers()?;
        let meta = tree.get_meta(document::find_node(&tree, id)?)?;
        let mut layer: Layer = document::read(&meta, &[ID])?;
        self.owner_check(&layer.origin, "layer", id)?;
        let before = layer.clone();
        update(&mut layer);
        if layer == before {
            return Ok(());
        }
        if layer.content != before.content {
            check_content(&layer.content, &page.view()?, &page.image_ids()?)?;
        }
        layer.origin = self.author.clone();
        self.wrote = true;
        document::write(&meta, &layer)
    }

    /// Moves a layer under `parent` (the page's top level when `None`) to
    /// `index` among its new siblings.
    pub fn move_layer(&mut self, page: Id, id: Id, parent: Option<Id>, index: usize) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let view = page.view()?;
        let node = find_layer(&view.layers, id).ok_or(Error::NotFound { kind: "layer", id })?;
        self.owner_check(&node.layer.origin, "layer", id)?;
        if parent.is_some_and(|parent| parent == id || find_layer(&node.children, parent).is_some())
        {
            return Err(Error::Invalid(format!(
                "layer {id} cannot move inside itself"
            )));
        }
        let tree = page.layers()?;
        let target = document::find_node(&tree, id)?;
        let parent = parent
            .map(|parent| group_node(&tree, &view.layers, parent))
            .transpose()?;
        let siblings = tree
            .children(parent)
            .unwrap_or_default()
            .into_iter()
            .filter(|sibling| *sibling != target)
            .count();
        self.wrote = true;
        tree.mov_to(target, parent, index.min(siblings))?;
        Ok(())
    }

    /// Removes a layer and every layer inside it.
    pub fn remove_layer(&mut self, page: Id, id: Id) -> Result<()> {
        let (_, page) = PageMap::find(self.doc, page)?;
        let view = page.view()?;
        let node = find_layer(&view.layers, id).ok_or(Error::NotFound { kind: "layer", id })?;
        let mut stack = vec![node];
        while let Some(node) = stack.pop() {
            self.owner_check(&node.layer.origin, "layer", node.id)?;
            stack.extend(&node.children);
        }
        let tree = page.layers()?;
        let target = document::find_node(&tree, id)?;
        self.wrote = true;
        tree.delete(target)?;
        Ok(())
    }

    /// A generation may change only what a generation produced.
    fn owner_check(&self, owner: &Origin, kind: &'static str, id: Id) -> Result<()> {
        if owner.is_user() && !self.author.is_user() {
            return Err(Error::Authorship { kind, id });
        }
        Ok(())
    }
}

fn check_image(image: &Image) -> Result<()> {
    if image.width == 0 || image.height == 0 || image.media_type.is_empty() {
        return Err(Error::Invalid(
            "an image needs a size and a media type".into(),
        ));
    }
    Ok(())
}

fn check_content(content: &LayerContent, view: &PageView, images: &BTreeSet<Id>) -> Result<()> {
    match content {
        LayerContent::Group | LayerContent::TextGroup => Ok(()),
        LayerContent::Text {
            text,
            fit_to,
            flows_in,
            ..
        } => {
            exists("text", *text, view.texts.contains_key(text))?;
            for region in [fit_to, flows_in].into_iter().flatten() {
                exists("region", *region, view.regions.contains_key(region))?;
            }
            Ok(())
        }
        LayerContent::Raster { image, .. } => exists("image", *image, images.contains(image)),
    }
}

/// The tree node of `parent`, which must be a group.
fn group_node(tree: &LoroTree, layers: &[LayerNode], parent: Id) -> Result<TreeID> {
    let node = find_layer(layers, parent).ok_or(Error::NotFound {
        kind: "layer",
        id: parent,
    })?;
    if !matches!(
        node.layer.content,
        LayerContent::Group | LayerContent::TextGroup
    ) {
        return Err(Error::Invalid(format!("layer {parent} is not a group")));
    }
    document::find_node(tree, parent)
}

fn find_layer(layers: &[LayerNode], id: Id) -> Option<&LayerNode> {
    layers.iter().find_map(|node| {
        if node.id == id {
            Some(node)
        } else {
            find_layer(&node.children, id)
        }
    })
}

fn any_layer(layers: &[LayerNode], matches: impl Fn(&LayerContent) -> bool + Copy) -> bool {
    layers
        .iter()
        .any(|node| matches(&node.layer.content) || any_layer(&node.children, matches))
}

fn record_map(records: &LoroMap, kind: &'static str, id: Id) -> Result<LoroMap> {
    document::child_map(records, &id.to_string()).map_err(|_| Error::NotFound { kind, id })
}

fn has(records: &LoroMap, id: Id) -> bool {
    records.get(&id.to_string()).is_some()
}

fn exists(kind: &'static str, id: Id, exists: bool) -> Result<()> {
    exists.then_some(()).ok_or(Error::NotFound { kind, id })
}

fn unreferenced(kind: &'static str, id: Id, in_use: bool) -> Result<()> {
    if in_use {
        return Err(Error::Referenced { kind, id });
    }
    Ok(())
}
