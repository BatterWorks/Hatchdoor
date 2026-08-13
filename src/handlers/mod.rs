mod api;
mod assets;
pub mod diagnostics;
mod downloads;
mod settings;
mod spa;
pub(crate) mod vault_collection_reads;
pub(crate) mod vault_content;
mod vault_write;
pub(crate) mod vaults;

pub use api::health_handler;
pub use settings::{
    MAX_IN_MEMORY_UPLOAD_BYTES, generate_mcp_token_handler, get_git_status_handler,
    get_index_status_handler, get_settings_handler, patch_settings_handler,
    reveal_mcp_token_handler, reveal_web_token_handler,
};
pub use spa::spa_index_handler;
pub use vault_collection_reads::{
    vault_scope_graph_handler, vault_scope_recent_handler, vault_scope_search_handler,
    vault_scope_stats_handler, vault_scope_tree_handler,
};
pub use vault_content::{
    vault_scoped_asset_handler, vault_scoped_note_download_handler, vault_scoped_note_handler,
    vault_scoped_note_links_handler, vault_scoped_resolve_batch_handler,
    vault_scoped_resolve_handler, vault_scoped_stats_detail_handler,
};
pub use vault_write::{
    vault_scoped_archive_note_handler, vault_scoped_create_note_handler,
    vault_scoped_delete_note_handler, vault_scoped_move_note_handler,
    vault_scoped_move_rename_note_handler, vault_scoped_rename_note_handler,
    vault_scoped_update_note_handler, vault_scoped_upload_attachment_handler,
    vault_scoped_write_capabilities_handler,
};
pub(crate) use vaults::demo_read_only_response;
pub use vaults::{
    create_vault_handler, disable_vault_handler, disconnect_vault_handler, edit_vault_handler,
    enable_vault_handler, list_vaults_handler, refresh_vault_handler, retry_vault_handler,
    sync_vault_handler, vault_collection_events_handler,
};
