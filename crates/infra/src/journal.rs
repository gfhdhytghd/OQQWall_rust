use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oqqwall_rust_core::EventEnvelope;
use serde::{Deserialize, Serialize};

use crate::record;
use crate::{InfraError, InfraResult};

const SEGMENT_SUFFIX: &str = ".log";
const HEADER_BYTES: u64 = record::HEADER_BYTES as u64;
const DEFAULT_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_FLUSH_BYTES: usize = 256 * 1024;
const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(50);
const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalCursor {
    pub segment: u64,
    pub offset: u64,
}

impl JournalCursor {
    pub fn origin() -> Self {
        Self {
            segment: 1,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JournalConfig {
    pub segment_size_bytes: u64,
    pub flush_bytes: usize,
    pub flush_interval: Duration,
    pub sync_on_flush: bool,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            segment_size_bytes: DEFAULT_SEGMENT_BYTES,
            flush_bytes: DEFAULT_FLUSH_BYTES,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
            sync_on_flush: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JournalCorruption {
    pub segment: u64,
    pub offset: u64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ReplayOutcome {
    pub events: u64,
    pub last_cursor: JournalCursor,
    pub corruption: Option<JournalCorruption>,
    pub legacy_records: u64,
}

pub struct LocalJournal {
    dir: PathBuf,
    config: JournalConfig,
    writer: Option<SegmentWriter>,
}

struct SegmentWriter {
    index: u64,
    writer: BufWriter<File>,
    offset: u64,
    pending_bytes: usize,
    last_flush: Instant,
}

struct SegmentInfo {
    index: u64,
    path: PathBuf,
    len: u64,
}

impl LocalJournal {
    pub fn open(data_dir: impl AsRef<Path>) -> InfraResult<Self> {
        let dir = data_dir.as_ref().join("journal");
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            config: JournalConfig::default(),
            writer: None,
        })
    }

    pub fn open_with_config(
        data_dir: impl AsRef<Path>,
        config: JournalConfig,
    ) -> InfraResult<Self> {
        let dir = data_dir.as_ref().join("journal");
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            config,
            writer: None,
        })
    }

    pub fn append(&mut self, env: &EventEnvelope) -> InfraResult<JournalCursor> {
        self.ensure_writer()?;
        let payload = bincode::serialize(env).map_err(|err| InfraError::Codec(err.to_string()))?;
        let body_len = record::versioned_body_len(payload.len())?;
        if body_len > MAX_RECORD_BYTES {
            return Err(InfraError::InvalidData(format!(
                "event too large: {} bytes",
                body_len
            )));
        }

        let record_bytes = HEADER_BYTES + body_len as u64;
        if record_bytes > self.config.segment_size_bytes {
            return Err(InfraError::InvalidData(format!(
                "record size {} exceeds segment size {}",
                record_bytes, self.config.segment_size_bytes
            )));
        }

        if let Some(writer) = self.writer.as_ref() {
            if writer.offset + record_bytes > self.config.segment_size_bytes {
                self.rotate_segment()?;
            }
        }

        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| InfraError::InvalidData("journal writer unavailable".to_string()))?;
        let record = record::encode_serialized_payload(&payload)?;
        writer.writer.write_all(&record)?;
        writer.offset = writer.offset.saturating_add(record_bytes);
        writer.pending_bytes = writer.pending_bytes.saturating_add(record_bytes as usize);

        if writer.pending_bytes >= self.config.flush_bytes
            || writer.last_flush.elapsed() >= self.config.flush_interval
        {
            flush_segment_writer(writer, self.config.sync_on_flush)?;
        }

        Ok(JournalCursor {
            segment: writer.index,
            offset: writer.offset,
        })
    }

