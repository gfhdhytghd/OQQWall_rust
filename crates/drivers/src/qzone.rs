use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Cursor;
#[cfg(debug_assertions)]
use std::io::{Read, Write};
#[cfg(debug_assertions)]
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::codecs::gif::{GifDecoder, GifEncoder, Repeat};
use image::codecs::jpeg::JpegEncoder;
use image::{
    self as image_rs, AnimationDecoder, DynamicImage, Frame as ImageFrame, GenericImageView,
    ImageFormat,
};
use oqqwall_rust_core::draft::{Draft, DraftBlock, IngressMessage, MediaKind, MediaReference};
use oqqwall_rust_core::event::{
    BlobEvent, DraftEvent, Event, IngressEvent, MediaEvent, RenderEvent, ReviewEvent,
    ScheduleEvent, SendEvent, SendPriority,
};
use oqqwall_rust_core::ids::{
    BlobId, ExternalCode, IngressId, PostId, ReviewCode, ReviewId, TimestampMs,
};
use oqqwall_rust_core::{Command, StateView, build_draft_from_messages, derive_blob_id};
use oqqwall_rust_infra::{LocalJournal, SnapshotStore};
use reqwest::Client;
#[cfg(debug_assertions)]
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::blob_cache;
use crate::napcat::{NapCatConfig, napcat_ws_request};

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

const EMOTION_PUBLISH_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/taotao.qzone.qq.com/cgi-bin/emotion_cgi_publish_v6";
const UPLOAD_IMAGE_URL: &str = "https://up.qzone.qq.com/cgi-bin/upload/cgi_upload_image";
const CHROME_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/116.1.3702.40 Safari/537.36 QBWebViewUA/2 QBWebViewType/1 WKType/1";
const MAX_UPLOAD_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const ERROR_BODY_PREVIEW_CHARS: usize = 512;
const PRESERVE_MAX_LONG_EDGE_1080P: u32 = 1920;
const PRESERVE_MAX_SHORT_EDGE_1080P: u32 = 1080;
const JPEG_MIN_DIMENSION: u32 = 512;
const JPEG_QUALITY_STEPS: [u8; 6] = [90, 82, 74, 66, 58, 50];
const STACKING_HOLD_MS: i64 = 365 * 24 * 60 * 60 * 1000;
#[cfg(debug_assertions)]
const EMUQZONE_PORT: u16 = 18080;
#[cfg(debug_assertions)]
const EMUQZONE_MAX_POSTS: usize = 50;

#[derive(Debug, Clone)]
pub struct QzoneRuntimeConfig {
    pub napcat_by_group: HashMap<String, NapCatConfig>,
    pub default_napcat: Option<NapCatConfig>,
    pub accounts_by_group: HashMap<String, Vec<String>>,
    pub at_unprived_sender: bool,
    pub max_queue_by_group: HashMap<String, usize>,
    pub max_images_per_post_by_group: HashMap<String, usize>,
    pub individual_images_by_group: HashMap<String, bool>,
    pub default_max_queue: usize,
    pub default_max_images_per_post: usize,
    pub default_individual_images: bool,
    #[cfg(debug_assertions)]
    pub use_virt_qzone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QzoneErrorKind {
    Network,
    RiskControl,
    Account,
    Unknown,
}

#[derive(Debug, Clone)]
struct QzoneError {
    kind: QzoneErrorKind,
    message: String,
}

impl QzoneError {
    fn new(kind: QzoneErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn network(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::Network, message)
    }

    fn risk(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::RiskControl, message)
    }

    fn account(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::Account, message)
    }

    fn unknown(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::Unknown, message)
    }

    fn with_context(self, ctx: &str) -> Self {
        Self {
            kind: self.kind,
            message: format!("{}: {}", ctx, self.message),
        }
    }
}

#[derive(Debug, Clone)]
struct IngressAuthor {
    user_id: String,
    sender_name: Option<String>,
}

#[derive(Debug, Clone)]
struct SendPlanMeta {
    group_id: String,
    not_before_ms: i64,
    priority: SendPriority,
    seq: u64,
}

#[derive(Debug, Clone)]
struct PreparedUploadImage {
    bytes: Vec<u8>,
    strategy: &'static str,
}

#[derive(Default)]
struct QzoneState {
    drafts: HashMap<PostId, Draft>,
    attempts: HashMap<PostId, u32>,
    cookie_cache: Option<CookieCache>,
    blob_paths: HashMap<BlobId, String>,
    ingress_messages: HashMap<IngressId, IngressMessage>,
    ingress_authors: HashMap<IngressId, IngressAuthor>,
    post_ingress: HashMap<PostId, Vec<IngressId>>,
    post_anonymous: HashMap<PostId, bool>,
    send_plans: HashMap<PostId, SendPlanMeta>,
    review_posts: HashMap<ReviewId, PostId>,
    review_codes: HashMap<PostId, ReviewCode>,
    external_codes: HashMap<PostId, ExternalCode>,
    render_blobs: HashMap<PostId, RenderBlobs>,
}

impl QzoneState {
    fn register_media_reference(&mut self, ingress_id: IngressId, idx: usize, blob_id: BlobId) {
        if let Some(message) = self.ingress_messages.get_mut(&ingress_id) {
            if let Some(attachment) = message.attachments.get_mut(idx) {
                attachment.reference = MediaReference::Blob { blob_id };
            }
        }
    }
}

fn load_state_view_cached() -> StateView {
    static CACHE: OnceLock<StateView> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let data_dir = env::var("OQQWALL_DATA_DIR").unwrap_or_else(|_| "data".to_string());
            let journal = match LocalJournal::open(&data_dir) {
                Ok(journal) => journal,
                Err(_err) => {
                    debug_log!("qzone preload skipped: journal open failed: {}", _err);
                    return StateView::default();
                }
            };
            let snapshot = match SnapshotStore::open(&data_dir) {
                Ok(snapshot) => snapshot,
                Err(_err) => {
                    debug_log!("qzone preload skipped: snapshot open failed: {}", _err);
                    return StateView::default();
                }
            };

            let mut state = StateView::default();
            let mut cursor = None;
            match snapshot.load() {
                Ok(Some(loaded)) => {
                    state = loaded.state;
                    cursor = loaded.journal_cursor;
                }
                Ok(None) => {}
                Err(_err) => {
                    debug_log!("qzone preload: snapshot load failed: {}", _err);
                }
            }

            if let Err(_err) = journal.replay(cursor, |env| {
                state = state.reduce(env);
            }) {
                debug_log!("qzone preload: journal replay failed: {}", _err);
            }

            state
        })
        .clone()
}

fn build_state_from_view(view: &StateView) -> QzoneState {
    let mut state = QzoneState::default();
    state.drafts = view.drafts.clone();
    state.ingress_messages = view.ingress_messages.clone();
    state.post_ingress = view.post_ingress.clone();
    for (post_id, post) in &view.posts {
        state.post_anonymous.insert(*post_id, post.is_anonymous);
    }

    for (ingress_id, meta) in &view.ingress_meta {
        state.ingress_authors.insert(
            *ingress_id,
            IngressAuthor {
                user_id: meta.user_id.clone(),
                sender_name: meta.sender_name.clone(),
            },
        );
    }

    for (review_id, review) in &view.reviews {
        state
            .review_codes
            .insert(review.post_id, review.review_code);
        state.review_posts.insert(*review_id, review.post_id);
    }
    state.external_codes = view.external_code_by_post.clone();

    for (post_id, plan) in &view.send_plans {
        state.send_plans.insert(
            *post_id,
            SendPlanMeta {
                group_id: plan.group_id.clone(),
                not_before_ms: plan.not_before_ms,
                priority: plan.priority,
                seq: plan.seq,
            },
        );
    }

    for (post_id, render) in &view.render {
        if render.png_blob.is_some() {
            state.render_blobs.insert(
                *post_id,
                RenderBlobs {
                    png: render.png_blob,
                },
            );
        }
    }

    for (blob_id, meta) in &view.blobs {
        if let Some(path) = meta.persisted_path.as_ref() {
            state.blob_paths.insert(*blob_id, path.clone());
        }
    }

    state
}

struct CookieCache {
    account_id: String,
    cookies: HashMap<String, String>,
    fetched_at_ms: TimestampMs,
}

#[derive(Debug, Default, Clone, Copy)]
struct RenderBlobs {
    png: Option<BlobId>,
}

#[cfg(debug_assertions)]
#[derive(Default)]
struct EmuQzoneState {
    posts: Vec<EmuQzonePost>,
    next_id: u64,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Serialize)]
struct EmuQzonePost {
    id: u64,
    timestamp_ms: TimestampMs,
    text: String,
    images: Vec<String>,
    image_count: usize,
    status: String,
}

