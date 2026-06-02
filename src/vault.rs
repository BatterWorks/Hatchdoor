mod index;
mod links;
mod paths;
#[cfg(test)]
mod tests;
mod types;
mod write;

#[cfg(test)]
pub use paths::strip_md_extension;
pub use paths::{content_snippet, normalize_link_target, normalize_title, slugify};
pub use types::{
    ExplorerFolder, ExplorerNote, ModifiedNote, Note, NoteEntry, NoteLink, NoteLinks, SearchHit,
    VaultIndex,
};
pub use write::{
    AttachmentOutcome, WriteError, WriteOutcome, allowed_attachment_extensions, append_note,
    create_note, delete_attachment, delete_note, import_attachment, list_note_attachments,
    move_attachment, move_or_rename_note, rename_attachment, update_note,
};
