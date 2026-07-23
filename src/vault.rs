mod exclude;
mod index;
mod layers;
mod links;
mod paths;
mod seed;
#[cfg(test)]
mod tests;
mod types;
mod write;

pub use exclude::{DEFAULT_EXCLUDE_PATTERNS, ExcludeMatcher};
pub use layers::{LayerDecl, LayerMap, MARKER_FILE_NAME, layer_from_frontmatter};
#[cfg(test)]
pub use paths::strip_md_extension;
pub use paths::{content_snippet, normalize_link_target, normalize_title, slugify};
pub use seed::seed_empty_vault;
pub use types::{
    ExplorerFolder, ExplorerNote, ModifiedNote, Note, NoteEntry, NoteLink, NoteLinks, NoteMetadata,
    NoteSummary, SearchHit, VaultIndex, VaultScanConfig,
};
pub use write::{
    AttachmentInfo, AttachmentOutcome, SectionMode, WriteError, WriteOutcome,
    allowed_attachment_extensions, append_note, archive_note, create_note, delete_attachment,
    delete_note, edit_note, import_attachment_bytes, list_note_attachments, move_attachment,
    move_or_rename_note, rename_attachment, replace_section, update_note,
};
