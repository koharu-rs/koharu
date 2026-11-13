//! A layer as the canvas handles it: the frame it's hit and outlined by, the
//! handles it offers, and the resize and turn they make, all in page space.

use cursor_icon::CursorIcon;
use koharu_scene::LayerId;
use vello::kurbo::{Affine, BezPath, Line, ParamCurveNearest as _, Point, Rect, Shape as _};

use crate::view::View;

/// A handle's reach, in device-independent pixels, as the editor's handles
/// took a 14-pixel square.
const REACH: f64 = 7.0;

/// How far the rotate handle stands above the top, in device-independent
/// pixels.
const ROTATE: f64 = 16.0;

/// A selected layer's handles, clockwise from the top left, and the rotate
/// handle above its top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    Rotate,
}

impl Handle {
    /// The pointer's shape over it, in CSS's names.
    pub fn cursor(self) -> CursorIcon {
        match self {
            Self::NorthWest => CursorIcon::NwResize,
            Self::North => CursorIcon::NResize,
            Self::NorthEast => CursorIcon::NeResize,
            Self::East => CursorIcon::EResize,
            Self::SouthEast => CursorIcon::SeResize,
            Self::South => CursorIcon::SResize,
            Self::SouthWest => CursorIcon::SwResize,
            Self::West => CursorIcon::WResize,
            Self::Rotate => CursorIcon::Alias,
        }
    }

    /// Which way it moves its sides: -1 the near one, 1 the far one.
    fn sides(self) -> (i8, i8) {
        match self {
            Self::NorthWest => (-1, -1),
            Self::North => (0, -1),
            Self::NorthEast => (1, -1),
            Self::East => (1, 0),
            Self::SouthEast => (1, 1),
            Self::South => (0, 1),
            Self::SouthWest => (-1, 1),
            Self::West => (-1, 0),
            Self::Rotate => (0, 0),
        }
    }
}

/// Which handles resize a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resize {
    All,
    /// The four sides only, as a text's frame resized in the editor.
    Edges,
    None,
}

/// What a control is of, for the edit a gesture on it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Text,
    /// A block's translation.
    Translation,
    /// A block's source, when it has no translation.
    Source,
    Image,
    Shape,
    Raster,
    /// A block's clean-up, which the Layers panel selects and a click never
    /// hits.
    Cleanup,
}

/// A layer as the canvas hits, outlines, and transforms it: `local` in the
/// layer's own coordinates, put on the page by `transform`.
#[derive(Clone, Debug)]
pub(crate) struct Control {
    pub layer: LayerId,
    pub part: Part,
    pub resize: Resize,
    pub rotates: bool,
    pub locked: bool,
    local: Rect,
    transform: Affine,
    /// `local` on the page, and its bounds, kept since every pointer move
    /// hit-tests them.
    outline: BezPath,
    bounds: Rect,
}

impl Control {
    pub fn new(
        layer: LayerId,
        part: Part,
        local: Rect,
        transform: Affine,
        resize: Resize,
        rotates: bool,
        locked: bool,
    ) -> Self {
        let outline = transform * local.to_path(0.1);
        let bounds = outline.bounding_box();
        Self {
            layer,
            part,
            resize,
            rotates,
            locked,
            local,
            transform,
            outline,
            bounds,
        }
    }

    pub fn local(&self) -> Rect {
        self.local
    }

    pub fn transform(&self) -> Affine {
        self.transform
    }

    /// Its frame on the page.
    pub fn outline(&self) -> &BezPath {
        &self.outline
    }

    pub fn contains(&self, page: Point) -> bool {
        let Rect { x0, y0, x1, y1 } = self.bounds;
        (x0..=x1).contains(&page.x) && (y0..=y1).contains(&page.y) && self.outline.contains(page)
    }

