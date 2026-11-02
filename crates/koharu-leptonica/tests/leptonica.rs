//! Each port against Leptonica 1.87.0's own output on the same image,
//! recorded from the C library before it left the repository. Every value
//! is exact, so a port that drifts from Leptonica fails here.

mod inputs;

use image::GrayImage;
use koharu_leptonica::{
    Skew, color_segment, find_skew, find_skew_sweep_and_search, find_stroke_width,
    otsu_adaptive_threshold, sauvola_binarize,
};

/// Foreground pixels and the image's fingerprint.
fn summary(image: &GrayImage) -> (usize, u64) {
    let foreground = image.pixels().filter(|pixel| pixel.0[0] == 255).count();
    (foreground, inputs::fingerprint(image.as_raw()))
}

#[test]
fn skew_matches_leptonica() {
    let skew = |angle, confidence| Some(Skew { angle, confidence });
    let find = |image: &GrayImage| find_skew(image).unwrap();

    assert_eq!(
        find(&inputs::text(301, 211, -35, 1)),
        skew(2.265625, 1.6475064)
    );
    assert_eq!(find(&inputs::text(400, 300, 52, 2)), skew(-2.96875, 2.2182));
    assert_eq!(
        find(&inputs::text(250, 180, 0, 3)),
        skew(-0.234375, 2.0487268)
    );
    // The best score is at the sweep's edge.
    assert_eq!(find(&inputs::text(320, 240, 200, 5)), skew(0.0, 0.0));
    // Leptonica fails the call.
    assert_eq!(find(&GrayImage::new(64, 48)), None);

    let image = inputs::text(320, 240, 90, 4);
    let found = find_skew_sweep_and_search(&image, 8, 1, 7.0, 1.0, 0.01).unwrap();
    assert_eq!(found, skew(-5.25, 4.0826197));
    let image = inputs::text(301, 211, -35, 1);
    let found = find_skew_sweep_and_search(&image, 1, 1, 5.0, 0.5, 0.05).unwrap();
    assert_eq!(found, skew(2.125, 1.69324));
}

#[test]
fn stroke_width_matches_leptonica() {
    let width = |image: &GrayImage| find_stroke_width(image, 0.15);

    assert_eq!(width(&inputs::strokes(97, 61, 3)), Some(2.8512278));
    assert_eq!(width(&inputs::strokes(97, 61, 5)), Some(4.9961815));
    assert_eq!(width(&inputs::text(301, 211, -35, 1)), Some(7.3942213));
    // Leptonica returns NaN.
    assert_eq!(width(&GrayImage::new(40, 30)), None);
}

#[test]
fn binarization_matches_leptonica() {
    let page = inputs::page(331, 257, 7);

    let otsu = otsu_adaptive_threshold(&page, 64, 64, 2, 2, 0.1).unwrap();
    assert_eq!(summary(&otsu), (21645, 9138852046938946006));
    let otsu = otsu_adaptive_threshold(&page, 32, 48, 0, 0, 0.0).unwrap();
    assert_eq!(summary(&otsu), (21705, 4366735469403778620));
    let sauvola = sauvola_binarize(&page, 7, 0.34).unwrap();
    assert_eq!(summary(&sauvola), (21657, 5369105125634338712));
    let sauvola = sauvola_binarize(&page, 2, 0.5).unwrap();
    assert_eq!(summary(&sauvola), (12923, 13201604322765798688));
}

#[test]
fn color_segmentation_matches_leptonica() {
    let image = inputs::colours(181, 133, 11);
    let segment = |max_distance, max_colors, sel_size, final_colors| {
        let segments = color_segment(&image, max_distance, max_colors, sel_size, final_colors)
            .unwrap()
            .unwrap();
        (
            segments.colors,
            inputs::fingerprint(segments.indices.as_raw()),
        )
    };

    assert_eq!(
        segment(100, 8, 4, 5),
        (
            vec![
                [239, 238, 230],
                [19, 17, 24],
                [199, 40, 50],
                [221, 192, 87],
                [109, 150, 123],
            ],
            0xc0a9_58cb_b0c7_1495,
        )
    );
    assert_eq!(
        segment(30, 20, 1, 6),
        (
            vec![
                [239, 238, 230],
                [19, 17, 24],
                [199, 40, 50],
                [39, 89, 199],
                [229, 200, 59],
                [119, 120, 120],
            ],
            0xd6a5_81ce_dd73_87d0,
        )
    );
    assert_eq!(
        segment(60, 6, 7, 2),
        (vec![[60, 126, 160], [205, 192, 83]], 0x1160_1f72_e4e2_c3da)
    );
}