    pub fn replay<F>(
        &self,
        start: Option<JournalCursor>,
        mut apply: F,
    ) -> InfraResult<ReplayOutcome>
    where
        F: FnMut(&EventEnvelope),
    {
        let segments = self.list_segments()?;
        if segments.is_empty() {
            return Ok(ReplayOutcome {
                events: 0,
                last_cursor: JournalCursor::origin(),
                corruption: None,
                legacy_records: 0,
            });
        }

        let start_cursor = match start {
            Some(cursor) => cursor,
            None => JournalCursor {
                segment: segments[0].index,
                offset: 0,
            },
        };

        if !segments.iter().any(|seg| seg.index == start_cursor.segment) {
            return Err(InfraError::InvalidData(format!(
                "start segment {} not found in journal",
                start_cursor.segment
            )));
        }

        let mut last_cursor = start_cursor;
        let mut events: u64 = 0;
        let mut legacy_records: u64 = 0;
        let mut corruption = None;

        'segments: for segment in segments {
            if segment.index < start_cursor.segment {
                continue;
            }
            let mut offset = if segment.index == start_cursor.segment {
                start_cursor.offset
            } else {
                0
            };
            if offset > segment.len {
                return Err(InfraError::InvalidData(format!(
                    "cursor offset {} beyond segment {} length {}",
                    offset, segment.index, segment.len
                )));
            }
            last_cursor = JournalCursor {
                segment: segment.index,
                offset,
            };

            let file = File::open(&segment.path)?;
            let mut reader = BufReader::new(file);
            reader.seek(SeekFrom::Start(offset))?;

            while offset < segment.len {
                if offset + HEADER_BYTES > segment.len {
                    corruption = Some(JournalCorruption {
                        segment: segment.index,
                        offset,
                        reason: "truncated header".to_string(),
                    });
                    break 'segments;
                }

                let mut header = [0u8; 8];
                if let Err(err) = reader.read_exact(&mut header) {
                    if err.kind() == std::io::ErrorKind::UnexpectedEof {
                        corruption = Some(JournalCorruption {
                            segment: segment.index,
                            offset,
                            reason: "truncated header".to_string(),
                        });
                        break 'segments;
                    }
                    return Err(InfraError::Io(err));
                }
                let len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
                let crc = u32::from_le_bytes(header[4..8].try_into().unwrap());

                if len > MAX_RECORD_BYTES {
                    corruption = Some(JournalCorruption {
                        segment: segment.index,
                        offset,
                        reason: format!("record too large: {} bytes", len),
                    });
                    break 'segments;
                }

                let next_offset = offset + HEADER_BYTES + len as u64;
                if next_offset > segment.len {
                    corruption = Some(JournalCorruption {
                        segment: segment.index,
                        offset,
                        reason: "truncated payload".to_string(),
                    });
                    break 'segments;
                }

                let mut body = vec![0u8; len];
                if let Err(err) = reader.read_exact(&mut body) {
                    if err.kind() == std::io::ErrorKind::UnexpectedEof {
                        corruption = Some(JournalCorruption {
                            segment: segment.index,
                            offset,
                            reason: "truncated payload".to_string(),
                        });
                        break 'segments;
                    }
                    return Err(InfraError::Io(err));
                }
                let decoded = match record::deserialize_body::<EventEnvelope>(&body, crc) {
                    Ok(decoded) => decoded,
                    Err(err) => {
                        corruption = Some(JournalCorruption {
                            segment: segment.index,
                            offset,
                            reason: err,
                        });
                        break 'segments;
                    }
                };
                if decoded.legacy_format {
                    legacy_records = legacy_records.saturating_add(1);
                }
                apply(&decoded.value);
                events = events.saturating_add(1);
                offset = next_offset;
                last_cursor = JournalCursor {
                    segment: segment.index,
                    offset,
                };
            }

            last_cursor = JournalCursor {
                segment: segment.index,
                offset,
            };
        }

        Ok(ReplayOutcome {
            events,
            last_cursor,
            corruption,
            legacy_records,
        })
    }

    pub fn truncate_tail(&mut self, cursor: JournalCursor) -> InfraResult<()> {
        self.writer = None;
        let segments = self.list_segments()?;
        for segment in segments.iter().filter(|seg| seg.index > cursor.segment) {
            let _ = fs::remove_file(&segment.path);
        }
        let target = self.segment_path(cursor.segment);
        if target.exists() {
            let file = OpenOptions::new().write(true).open(&target)?;
            file.set_len(cursor.offset)?;
        } else if cursor.offset != 0 {
            return Err(InfraError::InvalidData(format!(
                "segment {} missing for truncate",
                cursor.segment
            )));
        }
        Ok(())
    }

    pub fn delete_segments_before(&mut self, cursor: JournalCursor) -> InfraResult<()> {
        let segments = self.list_segments()?;
        let mut removed = false;
        for segment in segments.iter().filter(|seg| seg.index < cursor.segment) {
            match fs::remove_file(&segment.path) {
                Ok(()) => removed = true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(InfraError::Io(err)),
            }
        }
        if removed {
            sync_dir(&self.dir)?;
        }
        Ok(())
    }

    pub fn first_segment_index(&self) -> InfraResult<Option<u64>> {
        Ok(self.list_segments()?.first().map(|segment| segment.index))
    }

    fn ensure_writer(&mut self) -> InfraResult<()> {
        if self.writer.is_some() {
            return Ok(());
        }

        let segments = self.list_segments()?;
        let (index, offset) = if let Some(last) = segments.last() {
            (last.index, last.len)
        } else {
            let index = 1;
            let path = self.segment_path(index);
            let file = File::create(&path)?;
            file.sync_all()?;
            sync_dir(&self.dir)?;
            (index, 0)
        };

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.segment_path(index))?;
        self.writer = Some(SegmentWriter {
            index,
            writer: BufWriter::new(file),
            offset,
            pending_bytes: 0,
            last_flush: Instant::now(),
        });
        Ok(())
    }

    fn rotate_segment(&mut self) -> InfraResult<()> {
        if let Some(writer) = self.writer.as_mut() {
            flush_segment_writer(writer, self.config.sync_on_flush)?;
        }
        let next_index = self
            .writer
            .as_ref()
            .map(|writer| writer.index.saturating_add(1))
            .unwrap_or(1);
        let path = self.segment_path(next_index);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.sync_all()?;
        sync_dir(&self.dir)?;
        self.writer = Some(SegmentWriter {
            index: next_index,
            writer: BufWriter::new(file),
            offset: 0,
            pending_bytes: 0,
            last_flush: Instant::now(),
        });
        Ok(())
    }

    fn list_segments(&self) -> InfraResult<Vec<SegmentInfo>> {
        let mut segments = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let Some(index) = parse_segment_index(name) else {
                continue;
            };
            let len = entry.metadata()?.len();
            segments.push(SegmentInfo { index, path, len });
        }
        segments.sort_by_key(|seg| seg.index);
        Ok(segments)
    }

    fn segment_path(&self, index: u64) -> PathBuf {
        self.dir.join(format!("{:08}{}", index, SEGMENT_SUFFIX))
    }
}

