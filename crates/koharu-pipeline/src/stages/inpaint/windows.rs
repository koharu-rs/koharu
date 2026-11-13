//! The windows the inpainting model redraws, at the page's own scale. A whole
//! page fitted to a megapixel left lettering untouched, and a larger window
//! shrunk left glyph-shaped patches, so each window is about a megapixel at
//! most, and no longer on a side than the model redraws in one pass. Each
//! holds its lettering whole with context around it, and never spans two
//! panels.

use koharu_scene::Rect;

/// The context around a lettering the model sees.
const CONTEXT: i64 = 256;
/// The most pixels windows merge into.
const MOST: i64 = 1 << 20;

/// A lettering the model redraws, and the panel it's in, in page pixels.
pub(super) struct Region {
    pub(super) rect: Rect,
    pub(super) panel: Option<Rect>,
}

/// Each region's window, grown by the context within its panel and `side`,
/// merged with the windows it overlaps in the same panel while the merge
/// stays within a megapixel and `side`.
pub(super) fn plan(regions: &[Region], width: u32, height: u32, side: u32) -> Vec<Rect> {
    let side = i64::from(side);
    let page = Bounds::of(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    let mut windows: Vec<(Bounds, Bounds)> = regions
        .iter()
        .map(|region| {
            let limit = region
                .panel
                .map_or(page, |panel| Bounds::of(panel).meet(page));
            let lettering = Bounds::of(region.rect).meet(page);
            // The context shrinks to keep the window within `side`.
            let room = |length: i64| CONTEXT.min(((side - length) / 2).max(0));
            let grown = lettering
                .grow(
                    room(lettering.right - lettering.left),
                    room(lettering.bottom - lettering.top),
                )
                .meet(limit);
            // A lettering past its panel's edge stays whole.
            (limit, grown.join(lettering))
        })
        .collect();
    'merging: loop {
        for first in 0..windows.len() {
            for second in first + 1..windows.len() {
                let ((limit, a), (other, b)) = (windows[first], windows[second]);
                let joined = a.join(b);
                let fits = joined.area() <= MOST
                    && joined.right - joined.left <= side
                    && joined.bottom - joined.top <= side;
                if limit == other && a.overlaps(b) && fits {
                    windows[first].1 = joined;
                    windows.swap_remove(second);
                    continue 'merging;
                }
            }
        }
        break;
    }
    windows
        .into_iter()
        .filter(|(_, window)| window.area() > 0)
        .map(|(_, window)| window.rect())
        .collect()
}

/// A rectangle by its edges, the right and bottom ones exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Bounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl Bounds {
    fn of(rect: Rect) -> Self {
        let (left, top) = (i64::from(rect.x), i64::from(rect.y));
        Self {
            left,
            top,
            right: left + i64::from(rect.width),
            bottom: top + i64::from(rect.height),
        }
    }

    fn rect(self) -> Rect {
        Rect {
            x: self.left as i32,
            y: self.top as i32,
            width: (self.right - self.left) as u32,
            height: (self.bottom - self.top) as u32,
        }
    }

    fn grow(self, across: i64, down: i64) -> Self {
        Self {
            left: self.left - across,
            top: self.top - down,
            right: self.right + across,
            bottom: self.bottom + down,
        }
    }

    /// The part inside `other`, empty when there's none.
    fn meet(self, other: Self) -> Self {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        Self {
            left,
            top,
            right: self.right.min(other.right).max(left),
            bottom: self.bottom.min(other.bottom).max(top),
        }
    }

    fn join(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn overlaps(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    fn area(self) -> i64 {
        (self.right - self.left) * (self.bottom - self.top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn windows_merge_within_a_panel_and_a_megapixel_only() {
        let left = Some(rect(0, 0, 1000, 2000));
        let right = Some(rect(1000, 0, 1000, 2000));
        let regions = [
            // Near each other in the left panel: one window.
            Region {
                rect: rect(100, 100, 50, 80),
                panel: left,
            },
            Region {
                rect: rect(200, 150, 40, 60),
                panel: left,
            },
            // Close by, but across the panel's edge: a window of its own.
            Region {
                rect: rect(1050, 100, 50, 80),
                panel: right,
            },
            // Lower in the left panel: merging would pass a megapixel.
            Region {
                rect: rect(100, 600, 900, 400),
                panel: left,
            },
        ];
        let windows = plan(&regions, 2000, 2000, u32::MAX);
        assert_eq!(windows.len(), 3);
        assert!(windows.contains(&rect(0, 0, 496, 466)));
        assert!(windows.contains(&rect(1000, 0, 356, 436)));
        // Every window stays inside its lettering's panel.
        assert!(
            windows
                .iter()
                .all(|window| window.x + window.width as i32 <= 1000 || window.x >= 1000)
        );
    }
}
