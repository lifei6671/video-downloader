//! Space-scoped multi-device synchronization with one canonical storage format.
#[cfg(test)]
mod compatibility_online_audit;
#[cfg(test)]
mod compatibility_online_recovery;
pub mod conflicts;
pub(crate) mod library;
pub mod lifecycle;
pub mod model;
pub mod provider;
pub mod reconcile;
pub mod record;
pub mod repository;
pub mod restore;
pub mod s3_client;
pub mod s3_provider;
pub mod scheduler;
pub(crate) mod schema_migration;
pub mod settings;
pub mod spaces;
pub mod store;
pub mod sync_delete;
pub mod sync_engine;
pub mod sync_read;
mod telegram_lifecycle;
#[cfg(test)]
mod telegram_online_tests;
pub mod telegram_provider;
mod telegram_qr;
mod telegram_replay;
pub mod telegram_session;
mod telegram_spaces;
pub mod telegram_transfer;
mod telegram_transfer_journal;
mod telegram_transport;
mod telegram_vault;
#[cfg(test)]
mod tests;

mod delete_timing;
mod pull_timing;
