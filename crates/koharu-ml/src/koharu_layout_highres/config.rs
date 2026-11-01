use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct Config {
    pub(super) resolution: i64,
    pub(super) num_select: i64,
    pub(super) classes: BTreeMap<String, String>,
    pub(super) recommended_thresholds: Thresholds,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub(super) struct Thresholds {
    pub(super) text: f32,
    pub(super) onomatopoeia: f32,
    pub(super) bubble: f32,
    pub(super) panel: f32,
}

impl Thresholds {
    pub(super) fn with_background(self) -> [f32; 5] {
        [
            self.text,
            self.onomatopoeia,
            self.bubble,
            self.panel,
            f32::INFINITY,
        ]
    }
}

impl Config {
    pub(super) fn class_names(&self) -> Vec<String> {
        (0..self.classes.len())
            .map(|index| self.classes[&index.to_string()].clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Thresholds;

    #[test]
    fn class_threshold_order_matches_checkpoint_labels() {
        let thresholds = Thresholds {
            text: 0.1,
            onomatopoeia: 0.2,
            bubble: 0.3,
            panel: 0.4,
        };

        assert_eq!(
            thresholds.with_background(),
            [0.1, 0.2, 0.3, 0.4, f32::INFINITY]
        );
    }
}
