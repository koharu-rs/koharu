//! Duplicates of one object that separate queries predict, removed after
//! the network as RF-DETR's maintainers advise (roboflow/rf-detr#187 and
//! #434): its one-to-one matching only teaches the model to avoid them.

use super::{Detection, Mask};

/// A detection overlapping a more confident one of its label by at least
/// this IoU is the same object, as in supervision's `with_nms` default. The
/// checkpoint's training labels hold 116 such pairs among 81,638 panels and
/// hardly any among the other classes.
const IOU: f32 = 0.5;

/// A box with at least this share of it inside another nests in it. Text
/// nesting in text is the same block, a fragment of it, or it merged with an
/// aside: training labels nest text in text only 9 times among 127,678
/// boxes. On 212 pages of Blue Archive's comics, suppressing nested text kept
/// the right box of 11 fragment pairs of 11 and 15 merged pairs of 22, where
/// IoU alone kept 5 of each. Bubbles and panels nest hundreds of times, so
/// nesting alone suppresses only text.
const NESTED: f32 = 0.8;

/// Onomatopoeia whose boxes overlap as duplicates' do, and that share at
/// least this share of the smaller one's glyphs, are the same lettering
/// detected twice. Of the 435 onomatopoeia pairs in the training labels whose
/// boxes nest, only 4.1% share this much: the rest are separate lettering
/// whose boxes happen to nest. Glyphs alone aren't enough either, since a
/// mask can spill onto the next word's glyphs.
const SHARED_GLYPHS: f32 = 0.5;

/// Drops each detection that duplicates a more confident one, keeping the
/// rest in their order. Onomatopoeia are left to `merge`, which needs their
/// glyphs.
pub(super) fn suppress(detections: &mut Vec<Detection>) {
    let mut ranked: Vec<usize> = (0..detections.len()).collect();
    ranked.sort_by(|&left, &right| detections[right].score.total_cmp(&detections[left].score));
    let mut kept = vec![false; detections.len()];
    for candidate in ranked {
        kept[candidate] = !detections
            .iter()
            .zip(&kept)
            .any(|(other, &kept)| kept && duplicate(other, &detections[candidate]));
    }
    let mut kept = kept.into_iter();
    detections.retain(|_| kept.next() == Some(true));
}

/// Merges onomatopoeia detected twice into one, as supervision's
/// non-maximum merging does: the union of their boxes and of their glyphs,
/// with their scores weighted by box area. Keeping the more confident alone
/// could keep a fragment: a word's half can outscore the whole. A merge can
/// make a box that now overlaps another, as supervision's groups chain, so
/// it repeats until nothing merges.
pub(super) fn merge(detections: &mut Vec<Detection>) {
    let (mut kept, mut onomatopoeia): (Vec<Detection>, Vec<Detection>) = detections
        .drain(..)
        .partition(|detection| detection.label != "onomatopoeia");
    loop {
        let count = onomatopoeia.len();
        onomatopoeia = merge_pass(onomatopoeia);
        if onomatopoeia.len() == count {
            break;
        }
    }
    kept.extend(onomatopoeia);
    *detections = kept;
}

/// Gives each glyph pixel two lettering masks share to the lettering with
/// the smaller box. A box reaching over a smaller lettering beside it, as a
/// balloon's text over a remark in its balloon, takes that lettering's glyphs
/// in as its own on plain paper; its clean-up, drawn after, would cover the
/// smaller lettering's translation.
pub(super) fn disjoint(detections: &mut [Detection]) {
    let mut lettering: Vec<usize> = (0..detections.len())
        .filter(|&at| matches!(detections[at].label.as_str(), "text" | "onomatopoeia"))
        .collect();
    lettering.sort_by(|&left, &right| {
        area(detections[left].bbox).total_cmp(&area(detections[right].bbox))
    });
    for (rank, &small) in lettering.iter().enumerate() {
        let glyphs = detections[small].mask.clone();
        for &large in &lettering[rank + 1..] {
            let mask = &mut detections[large].mask;
            for (at, value) in mask.pixels.iter_mut().enumerate() {
                let (x, y) = (
                    mask.x + at as u32 % mask.width,
                    mask.y + at as u32 / mask.width,
                );
                if *value != 0 && glyphs.contains(x, y) {
                    *value = 0;
                }
            }
        }
    }
}

