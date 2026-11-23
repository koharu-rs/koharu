//! The gallery: reading a chapter of a site the built-in extensions read
//! into a project, and remembering how it reads, the shelf of the series the
//! user opened, and the client the extensions' requests go through. Nothing but its commands depends on
//! it.

mod chapter;
mod readings;
mod shelf;
mod sites;

pub use self::{
    chapter::{Loading, read},
    readings::{Reading, Readings},
    shelf::Shelf,
    sites::{client, sign_out},
};
