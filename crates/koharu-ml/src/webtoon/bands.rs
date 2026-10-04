use std::ops::RangeInclusive;

use super::{RowStat, SliceParams};

/// A maximal run of consecutive near-uniform rows, as `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlankBand {
    pub start: u32,
    pub end: u32,
}

impl BlankBand {
    pub fn width(&self) -> u32 {
        self.end - self.start
    }

    /// Row the cut aims for, which keeps a tie between equally wide bands
    /// resolved towards the earlier one.
    pub fn center(&self) -> u32 {
        (self.start + self.end - 1) / 2
    }

    /// Rows of this band that keep `airspace_px` blank rows on both sides.
    ///
    /// A flat row on its own does not prove the strip can be cut there: the
    /// gap between two lines of text inside a speech bubble the detector has
    /// not found yet is just as flat as a panel gutter. Requiring clearance
    /// above and below is what separates a real gutter from an accident of
    /// local content, so a band narrower than `2 * airspace_px + 1` offers no
    /// valid cut at all.
    pub fn cut_window(&self, params: &SliceParams) -> Option<RangeInclusive<u32>> {
        let start = self.start.checked_add(params.airspace_px)?;
        let end = self.end.checked_sub(params.airspace_px + 1)?;
        (start <= end).then_some(start..=end)
    }
}

/// Reports whether a row is uniform enough to be page padding.
pub(crate) fn is_blank_row(stat: &RowStat, params: &SliceParams) -> bool {
    stat.max - stat.min <= params.blank_range
        && (stat.mean >= f32::from(params.blank_luma) || stat.mean <= f32::from(params.dark_luma))
}

/// Collects the maximal runs of near-uniform rows in `profile`.
pub(crate) fn blank_bands(profile: &[RowStat], params: &SliceParams) -> Vec<BlankBand> {
    let mut bands = Vec::new();
    let mut start: Option<u32> = None;
    for stat in profile {
        if is_blank_row(stat, params) {
            start.get_or_insert(stat.y);
        } else if let Some(begin) = start.take() {
            bands.push(BlankBand {
                start: begin,
                end: stat.y,
            });
        }
    }
    if let Some(begin) = start {
        bands.push(BlankBand {
            start: begin,
            end: profile.last().map_or(begin, |stat| stat.y + 1),
        });
    }
    bands
}
