//! Koharu documents. A document is one `.loro` file: the Loro snapshot of a
//! Koharu project, page images included. Storage owns locking, loading, and
//! atomic writes; what the document means, and what to save, belong to the
//! scene.

mod document;
mod error;
mod lock;

pub use document::Document;
pub use error::{Error, Result};
