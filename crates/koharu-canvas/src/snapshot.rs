//! A snapshot of what the canvas shows: the frame, the page and project it
//! drew, and what each shown layer offers the canvas. Tools read the same
//! state the frame drew, never the session's newer one.

use std::{collections::HashMap, io::Cursor, sync::LazyLock};

use koharu_renderer::{Frame, ResolvedText};
use koharu_scene::{
    Content, Frame as TextFrame, Hash, Layer, LayerId, Offset, Page, PixelFormat, Project, Role,
    Session, Text,
};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _, Size};

use crate::control::{Control, Resize};

/// What `Canvas::show` shows. The host makes it from the session the frame
/// drew, on its own thread, so the canvas's never waits on the session.
pub struct Snapshot {
    frame: Frame,
    page: Page,
    project: Project,
    /// What each shown layer offers, bottom first.
    controls: Vec<Control>,
    /// Every panel's and balloon's outline, parsed once.
    outlines: HashMap<LayerId, BezPath>,
    /// The page's texts in the scene's reading order, each with whether
    /// it's a sound effect, which Story numbers apart.
    story: Vec<(LayerId, bool)>,
}

impl Snapshot {
    /// `None` when the frame's page is gone.
    pub fn new(frame: Frame, session: &Session) -> Option<Self> {
        let project = session.project();
        let page = project.page(frame.page()).ok()?.clone();
        let mut walk = Walk {
            frame: &frame,
            session,
            controls: Vec::new(),
        };
        walk.layers(&page.layers, false);
        let controls = walk.controls;
        let mut outlines = HashMap::new();
        outlines_of(&page.layers, &mut outlines);
        let story = project
            .story(page.id)
            .unwrap_or_default()
            .into_iter()
            .map(|id| {
                let sfx = project.layer(id).is_ok_and(|layer| {
                    matches!(&layer.content, Content::Block(block) if block.role == Role::Sfx)
                });
                (id, sfx)
            })
            .collect();
        Some(Self {
            frame,
            page,
            project,
            controls,
            outlines,
            story,
        })
    }

    pub(crate) fn story(&self) -> &[(LayerId, bool)] {
        &self.story
    }

    pub(crate) fn frame(&self) -> &Frame {
        &self.frame
    }

    pub(crate) fn page(&self) -> &Page {
        &self.page
    }

    pub(crate) fn project(&self) -> &Project {
        &self.project
    }

    pub(crate) fn size(&self) -> Size {
        Size::new(
            f64::from(self.page.size.width),
            f64::from(self.page.size.height),
        )
    }

    pub(crate) fn controls(&self) -> &[Control] {
        &self.controls
    }

    /// The topmost control a click at `page` picks.
    pub(crate) fn hit(&self, page: Point) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|control| control.picks && !control.locked && control.contains(page))
    }

    /// The topmost control of `layer`.
    pub(crate) fn control(&self, layer: LayerId) -> Option<&Control> {
        self.controls
            .iter()
            .rev()
            .find(|control| control.layer == layer)
    }

    /// A layer's own values, a block's parts not among them.
    pub(crate) fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.project.layer(id).ok()
    }

    /// A panel's or a balloon's outline on the page.
    pub(crate) fn outline(&self, id: LayerId) -> Option<&BezPath> {
        self.outlines.get(&id)
    }

    /// Which part of a block `id` is, when it's a block's clean-up or mask,
    /// shown or not.
    pub(crate) fn part(&self, id: LayerId) -> Option<Part> {
        fn find(layers: &[Layer], id: LayerId) -> Option<Part> {
            layers.iter().find_map(|layer| match &layer.content {
                Content::Block(block) if block.cleanup.id == id => Some(Part::Cleanup),
                Content::Block(block) if block.mask.id == id => Some(Part::Mask {
                    cleanup: block.cleanup.id,
                }),
                _ => find(&layer.children, id),
            })
        }
        find(&self.page.layers, id)
    }

    /// The offset and format of `id` when it's a raster, or a block's
    /// clean-up or mask, a brush can write to: shown and unlocked, with
    /// every layer it's in.
    pub(crate) fn writable(&self, id: LayerId) -> Option<(Offset, PixelFormat)> {
        fn find(layers: &[Layer], id: LayerId) -> Option<(Offset, PixelFormat)> {
            layers
                .iter()
                .filter(|layer| layer.visible && !layer.locked)
                .find_map(|layer| match &layer.content {
                    Content::Raster(raster) if layer.id == id => {
                        Some((raster.offset, PixelFormat::Rgba8))
                    }
                    Content::Block(block)
                        if block.cleanup.id == id
                            && block.cleanup.visible
                            && !block.cleanup.locked =>
                    {
                        Some((block.cleanup.content.offset, PixelFormat::Rgba8))
                    }
                    Content::Block(block)
                        if block.mask.id == id && block.mask.visible && !block.mask.locked =>
                    {
                        Some((block.mask.content.offset, PixelFormat::Gray8))
                    }
                    _ => find(&layer.children, id),
                })
        }
        find(&self.page.layers, id)
    }
}

