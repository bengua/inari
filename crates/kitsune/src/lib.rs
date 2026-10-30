//! Kitsune, Inari's voice second brain: notes said or written, found again by words and by
//! meaning, brought back by a time or a place. Everything lives in one local SQLite store,
//! whose schema moves forward and back by numbered migrations (`migrations/`): `brain.db`.

pub mod asr;
pub mod brain;
pub mod clock;
pub mod crypto;
pub mod mcp;
pub mod migrate;
pub mod net;
pub mod reminders;
pub mod search;
pub mod store;
pub mod tts;
pub mod vecext;

pub use asr::AsrConfig;
pub use brain::{Kitsune, Settings};
pub use crypto::{AudioVault, Key};
pub use mcp::Memory;
pub use migrate::{migrate_to, version, LATEST};
pub use search::{cosine, haversine_m, Filters, Geo, Hit, Mode, SearchOptions};
pub use store::{AudioRef, NewNote, Note, Reminder, Store, DIM};
pub use tts::TtsConfig;

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
