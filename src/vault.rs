mod index;
mod links;
mod paths;
#[cfg(test)]
mod tests;
mod types;

#[cfg(test)]
pub use paths::{normalize_link_target, normalize_title, slugify, strip_md_extension};
pub use types::{ExplorerFolder, Note, NoteLinks, SearchHit, VaultIndex};
