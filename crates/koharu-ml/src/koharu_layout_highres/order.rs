//! The order a reader of Japanese comics reads a page's lettering in: panel
//! by panel, all of a panel's text and onomatopoeia before the next panel's,
//! and among panels and among a panel's lettering alike, top to bottom and
//! then right to left.

use super::Detection;

/// A panel at least this share inside a larger one is set in it.
const NESTED: f32 = 0.9;

/// Lettering with at least this share of its box in a panel is read in it.
const IN_PANEL: f32 = 0.25;

/// Lettering mostly outside every panel but beside one, as a balloon
/// spilling over a panel's side or a caption in the margin is, is read with
/// the nearest panel spanning at least this share of its height.
const BESIDE: f32 = 0.5;

/// How far two boxes shrink from each side, as a share of their size,
/// before their extents count as overlapping: none at first, then more
/// each time they still overlap both ways, so boxes that only overlap at
/// their edges still separate. At a half, a box is its centre.
const EROSIONS: [f32; 6] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5];

/// Puts `detections`' text and onomatopoeia first, in reading order, and
/// the rest after them in their order.
pub(super) fn sort(detections: &mut Vec<Detection>) {
    let panels: Vec<usize> = (0..detections.len())
        .filter(|&index| detections[index].label == "panel")
        .collect();

    // The page's and each panel's content: the panels set in it, and the
    // lettering read in it.
    let mut page = Vec::new();
    let mut content = vec![Vec::new(); detections.len()];
    for &panel in &panels {
        match parent(detections, panel, &panels) {
            Some(parent) => content[parent].push(panel),
            None => page.push(panel),
        }
    }
    for (index, detection) in detections.iter().enumerate() {
        if !matches!(detection.label.as_str(), "text" | "onomatopoeia") {
            continue;
        }
        match holder(detections, index, &panels) {
            Some(panel) => content[panel].push(index),
            None => page.push(index),
        }
    }

    let mut order = Vec::new();
    read(detections, page, &content, &mut order);
    let mut slots: Vec<Option<Detection>> =
        std::mem::take(detections).into_iter().map(Some).collect();
    detections.extend(
        order
            .into_iter()
            .map(|index| slots[index].take().expect("read once")),
    );
    detections.extend(slots.into_iter().flatten());
}

/// The smallest larger panel `panel` is set in.
fn parent(detections: &[Detection], panel: usize, panels: &[usize]) -> Option<usize> {
    let area = |index: usize| area(detections[index].bbox);
    panels
        .iter()
        .copied()
        .filter(|&outer| {
            area(outer) > area(panel)
                && share(detections[panel].bbox, detections[outer].bbox) >= NESTED
        })
        .min_by(|&left, &right| area(left).total_cmp(&area(right)))
}

/// The panel lettering is read in: the one holding most of it, the smallest
/// of those holding as much, so lettering in a panel set in another is read
/// in the inner one; or else one beside it.
pub(super) fn holder(
    detections: &[Detection],
    lettering: usize,
    panels: &[usize],
) -> Option<usize> {
    let bbox = detections[lettering].bbox;
    let holding = panels
        .iter()
        .copied()
        .map(|panel| (panel, share(bbox, detections[panel].bbox)))
        .filter(|&(_, share)| share >= IN_PANEL)
        .max_by(|&(left, left_share), &(right, right_share)| {
            left_share
                .total_cmp(&right_share)
                .then_with(|| area(detections[right].bbox).total_cmp(&area(detections[left].bbox)))
        });
    if let Some((panel, _)) = holding {
        return Some(panel);
    }
    let distance = |panel: usize| distance(bbox, detections[panel].bbox);
    let spanned = |panel: usize| {
        let [_, top, _, bottom] = detections[panel].bbox;
        (bottom.min(bbox[3]) - top.max(bbox[1])) / (bbox[3] - bbox[1]).max(f32::EPSILON)
    };
    let nearest = panels
        .iter()
        .copied()
        .min_by(|&left, &right| distance(left).total_cmp(&distance(right)))?;
    // Above or below its nearest panel, it sits in a gutter between rows.
    if spanned(nearest) <= 0.0 {
        return None;
    }
    panels
        .iter()
        .copied()
        .filter(|&panel| spanned(panel) >= BESIDE)
        .min_by(|&left, &right| distance(left).total_cmp(&distance(right)))
}

/// The gap between two boxes, zero where they overlap.
fn distance(left: [f32; 4], right: [f32; 4]) -> f32 {
    let x = (right[0] - left[2]).max(left[0] - right[2]).max(0.0);
    let y = (right[1] - left[3]).max(left[1] - right[3]).max(0.0);
    x.hypot(y)
}

/// Appends `items`' lettering to `order`, a panel's as its content.
fn read(
    detections: &[Detection],
    items: Vec<usize>,
    content: &[Vec<usize>],
    order: &mut Vec<usize>,
) {
    for item in cut(detections, items) {
        if detections[item].label == "panel" {
            read(detections, content[item].clone(), content, order);
        } else {
            order.push(item);
        }
    }
}

/// `items` in reading order: split at gutters into rows read top to bottom,
/// or else into columns read right to left, and each part in turn the same
/// way. Where no gutter crosses them, as when a tall panel spans two rows
/// beside them, they're ordered pair by pair.
fn cut(detections: &[Detection], items: Vec<usize>) -> Vec<usize> {
    if items.len() < 2 {
        return items;
    }
    let extent = |index: usize, axis: usize| {
        let bbox = detections[index].bbox;
        (bbox[axis], bbox[axis + 2])
    };
    let rows = split(&items, |index| extent(index, 1));
    if rows.len() > 1 {
        return rows
            .into_iter()
            .flat_map(|row| cut(detections, row))
            .collect();
    }
    let columns = split(&items, |index| extent(index, 0));
    if columns.len() > 1 {
        return columns
            .into_iter()
            .rev()
            .flat_map(|column| cut(detections, column))
            .collect();
    }
    pairwise(detections, items)
}

