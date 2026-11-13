//! Layer ops. A layer sits in one list: its parent's children, masks among
//! them, or its page's top level.

use std::slice;

use jsonptr::Pointer;
use serde_json::Value;

use kurbo::{Affine, BezPath};

use super::{Place, Run};
use crate::{
    Content, Created, Error, Layer, LayerId, LayerKind, NewLayer, Offset, Path, Ref, Result,
    Transform, Uuid,
    project::{ids, locked_inside, may_hold},
};

/// What a patch can't name: what a layer is, what's inside it, and what only
/// `set_attribute` writes.
const FIXED: [&str; 4] = ["id", "children", "attributes", "kind"];

impl Run<'_, '_> {
    /// Drops the tiles of `layer` and everything inside it, once it's out of
    /// its page.
    pub(super) fn forget(&mut self, layer: &Layer) {
        for id in ids(slice::from_ref(layer)) {
            self.project.tiles.remove(&id);
        }
    }

    pub(super) fn add_layer(
        &mut self,
        op: usize,
        id: Option<&Ref>,
        page: &Ref,
        parent: Option<&Ref>,
        index: usize,
        new: &NewLayer,
    ) -> Result<()> {
        let page = self.page(page)?;
        let parent = parent.map(|parent| self.place(parent)).transpose()?;
        if let Some(parent) = &parent {
            if parent.page != page {
                return Err(Error::invalid(
                    op,
                    format!("parent {} is on another page", parent.id),
                ));
            }
            self.unlocked(op, parent.id)?;
        }
        let kind = LayerKind::from(&new.content);
        may_hold(parent.as_ref().map(|parent| self.get(parent).kind()), kind)
            .map_err(|reason| Error::invalid(op, reason))?;
        let list = parent.map(|parent| parent.path).unwrap_or_default();
        let len = self.project.pages[&page].list(&list).len();
        if index > len {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the end ({len})"),
            ));
        }
        let (uuid, label) = self.new_id(op, id)?;
        let id = LayerId(uuid);
        let mut layer = Layer::new(id, new.clone());
        self.check_names(op, &layer)?;
        Self::settle(op, &mut layer)?;
        self.created.push(Created::Layer {
            id,
            label: label.clone(),
        });
        self.number_parts(op, &mut layer, label.as_deref())?;
        self.project.insert(page, &list, index, layer);
        Ok(())
    }

    /// That the files a layer names were added.
    fn check_names(&self, op: usize, layer: &Layer) -> Result<()> {
        if let Content::Image(image) = &layer.content
            && !self.tx.has_blob(&image.file)?
        {
            return Err(Error::invalid(
                op,
                format!("file {} was not added", image.file),
            ));
        }
        Ok(())
    }

    pub(super) fn duplicate_layer(
        &mut self,
        op: usize,
        layer: &Ref,
        id: Option<&Ref>,
        index: Option<usize>,
    ) -> Result<()> {
        let place = self.place(layer)?;
        if let Some(parent) = self.parent(&place) {
            self.unlocked(op, parent)?;
        }
        let (list, at) = place.holder();
        let len = self.project.pages[&place.page].list(list).len();
        let index = index.unwrap_or(at + 1);
        if index > len {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the end ({len})"),
            ));
        }
        let (uuid, label) = self.new_id(op, id)?;
        let mut copy = self.get(&place).clone();
        self.renumber(op, &mut copy, LayerId(uuid), label)?;
        self.project.insert(place.page, list, index, copy);
        Ok(())
    }

    /// Gives `copy` the id `to` and everything inside it new ids, top first.
    /// Tile maps are shared, so a copy's pixels cost nothing until one of
    /// them is painted.
    fn renumber(
        &mut self,
        op: usize,
        copy: &mut Layer,
        to: LayerId,
        label: Option<String>,
    ) -> Result<()> {
        self.share_tiles(copy.id, to);
        copy.id = to;
        self.created.push(Created::Layer {
            id: to,
            label: label.clone(),
        });
        for (old, new) in self.number_parts(op, copy, label.as_deref())? {
            self.share_tiles(old, new);
        }
        for child in &mut copy.children {
            self.renumber(op, child, LayerId(Uuid::now_v7()), None)?;
        }
        Ok(())
    }

    fn share_tiles(&mut self, from: LayerId, to: LayerId) {
        if let Some(tiles) = self.project.tiles.get(&from).cloned() {
            self.project.tiles.insert(to, tiles);
        }
    }

    pub(super) fn move_layer(
        &mut self,
        op: usize,
        layer: &Ref,
        parent: Option<&Ref>,
        index: usize,
    ) -> Result<()> {
        let place = self.place(layer)?;
        self.unlocked(op, place.id)?;
        let parent = parent.map(|parent| self.place(parent)).transpose()?;
        if let Some(parent) = &parent {
            if parent.page != place.page {
                return Err(Error::invalid(op, "a layer can only move within its page"));
            }
            if parent.path.starts_with(&place.path) {
                return Err(Error::invalid(op, "a layer can't move inside itself"));
            }
            self.unlocked(op, parent.id)?;
        }
        may_hold(
            parent.as_ref().map(|parent| self.get(parent).kind()),
            self.get(&place).kind(),
        )
        .map_err(|reason| Error::invalid(op, reason))?;
        let moved = self.project.take(place.page, &place.path);
        // Taking the layer out can shift its new parent's path.
        let list = match &parent {
            Some(parent) => self.project.pages[&place.page]
                .position(parent.id)
                .expect("the parent stays on the page"),
            None => Vec::new(),
        };
        let len = self.project.pages[&place.page].list(&list).len();
        if index > len {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the end ({len})"),
            ));
        }
        self.project.insert(place.page, &list, index, moved);
        Ok(())
    }

    pub(super) fn remove_layer(&mut self, op: usize, layer: &Ref) -> Result<()> {
        let place = self.place(layer)?;
        if let Some(locked) = locked_inside(self.get(&place)) {
            return Err(Error::Locked { op, layer: locked });
        }
        if let Some(parent) = self.parent(&place) {
            self.unlocked(op, parent)?;
        }
        let removed = self.project.take(place.page, &place.path);
        self.forget(&removed);
        Ok(())
    }

    /// Merges `patch` into the layer's own values as JSON and parses the
    /// result back, so a misspelt field or a value of the wrong shape fails
    /// and nothing changes.
    pub(super) fn update_layer(&mut self, op: usize, layer: &Ref, patch: &Value) -> Result<()> {
        let place = self.place(layer)?;
        let old = self.get(&place).own();
        let Value::Object(fields) = patch else {
            return Err(Error::invalid(op, "a patch must be an object"));
        };
        if let Some(key) = FIXED.iter().find(|key| fields.contains_key(**key)) {
            return Err(Error::invalid(op, format!("a patch can't change `{key}`")));
        }
        let content = match old.kind() {
            LayerKind::Text => Some(Pointer::from_static("/content")),
            LayerKind::Block => Some(Pointer::from_static("/translation/content")),
            _ => None,
        };
        if let Some(content) = content
            && content.resolve(patch).is_ok()
        {
            return Err(Error::invalid(
                op,
                format!("{:?} changes only through set_text", content.as_str()),
            ));
        }
        let mut json = serde_json::to_value(&old).expect("layers serialize");
        json_patch::merge(&mut json, patch);
        let new: Layer =
            serde_json::from_value(json).map_err(|error| Error::invalid(op, error.to_string()))?;
        if new == old {
            return Ok(());
        }
        // A lock under a locked layer can't come off, but a layer's own lock
        // can.
        let only_lock = Layer {
            locked: old.locked,
            ..new.clone()
        } == old;
        match (only_lock, self.parent(&place)) {
            (true, Some(parent)) => self.unlocked(op, parent)?,
            (true, None) => {}
            (false, _) => self.unlocked(op, old.id)?,
        }
        self.check_names(op, &new)?;
        self.put(op, &place, new)
    }

    /// Puts `transform` on a layer's own geometry. Pixels never resample,
    /// so a picture takes only a whole-pixel translation.
    pub(super) fn transform_layer(
        &mut self,
        op: usize,
        layer: &Ref,
        transform: Transform,
    ) -> Result<()> {
        let id = LayerId(self.resolve(layer)?);
        let picture = self.project.picture(id);
        // A block's clean-up and mask live in its block, not in the tree.
        let holder = picture.as_ref().map_or(id, |picture| picture.holder);
        let (page, path) = self
            .project
            .place(holder)
            .ok_or_else(|| Error::NotFound(format!("layer {id}")))?;
        let place = Place {
            id: holder,
            page,
            path,
        };
        if picture.is_some_and(|picture| picture.locked) {
            return Err(Error::Locked { op, layer: id });
        }
        let affine = Affine::from(transform);
        let turned = |old: &mut Transform| *old = (affine * Affine::from(*old)).into();
        let reshaped = |old: &mut Path| {
            let mut path = BezPath::from_svg(old.as_str()).expect("a stored path parses");
            path.apply_affine(affine);
            *old = Path::try_from(path.to_svg()).expect("kurbo writes path data it reads");
        };
        let moved = |offset: &mut Offset| {
            let [a, b, c, d, x, y] = affine.as_coeffs();
            let by = |at: i32, by: f64| {
                let to = f64::from(at) + by;
                (to.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&to))
                    .then_some(to as i32)
            };
            let unturned = [a, b, c, d] == [1.0, 0.0, 0.0, 1.0];
            match (by(offset.x, x), by(offset.y, y)) {
                (Some(x), Some(y)) if unturned => {
                    *offset = Offset { x, y };
                    Ok(())
                }
                _ => Err(Error::invalid(
                    op,
                    format!("layer {id}'s pixels move only by whole pixels"),
                )),
            }
        };
        let old = self.get(&place).own();
        let mut new = old.clone();
        match &mut new.content {
            Content::Block(block) if block.mask.id == id => moved(&mut block.mask.content.offset)?,
            Content::Block(block) if block.cleanup.id == id => {
                moved(&mut block.cleanup.content.offset)?;
            }
            Content::Block(block) => {
                let text = block.translation.as_mut().unwrap_or(&mut block.source);
                turned(&mut text.transform);
            }
            Content::Text(text) => turned(&mut text.transform),
            Content::Image(image) => turned(&mut image.transform),
            Content::Shape(shape) => reshaped(&mut shape.path),
            Content::Panel(panel) => reshaped(&mut panel.outline),
            Content::Balloon(balloon) => reshaped(&mut balloon.outline),
            Content::Raster(raster) => moved(&mut raster.offset)?,
            Content::Mask(mask) => moved(&mut mask.offset)?,
            Content::Group(_) => {
                return Err(Error::invalid(op, "a group has no geometry of its own"));
            }
        }
        if new == old {
            return Ok(());
        }
        self.unlocked(op, holder)?;
        self.put(op, &place, new)
    }
}
