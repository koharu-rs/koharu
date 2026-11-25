use koharu_scene::{Content, Frame, Layer, LayerId, Page, Text};
use kurbo::{Affine, BezPath, Rect, Shape as _};

/// How many times its width a box widens to at most. Without a limit a text
/// covered a face beside it, and hardly more of them on 28 pages reached their
/// page's size.
const WIDEST: f64 = 2.5;

/// The widest box `source`'s translation may take outside a balloon, in its
/// own coordinates: about its source's, along the lines to `WIDEST` times its
/// shorter side but not past its longer, and across them to three of `line`,
/// off the edges of the panel `holder` is or of the page, and off the page's
/// other texts and balloons.
pub(super) fn widest(
    page: &Page,
    holder: Option<&Layer>,
    id: LayerId,
    source: &Text,
    line: Option<f64>,
) -> Option<Rect> {
    let Frame::Box { bounds, .. } = &source.frame else {
        return None;
    };
    let own = Rect::from(*bounds);
    let center = own.center();
    let (short, long) = (own.width().min(own.height()), own.width().max(own.height()));
    let line = line.unwrap_or(short);
    let gap = line / 4.0;
    // Moved about where its centre lies on the page. Its tilt isn't undone: a
    // page turned back by it bounds far more than the page.
    let moved = Affine::from(source.transform) * center - center;
    let into = |rect: Rect| rect - moved;

    let sheet = Rect::new(
        0.0,
        0.0,
        f64::from(page.size.width),
        f64::from(page.size.height),
    );
    let panel = holder.and_then(|holder| match &holder.content {
        Content::Panel(panel) => Some(BezPath::from(&panel.outline).bounding_box()),
        _ => None,
    });
    let whole = panel
        .map(|panel| into(panel.intersect(sheet)))
        .filter(|panel| panel.intersect(own).area() > 0.8 * own.area())
        .unwrap_or_else(|| into(sheet))
        .inflate(-gap, -gap);
    let mut others = Vec::new();
    collect(&page.layers, id, false, &mut others);
    let others: Vec<(Rect, bool)> = others
        .into_iter()
        .map(|(rect, grows)| (into(rect), grows))
        .filter(|(rect, grows)| *grows || rect.intersect(own).area() < 0.5 * own.area())
        .collect();

    let mut room = own;
    let most = (WIDEST * short).min(long).max(own.width());
    (room.x0, room.x1) = grown(
        (own.x0, own.x1),
        (center.x - most / 2.0, center.x + most / 2.0),
        (whole.x0, whole.x1),
        others
            .iter()
            .filter(|(other, _)| other.y1 > room.y0 && other.y0 < room.y1)
            .map(|(other, grows)| ((other.x0, other.x1), *grows)),
        gap,
    );
    let most = own.height().max(3.0 * line);
    (room.y0, room.y1) = grown(
        (own.y0, own.y1),
        (center.y - most / 2.0, center.y + most / 2.0),
        (whole.y0, whole.y1),
        others
            .iter()
            .filter(|(other, _)| other.x1 > room.x0 && other.x0 < room.x1)
            .map(|(other, grows)| ((other.y0, other.y1), *grows)),
        gap,
    );
    room.is_finite().then_some(room)
}

/// `own` grown to `wanted` along one axis: within `whole`, `gap` off the others
/// or, where one may grow toward it too, halfway to it. What one side has no
/// room for, the other takes.
fn grown(
    own: (f64, f64),
    wanted: (f64, f64),
    whole: (f64, f64),
    others: impl Iterator<Item = ((f64, f64), bool)>,
    gap: f64,
) -> (f64, f64) {
    let middle = (own.0 + own.1) / 2.0;
    let (mut low, mut high) = whole;
    for ((from, to), grows) in others {
        if to <= middle {
            low = low.max(if grows { (to + own.0) / 2.0 } else { to + gap });
        } else if from >= middle {
            high = high.min(if grows {
                (from + own.1) / 2.0
            } else {
                from - gap
            });
        }
    }
    let (low, high) = (low.min(own.0), high.max(own.1));
    let (mut start, mut end) = wanted;
    if start < low {
        end += low - start;
        start = low;
    }
    if end > high {
        start = (start - (end - high)).max(low);
        end = high;
    }
    (start.min(own.0), end.max(own.1))
}

/// Where the page's other texts and its balloons lie, each with whether it may
/// grow too, as a text outside a balloon may. A balloon's text counts beside
/// its balloon, which a text written over the balloon's edge may lie across.
fn collect(layers: &[Layer], id: LayerId, enclosed: bool, found: &mut Vec<(Rect, bool)>) {
    for layer in layers {
        let mut inside = enclosed;
        match &layer.content {
            Content::Balloon(balloon) => {
                found.push((BezPath::from(&balloon.outline).bounding_box(), false));
                inside = true;
            }
            Content::Block(block) if layer.id != id => {
                found.extend(block.source.bounds().map(|rect| (rect, !enclosed)));
            }
            _ => {}
        }
        collect(&layer.children, id, inside, found);
    }
}
