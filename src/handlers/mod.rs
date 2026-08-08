mod api;
mod assets;
pub mod diagnostics;
mod downloads;
mod settings;
mod spa;
mod vault_content;
mod vaults;
mod write_api;

pub use api::{
    graph_handler, health_handler, note_handler, note_links_handler, recently_modified_handler,
    refresh_handler, resolve_batch_handler, resolve_handler, search_handler, stats_handler,
    tree_handler, vault_events_handler,
};
pub use assets::vault_asset_handler;
pub use diagnostics::diagnostics_handler;
pub use downloads::note_download_handler;
pub use settings::{
    MAX_IN_MEMORY_UPLOAD_BYTES, generate_mcp_token_handler, get_git_status_handler,
    get_index_status_handler, get_settings_handler, patch_settings_handler,
    reveal_mcp_token_handler, reveal_web_token_handler,
};
pub use spa::spa_index_handler;
pub use vault_content::{
    vault_scoped_asset_handler, vault_scoped_note_download_handler, vault_scoped_note_handler,
    vault_scoped_note_links_handler, vault_scoped_resolve_batch_handler,
    vault_scoped_resolve_handler,
};
pub use vaults::{
    create_vault_handler, disable_vault_handler, disconnect_vault_handler, edit_vault_handler,
    enable_vault_handler, list_vaults_handler, retry_vault_handler, sync_vault_handler,
    vault_collection_events_handler,
};
pub use write_api::{
    archive_note_handler, create_note_handler, delete_note_handler, move_note_handler,
    move_rename_note_handler, rename_note_handler, update_note_handler, upload_attachment_handler,
    write_capabilities_handler,
};