/// Merges each onomatopoeia into the first more confident one it
/// duplicates.
fn merge_pass(mut onomatopoeia: Vec<Detection>) -> Vec<Detection> {
    onomatopoeia.sort_by(|left, right| right.score.total_cmp(&left.score));
    let mut merged: Vec<Detection> = Vec::new();
    for candidate in onomatopoeia {
        match merged.iter_mut().find(|other| {
            let (iou, nested) = overlap(other.bbox, candidate.bbox);
            (iou >= IOU || nested >= NESTED)
                && shared(&other.mask, &candidate.mask) >= SHARED_GLYPHS
        }) {
            Some(other) => {
                let (a, b) = (area(other.bbox), area(candidate.bbox));
                if a + b > 0.0 {
                    other.score = (other.score * a + candidate.score * b) / (a + b);
                }
                other.bbox = [
                    other.bbox[0].min(candidate.bbox[0]),
                    other.bbox[1].min(candidate.bbox[1]),
                    other.bbox[2].max(candidate.bbox[2]),
                    other.bbox[3].max(candidate.bbox[3]),
                ];
                other.mask = union(&other.mask, &candidate.mask);
            }
            None => merged.push(candidate),
        }
    }
    merged
}

/// The share of the smaller mask's set pixels that the other also sets.
fn shared(left: &Mask, right: &Mask) -> f32 {
    let count = |mask: &Mask| mask.pixels.iter().filter(|&&pixel| pixel != 0).count();
    let (smaller, larger) = if count(left) <= count(right) {
        (left, right)
    } else {
        (right, left)
    };
    let set = count(smaller);
    if set == 0 {
        return 0.0;
    }
    let both = (0..smaller.height)
        .flat_map(|y| (0..smaller.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            smaller.pixels[(y * smaller.width + x) as usize] != 0
                && larger.contains(smaller.x + x, smaller.y + y)
        })
        .count();
    both as f32 / set as f32
}

/// The pixels either mask sets, over both masks' extent.
fn union(left: &Mask, right: &Mask) -> Mask {
    let (x, y) = (left.x.min(right.x), left.y.min(right.y));
    let width = (left.x + left.width).max(right.x + right.width) - x;
    let height = (left.y + left.height).max(right.y + right.height) - y;
    Mask {
        x,
        y,
        width,
        height,
        pixels: (0..height)
            .flat_map(|row| (0..width).map(move |column| (x + column, y + row)))
            .map(|(px, py)| u8::from(left.contains(px, py) || right.contains(px, py)) * u8::MAX)
            .collect(),
    }
}

fn duplicate(kept: &Detection, candidate: &Detection) -> bool {
    if kept.label != candidate.label || candidate.label == "onomatopoeia" {
        return false;
    }
    let (iou, nested) = overlap(kept.bbox, candidate.bbox);
    iou >= IOU || candidate.label == "text" && nested >= NESTED
}

/// Two boxes' IoU, and the share of the smaller inside the other.
fn overlap(left: [f32; 4], right: [f32; 4]) -> (f32, f32) {
    let both = (left[2].min(right[2]) - left[0].max(right[0])).max(0.0)
        * (left[3].min(right[3]) - left[1].max(right[1])).max(0.0);
    let (left, right) = (area(left), area(right));
    let ratio = |whole: f32| if whole > 0.0 { both / whole } else { 0.0 };
    (ratio(left + right - both), ratio(left.min(right)))
}

