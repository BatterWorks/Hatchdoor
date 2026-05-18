mod api;
mod assets;
mod downloads;
mod spa;

pub use api::{
    health_handler, note_handler, note_links_handler, recently_modified_handler, refresh_handler,
    resolve_batch_handler, resolve_handler, search_handler, tree_handler, vault_events_handler,
};
pub use assets::vault_asset_handler;
pub use downloads::note_download_handler;
pub use spa::spa_index_handler;
