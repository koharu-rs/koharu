use std::cmp::Ordering;

use super::{BlankBand, RowStat, SliceParams, blank_bands};

/// Weight of band thickness against the distance from the ideal cut position.
///
/// Thickness dominates because a wide gutter is the strongest available
/// evidence that nothing important crosses the cut, while the ideal position
/// only decides between candidates that already qualify.
const BAND_WIDTH_WEIGHT: f64 = 2.5;
const IDEAL_DISTANCE_WEIGHT: f64 = 0.05;

/// A validated division of a webtoon strip into pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlicePlan {
    pub width: u32,
    pub height: u32,
    /// Ascending cut positions, each satisfying `0 < y < height`.
    pub cuts: Vec<u32>,
}

impl SlicePlan {
    /// Returns each page as `(y_offset, height)`.
    #[must_use]
    pub fn page_ranges(&self) -> Vec<(u32, u32)> {
        let mut ranges = Vec::with_capacity(self.page_count());
        let mut start = 0;
        for &cut in &self.cuts {
            ranges.push((start, cut - start));
            start = cut;
        }
        ranges.push((start, self.height - start));
        ranges
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.cuts.len() + 1
    }
}

/// Plans the page cuts for a webtoon strip, or returns `None` when the strip
/// should be left whole.
///
/// `None` is the signal that every later stage can keep assuming a single
/// page: the caller skips slicing entirely instead of receiving an empty plan.
///
/// `profile` must come from [`row_profile`](super::row_profile) over the same
/// `width` and `height`.
#[must_use]
pub fn plan_slices(
    width: u32,
    height: u32,
    profile: &[RowStat],
    params: &SliceParams,
) -> Option<SlicePlan> {
    // A strip that is short, or not extreme enough to be a webtoon, is left
    // untouched: slicing it would trade a readable page for a fragment.
    if height <= params.min_sliceable_height
        || height as f32 <= params.trigger_aspect * width as f32
    {
        return None;
    }
    if (profile.len() as u32) < height {
        return None;
    }

    let bands = blank_bands(profile, params);
    let page_budget = height.div_ceil(params.target_height).max(2);
    // Bounds the advance between two cuts even if a future edit relaxes the
    // search window; the window itself already starts at `min_height`.
    let minimum_advance = (params.min_height / 2).max(64);

    let mut cuts = Vec::new();
    let mut cursor = 0u32;
    let mut ordinal = 1u32;
    while height - cursor > params.max_height {
        // Clamping the upper bound to `height - min_height` is what keeps the
        // final page from collapsing into a sliver: a cut near the bottom edge
        // is geometrically legal but produces a page no reader can use.
        let window_start = cursor.saturating_add(params.min_height);
        let window_end = cursor
            .saturating_add(params.max_height)
            .min(height.saturating_sub(params.min_height));
        if window_start > window_end {
            // Unreachable while `max_height <= 2 * min_height`, which holds for
            // the defaults; a wider maximum would leave a tail too short to
            // split and too long to emit.
            break;
        }

        let ideal =
            (f64::from(ordinal) * f64::from(height) / f64::from(page_budget)).round() as u32;
        let cut = select_cut(profile, &bands, window_start, window_end, ideal, params)
            .max(cursor.saturating_add(minimum_advance));
        debug_assert!(
            cut > cursor && cut < height,
            "cut {cut} does not advance past {cursor}"
        );

        cuts.push(cut);
        cursor = cut;
        ordinal += 1;
    }

    let plan = SlicePlan {
        width,
        height,
        cuts,
    };
    validate(&plan, params);
    Some(plan)
}

/// Picks the cut inside `[window_start, window_end]`, preferring a validated
/// blank band and falling back to the flattest row when no band qualifies.
fn select_cut(
    profile: &[RowStat],
    bands: &[BlankBand],
    window_start: u32,
    window_end: u32,
    ideal: u32,
    params: &SliceParams,
) -> u32 {
    let band_cut = bands
        .iter()
        .filter_map(|band| {
            // A band without clearance on both sides is not a cut candidate.
            band.cut_window(params)?;
            let center = band.center();
            (window_start..=window_end).contains(&center).then_some((
                f64::from(band.width()) * BAND_WIDTH_WEIGHT
                    - f64::from(center.abs_diff(ideal)) * IDEAL_DISTANCE_WEIGHT,
                center,
            ))
        })
        // Highest score wins; the earlier band wins a tie so the result does
        // not depend on the iteration order of equal candidates.
        .min_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(Ordering::Equal)
                .then(left.1.cmp(&right.1))
        })
        .map(|(_, center)| center);

    band_cut.unwrap_or_else(|| {
        // No band survived validation, so nothing here is provably safe; the
        // flattest row is the least damaging guess and keeps pathological
        // inputs such as uniform noise sliceable instead of panicking.
        (window_start..=window_end)
            .map(|y| {
                let stat = &profile[y as usize];
                (stat.gradient + f64::from(stat.max - stat.min), y)
            })
            .min_by(|left, right| {
                left.0
                    .partial_cmp(&right.0)
                    .unwrap_or(Ordering::Equal)
                    .then(left.1.cmp(&right.1))
            })
            .map_or(window_start, |(_, y)| y)
    })
}

/// Asserts the guarantees callers rely on. Production paths express violations
/// through the returned plan instead, so this only fires in debug builds.
fn validate(plan: &SlicePlan, params: &SliceParams) {
    let ranges = plan.page_ranges();
    let last = ranges.len().saturating_sub(1);
    for (index, &(_, page_height)) in ranges.iter().enumerate() {
        debug_assert!(
            page_height <= params.max_height,
            "page {index} is {page_height} tall, above max_height {}",
            params.max_height
        );
        debug_assert!(
            index == last || page_height >= params.min_height,
            "page {index} is {page_height} tall, below min_height {}",
            params.min_height
        );
    }

    let minimum_spacing = (params.min_height / 2).max(64);
    for pair in plan.cuts.windows(2) {
        debug_assert!(
            pair[1] - pair[0] >= minimum_spacing,
            "cuts {} and {} are closer than {minimum_spacing}",
            pair[0],
            pair[1]
        );
    }
}
