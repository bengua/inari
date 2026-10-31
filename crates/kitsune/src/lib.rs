//! Kitsune, Inari's voice second brain: notes said or written, found again by words and by
//! meaning, brought back by a time or a place. Everything lives in one local SQLite store,
//! whose schema moves forward and back by numbered migrations (`migrations/`): `brain.db`.

pub mod asr;
pub mod brain;
pub mod clock;
pub mod crypto;
pub mod embed;
pub mod geocode;
pub mod lists;
pub mod mcp;
pub mod migrate;
pub mod net;
pub mod reminders;
pub mod rings;
pub mod search;
pub mod store;
pub mod tts;
pub mod understand;
pub mod vecext;

pub use asr::AsrConfig;
pub use brain::{Capture, Kitsune, Retried, Settings, VoiceConfig};
pub use crypto::{AudioVault, Key};
pub use embed::EmbedConfig;
pub use geocode::GeocoderConfig;
pub use lists::{Item, List, NewItem};
pub use mcp::Memory;
pub use migrate::{migrate_to, version, LATEST};
pub use rings::{RingConfig, Tick};
pub use search::{cosine, haversine_m, Filters, Geo, Hit, Mode, SearchOptions};
pub use store::{AudioRef, NewNote, Note, Reminder, Store, DIM};
pub use tts::TtsConfig;
pub use understand::SlmConfig;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    /// The capture could not be heard well enough to be trusted; nothing was kept.
    #[error("{0}")]
    Inaudible(String),
}

pub type Result<T> = std::result::Result<T, Error>;
