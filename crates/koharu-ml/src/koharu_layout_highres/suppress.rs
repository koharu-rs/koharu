//! Duplicates of one object that separate queries predict, removed after
//! the network as RF-DETR's maintainers advise (roboflow/rf-detr#187 and
//! #434): its one-to-one matching only teaches the model to avoid them.

use super::Detection;

/// A detection overlapping a more confident one of its label by at least
/// this IoU is the same object, as in supervision's `with_nms` default. The
/// checkpoint's training labels hold 116 such pairs among 81,638 panels and
/// hardly any among the other classes.
const IOU: f32 = 0.5;

/// Text with at least this share of the smaller box inside the other is
/// the same block, a fragment of it, or it merged with an aside. Training
/// labels nest text in text only 9 times among 127,678 boxes, but
/// onomatopoeia, bubbles and panels hundreds of times, so only text is
/// suppressed this way. On 212 pages of Blue Archive's comics, this kept the
/// right box of 11 fragment pairs of 11 and 15 merged pairs of 22, where IoU
/// alone kept 5 of each.
const NESTED_TEXT: f32 = 0.8;

/// Drops each detection that duplicates a more confident one, keeping the
/// rest in their order.
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

fn duplicate(kept: &Detection, candidate: &Detection) -> bool {
    if kept.label != candidate.label {
        return false;
    }
    let (left, right) = (kept.bbox, candidate.bbox);
    let overlap = (left[2].min(right[2]) - left[0].max(right[0])).max(0.0)
        * (left[3].min(right[3]) - left[1].max(right[1])).max(0.0);
    let (left, right) = (area(left), area(right));
    overlap >= IOU * (left + right - overlap)
        || candidate.label == "text" && overlap >= NESTED_TEXT * left.min(right)
}

fn area(bbox: [f32; 4]) -> f32 {
    (bbox[2] - bbox[0]).max(0.0) * (bbox[3] - bbox[1]).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::{super::Mask, Detection, suppress};

    fn detection(label: &str, score: f32, bbox: [f32; 4]) -> Detection {
        Detection {
            label_id: 0,
            label: label.to_owned(),
            score,
            bbox,
            area: 0,
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
}
