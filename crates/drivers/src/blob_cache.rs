use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use oqqwall_rust_core::draft::MediaKind;
use oqqwall_rust_core::ids::BlobId;

const DEFAULT_MAX_CACHE_MB: u64 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
    Image,
    Sticker,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRetention {
    RenderOnly,
    UntilSend,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub bytes: Arc<[u8]>,
    pub size_bytes: u64,
    pub kind: CacheKind,
    pub retention: CacheRetention,
    pub mime: Option<String>,
}

#[derive(Default)]
struct CacheState {
    max_bytes: u64,
    current_bytes: u64,
    entries: HashMap<BlobId, CacheEntry>,
}

static CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();

fn cache_state() -> &'static Mutex<CacheState> {
    CACHE.get_or_init(|| {
        Mutex::new(CacheState {
            max_bytes: DEFAULT_MAX_CACHE_MB.saturating_mul(1024 * 1024),
            ..Default::default()
        })
    })
}

pub fn configure_max_cache_mb(max_cache_mb: u64) {
    let mut state = cache_state().lock().unwrap_or_else(|err| err.into_inner());
    state.max_bytes = max_cache_mb.saturating_mul(1024 * 1024);
    if state.max_bytes == 0 {
        state.entries.clear();
        state.current_bytes = 0;
        return;
    }
    evict_to_limit(&mut state);
}

pub fn cache_policy_for_media(kind: MediaKind) -> Option<(CacheKind, CacheRetention)> {
    match kind {
        MediaKind::Image => Some((CacheKind::Image, CacheRetention::UntilSend)),
        MediaKind::Sticker => Some((CacheKind::Sticker, CacheRetention::RenderOnly)),
        MediaKind::Video | MediaKind::Audio | MediaKind::File | MediaKind::Other => None,
    }
}

pub fn store_bytes(
    blob_id: BlobId,
    bytes: Vec<u8>,
    kind: CacheKind,
    retention: CacheRetention,
    mime: Option<String>,
) -> Arc<[u8]> {
    let bytes: Arc<[u8]> = Arc::from(bytes);
    store_arc(blob_id, bytes.clone(), kind, retention, mime);
    bytes
}

pub fn store_arc(
    blob_id: BlobId,
    bytes: Arc<[u8]>,
    kind: CacheKind,
    retention: CacheRetention,
    mime: Option<String>,
) {
    let size_bytes = bytes.len() as u64;
    let mut state = cache_state().lock().unwrap_or_else(|err| err.into_inner());
    if state.max_bytes == 0 || size_bytes == 0 {
        return;
    }
    if state.max_bytes > 0 && size_bytes > state.max_bytes {
        return;
    }
    if let Some(prev) = state.entries.insert(
        blob_id,
        CacheEntry {
            bytes,
            size_bytes,
            kind,
            retention,
            mime,
        },
    ) {
        state.current_bytes = state
            .current_bytes
            .saturating_sub(prev.size_bytes)
            .saturating_add(size_bytes);
    } else {
        state.current_bytes = state.current_bytes.saturating_add(size_bytes);
    }
    evict_to_limit(&mut state);
}

pub fn get_entry(blob_id: BlobId) -> Option<CacheEntry> {
    let state = cache_state().lock().unwrap_or_else(|err| err.into_inner());
    state.entries.get(&blob_id).cloned()
}

pub fn get_bytes(blob_id: BlobId) -> Option<Arc<[u8]>> {
    get_entry(blob_id).map(|entry| entry.bytes)
}

pub fn release(blob_id: BlobId) {
    let mut state = cache_state().lock().unwrap_or_else(|err| err.into_inner());
    if let Some(entry) = state.entries.remove(&blob_id) {
        state.current_bytes = state.current_bytes.saturating_sub(entry.size_bytes);
    }
}

pub fn release_many<I>(blob_ids: I)
where
    I: IntoIterator<Item = BlobId>,
{
    let mut state = cache_state().lock().unwrap_or_else(|err| err.into_inner());
    for blob_id in blob_ids {
        if let Some(entry) = state.entries.remove(&blob_id) {
            state.current_bytes = state.current_bytes.saturating_sub(entry.size_bytes);
        }
    }
}

pub fn release_render_only<I>(blob_ids: I)
where
    I: IntoIterator<Item = BlobId>,
{
    let mut state = cache_state().lock().unwrap_or_else(|err| err.into_inner());
    for blob_id in blob_ids {
        let retention = state.entries.get(&blob_id).map(|entry| entry.retention);
        if retention != Some(CacheRetention::RenderOnly) {
            continue;
        }
        if let Some(entry) = state.entries.remove(&blob_id) {
            state.current_bytes = state.current_bytes.saturating_sub(entry.size_bytes);
        }
    }
}

fn evict_to_limit(state: &mut CacheState) {
    if state.max_bytes == 0 || state.current_bytes <= state.max_bytes {
        return;
    }
    let mut entries = state
        .entries
        .iter()
        .map(|(blob_id, entry)| (*blob_id, entry.size_bytes))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    for (blob_id, _size) in entries {
        if state.current_bytes <= state.max_bytes {
            break;
        }
        if let Some(entry) = state.entries.remove(&blob_id) {
            state.current_bytes = state.current_bytes.saturating_sub(entry.size_bytes);
        }
    }
}
