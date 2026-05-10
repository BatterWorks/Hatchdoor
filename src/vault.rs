mod index;
mod links;
mod paths;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use paths::{content_snippet, normalize_link_target, normalize_title, slugify};
#[cfg(test)]
pub(crate) use paths::strip_md_extension;
pub use types::{
    ExplorerFolder, ExplorerNote, Note, NoteEntry, NoteLink, NoteLinks, SearchHit, VaultIndex,
};
