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
    AttachmentInfo, AttachmentOutcome, SectionMode, WriteError, WriteOutcome,
    allowed_attachment_extensions, append_note, archive_note, create_note, delete_attachment,
    delete_note, edit_note, import_attachment, import_attachment_bytes, list_note_attachments,
    move_attachment, move_or_rename_note, rename_attachment, replace_section, update_note,
};
