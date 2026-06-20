use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

#[derive(Default)]
struct AvatarCacheState {
    entries: HashMap<String, Arc<[u8]>>,
    in_flight: HashMap<String, InFlightState>,
}

struct InFlightState {
    notify: Arc<Notify>,
    fetching: bool,
}

static AVATAR_CACHE: OnceLock<Mutex<AvatarCacheState>> = OnceLock::new();

fn cache() -> &'static Mutex<AvatarCacheState> {
    AVATAR_CACHE.get_or_init(|| Mutex::new(AvatarCacheState::default()))
}

fn normalize_user_id(user_id: &str) -> Option<String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn has_avatar(user_id: &str) -> bool {
    let Some(key) = normalize_user_id(user_id) else {
        return false;
    };
    let guard = cache().lock().expect("avatar cache lock poisoned");
    guard.entries.contains_key(&key)
}

pub fn get_avatar_bytes(user_id: &str) -> Option<Arc<[u8]>> {
    let Some(key) = normalize_user_id(user_id) else {
        return None;
    };
    let guard = cache().lock().expect("avatar cache lock poisoned");
    guard.entries.get(&key).cloned()
}

pub fn insert_avatar_bytes(user_id: &str, bytes: Arc<[u8]>) {
    if bytes.is_empty() {
        return;
    }
    let Some(key) = normalize_user_id(user_id) else {
        return;
    };
    let mut guard = cache().lock().expect("avatar cache lock poisoned");
    guard.entries.insert(key.clone(), bytes);
    if let Some(state) = guard.in_flight.remove(&key) {
        state.notify.notify_waiters();
    }
}

pub fn start_fetch(user_id: &str) -> Option<Arc<Notify>> {
    let Some(key) = normalize_user_id(user_id) else {
        return None;
    };
    let mut guard = cache().lock().expect("avatar cache lock poisoned");
    if guard.entries.contains_key(&key) {
        return None;
    }
    if let Some(state) = guard.in_flight.get_mut(&key) {
        if state.fetching {
            return None;
        }
        state.fetching = true;
        return Some(state.notify.clone());
    }
    let notify = Arc::new(Notify::new());
    guard.in_flight.insert(
        key,
        InFlightState {
            notify: notify.clone(),
            fetching: true,
        },
    );
    Some(notify)
}

pub fn ensure_in_flight(user_id: &str) -> Option<(Arc<Notify>, bool)> {
    let Some(key) = normalize_user_id(user_id) else {
        return None;
    };
    let mut guard = cache().lock().expect("avatar cache lock poisoned");
    if guard.entries.contains_key(&key) {
        return None;
    }
    if let Some(state) = guard.in_flight.get(&key) {
        return Some((state.notify.clone(), false));
    }
    let notify = Arc::new(Notify::new());
    guard.in_flight.insert(
        key,
        InFlightState {
            notify: notify.clone(),
            fetching: false,
        },
    );
    Some((notify, true))
}

pub fn wait_for_avatar(
    user_id: &str,
    notify: Arc<Notify>,
    timeout: std::time::Duration,
) -> impl std::future::Future<Output = Option<Arc<[u8]>>> {
    let user_id = user_id.to_string();
    async move {
        if let Some(bytes) = get_avatar_bytes(&user_id) {
            return Some(bytes);
        }
        let _ = tokio::time::timeout(timeout, notify.notified()).await;
        get_avatar_bytes(&user_id)
    }
}

pub fn finish_fetch(user_id: &str) {
    let Some(key) = normalize_user_id(user_id) else {
        return;
    };
    let mut guard = cache().lock().expect("avatar cache lock poisoned");
    if let Some(state) = guard.in_flight.remove(&key) {
        state.notify.notify_waiters();
    }
}

pub fn remove_avatar(user_id: &str) {
    let Some(key) = normalize_user_id(user_id) else {
        return;
    };
    let mut guard = cache().lock().expect("avatar cache lock poisoned");
    guard.entries.remove(&key);
    if let Some(state) = guard.in_flight.remove(&key) {
        state.notify.notify_waiters();
    }
}
