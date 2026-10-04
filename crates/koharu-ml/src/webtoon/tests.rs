use image::{GrayImage, Luma};

use super::{BlankBand, SliceParams, blank_bands, plan_slices, row_profile};

/// Geometry of the synthetic strips: nine content panels separated by gutters,
/// which is the 720x14317 shape observed on real vertical strips.
const WIDTH: u32 = 720;
const PANEL_HEIGHT: u32 = 1509;
const GUTTER_HEIGHT: u32 = 92;
const PANELS: u32 = 9;
const PERIOD: u32 = PANEL_HEIGHT + GUTTER_HEIGHT;
const HEIGHT: u32 = PANELS * PANEL_HEIGHT + (PANELS - 1) * GUTTER_HEIGHT;

/// Luma of a content pixel. Every row sweeps the full range, so content is
/// never mistaken for padding regardless of the blank detection thresholds.
fn content_luma(x: u32, y: u32) -> u8 {
    ((x * 7 + y * 13) % 256) as u8
}

fn is_gutter(y: u32) -> bool {
    y % PERIOD >= PANEL_HEIGHT
}

/// Builds a strip of content panels separated by uniform gutters.
fn panel_strip(gutter_luma: u8) -> GrayImage {
    GrayImage::from_fn(WIDTH, HEIGHT, |x, y| {
        Luma([if is_gutter(y) {
            gutter_luma
        } else {
            content_luma(x, y)
        }])
    })
}