fn area(bbox: [f32; 4]) -> f32 {
    (bbox[2] - bbox[0]).max(0.0) * (bbox[3] - bbox[1]).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::{super::Mask, Detection, disjoint, merge, suppress};

    fn detection(label: &str, score: f32, bbox: [f32; 4]) -> Detection {
        Detection {
            label: label.to_owned(),
            score,
            bbox,
            mask: Mask {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                pixels: Vec::new(),
            },
        }
    }

    #[test]
    fn duplicates_and_nested_text_give_way_to_the_more_confident() {
        let mut page = vec![
            // A balloon's text, a weaker copy, a fragment of it, and a box
            // merging it with an aside.
            detection("text", 0.8, [100.0, 100.0, 200.0, 400.0]),
            detection("text", 0.5, [105.0, 100.0, 200.0, 390.0]),
            detection("text", 0.4, [150.0, 110.0, 200.0, 300.0]),
            detection("text", 0.3, [40.0, 100.0, 200.0, 420.0]),
            // The aside, and the bubble around the balloon's text.
            detection("text", 0.35, [40.0, 300.0, 90.0, 420.0]),
            detection("bubble", 0.9, [90.0, 90.0, 210.0, 410.0]),
            // Onomatopoeia and a panel set in other ones stay.
            detection("onomatopoeia", 0.6, [300.0, 100.0, 500.0, 300.0]),
            detection("onomatopoeia", 0.5, [320.0, 120.0, 380.0, 200.0]),
            detection("panel", 0.9, [0.0, 0.0, 600.0, 500.0]),
            detection("panel", 0.8, [300.0, 50.0, 550.0, 350.0]),
        ];
        let kept: Vec<[f32; 4]> = [0, 4, 5, 6, 7, 8, 9]
            .into_iter()
            .map(|index| page[index].bbox)
            .collect();

        suppress(&mut page);

        assert_eq!(
            page.iter()
                .map(|detection| detection.bbox)
                .collect::<Vec<_>>(),
            kept
        );
    }

    #[test]
    fn onomatopoeia_detected_twice_merge_and_nested_ones_stay() {
        // Glyphs as filled rectangles on a 100 × 100 page.
        let glyphs = |strokes: &[[u32; 4]]| Mask {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            pixels: (0..100)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let inside = strokes
                        .iter()
                        .any(|&[x0, y0, x1, y1]| (x0..x1).contains(&x) && (y0..y1).contains(&y));
                    u8::from(inside) * u8::MAX
                })
                .collect(),
        };
        let onomatopoeia = |score, bbox, strokes: &[[u32; 4]]| Detection {
            mask: glyphs(strokes),
            ..detection("onomatopoeia", score, bbox)
        };
        let mut page = vec![
            // A word of two strokes, and its lower stroke scoring higher.
            onomatopoeia(
                0.47,
                [10.0, 10.0, 40.0, 90.0],
                &[[10, 10, 40, 40], [10, 60, 40, 90]],
            ),
            onomatopoeia(0.49, [10.0, 55.0, 40.0, 90.0], &[[10, 60, 40, 90]]),
            // Another word inside the first's box, in its own glyphs.
            onomatopoeia(0.6, [15.0, 45.0, 35.0, 55.0], &[[15, 45, 35, 55]]),
            // A word beside the first whose mask spills onto its glyphs.
            onomatopoeia(0.5, [38.0, 10.0, 70.0, 40.0], &[[10, 10, 40, 40]]),
        ];

        merge(&mut page);

        let mut boxes: Vec<[f32; 4]> = page.iter().map(|detection| detection.bbox).collect();
        boxes.sort_by(|left, right| {
            left[0]
                .total_cmp(&right[0])
                .then(left[1].total_cmp(&right[1]))
        });
        assert_eq!(
            boxes,
            [
                [10.0, 10.0, 40.0, 90.0],
                [15.0, 45.0, 35.0, 55.0],
                [38.0, 10.0, 70.0, 40.0]
            ]
        );
    }

    #[test]
    fn a_larger_box_gives_a_smaller_lettering_its_glyphs() {
        // A balloon's text whose box and mask took in the column of a remark
        // beside it, and the remark.
        let glyphs = |[x0, y0, x1, y1]: [u32; 4]| Mask {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            pixels: (0..100)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .map(|(x, y)| u8::from((x0..x1).contains(&x) && (y0..y1).contains(&y)) * u8::MAX)
                .collect(),
        };
        let mut page = vec![
            Detection {
                mask: glyphs([10, 10, 90, 90]),
                ..detection("text", 0.9, [10.0, 10.0, 90.0, 90.0])
            },
            Detection {
                mask: glyphs([70, 5, 90, 95]),
                ..detection("text", 0.5, [70.0, 5.0, 90.0, 95.0])
            },
        ];
        disjoint(&mut page);
        assert!(!page[0].mask.contains(80, 50) && page[0].mask.contains(50, 50));
        assert!(page[1].mask.contains(80, 50));
    }
}
