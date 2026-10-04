//! Webtoon strip slicing.
//!
//! Vertical strips are the one input shape the detection models cannot absorb:
//! RF-DETR Seg 2XL resizes every page to a fixed square, so a 720x14317 strip
//! loses roughly 95% of its vertical resolution and its speech bubbles stop
//! being detectable at all. Rather than teaching every downstream stage about
//! strips, the strip is divided into pages that keep the original aspect ratio
//! once, at import time.
//!
//! The planner is deliberately pure: it consumes a per-row luma profile and
//! returns cut positions, so the geometry can be tested exhaustively without an
//! image, a model, or a temporary file.

mod bands;
mod params;
mod plan;
mod profile;
#[cfg(test)]
mod tests;

pub use self::{
    params::SliceParams, plan::SlicePlan, plan::plan_slices, profile::RowStat, profile::row_profile,
};

use self::bands::{BlankBand, blank_bands};