pub fn spawn_qzone_sender(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    runtime: QzoneRuntimeConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug_log!(
            "qzone sender start: groups={} default_napcat={}",
            runtime.napcat_by_group.len(),
            runtime.default_napcat.is_some()
        );
        #[cfg(debug_assertions)]
        debug_log!(
            "qzone sender mode: use_virt_qzone={}",
            runtime.use_virt_qzone
        );
        #[cfg(debug_assertions)]
        let emu_state = if runtime.use_virt_qzone {
            let state = Arc::new(std::sync::Mutex::new(EmuQzoneState::default()));
            spawn_emuqzone_server(state.clone());
            Some(state)
        } else {
            None
        };
        let state_view = load_state_view_cached();
        let state = Arc::new(Mutex::new(build_state_from_view(&state_view)));
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
                    sender_name,
                    message,
                    ..
                })
                | Event::Ingress(IngressEvent::MessageSynced {
                    ingress_id,
                    user_id,
                    sender_name,
                    message,
                    ..
                }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.insert(ingress_id, message);
                    guard.ingress_authors.insert(
                        ingress_id,
                        IngressAuthor {
                            user_id,
                            sender_name,
                        },
                    );
                }
                Event::Ingress(IngressEvent::MessageIgnored { ingress_id, .. }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.remove(&ingress_id);
                    guard.ingress_authors.remove(&ingress_id);
                }
                Event::Ingress(IngressEvent::MessageRecalled { ingress_id, .. }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.remove(&ingress_id);
                    guard.ingress_authors.remove(&ingress_id);
                    for ingress_ids in guard.post_ingress.values_mut() {
                        ingress_ids.retain(|id| *id != ingress_id);
                    }
                }
                Event::Media(MediaEvent::MediaFetchSucceeded {
                    ingress_id,
                    attachment_index,
                    blob_id,
                }) => {
                    let mut guard = state.lock().await;
                    guard.register_media_reference(ingress_id, attachment_index, blob_id);
                }
                Event::Draft(DraftEvent::PostDraftCreated {
                    post_id,
                    draft,
                    ingress_ids,
                    is_anonymous,
                    ..
                }) => {
                    debug_log!(
                        "qzone draft cached: post_id={} blocks={}",
                        post_id.0,
                        draft.blocks.len()
                    );
                    let mut guard = state.lock().await;
                    guard.drafts.insert(post_id, draft);
                    guard.post_ingress.insert(post_id, ingress_ids);
                    guard.post_anonymous.insert(post_id, is_anonymous);
                }
                Event::Review(ReviewEvent::ReviewItemCreated {
                    review_id,
                    post_id,
                    review_code,
                }) => {
                    let mut guard = state.lock().await;
                    guard.review_codes.insert(post_id, review_code);
                    guard.review_posts.insert(review_id, post_id);
                }
                Event::Review(ReviewEvent::ReviewInfoSynced {
                    review_id,
                    post_id,
                    review_code,
                }) => {
                    let mut guard = state.lock().await;
                    guard.review_codes.insert(post_id, review_code);
                    guard.review_posts.insert(review_id, post_id);
                }
                Event::Review(ReviewEvent::ReviewAnonToggled { review_id }) => {
                    let mut guard = state.lock().await;
                    if let Some(post_id) = guard.review_posts.get(&review_id).copied() {
                        let entry = guard.post_anonymous.entry(post_id).or_insert(false);
                        *entry = !*entry;
                    }
                }
                Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
                    post_id,
                    external_code,
                    ..
                }) => {
                    let mut guard = state.lock().await;
                    guard.external_codes.insert(post_id, external_code);
                }
                Event::Review(ReviewEvent::ReviewExternalCodeCleared { post_id }) => {
                    let mut guard = state.lock().await;
                    guard.external_codes.remove(&post_id);
                }
                Event::Schedule(ScheduleEvent::SendPlanCreated {
                    post_id,
                    group_id,
                    not_before_ms,
                    priority,
                    seq,
                    ..
                })
                | Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                    post_id,
                    group_id,
                    not_before_ms,
                    priority,
                    seq,
                    ..
                }) => {
                    let mut guard = state.lock().await;
                    guard.send_plans.insert(
                        post_id,
                        SendPlanMeta {
                            group_id,
                            not_before_ms,
                            priority,
                            seq,
                        },
                    );
                }
                Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }) => {
                    let mut guard = state.lock().await;
                    guard.send_plans.remove(&post_id);
                }
                Event::Blob(BlobEvent::BlobPersisted { blob_id, path }) => {
                    let mut guard = state.lock().await;
                    guard.blob_paths.insert(blob_id, path);
                }
                Event::Blob(BlobEvent::BlobReleased { blob_id })
                | Event::Blob(BlobEvent::BlobGcRequested { blob_id }) => {
                    let mut guard = state.lock().await;
                    guard.blob_paths.remove(&blob_id);
                }
                Event::Render(RenderEvent::RenderRequested { post_id, .. }) => {
                    let mut guard = state.lock().await;
                    let entry = guard.render_blobs.entry(post_id).or_default();
                    entry.png = None;
                }
                Event::Render(RenderEvent::PngReady { post_id, blob_id }) => {
                    let mut guard = state.lock().await;
                    let entry = guard.render_blobs.entry(post_id).or_default();
                    entry.png = Some(blob_id);
                }
                Event::Render(RenderEvent::RenderFailed { post_id, .. }) => {
                    let mut guard = state.lock().await;
                    let entry = guard.render_blobs.entry(post_id).or_default();
                    entry.png = None;
                }
                Event::Send(SendEvent::SendSucceeded { post_id, .. })
                | Event::Send(SendEvent::SendGaveUp { post_id, .. }) => {
                    let blob_ids = {
                        let guard = state.lock().await;
                        collect_post_blob_ids(&guard, post_id)
                    };
                    if !blob_ids.is_empty() {
                        blob_cache::release_many(blob_ids);
                    }
                }
                Event::Send(SendEvent::SendStarted {
                    post_id,
                    group_id,
                    account_id,
                    started_at_ms,
                    ..
                }) => {
                    debug_log!(
                        "qzone send started: post_id={} group_id={} account_id={} started_at_ms={}",
                        post_id.0,
                        group_id,
                        account_id,
                        started_at_ms
                    );
                    let max_queue = runtime
                        .max_queue_by_group
                        .get(&group_id)
                        .copied()
                        .unwrap_or(runtime.default_max_queue);
                    let max_images_per_post = runtime
                        .max_images_per_post_by_group
                        .get(&group_id)
                        .copied()
                        .unwrap_or(runtime.default_max_images_per_post);
                    let include_original_images = runtime
                        .individual_images_by_group
                        .get(&group_id)
                        .copied()
                        .unwrap_or(runtime.default_individual_images);

                    let batch_result = {
                        let mut guard = state.lock().await;
                        let leader_meta = guard.send_plans.remove(&post_id);
                        let leader_priority = leader_meta
                            .map(|meta| meta.priority)
                            .unwrap_or(SendPriority::Normal);
                        let (batch_posts, requeue_posts) = collect_batch_post_ids(
                            &guard,
                            &group_id,
                            post_id,
                            leader_priority,
                            started_at_ms,
                            max_queue,
                            max_images_per_post,
                            include_original_images,
                        );
                        let publish_text = build_publish_text_for_batch(
                            &batch_posts,
                            &guard.external_codes,
                            &guard.review_codes,
                            &guard.post_ingress,
                            &guard.ingress_authors,
                            &guard.post_anonymous,
                            runtime.at_unprived_sender,
                        );
                        match collect_post_assets(&guard, &batch_posts, include_original_images) {
                            Ok(assets) => Ok((
                                batch_posts,
                                requeue_posts,
                                assets,
                                guard.blob_paths.clone(),
                                publish_text,
                            )),
                            Err(err) => Err(err),
                        }
                    };

                    let (batch_posts, requeue_posts, assets, blob_paths, publish_text) =
                        match batch_result {
                            Ok(data) => data,
                            Err(err) => {
                                debug_log!(
                                    "qzone send failed: kind={:?} message={}",
                                    err.kind,
                                    err.message
                                );
                                let retry_at =
                                    started_at_ms.saturating_add(retry_delay_ms(err.kind, 1));
                                let event = SendEvent::SendFailed {
                                    post_id,
                                    account_id,
                                    attempt: 1,
                                    retry_at_ms: retry_at,
                                    error: format!("[{:?}] {}", err.kind, err.message),
                                };
                                let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                                continue;
                            }
                        };
                    let hold_until_ms = started_at_ms.saturating_add(STACKING_HOLD_MS);
                    for requeue in requeue_posts {
                        let event = ScheduleEvent::SendPlanRescheduled {
                            post_id: requeue.post_id,
                            group_id: group_id.clone(),
                            not_before_ms: hold_until_ms,
                            priority: requeue.priority,
                            seq: requeue.seq,
                        };
                        let _ = cmd_tx
                            .send(Command::DriverEvent(Event::Schedule(event)))
                            .await;
                    }
                    let attempt = {
                        let mut guard = state.lock().await;
                        let entry = guard.attempts.entry(post_id).or_insert(0);
                        *entry += 1;
                        *entry
                    };
                    let target_accounts =
                        select_send_accounts(runtime.accounts_by_group.get(&group_id), &account_id);
                    let mut final_account_id = account_id.clone();
                    let mut first_tid: Option<String> = None;
                    let mut failed: Option<(String, QzoneError)> = None;
                    for target_account in target_accounts {
                        match publish_batch_for_account(
                            &state,
                            &runtime,
                            &group_id,
                            &target_account,
                            max_images_per_post,
                            attempt,
                            &assets,
                            &blob_paths,
                            &publish_text,
                            #[cfg(debug_assertions)]
                            emu_state.as_ref(),
                        )
                        .await
                        {
                            Ok(remote_id) => {
                                final_account_id = target_account.clone();
                                if first_tid.is_none() {
                                    first_tid = remote_id.clone();
                                }
                                let account_event = SendEvent::SendAccountSucceeded {
                                    post_id,
                                    account_id: target_account,
                                    finished_at_ms: started_at_ms,
                                    remote_id,
                                };
                                let _ = cmd_tx
                                    .send(Command::DriverEvent(Event::Send(account_event)))
                                    .await;
                            }
                            Err(err) => {
                                let failed_account_id = target_account.clone();
                                let account_event = SendEvent::SendAccountFailed {
                                    post_id,
                                    account_id: target_account,
                                    attempt,
                                    error: format!("[{:?}] {}", err.kind, err.message),
                                };
                                let _ = cmd_tx
                                    .send(Command::DriverEvent(Event::Send(account_event)))
                                    .await;
                                failed = Some((failed_account_id, err));
                                break;
                            }
                        }
                    }
                    if let Some((failed_account_id, err)) = failed {
                        let retry_at =
                            started_at_ms.saturating_add(retry_delay_ms(err.kind, attempt));
                        let event = SendEvent::SendFailed {
                            post_id,
                            account_id: failed_account_id,
                            attempt,
                            retry_at_ms: retry_at,
                            error: format!("[{:?}] {}", err.kind, err.message),
                        };
                        let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                        continue;
                    }

                    debug_log!("qzone publish success: post_id={} accounts=all", post_id.0);
                    let mut guard = state.lock().await;
                    guard.attempts.remove(&post_id);
                    drop(guard);
                    for other_post_id in batch_posts.iter().copied().filter(|id| *id != post_id) {
                        let event = ScheduleEvent::SendPlanCanceled {
                            post_id: other_post_id,
                        };
                        let _ = cmd_tx
                            .send(Command::DriverEvent(Event::Schedule(event)))
                            .await;
                    }
                    for batch_post_id in batch_posts {
                        let remote_id = if batch_post_id == post_id {
                            first_tid.clone()
                        } else {
                            None
                        };
                        let event = SendEvent::SendSucceeded {
                            post_id: batch_post_id,
                            account_id: final_account_id.clone(),
                            finished_at_ms: started_at_ms,
                            remote_id,
                        };
                        let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                    }
                }
                _ => {}
            }
        }
    })
}