fn assert_pages_are_legal(plan: &super::SlicePlan, params: &SliceParams) {
    assert_eq!(plan.cuts.len() + 1, plan.page_count());
    assert_eq!(plan.page_ranges().len(), plan.page_count());
    assert!(plan.cuts.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(plan.cuts.iter().all(|&cut| cut > 0 && cut < plan.height));

    let heights: Vec<u32> = plan.page_ranges().iter().map(|&(_, h)| h).collect();
    assert_eq!(heights.iter().sum::<u32>(), plan.height);
    for (index, &height) in heights.iter().enumerate() {
        assert!(
            (params.min_height..=params.max_height).contains(&height),
            "page {index} is {height} tall, outside {}..={}",
            params.min_height,
            params.max_height
        );
    }
    assert!(
        plan.cuts
            .windows(2)
            .all(|pair| pair[1] - pair[0] >= (params.min_height / 2).max(64))
    );
}

#[test]
fn row_profile_reports_luma_extremes_and_gradient() {
    let image = GrayImage::from_vec(3, 2, vec![0, 10, 20, 255, 0, 255]).unwrap();
    let profile = row_profile(&image);
    assert_eq!(profile.len(), 2);
    assert_eq!((profile[0].y, profile[0].min, profile[0].max), (0, 0, 20));
    assert_eq!(profile[0].mean, 10.0);
    // |0-0| + |10-0| + |20-10| spread over the two adjacent pixel pairs.
    assert_eq!(profile[0].gradient, 10.0);
    assert_eq!(
        (profile[1].min, profile[1].max, profile[1].mean),
        (0, 255, 170.0)
    );
    assert_eq!(profile[1].gradient, 255.0);
}

#[test]
fn webtoon_strip_is_split_into_nine_legal_pages() {
    let params = SliceParams::default();
    let image = panel_strip(250);
    let plan = plan_slices(WIDTH, HEIGHT, &row_profile(&image), &params).expect("should be sliced");

    assert_pages_are_legal(&plan, &params);
    assert_eq!(plan.page_count(), 9);
    let heights: Vec<u32> = plan.page_ranges().iter().map(|&(_, h)| h).collect();
    assert_eq!(
        heights,
        [1554, 1601, 1601, 1601, 1601, 1601, 1601, 1601, 1556]
    );
    // Every cut lands in a gutter rather than through a panel.
    for &cut in &plan.cuts {
        assert!(
            is_gutter(cut) && is_gutter(cut + 1),
            "cut {cut} is not inside a gutter"
        );
    }
}

#[test]
fn dark_background_gutters_are_usable_as_cuts() {
    let params = SliceParams::default();
    let image = panel_strip(30);
    let plan = plan_slices(WIDTH, HEIGHT, &row_profile(&image), &params).expect("should be sliced");

    assert_pages_are_legal(&plan, &params);
    assert_eq!(plan.page_count(), 9);
}

#[test]
fn a_flat_row_without_clearance_is_not_a_cut_candidate() {
    let params = SliceParams::default();
    // A real gutter wide enough to clear the airspace check, and a single flat
    // row sitting exactly on the ideal position between two content rows.
    let gutter = BlankBand {
        start: 1210,
        end: 1310,
    };
    let flat_row = 1591u32;
    let image = GrayImage::from_fn(WIDTH, HEIGHT, |x, y| {
        let luma = if y == flat_row || (gutter.start..gutter.end).contains(&y) {
            250
        } else {
            content_luma(x, y)
        };
        Luma([luma])
    });

    let profile = row_profile(&image);
    let bands = blank_bands(&profile, &params);
    let candidate = bands
        .iter()
        .find(|band| band.start <= flat_row && flat_row < band.end)
        .copied();
    assert_eq!(candidate.map(|band| band.width()), Some(1));
    assert_eq!(
        candidate.and_then(|band| band.cut_window(&params)),
        None,
        "a lone flat row must be rejected: text inside an undetected bubble is flat too"
    );

    let plan = plan_slices(WIDTH, HEIGHT, &profile, &params).expect("should be sliced");
    assert_pages_are_legal(&plan, &params);
    assert!(
        !plan.cuts.contains(&flat_row),
        "cut {} was placed on the unvalidated flat row",
        flat_row
    );
    assert_eq!(plan.cuts[0], 1259, "the validated gutter must win");
}

#[test]
fn strips_below_the_minimum_sliceable_height_are_left_whole() {
    let params = SliceParams::default();
    // Narrow enough to clear the aspect trigger, so only the height gate can
    // reject it.
    let image = GrayImage::from_pixel(100, 2000, Luma([255]));
    assert_eq!(
        plan_slices(100, image.height(), &row_profile(&image), &params),
        None
    );

    let sliceable = GrayImage::from_pixel(100, 2401, Luma([255]));
    assert!(plan_slices(100, sliceable.height(), &row_profile(&sliceable), &params).is_some());
}

#[test]
fn strips_below_the_trigger_aspect_are_left_whole() {
    let params = SliceParams::default();
    // Tall enough to clear the minimum sliceable height, so only the aspect
    // ratio can reject it: 3000 <= 3.0 * 1200.
    let wide = 1200;
    let image = GrayImage::from_pixel(wide, 3000, Luma([255]));
    assert_eq!(
        plan_slices(wide, image.height(), &row_profile(&image), &params),
        None
    );

    let tall = GrayImage::from_pixel(wide, 3601, Luma([255]));
    let plan = plan_slices(wide, tall.height(), &row_profile(&tall), &params)
        .expect("aspect above the trigger must be sliced");
    assert_pages_are_legal(&plan, &params);
}

#[test]
fn uniform_noise_falls_back_to_flat_rows_without_looping_forever() {
    let params = SliceParams::default();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let image = GrayImage::from_fn(WIDTH, HEIGHT, |_, _| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        Luma([(state >> 33) as u8])
    });

    let profile = row_profile(&image);
    assert!(
        blank_bands(&profile, &params).is_empty(),
        "noise must not produce blank bands"
    );

    let plan = plan_slices(WIDTH, HEIGHT, &profile, &params).expect("noise is still sliceable");
    assert_pages_are_legal(&plan, &params);
    assert!(plan.page_count() > 1);
}

#[test]
fn cuts_and_page_ranges_describe_the_same_division() {
    let params = SliceParams::default();
    let image = panel_strip(250);
    let plan = plan_slices(WIDTH, HEIGHT, &row_profile(&image), &params).unwrap();

    assert_eq!(plan.cuts.len(), plan.page_count() - 1);
    let ranges = plan.page_ranges();
    assert_eq!(ranges.len(), plan.page_count());
    let mut expected_start = 0;
    for (index, &(start, height)) in ranges.iter().enumerate() {
        assert_eq!(start, expected_start);
        expected_start += height;
        if index < plan.cuts.len() {
            assert_eq!(start + height, plan.cuts[index]);
        }
    }
    assert_eq!(expected_start, plan.height);
}
