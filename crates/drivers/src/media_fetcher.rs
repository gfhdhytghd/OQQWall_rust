use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use oqqwall_rust_core::draft::{IngressAttachment, IngressMessage, MediaKind, MediaReference};
use oqqwall_rust_core::event::{BlobEvent, Event, IngressEvent, MediaEvent};
use oqqwall_rust_core::ids::{BlobId, IngressId, TimestampMs};
use oqqwall_rust_core::{Command, derive_blob_id};
use reqwest::Client;
use reqwest::header::CONTENT_TYPE;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::avatar_cache;
use crate::blob_cache;

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

#[derive(Debug, Clone)]
pub struct MediaFetcherRuntimeConfig {
    pub blob_root: PathBuf,
    pub max_attempts: u32,
    pub timeout: Duration,
}

impl Default for MediaFetcherRuntimeConfig {
    fn default() -> Self {
        let blob_root = std::env::var("OQQWALL_BLOB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/blobs"));
        Self {
            blob_root,
            max_attempts: 3,
            timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Default)]
struct MediaFetchState {
    ingress_messages: HashMap<IngressId, IngressMessage>,
}

pub fn spawn_media_fetcher(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    runtime: MediaFetcherRuntimeConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let client = match Client::builder().timeout(runtime.timeout).build() {
            Ok(client) => client,
            Err(_err) => {
                debug_log!("media fetcher init failed: {}", _err);
                return;
            }
        };
        let state = Arc::new(Mutex::new(MediaFetchState::default()));
        let mut bus_rx = bus_rx;

        loop {
            let env = match bus_rx.recv().await {
                Ok(env) => env,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            match env.event {
                Event::Ingress(IngressEvent::MessageAccepted {
                    ingress_id,
                    user_id,
                    message,
                    ..
                })
                | Event::Ingress(IngressEvent::MessageSynced {
                    ingress_id,
                    user_id,
                    message,
                    ..
                }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.insert(ingress_id, message);
                    let user_id = user_id.clone();
                    if avatar_cache::start_fetch(&user_id).is_some() {
                        let client = client.clone();
                        tokio::spawn(async move {
                            prime_avatar_cache(client, user_id).await;
                        });
                    }
                }
                Event::Ingress(IngressEvent::MessageIgnored { ingress_id, .. }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.remove(&ingress_id);
                }
                Event::Ingress(IngressEvent::MessageRecalled { ingress_id, .. }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.remove(&ingress_id);
                }
                Event::Media(MediaEvent::MediaFetchRequested {
                    ingress_id,
                    attachment_index,
                    attempt,
                }) => {
                    let state = state.clone();
                    let cmd_tx = cmd_tx.clone();
                    let client = client.clone();
                    let runtime = runtime.clone();
                    tokio::spawn(async move {
                        handle_fetch(
                            cmd_tx,
                            client,
                            runtime,
                            state,
                            ingress_id,
                            attachment_index,
                            attempt,
                        )
                        .await;
                    });
                }
                Event::Media(MediaEvent::AvatarFetchRequested { user_id }) => {
                    if avatar_cache::start_fetch(&user_id).is_some() {
                        let client = client.clone();
                        let user_id = user_id.clone();
                        tokio::spawn(async move {
                            prime_avatar_cache(client, user_id).await;
                        });
                    }
                }
                _ => {}
            }
        }
    })
}

fn avatar_url_for_user(user_id: &str) -> Option<String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        return None;
    }
    Some(format!(
        "https://qlogo2.store.qq.com/qzone/{0}/{0}/640",
        trimmed
    ))
}

async fn prime_avatar_cache(client: Client, user_id: String) {
    if avatar_cache::has_avatar(&user_id) {
        avatar_cache::finish_fetch(&user_id);
        return;
    }
    let Some(url) = avatar_url_for_user(&user_id) else {
        avatar_cache::finish_fetch(&user_id);
        return;
    };
    let resp = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(_err) => {
            debug_log!("avatar fetch failed: {}: {}", user_id, _err);
            avatar_cache::finish_fetch(&user_id);
            return;
        }
    };
    if !resp.status().is_success() {
        debug_log!("avatar http status {}: {}", resp.status().as_u16(), user_id);
        avatar_cache::finish_fetch(&user_id);
        return;
    }
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(_err) => {
            debug_log!("avatar read failed: {}: {}", user_id, _err);
            avatar_cache::finish_fetch(&user_id);
            return;
        }
    };
    if bytes.is_empty() {
        avatar_cache::finish_fetch(&user_id);
        return;
    }
    avatar_cache::insert_avatar_bytes(&user_id, Arc::from(bytes.to_vec()));
}