fn select_send_accounts(accounts: Option<&Vec<String>>, fallback_account: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(accounts) = accounts {
        for account_id in accounts {
            let trimmed = account_id.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                out.push(trimmed.to_string());
            }
        }
    }
    if out.is_empty() {
        out.push(fallback_account.to_string());
    }
    out
}

async fn publish_batch_for_account(
    state: &Arc<Mutex<QzoneState>>,
    runtime: &QzoneRuntimeConfig,
    group_id: &str,
    account_id: &str,
    max_images_per_post: usize,
    _attempt: u32,
    assets: &[PostAssets],
    blob_paths: &HashMap<BlobId, String>,
    publish_text: &str,
    #[cfg(debug_assertions)] emu_state: Option<&Arc<std::sync::Mutex<EmuQzoneState>>>,
) -> Result<Option<String>, QzoneError> {
    #[cfg(debug_assertions)]
    if runtime.use_virt_qzone {
        let Some(emu_state) = emu_state else {
            return Err(QzoneError::unknown("missing emuqzone state"));
        };
        let images = collect_emuqzone_batch_images(assets, blob_paths)?;
        if images.is_empty() {
            return Err(QzoneError::unknown("empty images"));
        }
        let chunk_size = if max_images_per_post > 0 {
            max_images_per_post
        } else {
            images.len().max(1)
        };
        debug_log!(
            "emuqzone publish attempt: account={} attempt={} images={} content_len={}",
            account_id,
            _attempt,
            images.len(),
            publish_text.len()
        );
        for chunk in images.chunks(chunk_size) {
            append_emuqzone_post(emu_state, publish_text.to_string(), chunk.to_vec());
        }
        return Ok(None);
    }

    let napcat = runtime
        .napcat_by_group
        .get(group_id)
        .or_else(|| runtime.default_napcat.as_ref());
    if napcat.is_none() {
        return Err(QzoneError::unknown(format!(
            "missing napcat config for group_id={}",
            group_id
        )));
    }

    let cookies = match get_cookies(state, account_id).await {
        Ok(cookies) => cookies,
        Err(err) => {
            refresh_cookie_cache(state, account_id).await;
            return Err(err);
        }
    };
    let client = match QzoneClient::from_cookie_map(cookies) {
        Ok(client) => client,
        Err(err) => {
            refresh_cookie_cache(state, account_id).await;
            return Err(err);
        }
    };
    let images = match collect_batch_images(assets, blob_paths).await {
        Ok(images) => images,
        Err(err) => {
            refresh_cookie_cache(state, account_id).await;
            return Err(err);
        }
    };
    if images.is_empty() {
        return Err(QzoneError::unknown("empty images"));
    }
    debug_log!(
        "qzone publish attempt: account={} attempt={} images={} content_len={}",
        account_id,
        _attempt,
        images.len(),
        publish_text.len()
    );
    let chunk_size = if max_images_per_post > 0 {
        max_images_per_post
    } else {
        images.len().max(1)
    };
    let mut first_tid: Option<String> = None;
    for chunk in images.chunks(chunk_size) {
        match client.publish_emotion(publish_text, chunk).await {
            Ok(tid) => {
                if first_tid.is_none() {
                    first_tid = Some(tid);
                }
            }
            Err(err) => {
                refresh_cookie_cache(state, account_id).await;
                return Err(err);
            }
        }
    }
    Ok(first_tid)
}

#[derive(Clone)]
struct QzoneClient {
    cookies: HashMap<String, String>,
    gtk: String,
    uin: u64,
    client: Client,
}

impl QzoneClient {
    fn from_cookie_map(cookies: HashMap<String, String>) -> Result<Self, QzoneError> {
        debug_log!("qzone client init: cookie_count={}", cookies.len());
        let skey = cookies
            .get("p_skey")
            .or_else(|| cookies.get("skey"))
            .ok_or_else(|| QzoneError::account("missing p_skey/skey"))?
            .clone();
        let gtk = generate_gtk(&skey);
        let uin = cookies
            .get("uin")
            .and_then(|value| value.trim().trim_start_matches('o').parse::<u64>().ok())
            .unwrap_or(0);
        debug_log!("qzone client init: uin={}", uin);
        let client = Client::builder()
            .user_agent(CHROME_USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|err| QzoneError::network(format!("reqwest init failed: {}", err)))?;
        Ok(Self {
            cookies,
            gtk,
            uin,
            client,
        })
    }

