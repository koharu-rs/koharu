//! Page ops.

use super::Run;
use crate::{Background, Created, Error, Hash, Page, PageId, Ref, Result, Size};

impl Run<'_, '_> {
    pub(super) fn add_page(
        &mut self,
        op: usize,
        id: Option<&Ref>,
        index: usize,
        size: Option<Size>,
        background: Option<&Hash>,
    ) -> Result<()> {
        // A page made from a file takes that file's size.
        let (size, background) = match (size, background) {
            (Some(size), None) => (size, None),
            (None, Some(file)) => (
                self.image_size(op, file)?,
                Some(Background {
                    file: *file,
                    visible: true,
                }),
            ),
            _ => {
                return Err(Error::invalid(
                    op,
                    "a page takes a size or a background, not both",
                ));
            }
        };
        if size.width == 0 || size.height == 0 {
            return Err(Error::invalid(op, "a page needs a nonzero size"));
        }
        if index > self.project.order.len() {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the last page"),
            ));
        }
        let (uuid, label) = self.new_id(op, id)?;
        let id = PageId(uuid);
        self.project.order.insert(index, id);
        self.project.pages.insert(
            id,
            Page {
                id,
                size,
                background,
                layers: Vec::new(),
            },
        );
        self.created.push(Created::Page { id, label });
        Ok(())
    }

    pub(super) fn move_page(&mut self, op: usize, page: &Ref, index: usize) -> Result<()> {
        let id = self.page(page)?;
        if index >= self.project.order.len() {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the last page"),
            ));
        }
        let order = &mut self.project.order;
        let from = order.index_of(&id).expect("every page is in the order");
        order.remove(from);
        order.insert(index, id);
        Ok(())
    }

    /// A new background must be an image of the page's size.
    pub(super) fn set_background(
        &mut self,
        op: usize,
        page: &Ref,
        background: Option<&Background>,
    ) -> Result<()> {
        let id = self.page(page)?;
        if let Some(background) = background {
            let size = self.image_size(op, &background.file)?;
            let page = &self.project.pages[&id];
            if size != page.size {
                return Err(Error::invalid(
                    op,
                    format!(
                        "file {} is {}×{}, but the page is {}×{}",
                        background.file, size.width, size.height, page.size.width, page.size.height
                    ),
                ));
            }
        }
        self.project
            .pages
            .get_mut(&id)
            .expect("the page was just found")
            .background = background.copied();
        Ok(())
    }

    /// Its layers and their pixels go with it.
    pub(super) fn remove_page(&mut self, page: &Ref) -> Result<()> {
        let id = self.page(page)?;
        while !self.project.pages[&id].layers.is_empty() {
            let layer = self.project.take(id, &[0]);
            self.forget(&layer);
        }
        self.project.pages.remove(&id);
        self.project.order.retain(|page| *page != id);
        Ok(())
    }
}
