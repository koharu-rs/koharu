//! The layout as a layer tree in reading order.

use koharu_ml::koharu_layout_highres::{Layout, Lettering};
use kurbo::Rect;

/// A lettering with more than this share of its box in a kept block's is
/// that block's: found again, its box is the one the block got, unless the
/// user resized the block's.
const HELD: f64 = 0.5;

/// A detected part of the page and the parts inside it.
pub(super) struct Node {
    pub(super) part: Part,
    pub(super) children: Vec<Node>,
}

#[derive(Clone, Copy)]
pub(super) enum Part {
    Panel(usize),
    Bubble(usize),
    Lettering(usize),
}

/// The layout as a tree in reading order: panels in their order, each
/// balloon in the panel it's read in, and each lettering in its balloon, else
/// free in the panel it's read in, though it reaches past the panel's border.
/// The layout reads a panel's lettering together, so the tree keeps its order.
/// A lettering that a block kept on the page holds is left out.
pub(super) struct Tree<'l> {
    layout: &'l Layout,
    pub(super) top: Vec<Node>,
    /// Where each panel and balloon went: its index on the page, and a
    /// balloon's panel.
    panels: Vec<Option<usize>>,
    bubbles: Vec<Option<(Option<usize>, usize)>>,
}

impl<'l> Tree<'l> {
    pub(super) fn new(layout: &'l Layout, kept: &[Rect]) -> Self {
        let mut tree = Self {
            layout,
            top: Vec::new(),
            panels: vec![None; layout.panels.len()],
            bubbles: vec![None; layout.bubbles.len()],
        };
        for (at, lettering) in layout.lettering.iter().enumerate() {
            if held(lettering, kept) {
                continue;
            }
            let node = Node {
                part: Part::Lettering(at),
                children: Vec::new(),
            };
            match lettering.bubble {
                Some(bubble) => tree.bubble(bubble).push(node),
                None => tree.holder(lettering.panel).push(node),
            }
        }
        // Panels and balloons holding no lettering.
        if let Some(last) = layout.panels.len().checked_sub(1) {
            tree.panel(last);
        }
        for bubble in 0..layout.bubbles.len() {
            tree.bubble(bubble);
        }
        tree
    }

    /// Panel `at`'s index on the page, the panels before it placed first.
    fn panel(&mut self, at: usize) -> usize {
        for panel in 0..=at {
            if self.panels[panel].is_none() {
                self.top.push(Node {
                    part: Part::Panel(panel),
                    children: Vec::new(),
                });
                self.panels[panel] = Some(self.top.len() - 1);
            }
        }
        self.panels[at].expect("a panel placed above")
    }

    /// What a part in `panel`, or on the page, goes in.
    fn holder(&mut self, panel: Option<usize>) -> &mut Vec<Node> {
        match panel {
            Some(panel) => {
                let at = self.panel(panel);
                &mut self.top[at].children
            }
            None => &mut self.top,
        }
    }

    /// What a lettering in balloon `at` goes in.
    fn bubble(&mut self, at: usize) -> &mut Vec<Node> {
        let (panel, index) = match self.bubbles[at] {
            Some(place) => place,
            None => {
                let panel = self.layout.bubbles[at].panel;
                let holder = self.holder(panel);
                holder.push(Node {
                    part: Part::Bubble(at),
                    children: Vec::new(),
                });
                let place = (panel, holder.len() - 1);
                self.bubbles[at] = Some(place);
                place
            }
        };
        &mut self.holder(panel)[index].children
    }
}

/// Whether one of the `kept` boxes holds `lettering`.
fn held(lettering: &Lettering, kept: &[Rect]) -> bool {
    let [x0, y0, x1, y1] = lettering.bbox.map(f64::from);
    let own = Rect::new(x0, y0, x1, y1);
    kept.iter()
        .any(|kept| own.intersect(*kept).area() > HELD * own.area())
}

#[cfg(test)]
mod tests {
    use koharu_ml::koharu_layout_highres::{Bubble, Kind, Lettering, Mask, Panel};
    use kurbo::Shape as _;

    use super::*;

    fn mask() -> Mask {
        Mask {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            pixels: Vec::new(),
        }
    }

    fn lettering(bubble: Option<usize>, panel: Option<usize>) -> Lettering {
        Lettering {
            kind: Kind::Text,
            score: 0.9,
            bbox: [0.0, 0.0, 10.0, 10.0],
            mask: mask(),
            bubble,
            panel,
            room: None,
        }
    }

    fn shown(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|node| {
                let name = match node.part {
                    Part::Panel(at) => format!("P{at}"),
                    Part::Bubble(at) => format!("B{at}"),
                    Part::Lettering(at) => format!("L{at}"),
                };
                match node.children.is_empty() {
                    true => name,
                    false => format!("{name}({})", shown(&node.children)),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn parts_nest_in_reading_order() {
        let panel = || Panel {
            score: 0.9,
            bbox: [0.0, 0.0, 10.0, 10.0],
            mask: mask(),
        };
        let bubble = |lettering, panel| Bubble {
            score: 0.9,
            bbox: [0.0, 0.0, 10.0, 10.0],
            mask: mask(),
            outline: kurbo::Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1),
            lettering,
            panel,
        };
        let layout = Layout {
            width: 100,
            height: 100,
            // Free in the second panel, in a balloon there, free on the page.
            lettering: vec![
                lettering(None, Some(1)),
                lettering(Some(0), Some(1)),
                lettering(None, None),
            ],
            // The second is empty, read in no panel.
            bubbles: vec![bubble(vec![1], Some(1)), bubble(Vec::new(), None)],
            panels: vec![panel(), panel(), panel()],
        };
        assert_eq!(
            shown(&Tree::new(&layout, &[]).top),
            "P0 P1(L0 B0(L1)) L2 P2 B1"
        );
    }

    /// A balloon over its panel's top border, read between the panel's texts
    /// as the layout reads Japanese comics, stays between them.
    #[test]
    fn a_part_reaching_past_its_panel_stays_in_it() {
        let panel = Panel {
            score: 0.9,
            bbox: [0.0, 50.0, 200.0, 150.0],
            mask: mask(),
        };
        let crossing = Bubble {
            score: 0.9,
            bbox: [90.0, 30.0, 140.0, 110.0],
            mask: mask(),
            outline: kurbo::Rect::new(90.0, 30.0, 140.0, 110.0).to_path(0.1),
            lettering: vec![1],
            panel: Some(0),
        };
        let at = |bbox, bubble| Lettering {
            bbox,
            ..lettering(bubble, Some(0))
        };
        let layout = Layout {
            width: 200,
            height: 200,
            // On the right, in the balloon, and on the left.
            lettering: vec![
                at([150.0, 60.0, 180.0, 120.0], None),
                at([100.0, 40.0, 130.0, 100.0], Some(0)),
                at([20.0, 60.0, 50.0, 120.0], None),
            ],
            bubbles: vec![crossing],
            panels: vec![panel],
        };
        assert_eq!(shown(&Tree::new(&layout, &[]).top), "P0(L0 B0(L1) L2)");
    }
}