    async fn publish_emotion(
        &self,
        content: &str,
        images: &[Vec<u8>],
    ) -> Result<String, QzoneError> {
        debug_log!(
            "qzone publish request: content_len={} images={}",
            content.len(),
            images.len()
        );
        let cookie_header = build_cookie_header(&self.cookies);
        let mut form: HashMap<&str, String> = HashMap::new();
        form.insert("syn_tweet_verson", "1".to_string());
        form.insert("paramstr", "1".to_string());
        form.insert("who", "1".to_string());
        form.insert("con", content.to_string());
        form.insert("feedversion", "1".to_string());
        form.insert("ver", "1".to_string());
        form.insert("ugc_right", "1".to_string());
        form.insert("to_sign", "0".to_string());
        form.insert("hostuin", self.uin.to_string());
        form.insert("code_version", "1".to_string());
        form.insert("format", "fs".to_string());
        form.insert("pic_template", "".to_string());
        form.insert("special_url", "".to_string());
        form.insert(
            "qzreferrer",
            format!("https://user.qzone.qq.com/{}", self.uin),
        );

        if !images.is_empty() {
            let mut pic_bos = Vec::new();
            let mut richvals = Vec::new();
            form.insert("subrichtype", "1".to_string());
            for image in images {
                let upload = self.upload_image(image).await?;
                let (picbo, richval) = get_picbo_and_richval(&upload)?;
                pic_bos.push(picbo);
                richvals.push(richval);
            }
            form.insert("pic_bo", pic_bos.join("\t"));
            form.insert("richtype", "1".to_string());
            form.insert("richval", richvals.join("\t"));
        }

        debug_log!("qzone publish g_tk={}", self.gtk);
        if let Some(_format) = form.get("format") {
            debug_log!("qzone publish form format={}", _format);
        }
        if let Some(_pic_bo) = form.get("pic_bo") {
            debug_log!("qzone publish form pic_bo={}", _pic_bo);
        }
        if let Some(_richval) = form.get("richval") {
            debug_log!(
                "qzone publish form richval={}",
                _richval.replace('\t', "\\t")
            );
        }

        let res = self
            .client
            .post(EMOTION_PUBLISH_URL)
            .query(&[("g_tk", &self.gtk)])
            .header("user-agent", CHROME_USER_AGENT)
            .header("accept", "*/*")
            .header(
                "content-type",
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .header("referer", format!("https://user.qzone.qq.com/{}", self.uin))
            .header("origin", "https://user.qzone.qq.com")
            .header("cookie", cookie_header)
            .form(&form)
            .send()
            .await
            .map_err(|err| classify_reqwest_error("publish request", err))?;

        let status = res.status();
        let headers = res.headers().clone();
        let body = match res.text().await {
            Ok(text) => text,
            Err(err) => {
                if !status.is_success() {
                    let fallback = format!("<read body failed: {}>", err);
                    debug_log_http_failure("qzone publish", status, &headers, &fallback);
                    return Err(classify_http_status_with_body(
                        "publish http status",
                        status.as_u16(),
                        &fallback,
                    ));
                }
                return Err(classify_reqwest_error("publish read body", err));
            }
        };
        if !status.is_success() {
            debug_log_http_failure("qzone publish", status, &headers, &body);
            return Err(classify_http_status_with_body(
                "publish http status",
                status.as_u16(),
                &body,
            ));
        }
        let json = match parse_proxy_callback_json(&body) {
            Some(json) => json,
            None => {
                let trimmed = body.trim();
                if trimmed.is_empty() {
                    debug_log!("qzone publish response empty; treating as success");
                    return Ok("unknown".to_string());
                }
                let _sample: String = trimmed.chars().take(200).collect();
                debug_log!(
                    "qzone publish response unparseable: len={} head={}",
                    trimmed.len(),
                    _sample
                );
                return Err(QzoneError::unknown("invalid publish response body"));
            }
        };
        if let Ok(_pretty) = serde_json::to_string_pretty(&json) {
            debug_log!("qzone publish response json:\n{}", _pretty);
        }
        if let Some(err) = classify_response_error(&json) {
            return Err(err.with_context("publish response"));
        }
        let tid = json
            .get("tid")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(tid)
    }

    async fn upload_image(&self, image: &[u8]) -> Result<Value, QzoneError> {
        let prepared = prepare_upload_image(image)?;
        debug_log!(
            "qzone upload image: original_size_bytes={} prepared_size_bytes={} strategy={}",
            image.len(),
            prepared.bytes.len(),
            prepared.strategy
        );
        let cookie_header = build_cookie_header(&self.cookies);
        let skey = self
            .cookies
            .get("skey")
            .or_else(|| self.cookies.get("p_skey"))
            .ok_or_else(|| QzoneError::account("missing skey"))?
            .clone();
        let p_skey = self
            .cookies
            .get("p_skey")
            .ok_or_else(|| QzoneError::account("missing p_skey"))?
            .clone();
        let picfile = STANDARD.encode(prepared.bytes.as_slice());
        let mut form: HashMap<&str, String> = HashMap::new();
        form.insert("filename", "filename".to_string());
        form.insert("zzpanelkey", "".to_string());
        form.insert("uploadtype", "1".to_string());
        form.insert("albumtype", "7".to_string());
        form.insert("exttype", "0".to_string());
        form.insert("skey", skey);
        form.insert("zzpaneluin", self.uin.to_string());
        form.insert("p_uin", self.uin.to_string());
        form.insert("uin", self.uin.to_string());
        form.insert("p_skey", p_skey);
        form.insert("output_type", "jsonhtml".to_string());
        form.insert("qzonetoken", "".to_string());
        form.insert("refer", "shuoshuo".to_string());
        form.insert("charset", "utf-8".to_string());
        form.insert("output_charset", "utf-8".to_string());
        form.insert("upload_hd", "1".to_string());
        form.insert("hd_width", "2048".to_string());
        form.insert("hd_height", "10000".to_string());
        form.insert("hd_quality", "96".to_string());
        form.insert(
            "backUrls",
            "http://upbak.photo.qzone.qq.com/cgi-bin/upload/cgi_upload_image,http://119.147.64.75/cgi-bin/upload/cgi_upload_image".to_string(),
        );
        form.insert("url", format!("{}?g_tk={}", UPLOAD_IMAGE_URL, self.gtk));
        form.insert("base64", "1".to_string());
        form.insert("picfile", picfile);

        let res = self
            .client
            .post(UPLOAD_IMAGE_URL)
            .query(&[("g_tk", &self.gtk)])
            .header("user-agent", CHROME_USER_AGENT)
            .header("accept", "*/*")
            .header(
                "content-type",
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .header("referer", format!("https://user.qzone.qq.com/{}", self.uin))
            .header("origin", "https://user.qzone.qq.com")
            .header("cookie", cookie_header)
            .form(&form)
            .send()
            .await
            .map_err(|err| classify_reqwest_error("upload image request", err))?;

        let status = res.status();
        let headers = res.headers().clone();
        let body = match res.text().await {
            Ok(text) => text,
            Err(err) => {
                if !status.is_success() {
                    let fallback = format!("<read body failed: {}>", err);
                    debug_log_http_failure("qzone upload image", status, &headers, &fallback);
                    return Err(classify_http_status_with_body(
                        "upload image http status",
                        status.as_u16(),
                        &fallback,
                    ));
                }
                return Err(classify_reqwest_error("upload image read body", err));
            }
        };
        if !status.is_success() {
            debug_log_http_failure("qzone upload image", status, &headers, &body);
            return Err(classify_http_status_with_body(
                "upload image http status",
                status.as_u16(),
                &body,
            ));
        }
        let start = body
            .find('{')
            .ok_or_else(|| QzoneError::unknown("invalid upload response"))?;
        let end = body
            .rfind('}')
            .ok_or_else(|| QzoneError::unknown("invalid upload response"))?;
        let json_str = &body[start..=end];
        let json: Value = serde_json::from_str(json_str)
            .map_err(|err| QzoneError::unknown(format!("invalid upload json: {}", err)))?;
        if let Ok(_pretty) = serde_json::to_string_pretty(&json) {
            debug_log!("qzone upload response json:\n{}", _pretty);
        }
        if let Some(err) = classify_response_error(&json) {
            return Err(err.with_context("upload response"));
        }
        Ok(json)
    }
}

fn build_publish_text_for_batch(
    post_ids: &[PostId],
    external_codes: &HashMap<PostId, ExternalCode>,
    review_codes: &HashMap<PostId, ReviewCode>,
    post_ingress: &HashMap<PostId, Vec<IngressId>>,
    ingress_authors: &HashMap<IngressId, IngressAuthor>,
    post_anonymous: &HashMap<PostId, bool>,
    at_unprived_sender: bool,
) -> String {
    let mut codes = Vec::new();
    for post_id in post_ids {
        let code = external_codes
            .get(post_id)
            .map(|value| *value as u128)
            .or_else(|| review_codes.get(post_id).map(|value| *value as u128))
            .unwrap_or(post_id.0);
        codes.push(code);
    }

    let mut text = if let (Some(min), Some(max)) = (codes.iter().min(), codes.iter().max()) {
        if min == max {
            format!("#{}", min)
        } else {
            format!("#{}~{}", min, max)
        }
    } else {
        "#0".to_string()
    };

    if !at_unprived_sender {
        return text;
    }

    let mut mentions = Vec::new();
    let mut seen = HashSet::new();
    for post_id in post_ids {
        if post_anonymous.get(post_id).copied().unwrap_or(false) {
            continue;
        }
        let Some(ingress_ids) = post_ingress.get(post_id) else {
            continue;
        };
        for ingress_id in ingress_ids {
            let Some(author) = ingress_authors.get(ingress_id) else {
                continue;
            };
            if !should_mention(author) {
                continue;
            }
            let user_id = author.user_id.trim();
            if seen.insert(user_id.to_string()) {
                mentions.push(format!("@{{uin:{},nick:,who:1}}", user_id));
            }
        }
    }
    if !mentions.is_empty() {
        text.push(' ');
        text.push_str(&mentions.join(", "));
    }

    text
}

fn should_mention(author: &IngressAuthor) -> bool {
    let user_id = author.user_id.trim();
    if user_id.is_empty() || user_id == "unknown" || user_id == "0" {
        return false;
    }
    // Missing sender_name is treated as anonymous, so skip @.
    let sender_name = author.sender_name.as_deref().unwrap_or("").trim();
    !sender_name.is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequeuePostPlan {
    post_id: PostId,
    priority: SendPriority,
    seq: u64,
}

fn collect_batch_post_ids(
    state: &QzoneState,
    group_id: &str,
    leader: PostId,
    leader_priority: SendPriority,
    started_at_ms: i64,
    max_queue: usize,
    max_images_per_post: usize,
    include_original_images: bool,
) -> (Vec<PostId>, Vec<RequeuePostPlan>) {
    let mut queued = state
        .send_plans
        .iter()
        .filter(|(_, plan)| {
            plan.group_id == group_id
                && plan.priority == leader_priority
                && plan.not_before_ms <= started_at_ms
        })
        .map(|(post_id, plan)| (*post_id, plan.clone()))
        .collect::<Vec<_>>();
    queued.sort_by_key(|(post_id, plan)| (plan.seq, post_id.0));

    let requeue_posts = queued
        .iter()
        .filter_map(|(post_id, plan)| {
            (*post_id != leader).then_some(RequeuePostPlan {
                post_id: *post_id,
                priority: plan.priority,
                seq: plan.seq,
            })
        })
        .collect::<Vec<_>>();

    let max_batch_posts = if max_queue == 0 {
        usize::MAX
    } else {
        max_queue.max(1)
    };
    let mut batch = Vec::with_capacity(max_batch_posts.min(queued.len().saturating_add(1)));
    let mut total_images = 0usize;
    let mut append_post = |post_id: PostId, is_leader: bool| -> bool {
        let image_count = count_post_send_images(state, post_id, include_original_images);
        if !is_leader {
            if batch.len() >= max_batch_posts {
                return false;
            }
            if max_images_per_post > 0
                && total_images.saturating_add(image_count) > max_images_per_post
            {
                return false;
            }
        }
        batch.push(post_id);
        total_images = total_images.saturating_add(image_count);
        true
    };
    let _ = append_post(leader, true);
    if leader_priority != SendPriority::Normal || max_batch_posts <= 1 {
        return (batch, requeue_posts);
    }
    for (post_id, _) in queued {
        if post_id == leader {
            continue;
        }
        if !append_post(post_id, false) {
            break;
        }
    }
    (batch, requeue_posts)
}

fn count_post_send_images(
    state: &QzoneState,
    post_id: PostId,
    include_original_images: bool,
) -> usize {
    let mut total = render_preview_blobs(state, post_id).len();
    if !include_original_images {
        return total;
    }
    if let Some(draft) = resolve_draft_for_send(state, post_id) {
        total = total.saturating_add(
            draft
                .blocks
                .iter()
                .filter(|block| {
                    matches!(
                        block,
                        DraftBlock::Attachment {
                            kind: MediaKind::Image,
                            ..
                        }
                    )
                })
                .count(),
        );
    }
    total
}

struct PostAssets {
    draft: Draft,
    preview_blobs: Vec<BlobId>,
    include_original_images: bool,
}

fn collect_post_assets(
    state: &QzoneState,
    post_ids: &[PostId],
    include_original_images: bool,
) -> Result<Vec<PostAssets>, QzoneError> {
    let mut out = Vec::new();
    for post_id in post_ids {
        let Some(draft) = resolve_draft_for_send(state, *post_id) else {
            return Err(QzoneError::unknown(format!(
                "missing draft post_id={}",
                post_id.0
            )));
        };
        let preview_blobs = render_preview_blobs(state, *post_id);
        if preview_blobs.is_empty() {
            return Err(QzoneError::unknown(format!(
                "missing render preview post_id={}",
                post_id.0
            )));
        }
        out.push(PostAssets {
            draft,
            preview_blobs,
            include_original_images,
        });
    }
    Ok(out)
}

#[cfg(debug_assertions)]
fn collect_emuqzone_batch_images(
    posts: &[PostAssets],
    blob_paths: &HashMap<BlobId, String>,
) -> Result<Vec<String>, QzoneError> {
    let mut images = Vec::new();
    for post in posts {
        let mut part = collect_emuqzone_images(
            &post.draft,
            blob_paths,
            &post.preview_blobs,
            post.include_original_images,
        )?;
        images.append(&mut part);
    }
    Ok(images)
}

async fn collect_batch_images(
    posts: &[PostAssets],
    blob_paths: &HashMap<BlobId, String>,
) -> Result<Vec<Vec<u8>>, QzoneError> {
    let mut images = Vec::new();
    for post in posts {
        let mut part = collect_images(
            &post.draft,
            blob_paths,
            &post.preview_blobs,
            post.include_original_images,
        )
        .await?;
        images.append(&mut part);
    }
    Ok(images)
}

fn resolve_draft_for_send(state: &QzoneState, post_id: PostId) -> Option<Draft> {
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        let mut messages = Vec::new();
        for ingress_id in ingress_ids {
            let Some(message) = state.ingress_messages.get(ingress_id) else {
                return state.drafts.get(&post_id).cloned();
            };
            messages.push(message.clone());
        }
        if !messages.is_empty() {
            return Some(build_draft_from_messages(&messages));
        }
    }
    state.drafts.get(&post_id).cloned()
}

fn render_preview_blobs(state: &QzoneState, post_id: PostId) -> Vec<BlobId> {
    let Some(render) = state.render_blobs.get(&post_id) else {
        return Vec::new();
    };
    if let Some(png) = render.png {
        return vec![png];
    }
    Vec::new()
}

fn collect_post_blob_ids(state: &QzoneState, post_id: PostId) -> Vec<BlobId> {
    let mut blob_ids = Vec::new();
    if let Some(render) = state.render_blobs.get(&post_id) {
        if let Some(png) = render.png {
            blob_ids.push(png);
        }
    }
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            if let Some(message) = state.ingress_messages.get(ingress_id) {
                for attachment in &message.attachments {
                    if let MediaReference::Blob { blob_id } = attachment.reference {
                        blob_ids.push(blob_id);
                    }
                }
            }
        }
    }
    blob_ids
}

