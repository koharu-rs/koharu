//! Koharu's scene graph: Photoshop's layers, a page background, and the parts
//! of a manga page as typed kinds. The README is the design.

mod change;
mod color;
mod edit;
mod error;
mod external;
mod glossary;
mod ids;
mod kinds;
mod layer;
mod project;
mod query;
mod session;
mod store;
mod text;
mod tiles;
mod values;

pub use change::{Change, Created, History, PageChanges};
pub use color::{Color, Rgba};
pub use edit::{Edit, Op};
pub use error::{Error, Result};
pub use external::{ArchiveMember, ExternalImage, MAX_SOURCE_BYTES};
pub use glossary::{
    GlossaryEntry, GlossaryKind, SourceLine, TermIssue, matching_terms, valid_glossary,
};
pub use icu_locale_core::LanguageIdentifier;
pub use ids::{Hash, LayerId, PageId, ProjectId, Ref, Revision, Uuid};
pub use kinds::*;
pub use layer::{Attributes, Layer, NewLayer, PixelFormat};
pub use project::{Background, Page, Project};
pub use query::{Answer, PageSummary, ProjectAnswer, Query};
pub use session::{EXTENSION, Session};
pub use text::{CharStyle, RichText, Span, Text, TextStyle};
pub use tiles::{Tile, Tiles};
pub use values::*;
