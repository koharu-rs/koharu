//! A snapshot of what `set_frame` showed: the frame, the page and project
//! it drew, and what each shown layer offers the canvas. Tools read the same
//! state the frame drew, never the session's newer one.

use std::{collections::HashMap, io::Cursor};

use koharu_renderer::Frame;
use koharu_scene::{
    Content, Frame as TextFrame, Hash, Layer, LayerId, Offset, Page, Project, Session, Text,
};
use vello::kurbo::{Affine, BezPath, Point, Rect, Shape as _, Size};

use crate::control::{Control, Part, Resize};

pub(crate) struct Snapshot {
    frame: Frame,
    page: Page,
    project: Project,
    /// What each shown layer offers, bottom first.
    controls: Vec<Control>,
    /// Every panel's and balloon's outline, parsed once.
    outlines: HashMap<LayerId, BezPath>,
}

impl Snapshot {
    /// `None` when the frame's page is gone.
    pub fn new(frame: Frame, session: &Session, sizes: &mut ImageSizes) -> Option<Self> {
        let project = session.project();
        let page = project.page(frame.page()).ok()?.clone();
        let mut walk = Walk {
            frame: &frame,
            session,
            sizes,
            controls: Vec::new(),
        };
        walk.layers(&page.layers, false);
        let controls = walk.controls;
        let mut outlines = HashMap::new();
        outlines_of(&page.layers, &mut outlines);
        Some(Self {
            frame,
            page,
            project,
            controls,
            outlines,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    pub fn size(&self) -> Size {
        Size::new(
            f64::from(self.page.size.width),
            f64::from(self.page.size.height),
        )
    }

    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    /// The topmost control a click at `page` hits.
    pub fn hit(&self, page: Point) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|control| control.hittable() && control.contains(page))
    }

    /// The topmost control of `layer`.
    pub fn control(&self, layer: LayerId) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|control| control.layer == layer)
    }

    /// A layer's own values, a block's parts not among them.
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.project.layer(id).ok()
    }

    /// A panel's or a balloon's outline on the page.
    pub fn outline(&self, id: LayerId) -> Option<&BezPath> {
        self.outlines.get(&id)
    }

    /// The offset of `id` when it's a raster or a block's clean-up a brush
    /// can write to: shown and unlocked, with every layer it's in.
    pub fn writable(&self, id: LayerId) -> Option<Offset> {
        fn find(layers: &[Layer], id: LayerId) -> Option<Offset> {
            layers
                .iter()
                .filter(|layer| layer.visible && !layer.locked)
                .find_map(|layer| match &layer.content {
                    Content::Raster(raster) if layer.id == id => Some(raster.offset),
                    Content::Block(block)
                        if block.cleanup.id == id
                            && block.cleanup.visible
                            && !block.cleanup.locked =>
                    {
                        Some(block.cleanup.content.offset)
                    }
                    _ => find(&layer.children, id),
                })
        }
        find(&self.page.layers, id)
    }
}

/// Image files' pixel sizes, read from their headers once each.
#[derive(Default)]
pub(crate) struct ImageSizes(HashMap<Hash, Option<(u32, u32)>>);

impl ImageSizes {
    fn get(&mut self, session: &Session, file: Hash) -> Option<(u32, u32)> {
        *self.0.entry(file).or_insert_with(|| {
            let bytes = session.file(&file).ok()?;
            image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()
                .ok()?
                .into_dimensions()
                .ok()
        })
    }
}

struct Walk<'a> {
    frame: &'a Frame,
    session: &'a Session,
    sizes: &'a mut ImageSizes,
    controls: Vec<Control>,
}