fn resolve_blob_path(
    blob_paths: &HashMap<BlobId, String>,
    blob_id: BlobId,
) -> Result<String, QzoneError> {
    blob_paths
        .get(&blob_id)
        .cloned()
        .ok_or_else(|| QzoneError::unknown("missing blob path"))
}

#[allow(dead_code)]
fn resolve_local_image_path(
    kind: MediaKind,
    reference: &MediaReference,
    blob_paths: &HashMap<BlobId, String>,
) -> Result<String, QzoneError> {
    match reference {
        MediaReference::Blob { blob_id } => resolve_blob_path(blob_paths, *blob_id),
        MediaReference::RemoteUrl { url } => resolve_remote_image_path(kind, url),
    }
}

#[allow(dead_code)]
fn resolve_remote_image_path(kind: MediaKind, source: &str) -> Result<String, QzoneError> {
    if let Some(path) = source.strip_prefix("file://") {
        return Ok(path.to_string());
    }
    if Path::new(source).exists() {
        return Ok(source.to_string());
    }
    if let Some((mime, bytes)) = decode_inline_source(source)? {
        let ext = mime
            .as_deref()
            .and_then(ext_from_content_type)
            .unwrap_or_else(|| default_ext_for_kind(kind));
        return cache_inline_bytes(&bytes, ext);
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return Err(QzoneError::unknown(
            "remote http image url not allowed; require local file",
        ));
    }
    Err(QzoneError::unknown("unsupported image source"))
}

fn decode_inline_source(source: &str) -> Result<Option<(Option<String>, Vec<u8>)>, QzoneError> {
    if let Some(payload) = source.strip_prefix("data:") {
        let (meta, data) = payload
            .split_once(',')
            .ok_or_else(|| QzoneError::unknown("invalid data url"))?;
        let mime = meta
            .split(';')
            .next()
            .and_then(|value| (!value.is_empty()).then_some(value.to_string()));
        let bytes = if meta.contains(";base64") {
            STANDARD
                .decode(data)
                .map_err(|err| QzoneError::unknown(format!("invalid data url base64: {}", err)))?
        } else {
            data.as_bytes().to_vec()
        };
        return Ok(Some((mime, bytes)));
    }
    if let Some(encoded) = source.strip_prefix("base64://") {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|err| QzoneError::unknown(format!("invalid base64 image: {}", err)))?;
        return Ok(Some((None, bytes)));
    }
    Ok(None)
}

#[allow(dead_code)]
fn ext_from_content_type(content_type: &str) -> Option<&'static str> {
    let base = content_type.split(';').next()?.trim().to_ascii_lowercase();
    match base.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}

#[allow(dead_code)]
fn default_ext_for_kind(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "png",
        MediaKind::Video => "mp4",
        MediaKind::Audio => "mp3",
        MediaKind::File | MediaKind::Other => "bin",
        MediaKind::Sticker => "png",
    }
}

#[allow(dead_code)]
fn cache_inline_bytes(bytes: &[u8], ext: &str) -> Result<String, QzoneError> {
    let ext = ext.trim_start_matches('.');
    let parts: Vec<&[u8]> = vec![bytes, ext.as_bytes()];
    let blob_id = derive_blob_id(&parts);
    let dir = inline_cache_root();
    fs::create_dir_all(&dir)
        .map_err(|err| QzoneError::unknown(format!("create inline dir failed: {}", err)))?;
    let filename = format!("{}.{}", id128_hex(blob_id.0), ext);
    let path = dir.join(filename);
    if !path.exists() {
        fs::write(&path, bytes)
            .map_err(|err| QzoneError::unknown(format!("write inline image failed: {}", err)))?;
    }
    Ok(path.to_string_lossy().to_string())
}

#[allow(dead_code)]
fn inline_cache_root() -> PathBuf {
    std::env::var("OQQWALL_BLOB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/blobs"))
        .join("inline")
}

fn read_local_image_bytes(path: &str) -> Result<Vec<u8>, QzoneError> {
    let path = path.strip_prefix("file://").unwrap_or(path);
    fs::read(path).map_err(|err| QzoneError::unknown(format!("read file failed: {}", err)))
}

fn resolve_blob_bytes(
    blob_paths: &HashMap<BlobId, String>,
    blob_id: BlobId,
) -> Result<Vec<u8>, QzoneError> {
    if let Some(bytes) = blob_cache::get_bytes(blob_id) {
        return Ok(bytes.as_ref().to_vec());
    }
    let path = resolve_blob_path(blob_paths, blob_id)?;
    read_local_image_bytes(&path)
}

fn resolve_reference_bytes(
    kind: MediaKind,
    reference: &MediaReference,
    blob_paths: &HashMap<BlobId, String>,
) -> Result<Vec<u8>, QzoneError> {
    match reference {
        MediaReference::Blob { blob_id } => resolve_blob_bytes(blob_paths, *blob_id),
        MediaReference::RemoteUrl { url } => {
            if let Some((_mime, bytes)) = decode_inline_source(url)? {
                return Ok(bytes);
            }
            if url.starts_with("file://") || Path::new(url).exists() {
                return read_local_image_bytes(url);
            }
            if url.starts_with("http://") || url.starts_with("https://") {
                return Err(QzoneError::unknown(
                    "remote http image url not allowed; require local file",
                ));
            }
            Err(QzoneError::unknown(format!(
                "unsupported image source: kind={:?}",
                kind
            )))
        }
    }
}

#[allow(dead_code)]
fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

#[cfg(debug_assertions)]
fn data_url_from_cache(entry: &blob_cache::CacheEntry) -> String {
    let mime = entry.mime.as_deref().unwrap_or("image/jpeg");
    format!(
        "data:{};base64,{}",
        mime,
        STANDARD.encode(entry.bytes.as_ref())
    )
}

#[cfg(debug_assertions)]
fn collect_emuqzone_images(
    draft: &Draft,
    blob_paths: &HashMap<BlobId, String>,
    preview_blobs: &[BlobId],
    include_original_images: bool,
) -> Result<Vec<String>, QzoneError> {
    let mut images = Vec::new();
    for blob_id in preview_blobs {
        if let Some(entry) = blob_cache::get_entry(*blob_id) {
            images.push(data_url_from_cache(&entry));
        } else {
            let path = resolve_blob_path(blob_paths, *blob_id)?;
            images.push(process_emuqzone_image(&path));
        }
    }
    if include_original_images {
        for block in &draft.blocks {
            if let DraftBlock::Attachment {
                kind: MediaKind::Image,
                reference,
                ..
            } = block
            {
                match reference {
                    MediaReference::Blob { blob_id } => {
                        if let Some(entry) = blob_cache::get_entry(*blob_id) {
                            images.push(data_url_from_cache(&entry));
                        } else {
                            let path = resolve_blob_path(blob_paths, *blob_id)?;
                            images.push(process_emuqzone_image(&path));
                        }
                    }
                    _ => {
                        let path =
                            resolve_local_image_path(MediaKind::Image, reference, blob_paths)?;
                        images.push(process_emuqzone_image(&path));
                    }
                }
            }
        }
    }
    Ok(images)
}

#[cfg(debug_assertions)]
fn append_emuqzone_post(
    state: &Arc<std::sync::Mutex<EmuQzoneState>>,
    text: String,
    images: Vec<String>,
) {
    let timestamp_ms = now_ms();
    let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
    let post = EmuQzonePost {
        id: guard.next_id,
        timestamp_ms,
        text,
        image_count: images.len(),
        images,
        status: "success".to_string(),
    };
    guard.next_id = guard.next_id.saturating_add(1);
    guard.posts.push(post);
    if guard.posts.len() > EMUQZONE_MAX_POSTS {
        let drain = guard.posts.len().saturating_sub(EMUQZONE_MAX_POSTS);
        guard.posts.drain(0..drain);
    }
}

#[cfg(debug_assertions)]
fn process_emuqzone_image(source: &str) -> String {
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("data:image")
    {
        return source.to_string();
    }
    if let Some(path) = source.strip_prefix("file://") {
        if let Ok(bytes) = fs::read(path) {
            let mime = mime_from_path(path).unwrap_or("image/jpeg");
            return format!("data:{};base64,{}", mime, STANDARD.encode(bytes));
        }
    }
    if Path::new(source).exists() {
        if let Ok(bytes) = fs::read(source) {
            let mime = mime_from_path(source).unwrap_or("image/jpeg");
            return format!("data:{};base64,{}", mime, STANDARD.encode(bytes));
        }
    }
    if STANDARD.decode(source).is_ok() {
        return format!("data:image/jpeg;base64,{}", source);
    }
    source.to_string()
}

#[cfg(debug_assertions)]
fn mime_from_path(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" | "svgz" => Some("image/svg+xml"),
        _ => None,
    }
}

#[cfg(debug_assertions)]
fn spawn_emuqzone_server(state: Arc<std::sync::Mutex<EmuQzoneState>>) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", EMUQZONE_PORT)) {
            Ok(listener) => listener,
            Err(err) => {
                debug_log!("emuqzone server bind failed: {}", err);
                return;
            }
        };
        debug_log!(
            "emuqzone server listening: http://127.0.0.1:{}",
            EMUQZONE_PORT
        );
        for stream in listener.incoming() {
            let state = state.clone();
            match stream {
                Ok(stream) => {
                    std::thread::spawn(move || {
                        handle_emuqzone_conn(stream, state);
                    });
                }
                Err(err) => {
                    debug_log!("emuqzone accept failed: {}", err);
                    break;
                }
            }
        }
    });
}

#[cfg(debug_assertions)]
fn handle_emuqzone_conn(mut stream: TcpStream, state: Arc<std::sync::Mutex<EmuQzoneState>>) {
    let mut buffer = [0u8; 2048];
    let Ok(size) = stream.read(&mut buffer) else {
        return;
    };
    if size == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buffer[..size]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    match path {
        "/" => {
            let body = emuqzone_html();
            write_http_response(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                body.as_bytes(),
            );
        }
        "/data" => {
            let body = {
                let guard = state.lock().unwrap_or_else(|err| err.into_inner());
                serde_json::to_vec(&guard.posts).unwrap_or_else(|_| b"[]".to_vec())
            };
            write_http_response(&mut stream, 200, "application/json; charset=utf-8", &body);
        }
        _ => {
            write_http_response(&mut stream, 404, "text/plain; charset=utf-8", b"Not Found");
        }
    }
}

