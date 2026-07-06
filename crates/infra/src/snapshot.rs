use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use oqqwall_rust_core::StateView;
use serde::{Deserialize, Serialize};

use crate::journal::JournalCursor;
use crate::record;
use crate::{InfraError, InfraResult};

const SNAPSHOT_FILE: &str = "latest.snap";
const SNAPSHOT_TMP: &str = "latest.snap.tmp";
const SNAPSHOT_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub taken_at_ms: i64,
    pub journal_cursor: Option<JournalCursor>,
    pub state: StateView,
}

#[derive(Debug, Clone)]
pub struct LoadedSnapshot {
    pub snapshot: Snapshot,
    pub legacy_format: bool,
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
        Ok(self.load_with_format()?.map(|loaded| loaded.snapshot))
    }

    pub fn load_with_format(&self) -> InfraResult<Option<LoadedSnapshot>> {
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
        let body = &data[8..];
        let decoded = match record::deserialize_body::<Snapshot>(body, crc) {
            Ok(decoded) => decoded,
            Err(_) => return Ok(None),
        };
        if decoded.value.version != SNAPSHOT_VERSION {
            return Ok(None);
        }
        Ok(Some(LoadedSnapshot {
            snapshot: decoded.value,
            legacy_format: decoded.legacy_format,
        }))
    }

    pub fn write(&self, snapshot: &Snapshot) -> InfraResult<()> {
        let buf = record::serialize_record(snapshot)?;

        let tmp_path = self.dir.join(SNAPSHOT_TMP);
        let final_path = self.dir.join(SNAPSHOT_FILE);
        let mut file = File::create(&tmp_path)?;
        file.write_all(&buf)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, &final_path)?;
        sync_dir(&self.dir)?;
        Ok(())
    }
}

fn sync_dir(dir: &Path) -> InfraResult<()> {
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use oqqwall_rust_core::StateView;

    use super::*;

    #[test]
    fn load_reads_legacy_snapshot_record() {
        let data_dir = temp_data_dir("legacy-snapshot");
        let store = SnapshotStore::open(&data_dir).unwrap();
        let snapshot = Snapshot::new(
            123,
            Some(JournalCursor {
                segment: 2,
                offset: 10,
            }),
            StateView::default(),
        );
        std::fs::write(store.dir.join(SNAPSHOT_FILE), legacy_record(&snapshot)).unwrap();

        let loaded = store.load_with_format().unwrap().unwrap();
        assert!(loaded.legacy_format);
        assert_eq!(loaded.snapshot.taken_at_ms, 123);
        assert_eq!(
            loaded.snapshot.journal_cursor,
            Some(JournalCursor {
                segment: 2,
                offset: 10
            })
        );

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn write_uses_versioned_snapshot_record() {
        let data_dir = temp_data_dir("versioned-snapshot");
        let store = SnapshotStore::open(&data_dir).unwrap();
        let snapshot = Snapshot::new(456, None, StateView::default());
        store.write(&snapshot).unwrap();

        let data = std::fs::read(store.dir.join(SNAPSHOT_FILE)).unwrap();
        let len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let body = &data[record::HEADER_BYTES..];
        assert_eq!(data.len(), record::HEADER_BYTES + len);
        assert_eq!(body[0], record::FORMAT_VERSION);
        assert_eq!(crc32fast::hash(body), crc);

        std::fs::remove_dir_all(data_dir).ok();
    }

    fn legacy_record(snapshot: &Snapshot) -> Vec<u8> {
        let payload = bincode::serialize(snapshot).unwrap();
        let mut buf = Vec::with_capacity(record::HEADER_BYTES + payload.len());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
        buf.extend_from_slice(&payload);
        buf
    }

    fn temp_data_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "oqqwall-infra-{}-{}-{}",
            name,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
