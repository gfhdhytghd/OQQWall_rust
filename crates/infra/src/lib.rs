pub mod debug_log;
pub mod journal;
pub mod snapshot;

use std::fmt;

pub use journal::{JournalCorruption, JournalCursor, LocalJournal, ReplayOutcome};
pub use snapshot::{Snapshot, SnapshotStore};

#[derive(Debug)]
pub enum InfraError {
    Io(std::io::Error),
    Codec(String),
    InvalidData(String),
}

impl fmt::Display for InfraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InfraError::Io(err) => write!(f, "io error: {}", err),
            InfraError::Codec(err) => write!(f, "codec error: {}", err),
            InfraError::InvalidData(err) => write!(f, "invalid data: {}", err),
        }
    }
}

impl std::error::Error for InfraError {}

impl From<std::io::Error> for InfraError {
    fn from(err: std::io::Error) -> Self {
        InfraError::Io(err)
    }
}

pub type InfraResult<T> = Result<T, InfraError>;
