//! The gallery: reading a chapter of a site the built-in extensions read
//! into a project, and the client the extensions' requests go through, which
//! signs in as the user did in the browser. What the user keeps of it, the
//! series they opened and how each chapter reads, the interface keeps.
//! Nothing but its commands depends on it.

mod chapter;
mod sites;

pub use self::{
    chapter::{Loading, Reading, read},
    sites::{client, sign_out},
};