    /// Its corners on the page, clockwise from the top left.
    pub fn corners(&self) -> [Point; 4] {
        let Rect { x0, y0, x1, y1 } = self.local;
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|corner| self.transform * Point::from(corner))
    }

    /// Whether it lies wholly inside the page rect `rect`.
    pub fn inside(&self, rect: Rect) -> bool {
        rect.contains_rect(self.bounds)
    }

    /// Whether a click hits it.
    pub fn hittable(&self) -> bool {
        !self.locked && self.part != Part::Cleanup
    }

    /// The handles it offers, the rotate handle first.
    pub fn handles(&self) -> Vec<Handle> {
        if self.locked {
            return Vec::new();
        }
        let mut handles = Vec::new();
        if self.rotates {
            handles.push(Handle::Rotate);
        }
        if self.resize == Resize::All {
            handles.extend([
                Handle::NorthWest,
                Handle::NorthEast,
                Handle::SouthEast,
                Handle::SouthWest,
            ]);
        }
        if self.resize != Resize::None {
            handles.extend([Handle::North, Handle::East, Handle::South, Handle::West]);
        }
        handles
    }

    /// Where `handle` is on the page; the rotate handle stands a fixed
    /// distance on screen above the top.
    pub fn handle_point(&self, handle: Handle, view: &View) -> Point {
        let Rect { x0, y0, x1, y1 } = self.local;
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let local = match handle {
            Handle::NorthWest => (x0, y0),
            Handle::North | Handle::Rotate => (cx, y0),
            Handle::NorthEast => (x1, y0),
            Handle::East => (x1, cy),
            Handle::SouthEast => (x1, y1),
            Handle::South => (cx, y1),
            Handle::SouthWest => (x0, y1),
            Handle::West => (x0, cy),
        };
        let point = self.transform * Point::from(local);
        if handle != Handle::Rotate {
            return point;
        }
        let up = self.transform * Point::new(cx, y0 - 1.0) - point;
        match up.hypot() > 0.0 {
            true => point + up.normalize() * view.page_length(ROTATE),
            false => point,
        }
    }

    /// The handle at the page point `at`. A side of a control resizing by
    /// its edges is caught along its length.
    pub fn handle_at(&self, at: Point, view: &View) -> Option<Handle> {
        let reach = view.page_length(REACH);
        let [nw, ne, se, sw] = self.corners();
        self.handles().into_iter().find(|&handle| {
            let side = match handle {
                Handle::North => Some(Line::new(nw, ne)),
                Handle::East => Some(Line::new(ne, se)),
                Handle::South => Some(Line::new(se, sw)),
                Handle::West => Some(Line::new(sw, nw)),
                _ => None,
            };
            match (self.resize, side) {
                (Resize::Edges, Some(side)) => side.nearest(at, 1e-6).distance_sq <= reach * reach,
                _ => self.handle_point(handle, view).distance(at) <= reach,
            }
        })
    }

    /// The frame a resize from `handle` gives it with the pointer at the
    /// page point `to`: the opposite side or corner stays, as the editor's
    /// `resizeFrame` kept it, and no side gets shorter than `minimum`.
    pub fn resize_to(&self, handle: Handle, to: Point, minimum: f64) -> Rect {
        let at = self.transform.inverse() * to;
        let Rect {
            mut x0,
            mut y0,
            mut x1,
            mut y1,
        } = self.local;
        let (dx, dy) = handle.sides();
        match dx {
            1 => x1 = at.x.max(x0 + minimum),
            -1 => x0 = at.x.min(x1 - minimum),
            _ => {}
        }
        match dy {
            1 => y1 = at.y.max(y0 + minimum),
            -1 => y0 = at.y.min(y1 - minimum),
            _ => {}
        }
        Rect::new(x0, y0, x1, y1)
    }

    /// The turn about its centre that takes the pointer from `from` to `to`,
    /// as the editor's `rotateFrame` turned.
    pub fn turn(&self, from: Point, to: Point) -> Affine {
        let center = self.transform * self.local.center();
        let (from, to) = (from - center, to - center);
        if from.hypot() <= 1e-6 || to.hypot() <= 1e-6 {
            return Affine::IDENTITY;
        }
        Affine::rotate_about(from.cross(to).atan2(from.dot(to)), center)
    }

    /// It carried along the page-space `affine`.
    pub fn moved(&self, affine: Affine) -> Self {
        self.rebuilt(self.local, affine * self.transform)
    }

    /// It with the frame `local`.
    pub fn with_local(&self, local: Rect) -> Self {
        self.rebuilt(local, self.transform)
    }

    fn rebuilt(&self, local: Rect, transform: Affine) -> Self {
        Self::new(
            self.layer,
            self.part,
            local,
            transform,
            self.resize,
            self.rotates,
            self.locked,
        )
    }
}
