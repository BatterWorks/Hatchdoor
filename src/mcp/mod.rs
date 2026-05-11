pub(crate) mod config;
pub(crate) mod protocol;
pub(crate) mod routes;
pub(crate) mod tools;

pub(crate) use routes::{mcp_get_handler, mcp_post_handler};