async fn handle_fetch(
    cmd_tx: mpsc::Sender<Command>,
    client: Client,
    runtime: MediaFetcherRuntimeConfig,
    state: Arc<Mutex<MediaFetchState>>,
    ingress_id: IngressId,
    attachment_index: usize,
    initial_attempt: u32,
) {
    let attachment = {
        let guard = state.lock().await;
        guard
            .ingress_messages
            .get(&ingress_id)
            .and_then(|message| message.attachments.get(attachment_index))
            .cloned()
    };
    let Some(attachment) = attachment else {
        let error = "missing ingress attachment".to_string();
        let _ = send_media_failed(
            &cmd_tx,
            ingress_id,
            attachment_index,
            initial_attempt,
            error,
        )
        .await;
        return;
    };

    let url = match &attachment.reference {
        MediaReference::RemoteUrl { url } => url.clone(),
        _ => return,
    };

    let blob_id = derive_attachment_blob_id(ingress_id, attachment_index);
    let mut attempt = initial_attempt.max(1);
    let mut local_attempt = 1u32;
    let max_attempts = runtime.max_attempts.max(1);
    loop {
        match fetch_bytes(&client, &url).await {
            Ok(fetched) => {
                let ext = choose_extension(&attachment, &url, &fetched);
                let bytes: Arc<[u8]> = Arc::from(fetched.bytes);
                if let Some((kind, retention)) = blob_cache::cache_policy_for_media(attachment.kind)
                {
                    blob_cache::store_arc(
                        blob_id,
                        bytes.clone(),
                        kind,
                        retention,
                        fetched.content_type.clone(),
                    );
                }
                match persist_blob(
                    &runtime.blob_root,
                    kind_dir(&attachment.kind),
                    &ext,
                    blob_id,
                    bytes.as_ref(),
                ) {
                    Ok((path, size_bytes)) => {
                        let _ = send_event(
                            &cmd_tx,
                            Event::Blob(BlobEvent::BlobRegistered {
                                blob_id,
                                size_bytes,
                            }),
                        )
                        .await;
                        let _ = send_event(
                            &cmd_tx,
                            Event::Blob(BlobEvent::BlobPersisted { blob_id, path }),
                        )
                        .await;
                        let _ = send_event(
                            &cmd_tx,
                            Event::Media(MediaEvent::MediaFetchSucceeded {
                                ingress_id,
                                attachment_index,
                                blob_id,
                            }),
                        )
                        .await;
                    }
                    Err(err) => {
                        let _ =
                            send_media_failed(&cmd_tx, ingress_id, attachment_index, attempt, err)
                                .await;
                    }
                }
                break;
            }
            Err(err) => {
                if local_attempt >= max_attempts {
                    let _ = send_media_failed(&cmd_tx, ingress_id, attachment_index, attempt, err)
                        .await;
                    break;
                }
                let delay_ms = retry_delay_ms(attempt);
                debug_log!(
                    "media fetch retry scheduled: ingress_id={} idx={} attempt={} delay_ms={}",
                    ingress_id.0,
                    attachment_index,
                    attempt,
                    delay_ms
                );
                sleep(Duration::from_millis(delay_ms as u64)).await;
                attempt = attempt.saturating_add(1);
                local_attempt = local_attempt.saturating_add(1);
            }
        }
    }
}

struct FetchedBytes {
    bytes: Vec<u8>,
    content_type: Option<String>,
    source_hint: Option<String>,
}

