mod index;
mod links;
mod paths;
#[cfg(test)]
mod tests;
mod types;
mod write;

#[cfg(test)]
pub(crate) use paths::strip_md_extension;
pub(crate) use paths::{content_snippet, normalize_link_target, normalize_title, slugify};
pub use types::{
    ExplorerFolder, ExplorerNote, Note, NoteEntry, NoteLink, NoteLinks, SearchHit, VaultIndex,
};
pub(crate) use write::{
    WriteError, WriteOutcome, append_note, create_note, delete_note, move_or_rename_note,
    update_note,
};