/// A block's own picture layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Cleanup,
    /// The mask its block's clean-up shows through.
    Mask {
        cleanup: LayerId,
    },
}

/// Image files' pixel sizes, read from their headers once each. A file is
/// named by its contents' hash, so a size holds in every project.
static IMAGE_SIZES: LazyLock<quick_cache::sync::Cache<Hash, Option<(u32, u32)>>> =
    LazyLock::new(|| quick_cache::sync::Cache::new(4096));

/// `file`'s pixel size, read from its header the first time.
fn image_size(session: &Session, file: Hash) -> Option<(u32, u32)> {
    if let Some(size) = IMAGE_SIZES.get(&file) {
        return size;
    }
    let size = session.file(&file).ok().and_then(|bytes| {
        image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()
    });
    IMAGE_SIZES.insert(file, size);
    size
}

struct Walk<'a> {
    frame: &'a Frame,
    session: &'a Session,
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
        let plain = |local, transform, resize, rotates| {
            Control::new(id, local, transform, resize, rotates, locked)
        };
        let control = match &layer.content {
            Content::Text(text) => {
                let laid = self.frame.layer(id).and_then(|drawn| drawn.text());
                text_control(id, text, laid, locked)
            }
            Content::Block(block) => {
                // The clean-up draws under the translation, and is locked by
                // its own lock as well as its block's. Only Layers picks it.
                let cleanup = &block.cleanup;
                if let Some(bounds) = self
                    .frame
                    .layer(cleanup.id)
                    .and_then(|drawn| drawn.bounds())
                {
                    self.controls.push(
                        Control::new(
                            cleanup.id,
                            bounds,
                            Affine::IDENTITY,
                            Resize::None,
                            false,
                            locked || cleanup.locked,
                        )
                        .unpicked(),
                    );
                }
                match &block.translation {
                    Some(text) => {
                        let laid = self.frame.translation(id).and_then(|drawn| drawn.text());
                        text_control(id, text, laid, locked)
                    }
                    None => text_control(id, &block.source, None, locked),
                }
            }
            Content::Image(image) => image_size(self.session, image.file).map(|(width, height)| {
                let local = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
                plain(local, Affine::from(image.transform), Resize::All, true)
            }),
            Content::Shape(shape) => BezPath::from_svg(shape.path.as_str())
                .ok()
                .map(|path| plain(path.bounding_box(), Affine::IDENTITY, Resize::All, true)),
            Content::Raster(_) => self
                .frame
                .layer(id)
                .and_then(|drawn| drawn.bounds())
                .map(|bounds| plain(bounds, Affine::IDENTITY, Resize::None, false)),
            _ => None,
        };
        self.controls.extend(control);
    }
}

/// Every panel's and balloon's outline in `layers`, hidden ones too, which
/// the Layers panel can still select, and every block's mask's: its
/// source's frame, where the mask was made, since a mask draws nothing to
/// outline.
fn outlines_of(layers: &[Layer], outlines: &mut HashMap<LayerId, BezPath>) {
    for layer in layers {
        let path = match &layer.content {
            Content::Panel(panel) => BezPath::from_svg(panel.outline.as_str()).ok(),
            Content::Balloon(balloon) => BezPath::from_svg(balloon.outline.as_str()).ok(),
            Content::Block(block) => {
                if let Some(frame) = frame_outline(&block.source) {
                    outlines.insert(block.mask.id, frame);
                }
                None
            }
            _ => None,
        };
        if let Some(path) = path {
            outlines.insert(layer.id, path);
        }
        outlines_of(&layer.children, outlines);
    }
}

/// `text`'s frame on the page, its transform applied; `None` for point text.
fn frame_outline(text: &Text) -> Option<BezPath> {
    let path = match &text.frame {
        TextFrame::Point => return None,
        TextFrame::Box { bounds, .. } => Rect::from(*bounds).to_path(0.1),
        TextFrame::Path { path, .. } => BezPath::from_svg(path.as_str()).ok()?,
    };
    Some(Affine::from(text.transform) * path)
}

/// A text's control: the box it laid out in, or for one that drew nothing,
/// its frame's box, and whether what it laid out runs past its frame. Point
/// text that drew nothing has none.
fn text_control(
    layer: LayerId,
    text: &Text,
    laid: Option<&ResolvedText>,
    locked: bool,
) -> Option<Control> {
    let overflows = laid.is_some_and(|laid| laid.overflowed);
    let laid = laid.map(|laid| laid.frame);
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
        let mut control = Control::new(
            layer,
            local,
            Affine::from(text.transform),
            resize,
            true,
            locked,
        );
        control.overflows = overflows;
        control
    })
}
