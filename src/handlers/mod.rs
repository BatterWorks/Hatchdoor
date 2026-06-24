mod api;
mod assets;
mod downloads;
mod spa;
mod write_api;

pub use api::{
    graph_handler, health_handler, note_handler, note_links_handler, recently_modified_handler,
    refresh_handler, resolve_batch_handler, resolve_handler, search_handler, stats_handler,
    tree_handler, vault_events_handler,
};
pub use assets::vault_asset_handler;
pub use downloads::note_download_handler;
pub use spa::spa_index_handler;
pub use write_api::{
    archive_note_handler, create_note_handler, delete_note_handler, move_note_handler,
    move_rename_note_handler, rename_note_handler, update_note_handler, upload_attachment_handler,
    write_capabilities_handler,
};