#[cfg(debug_assertions)]
fn write_http_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        status,
        status_text,
        content_type,
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

#[cfg(debug_assertions)]
fn emuqzone_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>EmuQzone</title>
<style>
body {{ font-family: Arial, sans-serif; margin: 20px; }}
.post {{ border: 1px solid #ddd; margin: 10px 0; padding: 15px; border-radius: 6px; }}
.post-header {{ font-weight: bold; margin-bottom: 8px; }}
.post-content {{ white-space: pre-wrap; margin-bottom: 8px; }}
.post-images {{ display: flex; gap: 10px; flex-wrap: wrap; }}
.post-image {{ max-width: 220px; max-height: 220px; border-radius: 4px; }}
.refresh-btn {{ padding: 8px 14px; background: #1e88e5; color: white; border: none; border-radius: 4px; cursor: pointer; }}
</style>
</head>
<body>
<h1>EmuQzone</h1>
<p>Listening on http://127.0.0.1:{}</p>
<button class="refresh-btn" onclick="loadData()">Refresh</button>
<div id="posts"></div>
<script>
function loadData() {{
  fetch('/data')
    .then(r => r.json())
    .then(data => {{
      const posts = document.getElementById('posts');
      if (!data.length) {{
        posts.innerHTML = '<p>No posts yet.</p>';
        return;
      }}
      const ordered = [...data].sort((a, b) => b.timestamp_ms - a.timestamp_ms);
      posts.innerHTML = ordered.map(post => `
        <div class="post">
          <div class="post-header">Post #${{post.id}} - ${{post.timestamp_ms}}</div>
          <div class="post-content">${{post.text || ''}}</div>
          ${{post.images && post.images.length ? `
            <div class="post-images">
              ${{post.images.map(img => `<img src="${{img}}" alt="image" class="post-image">`).join('')}}
            </div>` : ''}}
          <div>Status: ${{post.status}} | Images: ${{post.image_count || 0}}</div>
        </div>
      `).join('');
    }})
    .catch(() => {{
      document.getElementById('posts').innerHTML = '<p>Failed to load data.</p>';
    }});
}}
loadData();
setInterval(loadData, 5000);
</script>
</body>
</html>"#,
        EMUQZONE_PORT
    )
}

fn parse_cookie_string(cookie_header: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut iter = trimmed.splitn(2, '=');
        let key = iter.next().unwrap_or("").trim();
        let value = iter.next().unwrap_or("").trim();
        if !key.is_empty() {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

fn build_cookie_header(cookies: &HashMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ")
}

fn generate_gtk(skey: &str) -> String {
    let mut hash_val: u32 = 5381;
    for &byte in skey.as_bytes() {
        hash_val = hash_val.wrapping_add((hash_val << 5).wrapping_add(byte as u32));
    }
    (hash_val & 0x7fffffff).to_string()
}

fn retry_delay_ms(kind: QzoneErrorKind, attempt: u32) -> TimestampMs {
    let (base, max) = match kind {
        QzoneErrorKind::Network => (5_000i64, 60_000i64),
        QzoneErrorKind::RiskControl => (60_000i64, 1_800_000i64),
        QzoneErrorKind::Account => (600_000i64, 3_600_000i64),
        QzoneErrorKind::Unknown => (60_000i64, 600_000i64),
    };
    let shift = attempt.saturating_sub(1).min(10);
    let delay = base.saturating_mul(1_i64 << shift);
    delay.min(max)
}

async fn get_cookies(
    state: &Arc<Mutex<QzoneState>>,
    account_id: &str,
) -> Result<HashMap<String, String>, QzoneError> {
    let now = now_ms();
    if let Some(cache) = state.lock().await.cookie_cache.as_ref() {
        if cache.account_id == account_id && now.saturating_sub(cache.fetched_at_ms) < 300_000 {
            debug_log!(
                "qzone cookies cache hit: age_ms={}",
                now.saturating_sub(cache.fetched_at_ms)
            );
            return Ok(cache.cookies.clone());
        }
    }

    debug_log!("qzone cookies cache miss: fetching via napcat ws");
    let cookies = fetch_cookies_ws(account_id)
        .await
        .map_err(QzoneError::network)?;
    let mut guard = state.lock().await;
    guard.cookie_cache = Some(CookieCache {
        account_id: account_id.to_string(),
        cookies: cookies.clone(),
        fetched_at_ms: now,
    });
    Ok(cookies)
}

async fn refresh_cookie_cache(state: &Arc<Mutex<QzoneState>>, account_id: &str) {
    match fetch_cookies_ws(account_id)
        .await
        .map_err(QzoneError::network)
    {
        Ok(cookies) => {
            let now = now_ms();
            let mut guard = state.lock().await;
            guard.cookie_cache = Some(CookieCache {
                account_id: account_id.to_string(),
                cookies,
                fetched_at_ms: now,
            });
            debug_log!("qzone cookies refreshed after send failure");
        }
        Err(_err) => {
            let mut guard = state.lock().await;
            guard.cookie_cache = None;
            debug_log!(
                "qzone cookies refresh failed: kind={:?} message={}",
                _err.kind,
                _err.message
            );
        }
    }
}

async fn fetch_cookies_ws(account_id: &str) -> Result<HashMap<String, String>, String> {
    let response = napcat_ws_request(
        account_id,
        "get_cookies",
        json!({ "domain": "user.qzone.qq.com" }),
        Duration::from_secs(10),
    )
    .await?;
    let cookie_str = response
        .get("data")
        .and_then(|data| data.get("cookies"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "napcat response missing cookies".to_string())?;
    Ok(parse_cookie_string(cookie_str))
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn classify_reqwest_error(context: &str, err: reqwest::Error) -> QzoneError {
    let status = err.status().map(|status| status.as_u16());
    let url = err.url().map(|url| url.to_string());
    let mut message = format!("{}: {}", context, err);
    let has_status = status.is_some();
    let status_text = status
        .map(|status| status.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    message.push_str(&format!("; status={}", status_text));
    if let Some(url) = url {
        if !message.contains(url.as_str()) {
            message.push_str(&format!("; url={}", url));
        }
    }
    let body_text = if has_status {
        "<unavailable from reqwest transport error>"
    } else {
        "<no response>"
    };
    message.push_str(&format!("; body={}", body_text));
    if err.is_timeout() || err.is_connect() {
        return QzoneError::network(message);
    }
    QzoneError::network(message)
}

fn classify_http_status(context: &str, status: u16) -> QzoneError {
    let kind = match status {
        401 | 403 => QzoneErrorKind::Account,
        429 => QzoneErrorKind::RiskControl,
        500..=599 => QzoneErrorKind::Network,
        _ => QzoneErrorKind::Unknown,
    };
    QzoneError::new(kind, format!("{}: http {}", context, status))
}

fn classify_http_status_with_body(context: &str, status: u16, body: &str) -> QzoneError {
    with_response_body(classify_http_status(context, status), body)
}

fn summarize_response_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    let total_chars = trimmed.chars().count();
    let preview: String = trimmed.chars().take(ERROR_BODY_PREVIEW_CHARS).collect();
    if total_chars > ERROR_BODY_PREVIEW_CHARS {
        format!("{}...(truncated, {} chars total)", preview, total_chars)
    } else {
        preview
    }
}

fn with_response_body(mut err: QzoneError, body: &str) -> QzoneError {
    err.message = format!("{}; body={}", err.message, summarize_response_body(body));
    err
}

fn with_response_code_and_body(
    mut err: QzoneError,
    code_label: &str,
    code: impl std::fmt::Display,
    body: &str,
) -> QzoneError {
    err.message = format!(
        "{}; {}={}; body={}",
        err.message,
        code_label,
        code,
        summarize_response_body(body)
    );
    err
}

fn classify_json_response_error(code: i64, message: &str, json: &Value) -> QzoneError {
    with_response_code_and_body(
        classify_text_error(message),
        "code",
        code,
        &json.to_string(),
    )
}

fn debug_log_http_failure(
    _context: &str,
    _status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) {
    let mut header_lines = Vec::new();
    for (key, value) in headers.iter() {
        let value = value.to_str().unwrap_or("<non-utf8>");
        header_lines.push(format!("{}: {}", key.as_str(), value));
    }
    let _header_block = if header_lines.is_empty() {
        "<empty>".to_string()
    } else {
        header_lines.join("\n")
    };
    debug_log!(
        "{} non-200: status={} reason={}\nheaders:\n{}\nbody:\n{}",
        _context,
        _status.as_u16(),
        _status.canonical_reason().unwrap_or("<none>"),
        _header_block,
        body
    );
    if let Ok(json) = serde_json::from_str::<Value>(body) {
        if let Ok(_pretty) = serde_json::to_string_pretty(&json) {
            debug_log!("{} non-200 json:\n{}", _context, _pretty);
        }
    }
}

fn parse_proxy_callback_json(body: &str) -> Option<Value> {
    if let Ok(json) = serde_json::from_str::<Value>(body) {
        return Some(json);
    }
    let trimmed = body.trim();
    if let Some(json) = extract_json_after_marker(trimmed, "frameElement.callback") {
        return Some(json);
    }
    if let Some(json) = extract_json_after_marker(trimmed, "cb(") {
        return Some(json);
    }
    if let Some(json) = extract_json_by_key(trimmed, "\"ret\"") {
        return Some(json);
    }
    let start = trimmed.find('{')?;
    extract_balanced_json(trimmed, start)
        .and_then(|slice| serde_json::from_str::<Value>(slice).ok())
}

fn extract_json_after_marker(body: &str, marker: &str) -> Option<Value> {
    let mut offset = 0;
    while let Some(pos) = body[offset..].find(marker) {
        let pos = offset + pos;
        let after = &body[pos + marker.len()..];
        if let Some(open) = after.find('{') {
            let start = pos + marker.len() + open;
            if let Some(slice) = extract_balanced_json(body, start) {
                if let Ok(json) = serde_json::from_str::<Value>(slice) {
                    return Some(json);
                }
            }
        }
        offset = pos + marker.len();
    }
    None
}

fn extract_json_by_key(body: &str, key: &str) -> Option<Value> {
    for (pos, _) in body.match_indices(key) {
        let start = body[..pos].rfind('{')?;
        if let Some(slice) = extract_balanced_json(body, start) {
            if let Ok(json) = serde_json::from_str::<Value>(slice) {
                return Some(json);
            }
        }
    }
    None
}

fn extract_balanced_json(body: &str, start: usize) -> Option<&str> {
    if !body.is_char_boundary(start) {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (idx, ch) in body[start..].char_indices() {
        let abs = start + idx;
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    let end = abs + ch.len_utf8();
                    return Some(&body[start..end]);
                }
            }
            _ => {}
        }
    }
    None
}

fn classify_response_error(json: &Value) -> Option<QzoneError> {
    let ret = json.get("ret").and_then(|v| v.as_i64());
    if let Some(ret) = ret {
        if ret != 0 {
            let message = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("qzone error");
            return Some(classify_json_response_error(ret, message, json));
        }
    }
    None
}

fn classify_text_error(message: &str) -> QzoneError {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("cookie")
        || lowered.contains("p_skey")
        || lowered.contains("skey")
        || lowered.contains("login")
        || lowered.contains("auth")
        || message.contains("登录")
    {
        return QzoneError::account(message);
    }
    if lowered.contains("risk")
        || lowered.contains("limit")
        || lowered.contains("频繁")
        || lowered.contains("风控")
        || lowered.contains("安全")
    {
        return QzoneError::risk(message);
    }
    if lowered.contains("timeout") || lowered.contains("connect") || lowered.contains("network") {
        return QzoneError::network(message);
    }
    QzoneError::unknown(message)
}

async fn collect_images(
    draft: &Draft,
    blob_paths: &HashMap<BlobId, String>,
    preview_blobs: &[BlobId],
    include_original_images: bool,
) -> Result<Vec<Vec<u8>>, QzoneError> {
    let mut images = Vec::new();
    for blob_id in preview_blobs {
        images.push(resolve_blob_bytes(blob_paths, *blob_id)?);
    }
    if !include_original_images {
        return Ok(images);
    }
    for block in &draft.blocks {
        if let DraftBlock::Attachment {
            kind: MediaKind::Image,
            reference,
            ..
        } = block
        {
            images.push(resolve_reference_bytes(
                MediaKind::Image,
                reference,
                blob_paths,
            )?);
        }
    }
    Ok(images)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadSourceFormat {
    Gif,
    Png,
    Other,
}

fn prepare_upload_image(image: &[u8]) -> Result<PreparedUploadImage, QzoneError> {
    if image.len() <= MAX_UPLOAD_IMAGE_BYTES {
        return Ok(PreparedUploadImage {
            bytes: image.to_vec(),
            strategy: "original",
        });
    }

    match detect_upload_source_format(image) {
        UploadSourceFormat::Gif => prepare_gif_upload_image(image),
        UploadSourceFormat::Png => {
            let decoded = decode_dynamic_image(image)?;
            if image_has_transparency(&decoded) {
                prepare_transparent_png_upload_image(decoded)
            } else {
                prepare_standard_upload_image(decoded)
            }
        }
        UploadSourceFormat::Other => prepare_standard_upload_image(decode_dynamic_image(image)?),
    }
}

fn detect_upload_source_format(image: &[u8]) -> UploadSourceFormat {
    match image_rs::guess_format(image).ok() {
        Some(ImageFormat::Gif) => UploadSourceFormat::Gif,
        Some(ImageFormat::Png) => UploadSourceFormat::Png,
        _ => UploadSourceFormat::Other,
    }
}

fn prepare_transparent_png_upload_image(
    image: DynamicImage,
) -> Result<PreparedUploadImage, QzoneError> {
    let preserve_image = resize_for_1080p(&image);
    let png_bytes = encode_png_image(&preserve_image)?;
    if png_bytes.len() <= MAX_UPLOAD_IMAGE_BYTES {
        return Ok(PreparedUploadImage {
            bytes: png_bytes,
            strategy: "png-preserved",
        });
    }

    Ok(PreparedUploadImage {
        bytes: compress_dynamic_as_jpeg(&preserve_image)?,
        strategy: "png-to-jpeg",
    })
}

fn prepare_gif_upload_image(image: &[u8]) -> Result<PreparedUploadImage, QzoneError> {
    let frames = decode_gif_frames(image)?;
    let first_frame = frames
        .first()
        .ok_or_else(|| QzoneError::unknown("gif contains no frames"))?;
    let (target_width, target_height, resized) =
        fit_within_1080p(first_frame.buffer().width(), first_frame.buffer().height());
    let frames = resize_gif_frames(frames, target_width, target_height);
    let fallback_image = DynamicImage::ImageRgba8(
        frames
            .first()
            .ok_or_else(|| QzoneError::unknown("gif contains no frames"))?
            .buffer()
            .clone(),
    );
    let gif_bytes = encode_gif_frames(frames)?;
    if gif_bytes.len() <= MAX_UPLOAD_IMAGE_BYTES {
        return Ok(PreparedUploadImage {
            bytes: gif_bytes,
            strategy: if resized {
                "gif-preserved"
            } else {
                "gif-reencoded"
            },
        });
    }

    Ok(PreparedUploadImage {
        bytes: compress_dynamic_as_jpeg(&fallback_image)?,
        strategy: "gif-to-jpeg",
    })
}

fn prepare_standard_upload_image(image: DynamicImage) -> Result<PreparedUploadImage, QzoneError> {
    Ok(PreparedUploadImage {
        bytes: compress_dynamic_as_jpeg(&image)?,
        strategy: "jpeg-reencoded",
    })
}

fn decode_dynamic_image(image: &[u8]) -> Result<DynamicImage, QzoneError> {
    image_rs::load_from_memory(image)
        .map_err(|err| QzoneError::unknown(format!("decode image failed: {}", err)))
}

fn decode_gif_frames(image: &[u8]) -> Result<Vec<ImageFrame>, QzoneError> {
    let decoder = GifDecoder::new(Cursor::new(image))
        .map_err(|err| QzoneError::unknown(format!("decode gif failed: {}", err)))?;
    decoder
        .into_frames()
        .collect_frames()
        .map_err(|err| QzoneError::unknown(format!("decode gif frames failed: {}", err)))
}

fn image_has_transparency(image: &DynamicImage) -> bool {
    image.to_rgba8().pixels().any(|pixel| pixel[3] < u8::MAX)
}

fn resize_for_1080p(image: &DynamicImage) -> DynamicImage {
    let (width, height) = image.dimensions();
    let (target_width, target_height, resized) = fit_within_1080p(width, height);
    if !resized {
        return image.clone();
    }
    image.resize_exact(
        target_width,
        target_height,
        image_rs::imageops::FilterType::Lanczos3,
    )
}

fn resize_gif_frames(
    frames: Vec<ImageFrame>,
    target_width: u32,
    target_height: u32,
) -> Vec<ImageFrame> {
    frames
        .into_iter()
        .map(|frame| {
            if frame.buffer().width() == target_width && frame.buffer().height() == target_height {
                return frame;
            }
            let delay = frame.delay();
            let resized = image_rs::imageops::resize(
                frame.buffer(),
                target_width,
                target_height,
                image_rs::imageops::FilterType::Lanczos3,
            );
            ImageFrame::from_parts(resized, 0, 0, delay)
        })
        .collect()
}

fn encode_png_image(image: &DynamicImage) -> Result<Vec<u8>, QzoneError> {
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|err| QzoneError::unknown(format!("encode png failed: {}", err)))?;
    Ok(cursor.into_inner())
}

fn encode_gif_frames(frames: Vec<ImageFrame>) -> Result<Vec<u8>, QzoneError> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = GifEncoder::new(&mut cursor);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|err| QzoneError::unknown(format!("encode gif repeat failed: {}", err)))?;
        for frame in frames {
            encoder
                .encode_frame(frame)
                .map_err(|err| QzoneError::unknown(format!("encode gif frame failed: {}", err)))?;
        }
    }
    Ok(cursor.into_inner())
}

fn compress_dynamic_as_jpeg(image: &DynamicImage) -> Result<Vec<u8>, QzoneError> {
    let mut working = image.clone();
    let mut smallest_size = usize::MAX;

    loop {
        for quality in JPEG_QUALITY_STEPS {
            let encoded = encode_jpeg_image(&working, quality)?;
            if encoded.len() <= MAX_UPLOAD_IMAGE_BYTES {
                return Ok(encoded);
            }
            smallest_size = smallest_size.min(encoded.len());
        }

        let (width, height) = working.dimensions();
        let Some((next_width, next_height)) = next_scaled_dimensions(width, height) else {
            break;
        };
        if next_width < JPEG_MIN_DIMENSION || next_height < JPEG_MIN_DIMENSION {
            break;
        }
        working = working.resize_exact(
            next_width,
            next_height,
            image_rs::imageops::FilterType::Lanczos3,
        );
    }

    Err(QzoneError::unknown(format!(
        "image remains above 4 MiB after compression: smallest_size_bytes={}",
        smallest_size
    )))
}

fn encode_jpeg_image(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, QzoneError> {
    let rgb = flatten_image_for_jpeg(image);
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image_rs::ColorType::Rgb8.into(),
        )
        .map_err(|err| QzoneError::unknown(format!("encode jpeg failed: {}", err)))?;
    Ok(out)
}

fn flatten_image_for_jpeg(image: &DynamicImage) -> image_rs::RgbImage {
    let rgba = image.to_rgba8();
    let mut rgb = image_rs::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u32;
        let blend = |channel: u8| -> u8 {
            let fg = channel as u32;
            let bg = 255u32;
            ((fg * alpha + bg * (255 - alpha) + 127) / 255) as u8
        };
        rgb.put_pixel(
            x,
            y,
            image_rs::Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])]),
        );
    }
    rgb
}