fn parse_segment_index(name: &str) -> Option<u64> {
    let trimmed = name.strip_suffix(SEGMENT_SUFFIX)?;
    trimmed.parse::<u64>().ok()
}

fn flush_segment_writer(writer: &mut SegmentWriter, sync_on_flush: bool) -> InfraResult<()> {
    writer.writer.flush()?;
    if sync_on_flush {
        writer.writer.get_ref().sync_all()?;
    }
    writer.pending_bytes = 0;
    writer.last_flush = Instant::now();
    Ok(())
}

fn sync_dir(dir: &Path) -> InfraResult<()> {
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    use oqqwall_rust_core::event::SystemEvent;
    use oqqwall_rust_core::{Event, EventEnvelope, Id128};

    use super::*;

    #[test]
    fn delete_segments_before_keeps_cursor_segment_and_is_idempotent() {
        let data_dir = temp_data_dir("delete-segments-before");
        let payload_len = bincode::serialize(&envelope(1)).unwrap().len() as u64;
        let config = JournalConfig {
            segment_size_bytes: HEADER_BYTES
                + record::versioned_body_len(payload_len as usize).unwrap() as u64,
            flush_bytes: 1,
            flush_interval: Duration::from_secs(60),
            sync_on_flush: false,
        };
        let mut journal = LocalJournal::open_with_config(&data_dir, config).unwrap();

        let _cursor1 = journal.append(&envelope(1)).unwrap();
        let cursor2 = journal.append(&envelope(2)).unwrap();
        let _cursor3 = journal.append(&envelope(3)).unwrap();
        assert_eq!(segment_indexes(&journal), vec![1, 2, 3]);

        journal.delete_segments_before(cursor2).unwrap();
        assert_eq!(segment_indexes(&journal), vec![2, 3]);

        journal.delete_segments_before(cursor2).unwrap();
        assert_eq!(segment_indexes(&journal), vec![2, 3]);

        let mut replayed = Vec::new();
        journal
            .replay(Some(cursor2), |env| replayed.push(env.id.0))
            .unwrap();
        assert_eq!(replayed, vec![3]);

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn replay_reads_legacy_records_when_payload_starts_with_version_byte() {
        let data_dir = temp_data_dir("legacy-records");
        let journal_dir = data_dir.join("journal");
        std::fs::create_dir_all(&journal_dir).unwrap();
        let mut file = File::create(journal_dir.join("00000001.log")).unwrap();
        let first = legacy_record(&envelope(record::FORMAT_VERSION as u128));
        assert_eq!(first[record::HEADER_BYTES], record::FORMAT_VERSION);
        file.write_all(&first).unwrap();
        file.write_all(&legacy_record(&envelope(3))).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let journal = LocalJournal::open(&data_dir).unwrap();
        let mut replayed = Vec::new();
        let outcome = journal.replay(None, |env| replayed.push(env.id.0)).unwrap();

        assert_eq!(replayed, vec![record::FORMAT_VERSION as u128, 3]);
        assert_eq!(outcome.events, 2);
        assert_eq!(outcome.legacy_records, 2);
        assert!(outcome.corruption.is_none());

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn append_writes_versioned_record_body() {
        let data_dir = temp_data_dir("versioned-record");
        let config = JournalConfig {
            flush_bytes: 1,
            sync_on_flush: false,
            ..JournalConfig::default()
        };
        let mut journal = LocalJournal::open_with_config(&data_dir, config).unwrap();
        journal.append(&envelope(1)).unwrap();

        let data = std::fs::read(data_dir.join("journal").join("00000001.log")).unwrap();
        let len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let body = &data[record::HEADER_BYTES..];
        assert_eq!(data.len(), record::HEADER_BYTES + len);
        assert_eq!(body[0], record::FORMAT_VERSION);
        assert_eq!(crc32fast::hash(body), crc);

        std::fs::remove_dir_all(data_dir).ok();
    }

    fn envelope(id: u128) -> EventEnvelope {
        EventEnvelope {
            id: Id128(id),
            ts_ms: id as i64,
            actor: Id128(1),
            correlation_id: None,
            event: Event::System(SystemEvent::Booted),
        }
    }

    fn segment_indexes(journal: &LocalJournal) -> Vec<u64> {
        journal
            .list_segments()
            .unwrap()
            .into_iter()
            .map(|segment| segment.index)
            .collect()
    }

    fn legacy_record(env: &EventEnvelope) -> Vec<u8> {
        let payload = bincode::serialize(env).unwrap();
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
