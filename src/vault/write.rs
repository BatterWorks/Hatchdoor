mod assets;
mod attachments;
mod fs_ops;
mod notes;
mod paths;
mod rewrites;
mod types;

#[cfg(test)]
mod tests;

pub use attachments::{
    delete_attachment, import_attachment, import_attachment_bytes, list_note_attachments,
    move_attachment, rename_attachment,
};
pub use notes::{
    SectionMode, append_note, create_note, delete_note, edit_note, move_or_rename_note,
    replace_section, update_note,
};
pub use paths::allowed_attachment_extensions;
pub use types::{AttachmentInfo, AttachmentOutcome, WriteError, WriteOutcome};