async fn fetch_bytes(client: &Client, source: &str) -> Result<FetchedBytes, String> {
    if let Some((mime, bytes)) = parse_data_url(source)? {
        return Ok(FetchedBytes {
            bytes,
            content_type: mime,
            source_hint: None,
        });
    }
    if let Some(bytes) = parse_base64_url(source)? {
        return Ok(FetchedBytes {
            bytes,
            content_type: None,
            source_hint: None,
        });
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        let response = client
            .get(source)
            .send()
            .await
            .map_err(|err| format!("download failed: {}", err))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("download http status {}", status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let bytes = response
            .bytes()
            .await
            .map_err(|err| format!("download read failed: {}", err))?
            .to_vec();
        return Ok(FetchedBytes {
            bytes,
            content_type,
            source_hint: Some(source.to_string()),
        });
    }
    if let Some(path) = source.strip_prefix("file://") {
        let bytes = fs::read(path).map_err(|err| format!("read file failed: {}", err))?;
        return Ok(FetchedBytes {
            bytes,
            content_type: None,
            source_hint: Some(path.to_string()),
        });
    }
    if Path::new(source).exists() {
        let bytes = fs::read(source).map_err(|err| format!("read file failed: {}", err))?;
        return Ok(FetchedBytes {
            bytes,
            content_type: None,
            source_hint: Some(source.to_string()),
        });
    }
    Err("unsupported media source".to_string())
}

fn parse_data_url(source: &str) -> Result<Option<(Option<String>, Vec<u8>)>, String> {
    let payload = match source.strip_prefix("data:") {
        Some(payload) => payload,
        None => return Ok(None),
    };
    let (meta, data) = payload
        .split_once(',')
        .ok_or_else(|| "invalid data url".to_string())?;
    let mime = meta.split(';').next().map(|s| s.to_string());
    let bytes = if meta.contains(";base64") {
        STANDARD
            .decode(data)
            .map_err(|err| format!("invalid base64: {}", err))?
    } else {
        data.as_bytes().to_vec()
    };
    Ok(Some((mime, bytes)))
}

fn parse_base64_url(source: &str) -> Result<Option<Vec<u8>>, String> {
    let payload = match source.strip_prefix("base64://") {
        Some(payload) => payload,
        None => return Ok(None),
    };
    let bytes = STANDARD
        .decode(payload)
        .map_err(|err| format!("invalid base64: {}", err))?;
    Ok(Some(bytes))
}

fn choose_extension(
    attachment: &IngressAttachment,
    source: &str,
    fetched: &FetchedBytes,
) -> String {
    if let Some(content_type) = fetched.content_type.as_deref() {
        if let Some(ext) = ext_from_content_type(content_type) {
            return ext.to_string();
        }
    }
    if let Some(ext) = ext_from_source(fetched.source_hint.as_deref().unwrap_or(source)) {
        return ext;
    }
    if let Some(name) = attachment.name.as_deref() {
        if let Some(ext) = ext_from_source(name) {
            return ext;
        }
    }
    match attachment.kind {
        MediaKind::Image => "jpg".to_string(),
        MediaKind::Video => "mp4".to_string(),
        MediaKind::Audio => "mp3".to_string(),
        MediaKind::File | MediaKind::Other => "bin".to_string(),
        MediaKind::Sticker => "jpg".to_string(),
    }
}

fn ext_from_content_type(content_type: &str) -> Option<&'static str> {
    let base = content_type.split(';').next()?.trim().to_ascii_lowercase();
    match base.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/svg+xml" => Some("svg"),
        "video/mp4" => Some("mp4"),
        "audio/mpeg" => Some("mp3"),
        "audio/ogg" => Some("ogg"),
        "audio/wav" => Some("wav"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

fn ext_from_source(source: &str) -> Option<String> {
    if source.starts_with("data:") {
        if let Ok(Some((mime, _))) = parse_data_url(source) {
            if let Some(mime) = mime {
                if let Some(ext) = ext_from_content_type(&mime) {
                    return Some(ext.to_string());
                }
            }
        }
        return None;
    }
    if source.starts_with("base64://") {
        return None;
    }
    let trimmed = source
        .split('#')
        .next()
        .unwrap_or(source)
        .split('?')
        .next()
        .unwrap_or(source);
    let tail = trimmed.rsplit('/').next().unwrap_or(trimmed);
    Path::new(tail)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

fn kind_dir(kind: &MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::File => "file",
        MediaKind::Audio => "audio",
        MediaKind::Other => "other",
        MediaKind::Sticker => "image",
    }
}

fn persist_blob(
    root: &Path,
    kind_dir: &str,
    ext: &str,
    blob_id: BlobId,
    bytes: &[u8],
) -> Result<(String, u64), String> {
    let dir = root.join(kind_dir);
    fs::create_dir_all(&dir).map_err(|err| format!("create blob dir failed: {}", err))?;
    let filename = format!("{}.{}", id128_hex(blob_id.0), ext);
    let path = dir.join(filename);
    fs::write(&path, bytes).map_err(|err| format!("write blob failed: {}", err))?;
    let size_bytes = bytes.len() as u64;
    Ok((path.to_string_lossy().to_string(), size_bytes))
}

fn derive_attachment_blob_id(ingress_id: IngressId, attachment_index: usize) -> BlobId {
    let idx = (attachment_index as u64).to_be_bytes();
    derive_blob_id(&[&ingress_id.to_be_bytes(), &idx])
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

async fn send_event(cmd_tx: &mpsc::Sender<Command>, event: Event) -> Result<(), String> {
    cmd_tx
        .send(Command::DriverEvent(event))
        .await
        .map_err(|_| "driver event send failed".to_string())
}

async fn send_media_failed(
    cmd_tx: &mpsc::Sender<Command>,
    ingress_id: IngressId,
    attachment_index: usize,
    attempt: u32,
    error: String,
) -> Result<(), String> {
    let retry_at_ms = now_ms().saturating_add(retry_delay_ms(attempt));
    send_event(
        cmd_tx,
        Event::Media(MediaEvent::MediaFetchFailed {
            ingress_id,
            attachment_index,
            attempt,
            retry_at_ms,
            error,
        }),
    )
    .await
}

fn retry_delay_ms(attempt: u32) -> TimestampMs {
    let base = 1_000i64;
    let max = 30_000i64;
    let shift = attempt.saturating_sub(1).min(10);
    let delay = base.saturating_mul(1_i64 << shift);
    delay.min(max)
}

fn now_ms() -> TimestampMs {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as TimestampMs
}
