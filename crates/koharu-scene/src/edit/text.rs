//! Text and attribute ops.

use serde_json::Value;

use super::Run;
use crate::{Content, Error, Layer, Ref, Result, RichText, Span, Text, text};

impl Run<'_, '_> {
    /// A text layer's text, or a block's translation, added empty when the
    /// block has none.
    fn text_mut<'l>(&self, op: usize, layer: &'l mut Layer) -> Result<&'l mut Text> {
        let id = layer.id;
        match &mut layer.content {
            Content::Text(text) => Ok(text),
            Content::Block(block) => Ok(block.translation.get_or_insert_default()),
            _ => Err(Error::invalid(op, format!("layer {id} holds no text"))),
        }
    }

    fn write_text(
        &mut self,
        op: usize,
        layer: &Ref,
        content: impl FnOnce(&RichText) -> RichText,
    ) -> Result<()> {
        let place = self.place(layer)?;
        let mut layer = self.get(&place).own();
        self.unlocked(op, layer.id)?;
        let text = self.text_mut(op, &mut layer)?;
        text.content = content(&text.content);
        self.put(op, &place, layer)
    }

    /// Styling on the characters that stayed is kept.
    pub(super) fn set_text(&mut self, op: usize, layer: &Ref, new: &str) -> Result<()> {
        self.write_text(op, layer, |old| text::replace_keeping_marks(old, new))
    }

    pub(super) fn set_rich_text(&mut self, op: usize, layer: &Ref, spans: &[Span]) -> Result<()> {
        self.write_text(op, layer, |_| text::joined(spans.iter().cloned()))
    }

    pub(super) fn set_attribute(
        &mut self,
        op: usize,
        layer: &Ref,
        namespace: &str,
        key: &str,
        value: Option<&Value>,
    ) -> Result<()> {
        let place = self.place(layer)?;
        let mut layer = self.get(&place).own();
        self.unlocked(op, layer.id)?;
        match value {
            Some(value) => {
                layer
                    .attributes
                    .entry(namespace.to_owned())
                    .or_default()
                    .insert(key.to_owned(), value.clone());
            }
            None => {
                if let Some(keys) = layer.attributes.get_mut(namespace) {
                    keys.remove(key);
                    if keys.is_empty() {
                        layer.attributes.remove(namespace);
                    }
                }
            }
        }
        self.put(op, &place, layer)
    }
}