fn fit_within_1080p(width: u32, height: u32) -> (u32, u32, bool) {
    if width == 0 || height == 0 {
        return (width, height, false);
    }
    let (max_width, max_height) = if width >= height {
        (PRESERVE_MAX_LONG_EDGE_1080P, PRESERVE_MAX_SHORT_EDGE_1080P)
    } else {
        (PRESERVE_MAX_SHORT_EDGE_1080P, PRESERVE_MAX_LONG_EDGE_1080P)
    };
    fit_within_bounds(width, height, max_width, max_height)
}

fn fit_within_bounds(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32, bool) {
    if width <= max_width && height <= max_height {
        return (width, height, false);
    }

    let width_scale = max_width as f64 / width as f64;
    let height_scale = max_height as f64 / height as f64;
    let scale = width_scale.min(height_scale);
    let target_width = ((width as f64 * scale).floor() as u32).max(1);
    let target_height = ((height as f64 * scale).floor() as u32).max(1);
    (target_width, target_height, true)
}

fn next_scaled_dimensions(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let next_width = ((width as f64 * 0.85).floor() as u32).max(1);
    let next_height = ((height as f64 * 0.85).floor() as u32).max(1);
    if next_width == width && next_height == height {
        return None;
    }
    Some((next_width, next_height))
}

