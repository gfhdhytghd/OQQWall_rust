use std::fs;
use std::path::{Path, PathBuf};

use oqqwall_rust_core::StateView;
use serde::{Deserialize, Serialize};

use crate::journal::JournalCursor;
use crate::{InfraError, InfraResult};

const SNAPSHOT_FILE: &str = "latest.snap";
const SNAPSHOT_TMP: &str = "latest.snap.tmp";
const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub taken_at_ms: i64,
    pub journal_cursor: Option<JournalCursor>,
    pub state: StateView,
}

impl Snapshot {
    pub fn new(taken_at_ms: i64, journal_cursor: Option<JournalCursor>, state: StateView) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            taken_at_ms,
            journal_cursor,
            state,
        }
    }
}

pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub fn open(data_dir: impl AsRef<Path>) -> InfraResult<Self> {
        let dir = data_dir.as_ref().join("snapshot");
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn load(&self) -> InfraResult<Option<Snapshot>> {
        let path = self.dir.join(SNAPSHOT_FILE);
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(InfraError::Io(err)),
        };

        if data.len() < 8 {
            return Ok(None);
        }
        let len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(data[4..8].try_into().unwrap());
        if data.len() != 8 + len {
            return Ok(None);
        }
        let payload = &data[8..];
        if crc32fast::hash(payload) != crc {
            return Ok(None);
        }
        let snapshot: Snapshot = match bincode::deserialize(payload) {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(None),
        };
        if snapshot.version != SNAPSHOT_VERSION {
            return Ok(None);
        }
        Ok(Some(snapshot))
    }

    pub fn write(&self, snapshot: &Snapshot) -> InfraResult<()> {
        let payload =
            bincode::serialize(snapshot).map_err(|err| InfraError::Codec(err.to_string()))?;
        let len = payload.len() as u32;
        let crc = crc32fast::hash(&payload);

        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&payload);

        let tmp_path = self.dir.join(SNAPSHOT_TMP);
        let final_path = self.dir.join(SNAPSHOT_FILE);
        fs::write(&tmp_path, &buf)?;
        fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }
}
