//! The Session State Capsule (`ssc/1`): a directory of plain files that fully
//! describes an agent thread. See `docs/ssc.md` for the format.

pub mod attach;
pub mod io;
pub mod model;
pub mod timeline;

pub use io::{append_event, load, pack, seal, unpack, verify, Capsule};
pub use model::{EventKind, FileDigest, Manifest, Snapshot, TimelineEvent};
pub use timeline::Timeline;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("integrity: {0}")]
    Integrity(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
