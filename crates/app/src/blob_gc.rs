use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::engine::EngineHandle;
use oqqwall_rust_core::{BlobId, Id128};

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

const ORPHAN_BLOB_MIN_AGE: Duration = Duration::from_secs(3 * 24 * 60 * 60);

pub fn sweep_orphan_blobs(
    handle: &EngineHandle,
    data_dir: impl AsRef<Path>,
    enabled: bool,
) -> Result<usize, String> {
    if !enabled {
        return Ok(0);
    }

    let blob_root = data_dir.as_ref().join("blobs");
    if !blob_root.exists() {
        return Ok(0);
    }

    let live_blob_ids = {
        let state = handle.state();
        let guard = state
            .read()
            .map_err(|_| "state lock poisoned during blob orphan sweep".to_string())?;
        guard.blobs.keys().copied().collect::<HashSet<_>>()
    };

    let now = SystemTime::now();
    let mut removed = 0usize;
    let mut stack = vec![blob_root];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_err) => {
                debug_log!(
                    "blob orphan sweep read_dir failed: path={:?} error={}",
                    dir,
                    _err
                );
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(blob_id) = parse_blob_id_from_path(&path) else {
                continue;
            };
            if live_blob_ids.contains(&blob_id) {
                continue;
            }
            if !file_is_older_than(&path, now, ORPHAN_BLOB_MIN_AGE) {
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => removed = removed.saturating_add(1),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_err) => {
                    debug_log!(
                        "blob orphan sweep remove failed: path={:?} error={}",
                        path,
                        _err
                    );
                }
            }
        }
    }
    Ok(removed)
}

fn file_is_older_than(path: &Path, now: SystemTime, min_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= min_age)
}

fn parse_blob_id_from_path(path: &Path) -> Option<BlobId> {
    let file_name = path.file_name()?.to_str()?;
    let hex = file_name.split('.').next()?;
    if hex.len() != 32 || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u128::from_str_radix(hex, 16).ok().map(Id128::from_u128)
}
