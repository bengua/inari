//! Kitsune, Inari's voice second brain: notes said or written, found again by words and by
//! meaning, brought back by a time or a place. Everything lives in one local SQLite store,
//! whose schema moves forward and back by numbered migrations (`migrations/`).

pub mod crypto;
pub mod mcp;
pub mod migrate;
pub mod search;
pub mod store;
pub mod triggers;

pub use crypto::{AudioVault, Key};
pub use migrate::{migrate_to, version, LATEST};
pub use search::{cosine, haversine_m, Filters, Geo, Hit, SearchOptions};
pub use store::{AudioRef, NewNote, Note, Store, Trigger};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
