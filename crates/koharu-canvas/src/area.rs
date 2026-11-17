//! The area the marquee and the lasso mark on the page, as Photoshop's
//! selection is: a byte a page pixel saying how far each is in, which a new
//! shape replaces, adds to, or takes from, and whose edge shows as marching
//! ants. A fill redraws it.

use image::{GrayImage, imageops};
use imageproc::contours::find_contours_with_threshold;
use kurbo::{BezPath, Size};

use crate::tools::brush::coverage;

/// How a new shape joins the area, as the modifier held at its press picks
/// with Photoshop's selection tools: on its own, Shift adding to it, and Alt
/// taking from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Replace,
    Add,
    Subtract,
}

pub(crate) struct Area {
    /// Over the whole page.
    mask: GrayImage,
    /// Where it isn't 0.
    bounds: koharu_scene::Rect,
    /// Where it's more in than out, through its pixels' centres, in page
    /// pixels.
    edge: BezPath,
}

impl Area {
    /// `shape` joined to `old` by `mode` on a page of `size`; `None` once
    /// nothing is left, as when a click marks nothing on its own, which
    /// deselects, as a click with Photoshop's marquee does.
    pub fn marked(old: Option<Area>, shape: &BezPath, mode: Mode, size: Size) -> Option<Area> {
        let (width, height) = (size.width as u32, size.height as u32);
        let mut mask = match (mode, old) {
            (Mode::Add | Mode::Subtract, Some(old)) if old.mask.dimensions() == (width, height) => {
                old.mask
            }
            _ => GrayImage::new(width, height),
        };
        // The coverage lies on the page.
        if let Some((rect, covered)) = coverage(shape, size) {
            for (index, &amount) in covered.iter().enumerate() {
                let index = index as u32;
                let (x, y) = (
                    rect.x as u32 + index % rect.width,
                    rect.y as u32 + index / rect.width,
                );
                let pixel = &mut mask.get_pixel_mut(x, y).0[0];
                *pixel = match mode {
                    Mode::Replace | Mode::Add => (*pixel).max(amount),
                    Mode::Subtract => {
                        let kept = u16::from(*pixel) * (255 - u16::from(amount));
                        ((kept + 127) / 255) as u8
                    }
                };
            }
        }
        Self::of(mask)
    }

    /// `mask` as an area, `None` when it's empty.
    fn of(mask: GrayImage) -> Option<Area> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for (x, y, pixel) in mask.enumerate_pixels() {
            if pixel.0[0] != 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        if x0 > x1 {
            return None;
        }
        let bounds = koharu_scene::Rect {
            x: x0 as i32,
            y: y0 as i32,
            width: x1 - x0 + 1,
            height: y1 - y0 + 1,
        };
        // Traced inside a pixel of space: imageproc starts an outer border only
        // past a pixel that's out, so one along the image's edge is never
        // traced.
        let mut room = GrayImage::new(bounds.width + 2, bounds.height + 2);
        imageops::replace(
            &mut room,
            &*imageops::crop_imm(&mask, x0, y0, bounds.width, bounds.height),
            1,
            1,
        );
        let mut edge = BezPath::new();
        for contour in find_contours_with_threshold::<u32>(&room, 127) {
            let mut points = contour
                .points
                .iter()
                .map(|point| (f64::from(x0 + point.x) - 0.5, f64::from(y0 + point.y) - 0.5));
            let Some(first) = points.next() else {
                continue;
            };
            edge.move_to(first);
            for point in points {
                edge.line_to(point);
            }
            edge.close_path();
        }
        Some(Self { mask, bounds, edge })
    }

    /// Where it lies on the page, in page pixels.
    pub fn bounds(&self) -> kurbo::Rect {
        let koharu_scene::Rect {
            x,
            y,
            width,
            height,
        } = self.bounds;
        kurbo::Rect::new(
            f64::from(x),
            f64::from(y),
            f64::from(x) + f64::from(width),
            f64::from(y) + f64::from(height),
        )
    }

    pub fn edge(&self) -> &BezPath {
        &self.edge
    }

    /// Its bounds and the mask over them, a byte a pixel, as the
    /// pipeline's area run takes them.
    pub fn cropped(&self) -> (koharu_scene::Rect, Vec<u8>) {
        let koharu_scene::Rect {
            x,
            y,
            width,
            height,
        } = self.bounds;
        let crop = imageops::crop_imm(&self.mask, x as u32, y as u32, width, height);
        (self.bounds, crop.to_image().into_raw())
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Rect, Shape as _, Size};

    use super::{Area, Mode};

    #[test]
    fn an_area_filling_its_bounds_to_their_edges_has_an_edge() {
        let shape = Rect::new(10.0, 10.0, 50.0, 50.0).to_path(0.1);
        let area = Area::marked(None, &shape, Mode::Replace, Size::new(100.0, 100.0))
            .expect("the rectangle marks an area");
        assert_eq!(
            area.edge().bounding_box(),
            Rect::new(10.5, 10.5, 49.5, 49.5)
        );
    }
}