/// `items` read one at a time, each the one the fewest unread items come
/// before, so a cycle in the pairwise order still yields every item once.
fn pairwise(detections: &[Detection], mut items: Vec<usize>) -> Vec<usize> {
    let centre = |index: usize| {
        let [x1, y1, x2, y2] = detections[index].bbox;
        ((x1 + x2) / 2.0, (y1 + y2) / 2.0)
    };
    let mut order = Vec::with_capacity(items.len());
    while !items.is_empty() {
        let earlier = |item: usize| {
            items
                .iter()
                .filter(|&&other| other != item && before(detections, other, item))
                .count()
        };
        let next = (0..items.len())
            .min_by(|&left, &right| {
                let (left, right) = (items[left], items[right]);
                earlier(left).cmp(&earlier(right)).then_with(|| {
                    let ((left_x, left_y), (right_x, right_y)) = (centre(left), centre(right));
                    left_y.total_cmp(&right_y).then(right_x.total_cmp(&left_x))
                })
            })
            .expect("items isn't empty");
        order.push(items.remove(next));
    }
    order
}

/// Whether `first` is read before `second`: the one above, or else the one
/// to the right, at the smallest erosion that separates them; or else the
/// one whose centre is higher, then further right.
fn before(detections: &[Detection], first: usize, second: usize) -> bool {
    let (a, b) = (detections[first].bbox, detections[second].bbox);
    for erosion in EROSIONS {
        let erode = |[x1, y1, x2, y2]: [f32; 4]| {
            let (dx, dy) = ((x2 - x1).max(0.0) * erosion, (y2 - y1).max(0.0) * erosion);
            [x1 + dx, y1 + dy, x2 - dx, y2 - dy]
        };
        let (a, b) = (erode(a), erode(b));
        if a[3] <= b[1] || b[3] <= a[1] {
            return a[3] <= b[1];
        }
        if a[0] >= b[2] || b[0] >= a[2] {
            return a[0] >= b[2];
        }
    }
    let (ay, by) = (a[1] + a[3], b[1] + b[3]);
    if ay != by {
        return ay < by;
    }
    a[0] + a[2] > b[0] + b[2]
}

/// `items` in groups whose extents chain into one, in order along the axis.
fn split(items: &[usize], extent: impl Fn(usize) -> (f32, f32)) -> Vec<Vec<usize>> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|&left, &right| extent(left).0.total_cmp(&extent(right).0));
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut end = f32::NEG_INFINITY;
    for item in sorted {
        let (start, item_end) = extent(item);
        match groups.last_mut() {
            Some(group) if start < end => group.push(item),
            _ => groups.push(vec![item]),
        }
        end = end.max(item_end);
    }
    groups
}

/// The share of `inner`'s area inside `outer`.
fn share(inner: [f32; 4], outer: [f32; 4]) -> f32 {
    let overlap = (inner[2].min(outer[2]) - inner[0].max(outer[0])).max(0.0)
        * (inner[3].min(outer[3]) - inner[1].max(outer[1])).max(0.0);
    let area = area(inner);
    if area > 0.0 { overlap / area } else { 0.0 }
}

fn area(bbox: [f32; 4]) -> f32 {
    (bbox[2] - bbox[0]).max(0.0) * (bbox[3] - bbox[1]).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::{super::Mask, Detection, sort};

    fn detection(label: &str, bbox: [f32; 4]) -> Detection {
        Detection {
            label: label.to_owned(),
            score: 1.0,
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
    fn panels_and_their_lettering_read_top_to_bottom_then_right_to_left() {
        // A wide panel over a tall one on the left of two stacked on the
        // right, with a title above, a balloon spilling over the lower
        // right panel's side, and a credit below.
        let mut page = vec![
            detection("text", [500.0, 960.0, 780.0, 990.0]),
            detection("panel", [0.0, 310.0, 390.0, 900.0]),
            detection("text", [300.0, 330.0, 380.0, 880.0]),
            detection("panel", [410.0, 610.0, 800.0, 900.0]),
            detection("text", [700.0, 630.0, 780.0, 880.0]),
            detection("text", [790.0, 650.0, 840.0, 850.0]),
            detection("panel", [410.0, 310.0, 800.0, 600.0]),
            detection("text", [700.0, 330.0, 780.0, 580.0]),
            detection("bubble", [680.0, 320.0, 790.0, 590.0]),
            detection("panel", [0.0, 60.0, 800.0, 300.0]),
            detection("text", [100.0, 80.0, 200.0, 280.0]),
            detection("onomatopoeia", [350.0, 100.0, 450.0, 200.0]),
            detection("text", [600.0, 80.0, 700.0, 280.0]),
            detection("text", [500.0, 10.0, 700.0, 50.0]),
        ];
        let expected: Vec<[f32; 4]> = [13, 12, 11, 10, 7, 5, 4, 2, 0, 1, 3, 6, 8, 9]
            .into_iter()
            .map(|index| page[index].bbox)
            .collect();

        sort(&mut page);

        assert_eq!(
            page.iter()
                .map(|detection| detection.bbox)
                .collect::<Vec<_>>(),
            expected
        );
    }
}