impl Walk<'_> {
    /// Hidden layers draw nothing to hit; a locked group locks what's in it.
    fn layers(&mut self, layers: &[Layer], locked: bool) {
        for layer in layers.iter().filter(|layer| layer.visible) {
            let locked = locked || layer.locked;
            self.layer(layer, locked);
            self.layers(&layer.children, locked);
        }
    }

    fn layer(&mut self, layer: &Layer, locked: bool) {
        let id = layer.id;
        let plain = |part, local, transform, resize, rotates| {
            Control::new(id, part, local, transform, resize, rotates, locked)
        };
        let control = match &layer.content {
            Content::Text(text) => {
                let laid = self
                    .frame
                    .layer(id)
                    .and_then(|drawn| drawn.text())
                    .map(|text| text.frame);
                text_control(id, Part::Text, text, laid, locked)
            }
            Content::Block(block) => {
                // The clean-up draws under the translation, and is locked by
                // its own lock as well as its block's.
                let cleanup = &block.cleanup;
                if let Some(bounds) = self
                    .frame
                    .layer(cleanup.id)
                    .and_then(|drawn| drawn.bounds())
                {
                    self.controls.push(Control::new(
                        cleanup.id,
                        Part::Cleanup,
                        bounds,
                        Affine::IDENTITY,
                        Resize::None,
                        false,
                        locked || cleanup.locked,
                    ));
                }
                match &block.translation {
                    Some(text) => {
                        let laid = self
                            .frame
                            .translation(id)
                            .and_then(|drawn| drawn.text())
                            .map(|text| text.frame);
                        text_control(id, Part::Translation, text, laid, locked)
                    }
                    None => text_control(id, Part::Source, &block.source, None, locked),
                }
            }
            Content::Image(image) => {
                self.sizes
                    .get(self.session, image.file)
                    .map(|(width, height)| {
                        let local = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
                        plain(
                            Part::Image,
                            local,
                            Affine::from(image.transform),
                            Resize::All,
                            true,
                        )
                    })
            }
            Content::Shape(shape) => BezPath::from_svg(shape.path.as_str()).ok().map(|path| {
                plain(
                    Part::Shape,
                    path.bounding_box(),
                    Affine::IDENTITY,
                    Resize::All,
                    true,
                )
            }),
            Content::Raster(_) => self
                .frame
                .layer(id)
                .and_then(|drawn| drawn.bounds())
                .map(|bounds| plain(Part::Raster, bounds, Affine::IDENTITY, Resize::None, false)),
            _ => None,
        };
        self.controls.extend(control);
    }
}

/// Every panel's and balloon's outline in `layers`, hidden ones too, which
/// the Layers panel can still select.
fn outlines_of(layers: &[Layer], outlines: &mut HashMap<LayerId, BezPath>) {
    for layer in layers {
        let path = match &layer.content {
            Content::Panel(panel) => Some(panel.outline.as_str()),
            Content::Balloon(balloon) => Some(balloon.outline.as_str()),
            _ => None,
        };
        if let Some(Ok(path)) = path.map(BezPath::from_svg) {
            outlines.insert(layer.id, path);
        }
        outlines_of(&layer.children, outlines);
    }
}

/// A text's control: the box it laid out in, or for one that drew nothing,
/// its frame's box. Point text that drew nothing has none.
fn text_control(
    layer: LayerId,
    part: Part,
    text: &Text,
    laid: Option<Rect>,
    locked: bool,
) -> Option<Control> {
    let (local, resize) = match &text.frame {
        TextFrame::Point => (laid?, Resize::None),
        TextFrame::Box { bounds, .. } => (
            laid.unwrap_or(Rect::new(
                bounds.x,
                bounds.y,
                bounds.x + bounds.width,
                bounds.y + bounds.height,
            )),
            Resize::Edges,
        ),
        TextFrame::Path { path, .. } => (
            laid.or_else(|| {
                BezPath::from_svg(path.as_str())
                    .ok()
                    .map(|path| path.bounding_box())
            })?,
            Resize::Edges,
        ),
    };
    (local.width() > 0.0 && local.height() > 0.0).then(|| {
        Control::new(
            layer,
            part,
            local,
            Affine::from(text.transform),
            resize,
            true,
            locked,
        )
    })
}
