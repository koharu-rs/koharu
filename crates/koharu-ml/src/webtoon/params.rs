/// Tuning knobs for the webtoon slice planner.
///
/// The defaults encode the geometry the planner is designed around: a slice
/// should stay close to the reader's viewport height, but the search window has
/// to be wide enough that a misplaced cut never forces a page outside the
/// model's usable range. `max_height == 2 * min_height` is a hard requirement of
/// the search, not a coincidence: it guarantees that whenever the remaining
/// tail is longer than `max_height` there is still room to cut and leave a
/// legal final page behind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliceParams {
    /// Preferred page height. Drives how many pages the strip is divided into.
    pub target_height: u32,
    /// Shortest page the planner may emit, excluding a final remainder page.
    pub min_height: u32,
    /// Longest page the planner may emit.
    pub max_height: u32,
    /// Height-to-width ratio above which a strip is considered a webtoon.
    pub trigger_aspect: f32,
    /// Strips shorter than this are never sliced, because slicing them would
    /// only produce one full page plus a fragment.
    pub min_sliceable_height: u32,
    /// Maximum luma spread within a row that still counts as near-uniform.
    pub blank_range: u8,
    /// Near-uniform rows at or above this mean are treated as white space.
    pub blank_luma: u8,
    /// Near-uniform rows at or below this mean are treated as dark space.
    pub dark_luma: u8,
    /// Blank rows required above and below a cut for it to be considered safe.
    pub airspace_px: u32,
}

impl Default for SliceParams {
    fn default() -> Self {
        Self {
            target_height: 1600,
            min_height: 1200,
            max_height: 2400,
            trigger_aspect: 3.0,
            min_sliceable_height: 2400,
            blank_range: 24,
            blank_luma: 200,
            dark_luma: 60,
            airspace_px: 3,
        }
    }
}
