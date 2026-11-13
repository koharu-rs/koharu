//! The layout as a layer tree in reading order.

use koharu_ml::koharu_layout_highres::{Layout, Mask};

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
/// balloon in its panel, and each lettering in its balloon, else free in its
/// panel. A panel clips what it holds, so a balloon or a free lettering that
/// reaches past its panel's border sits on the page, where it's read.
pub(super) struct Tree<'l> {
    layout: &'l Layout,
    pub(super) top: Vec<Node>,
    /// Where each panel and balloon went: its index on the page, and a
    /// balloon's panel.
    panels: Vec<Option<usize>>,
    bubbles: Vec<Option<(Option<usize>, usize)>>,
}

impl<'l> Tree<'l> {
    pub(super) fn new(layout: &'l Layout) -> Self {
        let mut tree = Self {
            layout,
            top: Vec::new(),
            panels: vec![None; layout.panels.len()],
            bubbles: vec![None; layout.bubbles.len()],
        };
        for (at, lettering) in layout.lettering.iter().enumerate() {
            let node = Node {
                part: Part::Lettering(at),
                children: Vec::new(),
            };
            match lettering.bubble {
                Some(bubble) => tree.bubble(bubble).push(node),
                None => {
                    let panel = within(layout, lettering.bbox, &lettering.mask, lettering.panel);
                    tree.holder(panel).push(node);
                }
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
                let bubble = &self.layout.bubbles[at];
                let panel = within(self.layout, bubble.bbox, &bubble.mask, bubble.panel);
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

/// `panel`, when a part with `bbox` and `mask` lies wholly inside the box
/// its outline is drawn from.
fn within(layout: &Layout, bbox: [f32; 4], mask: &Mask, panel: Option<usize>) -> Option<usize> {
    let [left, top, right, bottom] = layout.panels[panel?].bbox;
    let inside =
        |[x0, y0, x1, y1]: [f32; 4]| x0 >= left && y0 >= top && x1 <= right && y1 <= bottom;
    let masked = mask.width == 0
        || mask.height == 0
        || inside(
            [mask.x, mask.y, mask.x + mask.width, mask.y + mask.height].map(|edge| edge as f32),
        );
    (inside(bbox) && masked).then_some(panel?)
}

#[cfg(test)]
mod tests {
    use koharu_ml::koharu_layout_highres::{Bubble, Kind, Lettering, Panel};
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
            // The second is empty, crossing panels.
            bubbles: vec![bubble(vec![1], Some(1)), bubble(Vec::new(), None)],
            panels: vec![panel(), panel(), panel()],
        };
        assert_eq!(shown(&Tree::new(&layout).top), "P0 P1(L0 B0(L1)) L2 P2 B1");
    }

    #[test]
    fn a_part_reaching_past_its_panel_sits_on_the_page() {
        let panel = Panel {
            score: 0.9,
            bbox: [0.0, 0.0, 100.0, 100.0],
            mask: mask(),
        };
        let mut inside = lettering(None, Some(0));
        inside.bbox = [10.0, 10.0, 40.0, 90.0];
        let mut crossing = lettering(None, Some(0));
        crossing.bbox = [-5.0, 10.0, 30.0, 90.0];
        let mut spilling = lettering(None, Some(0));
        spilling.bbox = [50.0, 10.0, 80.0, 90.0];
        spilling.mask = Mask {
            x: 60,
            y: 10,
            width: 45,
            height: 10,
            pixels: vec![255; 450],
        };
        let layout = Layout {
            width: 200,
            height: 200,
            lettering: vec![inside, crossing, spilling],
            bubbles: Vec::new(),
            panels: vec![panel],
        };
        assert_eq!(shown(&Tree::new(&layout).top), "P0(L0) L1 L2");
    }
}
