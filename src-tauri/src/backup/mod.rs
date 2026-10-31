//! Space-scoped multi-device synchronization with one canonical storage format.
pub mod lifecycle;
pub mod model;
pub mod provider;
pub mod reconcile;
pub mod record;
pub mod repository;
pub mod restore;
pub mod s3_client;
pub mod scheduler;
pub mod settings;
pub mod store;
pub mod s3_provider;
pub(crate) mod library;
pub mod sync_engine;
pub mod spaces;
pub mod sync_read;
pub mod sync_delete;
pub mod conflicts;
#[cfg(test)]
mod tests;