fn get_picbo_and_richval(upload: &Value) -> Result<(String, String), QzoneError> {
    let ret = upload.get("ret").and_then(|v| v.as_i64()).unwrap_or(-1);
    if ret != 0 {
        let message = upload
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("upload image failed");
        return Err(classify_json_response_error(ret, message, upload));
    }
    let data = upload
        .get("data")
        .ok_or_else(|| QzoneError::unknown("upload response missing data"))?;
    let url = data
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| QzoneError::unknown("upload response missing url"))?;
    let url_bo =
        extract_bo(url).ok_or_else(|| QzoneError::unknown("upload response missing picbo"))?;
    let pre_bo = data
        .get("pre")
        .and_then(|v| v.as_str())
        .and_then(extract_bo);
    let picbo = if let Some(pre_bo) = pre_bo {
        format!("{}\t{}", pre_bo, url_bo)
    } else {
        format!("{}\t{}", url_bo, url_bo)
    };

    let albumid = field_to_string(data, "albumid")?;
    let lloc = field_to_string(data, "lloc")?;
    let sloc = field_to_string(data, "sloc")?;
    let kind = field_to_string(data, "type")?;
    let height = field_to_string(data, "height")?;
    let width = field_to_string(data, "width")?;

    let richval = format!(
        ",{},{},{},{},{},{},,{},{}",
        albumid, lloc, sloc, kind, height, width, height, width
    );
    Ok((picbo, richval))
}

fn extract_bo(value: &str) -> Option<String> {
    let marker = "bo=";
    let start = value.find(marker)? + marker.len();
    let rest = &value[start..];
    let end = rest.find('&').unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some(rest[..end].to_string())
}

fn field_to_string(data: &Value, key: &str) -> Result<String, QzoneError> {
    let value = data
        .get(key)
        .ok_or_else(|| QzoneError::unknown(format!("upload response missing {}", key)))?;
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(QzoneError::unknown(format!("invalid field {}", key))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Delay, Frame as ImageFrame, Rgba, RgbaImage};
    use serde_json::json;

    #[test]
    fn small_image_keeps_original_bytes() {
        let image = make_rgba_pattern(320, 240, false);
        let bytes = encode_png_image(&DynamicImage::ImageRgba8(image)).expect("encode png");
        assert!(bytes.len() < MAX_UPLOAD_IMAGE_BYTES);

        let prepared = prepare_upload_image(&bytes).expect("prepare image");
        assert_eq!(prepared.strategy, "original");
        assert_eq!(prepared.bytes, bytes);
    }

    #[test]
    fn oversized_transparent_png_prefers_png_before_jpeg() {
        let image = make_rgba_pattern(640, 360, true);
        let mut bytes = encode_png_image(&DynamicImage::ImageRgba8(image)).expect("encode png");
        pad_to_oversized(&mut bytes);
        assert!(bytes.len() > MAX_UPLOAD_IMAGE_BYTES);

        let prepared = prepare_upload_image(&bytes).expect("prepare png");
        assert_eq!(prepared.strategy, "png-preserved");
        assert!(prepared.bytes.len() <= MAX_UPLOAD_IMAGE_BYTES);
        assert!(is_png(&prepared.bytes));
    }

    #[test]
    fn oversized_transparent_png_falls_back_to_jpeg_at_1080p() {
        let image = make_noisy_rgba(1920, 1080, true);
        let bytes = encode_png_image(&DynamicImage::ImageRgba8(image)).expect("encode png");
        assert!(bytes.len() > MAX_UPLOAD_IMAGE_BYTES);

        let prepared = prepare_upload_image(&bytes).expect("prepare png");
        assert_eq!(prepared.strategy, "png-to-jpeg");
        assert!(prepared.bytes.len() <= MAX_UPLOAD_IMAGE_BYTES);
        assert!(is_jpeg(&prepared.bytes));
    }

    #[test]
    fn oversized_gif_prefers_gif_before_jpeg() {
        let mut frames = Vec::new();
        for shift in [0u8, 17u8] {
            let mut frame = make_rgba_pattern(640, 360, false);
            for pixel in frame.pixels_mut() {
                pixel.0[0] = pixel.0[0].wrapping_add(shift);
            }
            frames.push(ImageFrame::from_parts(
                frame,
                0,
                0,
                Delay::from_numer_denom_ms(120, 1),
            ));
        }
        let mut bytes = encode_gif_frames(frames).expect("encode gif");
        pad_to_oversized(&mut bytes);
        assert!(bytes.len() > MAX_UPLOAD_IMAGE_BYTES);

        let prepared = prepare_upload_image(&bytes).expect("prepare gif");
        assert!(
            prepared.strategy == "gif-preserved" || prepared.strategy == "gif-reencoded",
            "unexpected strategy: {}",
            prepared.strategy
        );
        assert!(prepared.bytes.len() <= MAX_UPLOAD_IMAGE_BYTES);
        assert!(is_gif(&prepared.bytes));
    }

    #[test]
    fn classify_response_error_includes_code_and_body() {
        let json = json!({
            "ret": 10045,
            "message": "upload blocked",
            "data": { "reason": "quota exceeded" }
        });

        let err = classify_response_error(&json).expect("expected response error");
        assert!(err.message.contains("upload blocked"));
        assert!(err.message.contains("code=10045"));
        assert!(err.message.contains("\"ret\":10045"));
        assert!(err.message.contains("\"reason\":\"quota exceeded\""));
    }

    #[test]
    fn collect_batch_post_ids_respects_max_queue_limit() {
        let mut state = QzoneState::default();
        let leader = seed_batch_post(&mut state, 1, 1, 2, 0);
        let second = seed_batch_post(&mut state, 2, 2, 2, 0);
        let third = seed_batch_post(&mut state, 3, 3, 2, 0);

        let (batch, requeue) =
            collect_batch_post_ids(&state, "g", leader, SendPriority::Normal, 1_000, 2, 0, true);

        assert_eq!(batch, vec![leader, second]);
        assert_eq!(requeue_ids(&requeue), vec![second, third]);
    }

    #[test]
    fn collect_batch_post_ids_stops_before_image_limit_overflow() {
        let mut state = QzoneState::default();
        let leader = seed_batch_post(&mut state, 1, 1, 3, 0);
        let second = seed_batch_post(&mut state, 2, 2, 3, 0);
        let third = seed_batch_post(&mut state, 3, 3, 5, 0);

        let (batch, requeue) =
            collect_batch_post_ids(&state, "g", leader, SendPriority::Normal, 1_000, 3, 6, true);

        assert_eq!(batch, vec![leader, second]);
        assert_eq!(requeue_ids(&requeue), vec![second, third]);
    }

    fn requeue_ids(plans: &[RequeuePostPlan]) -> Vec<PostId> {
        plans.iter().map(|plan| plan.post_id).collect()
    }

    fn seed_batch_post(
        state: &mut QzoneState,
        raw_post_id: u128,
        seq: u64,
        total_images: usize,
        not_before_ms: i64,
    ) -> PostId {
        let post_id = PostId::from_u128(raw_post_id);
        state.send_plans.insert(
            post_id,
            SendPlanMeta {
                group_id: "g".to_string(),
                not_before_ms,
                priority: SendPriority::Normal,
                seq,
            },
        );
        if total_images > 0 {
            state.render_blobs.insert(
                post_id,
                RenderBlobs {
                    png: Some(BlobId::from_u128(raw_post_id.saturating_add(10_000))),
                },
            );
        }
        let ingress_id = IngressId::from_u128(raw_post_id.saturating_add(20_000));
        let attachments = (0..total_images.saturating_sub(1))
            .map(|idx| oqqwall_rust_core::draft::IngressAttachment {
                kind: MediaKind::Image,
                name: None,
                reference: MediaReference::RemoteUrl {
                    url: format!("file:///tmp/{}_{}.png", raw_post_id, idx),
                },
                size_bytes: None,
            })
            .collect();
        state.post_ingress.insert(post_id, vec![ingress_id]);
        state.ingress_messages.insert(
            ingress_id,
            IngressMessage {
                text: String::new(),
                attachments,
            },
        );
        post_id
    }

    fn make_rgba_pattern(width: u32, height: u32, transparent: bool) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let alpha = if transparent {
                ((x.wrapping_mul(17) ^ y.wrapping_mul(31)) & 0xFF) as u8
            } else {
                u8::MAX
            };
            *pixel = Rgba([
                ((x * 13 + y * 7) & 0xFF) as u8,
                ((x * 3 + y * 19) & 0xFF) as u8,
                ((x * 23 + y * 5) & 0xFF) as u8,
                alpha,
            ]);
        }
        image
    }

    fn make_noisy_rgba(width: u32, height: u32, transparent: bool) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        let mut state = 0x1234_5678u32;
        for (_, _, pixel) in image.enumerate_pixels_mut() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let r = (state >> 24) as u8;
            let g = (state >> 16) as u8;
            let b = (state >> 8) as u8;
            let a = if transparent { state as u8 } else { u8::MAX };
            *pixel = Rgba([r, g, b, a]);
        }
        image
    }

    fn pad_to_oversized(bytes: &mut Vec<u8>) {
        let target = MAX_UPLOAD_IMAGE_BYTES + 1024;
        if bytes.len() < target {
            bytes.resize(target, 0);
        }
    }

    fn is_png(bytes: &[u8]) -> bool {
        bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    }

    fn is_jpeg(bytes: &[u8]) -> bool {
        bytes.starts_with(&[0xFF, 0xD8, 0xFF])
    }

    fn is_gif(bytes: &[u8]) -> bool {
        bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
    }
}
