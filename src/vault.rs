mod exclude;
mod index;
mod layers;
mod link_style;
mod links;
mod markdown_links;
mod paths;
mod seed;
#[cfg(test)]
mod tests;
mod types;
mod write;

pub use exclude::{DEFAULT_EXCLUDE_PATTERNS, ExcludeMatcher};
pub use layers::{LayerDecl, LayerMap, MARKER_FILE_NAME};
pub use link_style::{LinkPathForm, LinkStyle, VaultLinkStyle, count_link_forms, vault_link_style};
#[cfg(test)]
pub use paths::strip_md_extension;
pub use paths::{
    content_snippet, is_servable_asset, normalize_link_target, normalize_title, slugify,
    split_wikilink_asset_body,
};
pub use seed::{SeedError, seed_empty_vault, seed_new_vault};
pub use types::{
    ExplorerFolder, ExplorerNote, ModifiedNote, Note, NoteEntry, NoteLink, NoteLinks, NoteMetadata,
    NoteSummary, SearchHit, VaultIndex, VaultScanConfig,
};
pub use write::{
    AttachmentInfo, AttachmentOutcome, NestedTag, NoteTarget, SectionMode, TagDelete,
    TagDeleteError, TagDeleteNote, TagRename, TagRenameError, TagRenameNote, UnrewritableNote,
    UnsupportedTagNote, WriteError, WriteOutcome, allowed_attachment_extensions, append_note,
    archive_note, check_attachment_import_target, check_note_content_hash, create_note,
    delete_attachment, delete_note, delete_tag, edit_note, import_attachment_bytes,
    list_note_attachments, move_attachment, move_or_rename_note, note_exists_conflict, note_target,
    rename_attachment, rename_tag, replace_section, update_note, update_note_frontmatter,
};
