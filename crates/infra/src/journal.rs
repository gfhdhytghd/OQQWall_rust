use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oqqwall_rust_core::EventEnvelope;
use serde::{Deserialize, Serialize};

use crate::{InfraError, InfraResult};

const SEGMENT_SUFFIX: &str = ".log";
const HEADER_BYTES: u64 = 8;
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
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            segment_size_bytes: DEFAULT_SEGMENT_BYTES,
            flush_bytes: DEFAULT_FLUSH_BYTES,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
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
        if payload.len() > MAX_RECORD_BYTES {
            return Err(InfraError::InvalidData(format!(
                "event too large: {} bytes",
                payload.len()
            )));
        }

        let record_bytes = HEADER_BYTES + payload.len() as u64;
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
        let checksum = crc32fast::hash(&payload);
        writer
            .writer
            .write_all(&(payload.len() as u32).to_le_bytes())?;
        writer.writer.write_all(&checksum.to_le_bytes())?;
        writer.writer.write_all(&payload)?;
        writer.offset = writer.offset.saturating_add(record_bytes);
        writer.pending_bytes = writer.pending_bytes.saturating_add(record_bytes as usize);

        if writer.pending_bytes >= self.config.flush_bytes
            || writer.last_flush.elapsed() >= self.config.flush_interval
        {
            writer.writer.flush()?;
            writer.pending_bytes = 0;
            writer.last_flush = Instant::now();
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

                let mut payload = vec![0u8; len];
                if let Err(err) = reader.read_exact(&mut payload) {
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
                if crc32fast::hash(&payload) != crc {
                    corruption = Some(JournalCorruption {
                        segment: segment.index,
                        offset,
                        reason: "crc mismatch".to_string(),
                    });
                    break 'segments;
                }

                let env: EventEnvelope = match bincode::deserialize(&payload) {
                    Ok(env) => env,
                    Err(err) => {
                        corruption = Some(JournalCorruption {
                            segment: segment.index,
                            offset,
                            reason: format!("decode failed: {}", err),
                        });
                        break 'segments;
                    }
                };
                apply(&env);
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
            File::create(&path)?;
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
            writer.writer.flush()?;
        }
        let next_index = self
            .writer
            .as_ref()
            .map(|writer| writer.index.saturating_add(1))
            .unwrap_or(1);
        let path = self.segment_path(next_index);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
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
