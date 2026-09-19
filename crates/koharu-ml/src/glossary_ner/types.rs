/// Glossary category predicted by the model layer.
///
/// Application crates can map these categories to their own scene or glossary
/// types without reversing the `koharu-ml` dependency direction.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GlossaryEntityKind {
    Person,
    Place,
    Organization,
    Item,
    Ability,
    WorkSpecificTerm,
}

impl GlossaryEntityKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::Person,
        Self::Place,
        Self::Organization,
        Self::Item,
        Self::Ability,
        Self::WorkSpecificTerm,
    ];

    #[must_use]
    pub const fn model_label(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Place => "place",
            Self::Organization => "organization",
            Self::Item => "item",
            Self::Ability => "ability",
            Self::WorkSpecificTerm => "work-specific term",
        }
    }
}

/// One extracted glossary entity. `start` and `end` are UTF-8 byte offsets
/// into the original input, with `end` exclusive.
#[derive(Clone, Debug, PartialEq)]
pub struct GlossaryEntity {
    pub start: usize,
    pub end: usize,
    pub surface: String,
    pub kind: GlossaryEntityKind,
    pub confidence: f32,
}

impl GlossaryEntity {
    pub(crate) fn new(
        start: usize,
        end: usize,
        surface: String,
        kind: GlossaryEntityKind,
        confidence: f32,
    ) -> Self {
        Self {
            start,
            end,
            surface,
            kind,
            confidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_kinds_have_model_labels() {
        assert_eq!(GlossaryEntityKind::Person.model_label(), "person");
        assert_eq!(GlossaryEntityKind::Place.model_label(), "place");
        assert_eq!(
            GlossaryEntityKind::Organization.model_label(),
            "organization"
        );
        assert_eq!(GlossaryEntityKind::Item.model_label(), "item");
        assert_eq!(GlossaryEntityKind::Ability.model_label(), "ability");
        assert_eq!(
            GlossaryEntityKind::WorkSpecificTerm.model_label(),
            "work-specific term"
        );
    }
}
