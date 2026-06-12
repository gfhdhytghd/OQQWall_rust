use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::command::{
    GlobalAction, GlobalActionBatchCommand, GlobalActionCommand, ReviewAction,
    ReviewActionBatchCommand, ReviewActionCommand, ShortcutScope,
};
use oqqwall_rust_core::draft::{
    IngressAttachment, IngressMessage, MediaKind, MediaReference, ReplyPreview, json_card_marker,
    poke_marker, reply_marker,
};
use oqqwall_rust_core::event::{
    BlobEvent, DraftEvent, Event, IngressEvent, InputStatusKind, MediaEvent, RenderEvent,
    ReviewDecision, ReviewEvent, ReviewSubmitterNoticeKind, ScheduleEvent, SendEvent, SendPriority,
};
use oqqwall_rust_core::ids::{BlobId, ExternalCode, IngressId, PostId, ReviewCode, ReviewId};
use oqqwall_rust_core::{Command, IngressCommand, StateView, derive_blob_id, derive_ingress_id};
use oqqwall_rust_infra::{LocalJournal, SnapshotStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::handshake::server::{ErrorResponse, Request, Response},
    tungstenite::http::StatusCode,
};

use crate::blob_cache;
use crate::shortcut::{
    RAW_BUILTIN_PREFIX, ShortcutTemplateContext, is_builtin_review_command_name,
    parse_builtin_global_action, parse_builtin_review_action, parse_global_shortcut_actions,
    parse_review_shortcut_actions, shortcut_field_name, shortcut_scope_label,
    validate_global_shortcut_definition, validate_review_shortcut_definition,
    validate_shortcut_name,
};

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
pub struct NapCatConfig {
    pub base_url: String,
    pub access_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NapCatRuntimeConfig {
    pub napcat: NapCatConfig,
    pub audit_group_id: Option<String>,
    pub group_id: String,
    pub accounts: Vec<String>,
    pub tz_offset_minutes: i32,
    pub friend_request_window_sec: u32,
    pub friend_add_message: Option<String>,
    pub max_queue: usize,
    pub quick_replies: Arc<std::sync::Mutex<HashMap<String, String>>>,
    pub review_shortcuts: Arc<std::sync::Mutex<HashMap<String, String>>>,
    pub global_shortcuts: Arc<std::sync::Mutex<HashMap<String, String>>>,
}

const MAX_FORWARD_DEPTH: u32 = 4;
const FRIEND_APPROVE_DELAY_MAX_SEC: u64 = 240;
const FRIEND_NOTIFY_DELAY_SEC: u64 = 30;
const FRIEND_REQUEST_ID_MAX_LEN: usize = 20;
const FRIEND_SUPPRESS_REMOVE_CHARS: &str =
    r#"　“”‘’《》〈〉【】。，：；？！（）、「」『』—［］＂＇"'`~!@#$%^&*()_+-={}[]|:;<>?,./"#;
static STARTUP_NOTICE_SENT: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();
static WS_SESSIONS: OnceLock<std::sync::Mutex<HashMap<String, NapCatWsSession>>> = OnceLock::new();
static GROUP_ACCOUNTS: OnceLock<std::sync::Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct ReviewInfo {
    review_code: ReviewCode,
    post_id: PostId,
    group_id: String,
}

#[derive(Debug, Clone)]
struct SendPlanInfo {
    group_id: String,
    not_before_ms: i64,
    priority: SendPriority,
    seq: u64,
}

#[derive(Debug, Clone)]
struct SendingInfo {
    group_id: String,
    started_at_ms: i64,
    batch_leader: PostId,
    batch_label: String,
}

#[derive(Debug, Clone)]
struct IngressSummary {
    user_id: String,
    sender_name: Option<String>,
    text: String,
    attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtractedMessage {
    pub(crate) text: String,
    pub(crate) summary_text: String,
    pub(crate) attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone)]
struct MessageChunk {
    text: String,
    summary_text: String,
    attachments: Vec<IngressAttachment>,
}

#[derive(Debug)]
struct ForwardResolver {
    account_id: String,
    cache: HashMap<String, Vec<MessageChunk>>,
    seen: HashSet<String>,
}

#[derive(Debug, Clone)]
struct AuditMessage {
    text: String,
    images: Vec<String>,
}

#[derive(Debug)]
enum PendingAction {
    SendAuditMessage {
        review_id: ReviewId,
        attempt: u32,
    },
    WsRequest {
        resp_tx: oneshot::Sender<Result<Value, String>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AuditCommand {
    Review {
        review_code: Option<ReviewCode>,
        action: ParsedReviewAction,
    },
    Global(ParsedGlobalAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedReviewAction {
    Builtin(ReviewAction),
    Shortcut { key: String, args: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedGlobalAction {
    Builtin(GlobalAction),
    Batch(Vec<GlobalAction>),
}

#[derive(Debug, Clone)]
struct SuppressionEntry {
    comment_norm: String,
    expire_at_ms: i64,
}

#[derive(Debug, Clone)]
struct BufferedMessage {
    message: Value,
}

#[derive(Debug, Clone)]
struct SubmissionSession {
    messages: Vec<BufferedMessage>,
    started_at_ms: i64,
    group_id: String,
    confirming: bool,
}

#[derive(Default)]
struct NapCatState {
    review_info: HashMap<ReviewId, ReviewInfo>,
    review_by_code: HashMap<ReviewCode, ReviewId>,
    review_publish_attempts: HashMap<ReviewId, u32>,
    ingress_summary: HashMap<IngressId, IngressSummary>,
    pending_summary: HashMap<IngressId, String>,
    post_ingress: HashMap<PostId, Vec<IngressId>>,
    post_group: HashMap<PostId, String>,
    post_safe: HashMap<PostId, bool>,
    post_render_blobs: HashMap<PostId, Vec<BlobId>>,
    post_review_code: HashMap<PostId, ReviewCode>,
    post_external_code: HashMap<PostId, ExternalCode>,
    review_submitter: HashMap<ReviewId, String>,
    blacklist: HashMap<String, HashMap<String, Option<String>>>,
    send_plans: HashMap<PostId, SendPlanInfo>,
    sending: HashMap<PostId, SendingInfo>,
    audit_msg_to_review: HashMap<String, ReviewId>,
    processed_reviews: HashSet<ReviewId>,
    pending: HashMap<String, PendingAction>,
    friend_req_cache: HashMap<String, i64>,
    friend_suppression: HashMap<String, Vec<SuppressionEntry>>,
    submission_sessions: HashMap<String, SubmissionSession>,
    blob_paths: HashMap<BlobId, String>,
    next_echo: u64,
}

#[derive(Clone)]
struct NapCatWsSession {
    out_tx: mpsc::Sender<String>,
    state: Arc<Mutex<NapCatState>>,
}

fn load_state_view_cached() -> StateView {
    static CACHE: OnceLock<StateView> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let data_dir = env::var("OQQWALL_DATA_DIR").unwrap_or_else(|_| "data".to_string());
            let journal = match LocalJournal::open(&data_dir) {
                Ok(journal) => journal,
                Err(_err) => {
                    debug_log!("napcat preload skipped: journal open failed: {}", _err);
                    return StateView::default();
                }
            };
            let snapshot = match SnapshotStore::open(&data_dir) {
                Ok(snapshot) => snapshot,
                Err(_err) => {
                    debug_log!("napcat preload skipped: snapshot open failed: {}", _err);
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
                    debug_log!("napcat preload: snapshot load failed: {}", _err);
                }
            }

            if let Err(_err) = journal.replay(cursor, |env| {
                state = state.reduce(env);
            }) {
                debug_log!("napcat preload: journal replay failed: {}", _err);
            }

            state
        })
        .clone()
}

fn build_state_from_view(view: &StateView) -> NapCatState {
    let mut state = NapCatState::default();
    for (ingress_id, meta) in &view.ingress_meta {
        let (text, attachments) = match view.ingress_messages.get(ingress_id) {
            Some(message) => (message.text.clone(), message.attachments.clone()),
            None => (String::new(), Vec::new()),
        };
        state.ingress_summary.insert(
            *ingress_id,
            IngressSummary {
                user_id: meta.user_id.clone(),
                sender_name: meta.sender_name.clone(),
                text,
                attachments,
            },
        );
    }
    for (post_id, ingress_ids) in &view.post_ingress {
        state.post_ingress.insert(*post_id, ingress_ids.clone());
    }
    for (post_id, post) in &view.posts {
        state.post_group.insert(*post_id, post.group_id.clone());
        state.post_safe.insert(*post_id, post.is_safe);
    }
    for (post_id, render) in &view.render {
        let mut blob_ids = render.png_blobs.clone();
        if blob_ids.is_empty() {
            if let Some(blob_id) = render.png_blob {
                blob_ids.push(blob_id);
            }
        }
        if !blob_ids.is_empty() {
            state.post_render_blobs.insert(*post_id, blob_ids);
        }
    }
    for (review_id, review) in &view.reviews {
        let group_id = state
            .post_group
            .get(&review.post_id)
            .cloned()
            .unwrap_or_default();
        state.review_info.insert(
            *review_id,
            ReviewInfo {
                review_code: review.review_code,
                post_id: review.post_id,
                group_id,
            },
        );
        state.review_by_code.insert(review.review_code, *review_id);
        state
            .post_review_code
            .insert(review.post_id, review.review_code);
        if let Some(audit_msg_id) = review.audit_msg_id.as_ref() {
            state
                .audit_msg_to_review
                .insert(audit_msg_id.clone(), *review_id);
        }
        if matches!(
            review.decision,
            Some(
                ReviewDecision::Approved
                    | ReviewDecision::Rejected
                    | ReviewDecision::Skipped
                    | ReviewDecision::Deleted
            )
        ) {
            state.processed_reviews.insert(*review_id);
        }
        if review.publish_attempt > 0 {
            state
                .review_publish_attempts
                .insert(*review_id, review.publish_attempt);
        }
        if let Some(user_id) = resolve_post_submitter(&state, review.post_id) {
            state.review_submitter.insert(*review_id, user_id);
        }
    }
    for (post_id, external_code) in &view.external_code_by_post {
        state.post_external_code.insert(*post_id, *external_code);
    }
    for (group_id, entries) in &view.blacklist {
        state.blacklist.insert(group_id.clone(), entries.clone());
    }
    for (post_id, plan) in &view.send_plans {
        state.send_plans.insert(
            *post_id,
            SendPlanInfo {
                group_id: plan.group_id.clone(),
                not_before_ms: plan.not_before_ms,
                priority: plan.priority,
                seq: plan.seq,
            },
        );
    }
    for (post_id, meta) in &view.sending {
        let label = post_label(&state, *post_id);
        state.sending.insert(
            *post_id,
            SendingInfo {
                group_id: meta.group_id.clone(),
                started_at_ms: meta.started_at_ms,
                batch_leader: *post_id,
                batch_label: label,
            },
        );
    }
    for (blob_id, meta) in &view.blobs {
        if let Some(path) = meta.persisted_path.as_ref() {
            state.blob_paths.insert(*blob_id, path.clone());
        }
    }
    state
}

#[derive(Clone)]
struct RuntimeEntry {
    runtime: NapCatRuntimeConfig,
    state: Arc<Mutex<NapCatState>>,
}

#[derive(Debug, Clone)]
struct ReverseBaseUrl {
    bind_addr: String,
    path: String,
}

fn parse_reverse_base_url(raw: &str) -> Result<ReverseBaseUrl, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("napcat base_url is empty".to_string());
    }
    let without_scheme = trimmed
        .strip_prefix("ws://")
        .or_else(|| trimmed.strip_prefix("wss://"))
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    let mut parts = without_scheme.splitn(2, '/');
    let host_port = parts.next().unwrap_or_default();
    if host_port.is_empty() {
        return Err("napcat base_url missing host".to_string());
    }
    let path = parts
        .next()
        .map(|rest| format!("/{}", rest))
        .unwrap_or_else(|| "/".to_string());
    let path = normalize_base_path(&path);
    let (host, port) = split_host_port(host_port)?;
    Ok(ReverseBaseUrl {
        bind_addr: format!("{}:{}", host, port),
        path,
    })
}

fn split_host_port(value: &str) -> Result<(String, u16), String> {
    if value.starts_with('[') {
        let end = value
            .find(']')
            .ok_or_else(|| "napcat base_url invalid host".to_string())?;
        let host = &value[..=end];
        let rest = &value[end + 1..];
        let port_str = rest
            .strip_prefix(':')
            .ok_or_else(|| "napcat base_url missing port".to_string())?;
        let port = port_str
            .parse::<u16>()
            .map_err(|_| "napcat base_url invalid port".to_string())?;
        return Ok((host.to_string(), port));
    }
    let mut parts = value.rsplitn(2, ':');
    let port_str = parts
        .next()
        .ok_or_else(|| "napcat base_url missing port".to_string())?;
    let host = parts
        .next()
        .ok_or_else(|| "napcat base_url missing host".to_string())?;
    let port = port_str
        .parse::<u16>()
        .map_err(|_| "napcat base_url invalid port".to_string())?;
    Ok((host.to_string(), port))
}

fn normalize_base_path(raw: &str) -> String {
    let mut path = raw.trim().to_string();
    if path.is_empty() {
        return "/".to_string();
    }
    if !path.starts_with('/') {
        path = format!("/{}", path);
    }
    if path.len() > 1 {
        path = path.trim_end_matches('/').to_string();
    }
    path
}

fn extract_account_from_path(path: &str, base_path: &str) -> Option<String> {
    let path = if path.is_empty() { "/" } else { path };
    let path = path.trim_end_matches('/');
    let base_path = if base_path.is_empty() { "/" } else { base_path };
    if base_path == "/" {
        let account = path.trim_start_matches('/');
        if account.is_empty() || account.contains('/') {
            return None;
        }
        return Some(account.to_string());
    }
    if !path.starts_with(base_path) {
        return None;
    }
    let rest = &path[base_path.len()..];
    let rest = rest.strip_prefix('/')?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

fn request_token(req: &Request) -> Option<String> {
    if let Some(value) = req.headers().get("Authorization") {
        if let Ok(raw) = value.to_str() {
            if let Some(stripped) = raw.strip_prefix("Bearer ") {
                return Some(stripped.trim().to_string());
            }
            if let Some(stripped) = raw.strip_prefix("bearer ") {
                return Some(stripped.trim().to_string());
            }
        }
    }
    let query = req.uri().query()?;
    query_param(query, "access_token")
        .or_else(|| query_param(query, "token"))
        .map(|value| value.to_string())
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for part in query.split('&') {
        let mut iter = part.splitn(2, '=');
        let name = iter.next()?.trim();
        if name != key {
            continue;
        }
        let value = iter.next().unwrap_or("").trim();
        if value.is_empty() {
            return None;
        }
        return Some(value);
    }
    None
}

fn reject_response(status: StatusCode, message: &str) -> Result<Response, ErrorResponse> {
    let response = tokio_tungstenite::tungstenite::http::Response::builder()
        .status(status)
        .body(Some(message.to_string()))
        .unwrap_or_else(|_| {
            tokio_tungstenite::tungstenite::http::Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(None)
                .unwrap()
        });
    Err(response)
}

pub fn spawn_napcat_ws(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    base_url: String,
    runtimes: Vec<NapCatRuntimeConfig>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let base = match parse_reverse_base_url(&base_url) {
            Ok(base) => base,
            Err(_err) => {
                debug_log!(
                    "napcat ws server skipped: base_url={} err={}",
                    base_url,
                    _err
                );
                return;
            }
        };
        debug_log!(
            "napcat ws server start: base_url={} bind_addr={} path={}",
            base_url,
            base.bind_addr,
            base.path
        );
        let state_view = load_state_view_cached();
        let mut account_map: HashMap<String, RuntimeEntry> = HashMap::new();
        let mut fallback_entry: Option<RuntimeEntry> = None;
        for runtime in runtimes {
            set_group_accounts(&runtime.group_id, runtime.accounts.clone());
            if runtime.accounts.is_empty() {
                let entry = RuntimeEntry {
                    runtime: runtime.clone(),
                    state: Arc::new(Mutex::new(build_state_from_view(&state_view))),
                };
                if fallback_entry.is_none() {
                    debug_log!(
                        "napcat ws fallback enabled: group_id={} reason=accounts_empty",
                        runtime.group_id
                    );
                    fallback_entry = Some(entry);
                } else {
                    debug_log!(
                        "napcat ws skipped: group_id={} reason=accounts_empty",
                        runtime.group_id
                    );
                }
                continue;
            }
            let entry = RuntimeEntry {
                runtime: runtime.clone(),
                state: Arc::new(Mutex::new(build_state_from_view(&state_view))),
            };
            for account in &runtime.accounts {
                if account_map.contains_key(account) {
                    debug_log!(
                        "napcat ws account ignored: account_id={} group_id={}",
                        account,
                        runtime.group_id
                    );
                    continue;
                }
                account_map.insert(account.clone(), entry.clone());
            }
        }
        if account_map.is_empty() && fallback_entry.is_none() {
            debug_log!("napcat ws server skipped: no accounts registered");
            return;
        }
        let account_map = Arc::new(account_map);
        let fallback_entry = fallback_entry;
        let active_accounts: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let listener = match TcpListener::bind(&base.bind_addr).await {
            Ok(listener) => listener,
            Err(_err) => {
                debug_log!(
                    "napcat ws server bind failed: addr={} err={}",
                    base.bind_addr,
                    _err
                );
                return;
            }
        };

        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_err) => {
                    debug_log!("napcat ws accept failed: err={}", _err);
                    continue;
                }
            };
            let account_map = Arc::clone(&account_map);
            let active_accounts = Arc::clone(&active_accounts);
            let fallback_entry = fallback_entry.clone();
            let base_path = base.path.clone();
            let cmd_tx = cmd_tx.clone();
            let bus_rx = bus_rx.resubscribe();
            tokio::spawn(async move {
                let account_capture = Arc::new(std::sync::Mutex::new(None::<String>));
                let capture = Arc::clone(&account_capture);
                let account_map_cb = Arc::clone(&account_map);
                let fallback_entry_cb = fallback_entry.clone();
                let accept_result =
                    accept_hdr_async(stream, move |req: &Request, resp: Response| {
                        let account = extract_account_from_path(req.uri().path(), &base_path);
                        *capture.lock().unwrap() = account.clone();
                        let Some(account) = account else {
                            return reject_response(StatusCode::NOT_FOUND, "missing account");
                        };
                        let entry = account_map_cb
                            .get(&account)
                            .cloned()
                            .or_else(|| fallback_entry_cb.clone());
                        let Some(entry) = entry else {
                            return reject_response(StatusCode::NOT_FOUND, "unknown account");
                        };
                        if let Some(expected) = entry.runtime.napcat.access_token.as_ref() {
                            if request_token(req).as_deref() != Some(expected.as_str()) {
                                return reject_response(StatusCode::UNAUTHORIZED, "invalid token");
                            }
                        }
                        Ok(resp)
                    })
                    .await;
                let mut ws_stream = match accept_result {
                    Ok(ws_stream) => ws_stream,
                    Err(_err) => {
                        debug_log!("napcat ws handshake failed: addr={} err={:?}", _addr, _err);
                        return;
                    }
                };
                let account = {
                    let guard = account_capture.lock().unwrap();
                    guard.clone()
                };
                let account = match account {
                    Some(account) => account,
                    None => {
                        let _ = ws_stream.close(None).await;
                        return;
                    }
                };
                let entry = account_map
                    .get(&account)
                    .cloned()
                    .or_else(|| fallback_entry.clone());
                let Some(entry) = entry else {
                    let _ = ws_stream.close(None).await;
                    return;
                };
                let inserted = {
                    let mut guard = active_accounts.lock().await;
                    if guard.contains(&account) {
                        false
                    } else {
                        guard.insert(account.clone());
                        true
                    }
                };
                if !inserted {
                    debug_log!(
                        "napcat ws duplicate connection ignored: account_id={}",
                        account
                    );
                    let _ = ws_stream.close(None).await;
                    return;
                }
                println!(
                    "NapCat WS 已连接: account_id={} group_id={}",
                    account, entry.runtime.group_id
                );
                run_napcat_session(
                    cmd_tx,
                    bus_rx,
                    entry.runtime.clone(),
                    Arc::clone(&entry.state),
                    account.clone(),
                    ws_stream,
                )
                .await;
                let mut guard = active_accounts.lock().await;
                guard.remove(&account);
                debug_log!(
                    "napcat ws disconnected: account_id={} group_id={}",
                    account,
                    entry.runtime.group_id
                );
            });
        }
    })
}

async fn run_napcat_session(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    runtime: NapCatRuntimeConfig,
    state: Arc<Mutex<NapCatState>>,
    account_id: String,
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) {
    let (mut ws_write, mut ws_read) = ws_stream.split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    let state_ref = Arc::clone(&state);
    register_ws_session(
        &account_id,
        NapCatWsSession {
            out_tx: out_tx.clone(),
            state: Arc::clone(&state_ref),
        },
    );
    notify_account_online_change(&runtime, &account_id, true).await;
    let startup_group_id = runtime
        .audit_group_id
        .as_deref()
        .unwrap_or(&runtime.group_id);
    if is_effective_primary_account(&runtime, &account_id)
        && should_send_startup_notice(startup_group_id)
    {
        send_group_text(&out_tx, startup_group_id, "系统已启动").await;
    }

    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_write
                .send(tokio_tungstenite::tungstenite::Message::Text(msg))
                .await
                .is_err()
            {
                debug_log!("napcat ws writer send failed");
                break;
            }
        }
    });

    let cmd_tx_read = cmd_tx.clone();
    let runtime_read = runtime.clone();
    let state_read = Arc::clone(&state_ref);
    let out_tx_read = out_tx.clone();
    let account_id_read = account_id.clone();
    let reader = tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            let msg = match msg {
                Ok(msg) => msg,
                Err(_err) => {
                    debug_log!("napcat ws read error: {}", _err);
                    break;
                }
            };
            if !msg.is_text() {
                debug_log!("napcat ws ignoring non-text message");
                continue;
            }
            let text = match msg.to_text() {
                Ok(text) => text,
                Err(_err) => {
                    debug_log!("napcat ws text decode error: {}", _err);
                    continue;
                }
            };
            let Ok(value) = serde_json::from_str::<Value>(text) else {
                debug_log!("napcat ws invalid json: {}", text);
                continue;
            };
            if let Some(echo) = value.get("echo").and_then(|v| v.as_str()) {
                if let Some(event) = handle_action_response(&state_read, echo, &value).await {
                    debug_log!("napcat ws action response: echo={} event={:?}", echo, event);
                    let _ = cmd_tx_read.send(Command::DriverEvent(event)).await;
                }
                continue;
            }
            if let Some(command) = parse_inbound_event(
                &runtime_read,
                &state_read,
                &out_tx_read,
                &account_id_read,
                &value,
            )
            .await
            {
                debug_log!("napcat ws inbound command: {:?}", command);
                let _ = cmd_tx_read.send(command).await;
            }
        }
    });

    let mut bus_task_rx = bus_rx;
    let state_bus = Arc::clone(&state_ref);
    let runtime_bus = runtime.clone();
    let out_tx_bus = out_tx.clone();
    let account_id_bus = account_id.clone();
    let bus_task = tokio::spawn(async move {
        loop {
            let env = match bus_task_rx.recv().await {
                Ok(env) => env,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            let action = build_action_from_event(
                &runtime_bus,
                &state_bus,
                &out_tx_bus,
                &account_id_bus,
                env.event,
            )
            .await;
            if let Some(action) = action {
                debug_log!(
                    "napcat ws outbound action: group_id={} bytes={}",
                    runtime_bus.group_id,
                    action.len()
                );
                if out_tx_bus.send(action).await.is_err() {
                    debug_log!("napcat ws outbound channel closed");
                    break;
                }
            }
        }
    });

    let _ = tokio::join!(writer, reader, bus_task);
    unregister_ws_session(&account_id);
    notify_account_online_change(&runtime, &account_id, false).await;
}

fn should_send_startup_notice(group_id: &str) -> bool {
    let lock = STARTUP_NOTICE_SENT.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(group_id.to_string())
}

fn ws_sessions() -> &'static std::sync::Mutex<HashMap<String, NapCatWsSession>> {
    WS_SESSIONS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn group_accounts() -> &'static std::sync::Mutex<HashMap<String, Vec<String>>> {
    GROUP_ACCOUNTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn register_ws_session(account_id: &str, session: NapCatWsSession) {
    let mut guard = match ws_sessions().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(account_id.to_string(), session);
}

fn unregister_ws_session(account_id: &str) {
    let mut guard = match ws_sessions().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.remove(account_id);
}

fn lookup_ws_session(account_id: &str) -> Option<NapCatWsSession> {
    let guard = match ws_sessions().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.get(account_id).cloned()
}

pub fn napcat_account_online(account_id: &str) -> bool {
    lookup_ws_session(account_id).is_some()
}

fn set_group_accounts(group_id: &str, accounts: Vec<String>) {
    let mut guard = match group_accounts().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(group_id.to_string(), accounts);
}

pub fn napcat_account_for_group(group_id: &str) -> Option<String> {
    let guard = match group_accounts().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    let Some(accounts) = guard.get(group_id) else {
        return None;
    };
    for account_id in accounts {
        if lookup_ws_session(account_id).is_some() {
            return Some(account_id.clone());
        }
    }
    None
}

fn effective_primary_account(runtime: &NapCatRuntimeConfig) -> Option<String> {
    for account_id in &runtime.accounts {
        if lookup_ws_session(account_id).is_some() {
            return Some(account_id.clone());
        }
    }
    None
}

fn is_effective_primary_account(runtime: &NapCatRuntimeConfig, account_id: &str) -> bool {
    effective_primary_account(runtime).is_some_and(|value| value == account_id)
}

fn account_status_text(account_id: &str, online: bool) -> String {
    if online {
        format!("账号{}已上线", account_id)
    } else {
        format!("账号{}已离线", account_id)
    }
}

async fn notify_account_online_change(
    runtime: &NapCatRuntimeConfig,
    changed_account_id: &str,
    online: bool,
) {
    let Some(primary_account_id) = effective_primary_account(runtime) else {
        return;
    };
    let Some(session) = lookup_ws_session(&primary_account_id) else {
        return;
    };
    let target_group_id = runtime
        .audit_group_id
        .as_deref()
        .unwrap_or(&runtime.group_id);
    let text = account_status_text(changed_account_id, online);
    send_group_text(&session.out_tx, target_group_id, &text).await;
}

pub async fn napcat_ws_request(
    account_id: &str,
    action: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let session = lookup_ws_session(account_id)
        .ok_or_else(|| format!("napcat ws session missing: {}", account_id))?;
    let (resp_tx, resp_rx) = oneshot::channel();
    let echo = {
        let mut guard = session.state.lock().await;
        let echo = next_echo(&mut guard);
        guard
            .pending
            .insert(echo.clone(), PendingAction::WsRequest { resp_tx });
        echo
    };
    let payload = serde_json::json!({
        "action": action,
        "params": params,
        "echo": echo
    });
    if session.out_tx.send(payload.to_string()).await.is_err() {
        let mut guard = session.state.lock().await;
        guard.pending.remove(&echo);
        return Err("napcat ws send failed".to_string());
    }
    match tokio::time::timeout(timeout, resp_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("napcat ws response channel closed".to_string()),
        Err(_) => {
            let mut guard = session.state.lock().await;
            guard.pending.remove(&echo);
            Err("napcat ws request timeout".to_string())
        }
    }
}

async fn build_action_from_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    account_id: &str,
    event: Event,
) -> Option<String> {
    if !is_effective_primary_account(runtime, account_id) {
        return None;
    }
    match event {
        Event::Ingress(IngressEvent::InputStatusUpdated { .. }) => None,
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
            let IngressMessage { text, attachments } = message;
            let summary_text = guard
                .pending_summary
                .remove(&ingress_id)
                .unwrap_or_else(|| text.clone());
            guard.ingress_summary.insert(
                ingress_id,
                IngressSummary {
                    user_id,
                    sender_name,
                    text: summary_text,
                    attachments,
                },
            );
            None
        }
        Event::Ingress(IngressEvent::MessageIgnored { ingress_id, .. }) => {
            let mut guard = state.lock().await;
            guard.pending_summary.remove(&ingress_id);
            None
        }
        Event::Ingress(IngressEvent::MessageRecalled { ingress_id, .. }) => {
            let mut guard = state.lock().await;
            guard.pending_summary.remove(&ingress_id);
            guard.ingress_summary.remove(&ingress_id);
            for ingress_ids in guard.post_ingress.values_mut() {
                ingress_ids.retain(|id| *id != ingress_id);
            }
            None
        }
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            ingress_ids,
            group_id,
            is_safe,
            ..
        }) => {
            let mut guard = state.lock().await;
            guard.post_ingress.insert(post_id, ingress_ids);
            guard.post_group.insert(post_id, group_id);
            guard.post_safe.insert(post_id, is_safe);
            None
        }
        Event::Render(RenderEvent::RenderRequested { post_id, .. }) => {
            let mut guard = state.lock().await;
            guard.post_render_blobs.remove(&post_id);
            None
        }
        Event::Render(RenderEvent::PngReady { post_id, blob_id }) => {
            let mut guard = state.lock().await;
            let blobs = guard.post_render_blobs.entry(post_id).or_default();
            if !blobs.contains(&blob_id) {
                blobs.push(blob_id);
            }
            None
        }
        Event::Render(RenderEvent::PngBatchReady { post_id, blob_ids }) => {
            let mut guard = state.lock().await;
            let blobs = guard.post_render_blobs.entry(post_id).or_default();
            for blob_id in blob_ids {
                if !blobs.contains(&blob_id) {
                    blobs.push(blob_id);
                }
            }
            None
        }
        Event::Render(RenderEvent::RenderFailed { .. }) => None,
        Event::Review(ReviewEvent::ReviewInfoSynced {
            review_id,
            post_id,
            review_code,
        }) => {
            let mut guard = state.lock().await;
            let group_id = guard.post_group.get(&post_id).cloned().unwrap_or_default();
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                    group_id,
                },
            );
            guard.review_by_code.insert(review_code, review_id);
            guard.post_review_code.insert(post_id, review_code);
            if let Some(user_id) = resolve_post_submitter(&guard, post_id) {
                guard.review_submitter.insert(review_id, user_id);
            }
            None
        }
        Event::Media(MediaEvent::MediaFetchSucceeded {
            ingress_id,
            attachment_index,
            blob_id,
        }) => {
            let mut guard = state.lock().await;
            if let Some(summary) = guard.ingress_summary.get_mut(&ingress_id) {
                if let Some(attachment) = summary.attachments.get_mut(attachment_index) {
                    attachment.reference = MediaReference::Blob { blob_id };
                }
            }
            None
        }
        Event::Blob(BlobEvent::BlobPersisted { blob_id, path }) => {
            let mut guard = state.lock().await;
            guard.blob_paths.insert(blob_id, path.clone());
            None
        }
        Event::Blob(BlobEvent::BlobReleased { blob_id })
        | Event::Blob(BlobEvent::BlobGcRequested { blob_id }) => {
            let mut guard = state.lock().await;
            guard.blob_paths.remove(&blob_id);
            None
        }
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        }) => {
            debug_log!(
                "napcat review created: review_id={} post_id={} review_code={}",
                review_id.0,
                post_id.0,
                review_code
            );
            let mut guard = state.lock().await;
            let group_id = guard.post_group.get(&post_id).cloned().unwrap_or_default();
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                    group_id,
                },
            );
            guard.review_by_code.insert(review_code, review_id);
            guard.post_review_code.insert(post_id, review_code);
            if let Some(user_id) = resolve_post_submitter(&guard, post_id) {
                guard.review_submitter.insert(review_id, user_id);
            }
            None
        }
        Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
            post_id,
            external_code,
            ..
        }) => {
            let mut guard = state.lock().await;
            guard.post_external_code.insert(post_id, external_code);
            None
        }
        Event::Review(ReviewEvent::ReviewExternalCodeCleared { post_id }) => {
            let mut guard = state.lock().await;
            guard.post_external_code.remove(&post_id);
            None
        }
        Event::Review(ReviewEvent::ReviewPublished {
            review_id,
            audit_msg_id,
        }) => {
            debug_log!(
                "napcat review published: review_id={} audit_msg_id={}",
                review_id.0,
                audit_msg_id
            );
            let mut guard = state.lock().await;
            guard.audit_msg_to_review.insert(audit_msg_id, review_id);
            guard.review_publish_attempts.remove(&review_id);
            None
        }
        Event::Review(ReviewEvent::ReviewPublishFailed {
            review_id,
            attempt,
            error: _error,
            ..
        }) => {
            debug_log!(
                "napcat review publish failed: review_id={} attempt={} err={}",
                review_id.0,
                attempt,
                _error
            );
            let mut guard = state.lock().await;
            guard.review_publish_attempts.insert(review_id, attempt);
            None
        }
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision,
            decided_by,
            ..
        }) => {
            let should_notify_recall_deleted =
                matches!(decision, ReviewDecision::Deleted) && decided_by == "system_recall";
            let recall_group_msg = {
                let mut guard = state.lock().await;
                match decision {
                    ReviewDecision::Approved
                    | ReviewDecision::Rejected
                    | ReviewDecision::Skipped
                    | ReviewDecision::Deleted => {
                        guard.processed_reviews.insert(review_id);
                    }
                    ReviewDecision::Deferred => {
                        guard.processed_reviews.remove(&review_id);
                    }
                }
                if should_notify_recall_deleted {
                    guard.review_info.get(&review_id).map(|info| {
                        format!("发件者撤回了#{}的全部内容,已自动删除稿件", info.review_code)
                    })
                } else {
                    None
                }
            };
            if let Some(text) = recall_group_msg {
                let target_group_id = runtime
                    .audit_group_id
                    .as_deref()
                    .unwrap_or(runtime.group_id.as_str());
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(target_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                return Some(payload.to_string());
            }
            None
        }
        Event::Review(ReviewEvent::ReviewDecisionReasonRecorded { .. }) => None,
        Event::Review(ReviewEvent::ReviewSubmitterNoticeRequested {
            review_id,
            kind,
            reason,
        }) => {
            if matches!(kind, ReviewSubmitterNoticeKind::Deleted) {
                return None;
            }
            let submitter = {
                let guard = state.lock().await;
                resolve_review_submitter(&guard, review_id)
            };
            let Some((group_id, user_id)) = submitter else {
                debug_log!(
                    "napcat submitter notice skipped: missing submitter info review_id={}",
                    review_id.0
                );
                return None;
            };
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            let text = format_review_submitter_notice(kind, reason.as_deref());
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewReplyRequested { review_id, text }) => {
            if text.trim().is_empty() {
                debug_log!("napcat reply skipped: empty text");
                return None;
            }
            let submitter = {
                let guard = state.lock().await;
                resolve_review_submitter(&guard, review_id)
            };
            let Some((group_id, user_id)) = submitter else {
                debug_log!("napcat reply skipped: missing submitter info");
                return None;
            };
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewQuickReplyRequested { review_id, key }) => {
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let submitter = {
                let guard = state.lock().await;
                resolve_review_submitter(&guard, review_id)
            };
            let Some((group_id, user_id)) = submitter else {
                debug_log!("napcat quick reply skipped: missing submitter info");
                return None;
            };
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            let reply_text = {
                let guard = runtime
                    .quick_replies
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                guard.get(key).cloned()
            };
            let Some(reply_text) = reply_text else {
                let audit_group = runtime
                    .audit_group_id
                    .as_deref()
                    .unwrap_or(runtime.group_id.as_str());
                let msg = format!("快捷回复不存在：{}", key);
                send_group_text(out_tx, audit_group, &msg).await;
                return None;
            };
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&reply_text)
                }
            });
            let audit_group = runtime
                .audit_group_id
                .as_deref()
                .unwrap_or(runtime.group_id.as_str());
            let ack = format!("已发送快捷回复：{}", key);
            send_group_text(out_tx, audit_group, &ack).await;
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewBlacklistRequested { review_id, reason }) => {
            let mut guard = state.lock().await;
            let Some((group_id, sender_id)) = resolve_review_submitter(&guard, review_id) else {
                debug_log!("napcat blacklist skipped: missing review submitter");
                return None;
            };
            let entry = guard
                .blacklist
                .entry(group_id)
                .or_default()
                .entry(sender_id)
                .or_insert(None);
            if reason.is_some() {
                *entry = reason.clone();
            }
            None
        }
        Event::Review(ReviewEvent::ReviewBlacklistAdded {
            group_id,
            sender_id,
            reason,
        }) => {
            let mut guard = state.lock().await;
            let entry = guard
                .blacklist
                .entry(group_id)
                .or_default()
                .entry(sender_id)
                .or_insert(None);
            if reason.is_some() {
                *entry = reason.clone();
            }
            None
        }
        Event::Review(ReviewEvent::ReviewBlacklistRemoved {
            group_id,
            sender_id,
        }) => {
            let mut guard = state.lock().await;
            if let Some(group) = guard.blacklist.get_mut(&group_id) {
                group.remove(&sender_id);
                if group.is_empty() {
                    guard.blacklist.remove(&group_id);
                }
            }
            None
        }
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        }) => {
            let stacking_enabled = runtime.max_queue > 1;
            let (label, label_plain, code_text, submitter_id, should_notify, audit_group_id) = {
                let mut guard = state.lock().await;
                guard.send_plans.insert(
                    post_id,
                    SendPlanInfo {
                        group_id: group_id.clone(),
                        not_before_ms,
                        priority,
                        seq,
                    },
                );
                (
                    post_label(&guard, post_id),
                    post_label_plain(&guard, post_id),
                    post_code_text(&guard, post_id),
                    resolve_post_submitter(&guard, post_id),
                    group_id == runtime.group_id,
                    runtime.audit_group_id.clone(),
                )
            };
            if !should_notify {
                return None;
            }
            let Some(audit_group_id) = audit_group_id else {
                return None;
            };
            if stacking_enabled {
                if let (Some(code), Some(user_id)) = (code_text, submitter_id) {
                    let text = format!("#{}已通过审核,待发送", code);
                    let payload = serde_json::json!({
                        "action": "send_private_msg",
                        "params": {
                            "user_id": json_id(&user_id),
                            "message": message_segments_from_text(&text)
                        }
                    });
                    let _ = out_tx.send(payload.to_string()).await;
                }
                let text = format!("{}已存入暂存区", label_plain);
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(&audit_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                return Some(payload.to_string());
            }
            let text = format!("{}正在发送...", label);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(&audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        }) => {
            let mut guard = state.lock().await;
            guard.send_plans.insert(
                post_id,
                SendPlanInfo {
                    group_id,
                    not_before_ms,
                    priority,
                    seq,
                },
            );
            None
        }
        Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }) => {
            let mut guard = state.lock().await;
            guard.send_plans.remove(&post_id);
            None
        }
        Event::Send(SendEvent::SendStarted {
            post_id,
            group_id,
            started_at_ms,
            ..
        }) => {
            let stacking_enabled = runtime.max_queue > 1;
            let (batch_label, should_notify, audit_group_id) = {
                let mut guard = state.lock().await;
                let leader_priority = guard
                    .send_plans
                    .remove(&post_id)
                    .map(|plan| plan.priority)
                    .unwrap_or(SendPriority::Normal);
                let batch_posts = collect_batch_post_ids_for_notify(
                    &guard,
                    &group_id,
                    post_id,
                    leader_priority,
                    runtime.max_queue,
                );
                let batch_label = post_batch_label(&guard, &batch_posts);
                for batch_post_id in batch_posts {
                    guard.sending.insert(
                        batch_post_id,
                        SendingInfo {
                            group_id: group_id.clone(),
                            started_at_ms,
                            batch_leader: post_id,
                            batch_label: batch_label.clone(),
                        },
                    );
                }
                (
                    batch_label,
                    group_id == runtime.group_id,
                    runtime.audit_group_id.clone(),
                )
            };
            if !stacking_enabled || !should_notify {
                return None;
            }
            let Some(audit_group_id) = audit_group_id else {
                return None;
            };
            let text = format!("{}正在发送中", batch_label);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(&audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendSucceeded { post_id, .. }) => {
            let (group_id, send_code, submitter_id, _batch_label, _notify_group) = {
                let mut guard = state.lock().await;
                let sending_info = guard.sending.remove(&post_id);
                let group_id = sending_info
                    .as_ref()
                    .map(|info| info.group_id.clone())
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_else(|| runtime.group_id.clone());
                let batch_label = sending_info
                    .as_ref()
                    .map(|info| info.batch_label.clone())
                    .unwrap_or_else(|| post_label(&guard, post_id));
                let notify_group = sending_info
                    .as_ref()
                    .map(|info| info.batch_leader == post_id)
                    .unwrap_or(true);
                let send_code = post_code_text(&guard, post_id);
                let submitter_id = resolve_post_submitter(&guard, post_id);
                (group_id, send_code, submitter_id, batch_label, notify_group)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            if let (Some(code), Some(user_id)) = (send_code, submitter_id) {
                let text = format!("#{}已发送", code);
                let payload = serde_json::json!({
                    "action": "send_private_msg",
                    "params": {
                        "user_id": json_id(&user_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                let _ = out_tx.send(payload.to_string()).await;
            }
            None
        }
        Event::Send(SendEvent::SendAccountSucceeded {
            post_id,
            account_id,
            ..
        }) => {
            let (group_id, batch_label) = {
                let guard = state.lock().await;
                let group_id = guard
                    .sending
                    .get(&post_id)
                    .map(|info| info.group_id.clone())
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_else(|| runtime.group_id.clone());
                let batch_label = guard
                    .sending
                    .get(&post_id)
                    .map(|info| info.batch_label.clone())
                    .unwrap_or_else(|| post_label(&guard, post_id));
                (group_id, batch_label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let text = format!("{} {}已发送", batch_label, account_id);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendAccountFailed {
            post_id,
            account_id,
            error,
            ..
        }) => {
            let (group_id, batch_label) = {
                let guard = state.lock().await;
                let group_id = guard
                    .sending
                    .get(&post_id)
                    .map(|info| info.group_id.clone())
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_else(|| runtime.group_id.clone());
                let batch_label = guard
                    .sending
                    .get(&post_id)
                    .map(|info| info.batch_label.clone())
                    .unwrap_or_else(|| post_label(&guard, post_id));
                (group_id, batch_label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let text = format!("{} {}发送失败：{}", batch_label, account_id, error);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendFailed {
            post_id,
            account_id,
            attempt,
            error,
            ..
        }) => {
            let (group_id, label) = {
                let mut guard = state.lock().await;
                let sending_info = guard.sending.remove(&post_id);
                if let Some(ref info) = sending_info {
                    let extra_ids = guard
                        .sending
                        .iter()
                        .filter_map(|(id, item)| {
                            if item.batch_leader == info.batch_leader {
                                Some(*id)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    for id in extra_ids {
                        guard.sending.remove(&id);
                    }
                }
                let group_id = sending_info
                    .as_ref()
                    .map(|info| info.group_id.clone())
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_default();
                let label = post_label(&guard, post_id);
                (group_id, label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            if !is_send_timeout_error(&error) {
                return None;
            }
            let text = format!(
                "{} 发送超时（账号{} 第{}次）：{}",
                label, account_id, attempt, error
            );
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendGaveUp { post_id, reason }) => {
            let (group_id, label) = {
                let mut guard = state.lock().await;
                let sending_info = guard.sending.remove(&post_id);
                if let Some(ref info) = sending_info {
                    let extra_ids = guard
                        .sending
                        .iter()
                        .filter_map(|(id, item)| {
                            if item.batch_leader == info.batch_leader {
                                Some(*id)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    for id in extra_ids {
                        guard.sending.remove(&id);
                    }
                }
                let group_id = sending_info
                    .as_ref()
                    .map(|info| info.group_id.clone())
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_default();
                let label = post_label(&guard, post_id);
                (group_id, label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let text = format!("{} 发送失败已停止重试：{}", label, reason);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::QzonePostWithdrawSucceeded { post_id, .. }) => {
            let (group_id, label) = {
                let guard = state.lock().await;
                let group_id = guard.post_group.get(&post_id).cloned().unwrap_or_default();
                let label = post_code_text(&guard, post_id)
                    .map(|code| format!("#{}", code))
                    .unwrap_or_else(|| post_label(&guard, post_id));
                (group_id, label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let text = format!("{} 已从空间动态撤回图片", label);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::QzonePostWithdrawFailed { post_id, error, .. }) => {
            let (group_id, label) = {
                let guard = state.lock().await;
                let group_id = guard.post_group.get(&post_id).cloned().unwrap_or_default();
                let label = post_code_text(&guard, post_id)
                    .map(|code| format!("#{}", code))
                    .unwrap_or_else(|| post_label(&guard, post_id));
                (group_id, label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let text = format!("{} 空间撤回失败：{}", label, error);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id }) => {
            let Some(group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let mut guard = state.lock().await;
            let Some(info) = guard.review_info.get(&review_id).cloned() else {
                debug_log!("napcat review publish requested but missing review info");
                return None;
            };
            let attempt = {
                let entry = guard
                    .review_publish_attempts
                    .entry(review_id)
                    .and_modify(|value| *value = value.saturating_add(1))
                    .or_insert(1);
                *entry
            };
            let ingress_ids = guard
                .post_ingress
                .get(&info.post_id)
                .cloned()
                .unwrap_or_default();
            if let Some(user_id) = resolve_post_submitter_with_ingress(&guard, &ingress_ids) {
                guard.review_submitter.insert(review_id, user_id);
            }
            let render_blob_ids = rendered_png_blob_ids(&guard, info.post_id);
            let previews = rendered_png_previews(info.post_id, &render_blob_ids, &guard.blob_paths);
            let is_safe = guard.post_safe.get(&info.post_id).copied().unwrap_or(true);
            let summary = build_audit_message(
                info.review_code,
                info.post_id,
                &ingress_ids,
                &guard.ingress_summary,
                previews,
                &guard.blob_paths,
                is_safe,
            );
            let echo = next_echo(&mut guard);
            guard.pending.insert(
                echo.clone(),
                PendingAction::SendAuditMessage { review_id, attempt },
            );

            let mut message = message_segments_from_text(&summary.text);
            for image in summary.images {
                message.push(serde_json::json!({
                    "type": "image",
                    "data": { "file": image }
                }));
            }
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(group_id),
                    "message": message
                },
                "echo": echo
            });
            Some(payload.to_string())
        }
        _ => None,
    }
}

async fn handle_action_response(
    state: &Arc<Mutex<NapCatState>>,
    echo: &str,
    value: &Value,
) -> Option<Event> {
    let mut guard = state.lock().await;
    let pending = guard.pending.remove(echo)?;
    // OneBot/NapCat action responses look like:
    // {"status":"ok","retcode":0,"data":{...},"echo":"..."}
    // If failed (e.g. wrong group_id type/permission issues), data may be empty.
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let retcode = value.get("retcode").and_then(|v| v.as_i64()).unwrap_or(-1);
    match pending {
        PendingAction::SendAuditMessage { review_id, attempt } => {
            if status != "ok" || retcode != 0 {
                debug_log!(
                    "napcat action failed: echo={} status={} retcode={} raw={}",
                    echo,
                    status,
                    retcode,
                    value
                );
                let msg = value
                    .get("msg")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let mut error = format!("action failed status={} retcode={}", status, retcode);
                if !msg.is_empty() {
                    error.push_str(&format!(" msg={}", msg));
                }
                let retry_at_ms = now_ms().saturating_add(review_retry_delay_ms(attempt));
                return Some(Event::Review(ReviewEvent::ReviewPublishFailed {
                    review_id,
                    attempt,
                    retry_at_ms,
                    error,
                }));
            }

            let message_id = value
                .get("data")
                .and_then(|data| data.get("message_id"))
                .and_then(value_to_string);
            let Some(message_id) = message_id else {
                let retry_at_ms = now_ms().saturating_add(review_retry_delay_ms(attempt));
                return Some(Event::Review(ReviewEvent::ReviewPublishFailed {
                    review_id,
                    attempt,
                    retry_at_ms,
                    error: "missing message_id in action response".to_string(),
                }));
            };
            debug_log!(
                "napcat audit message sent: review_id={} message_id={}",
                review_id.0,
                message_id
            );
            guard
                .audit_msg_to_review
                .insert(message_id.clone(), review_id);
            Some(Event::Review(ReviewEvent::ReviewPublished {
                review_id,
                audit_msg_id: message_id,
            }))
        }
        PendingAction::WsRequest { resp_tx } => {
            if status != "ok" || retcode != 0 {
                let msg = value
                    .get("msg")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let mut error = format!("status={} retcode={}", status, retcode);
                if !msg.is_empty() {
                    error.push_str(&format!(" msg={}", msg));
                }
                let _ = resp_tx.send(Err(error));
                return None;
            }
            let _ = resp_tx.send(Ok(value.clone()));
            None
        }
    }
}

fn json_id(id: &str) -> Value {
    let trimmed = id.trim();
    if let Ok(n) = trimmed.parse::<i64>() {
        Value::Number(n.into())
    } else {
        Value::String(trimmed.to_string())
    }
}

fn is_send_timeout_error(error: &str) -> bool {
    error.starts_with("send timeout")
}

fn review_retry_delay_ms(attempt: u32) -> i64 {
    let base = 5_000i64;
    let max = 60_000i64;
    let shift = attempt.saturating_sub(1).min(10);
    let delay = base.saturating_mul(1_i64 << shift);
    delay.min(max)
}

async fn parse_inbound_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    account_id: &str,
    value: &Value,
) -> Option<Command> {
    let post_type = value.get("post_type").and_then(|v| v.as_str())?;
    if post_type == "notice" {
        return parse_notice_event(runtime, value);
    }
    if post_type == "request" {
        handle_friend_request(runtime, state, out_tx, value).await;
        return None;
    }
    if post_type != "message" && post_type != "message_sent" {
        return None;
    }

    let message_type = value.get("message_type").and_then(|v| v.as_str())?;
    debug_log!(
        "napcat inbound: post_type={} message_type={}",
        post_type,
        message_type
    );
    let user_id = value_opt_to_string(value.get("user_id"))?;
    let self_id = value_opt_to_string(value.get("self_id")).unwrap_or_else(|| "napcat".to_string());
    let message_id =
        value_opt_to_string(value.get("message_id")).unwrap_or_else(|| "0".to_string());
    let sender_name = extract_sender_name(value);
    let timestamp_ms = inbound_timestamp_ms(value);

    if message_type == "private" && (post_type == "message_sent" || user_id == self_id) {
        debug_log!(
            "napcat inbound ignored private sent/self message: post_type={} user_id={} self_id={}",
            post_type,
            user_id,
            self_id
        );
        return None;
    }

    if message_type == "group" {
        if !is_effective_primary_account(runtime, account_id) {
            return None;
        }
        let message_value = value.get("message");
        let mut forward_resolver = if message_has_forward(value.get("message")) {
            Some(ForwardResolver {
                account_id: self_id.clone(),
                cache: HashMap::new(),
                seen: HashSet::new(),
            })
        } else {
            None
        };
        let (extracted, reply_id) = extract_message(message_value, &mut forward_resolver).await;
        let ExtractedMessage {
            text,
            summary_text: _,
            attachments: _attachments,
        } = extracted;
        debug_log!(
            "napcat inbound content: text_len={} attachments={} reply_id_present={}",
            text.len(),
            _attachments.len(),
            reply_id.is_some()
        );
        let chat_group_id = value_opt_to_string(value.get("group_id"))?;
        let is_audit_group = runtime.audit_group_id.as_deref() == Some(chat_group_id.as_str());
        if runtime.audit_group_id.is_some() && !is_audit_group {
            return None;
        }
        let mentions_self = message_mentions_self(message_value, &self_id);
        let reply_bound = if let Some(reply_msg_id) = reply_id.as_ref() {
            let guard = state.lock().await;
            guard
                .audit_msg_to_review
                .contains_key(reply_msg_id.as_str())
        } else {
            false
        };
        if let Some(command) = parse_audit_command(&text, reply_id.is_some(), runtime) {
            if !command_context_allowed(&command, mentions_self, reply_bound) {
                return None;
            }
            if !is_admin_sender(value) {
                send_group_text(out_tx, &chat_group_id, "无权限执行指令").await;
                return None;
            }
            match command {
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::Help)) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    send_group_text(out_tx, &chat_group_id, HELP_TEXT).await;
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::PendingList)) => {
                    let pending_text = {
                        let guard = state.lock().await;
                        build_pending_list_text(&guard, &runtime.group_id)
                    };
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    send_group_text(out_tx, &chat_group_id, &pending_text).await;
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::BlacklistList)) => {
                    let blacklist_text = {
                        let guard = state.lock().await;
                        build_blacklist_list_text(&guard, &runtime.group_id)
                    };
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    send_group_text(out_tx, &chat_group_id, &blacklist_text).await;
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(
                    GlobalAction::BlacklistRemove { sender_id },
                )) => {
                    let removed = {
                        let mut guard = state.lock().await;
                        if let Some(group) = guard.blacklist.get_mut(&runtime.group_id) {
                            let removed = group.remove(&sender_id).is_some();
                            if group.is_empty() {
                                guard.blacklist.remove(&runtime.group_id);
                            }
                            removed
                        } else {
                            false
                        }
                    };
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let text = if removed {
                        format!("已取消拉黑 {}", sender_id)
                    } else {
                        format!("黑名单中不存在 {}", sender_id)
                    };
                    send_group_text(out_tx, &chat_group_id, &text).await;
                    return Some(Command::GlobalAction(GlobalActionCommand {
                        group_id: runtime.group_id.clone(),
                        action: GlobalAction::BlacklistRemove { sender_id },
                        operator_id: user_id.to_string(),
                        now_ms: timestamp_ms,
                        tz_offset_minutes: runtime.tz_offset_minutes,
                    }));
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::QuickReplyList)) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let list_text = build_quick_reply_list_text(runtime);
                    send_group_text(out_tx, &chat_group_id, &list_text).await;
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(
                    GlobalAction::QuickReplyAdd { key, text },
                )) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let key = key.trim().to_string();
                    let text = text.trim().to_string();
                    if key.is_empty() || text.is_empty() {
                        send_group_text(out_tx, &chat_group_id, "错误：快捷回复键和值均不能为空")
                            .await;
                        return None;
                    }
                    if quick_reply_key_conflicts(&key) {
                        send_group_text(
                            out_tx,
                            &chat_group_id,
                            "错误：快捷回复指令与审核指令冲突，请更换指令名",
                        )
                        .await;
                        return None;
                    }
                    let review_shortcut_conflict = {
                        let guard = runtime
                            .review_shortcuts
                            .lock()
                            .unwrap_or_else(|err| err.into_inner());
                        guard.contains_key(&key)
                    };
                    if review_shortcut_conflict {
                        send_group_text(
                            out_tx,
                            &chat_group_id,
                            "错误：快捷回复指令与审核快捷指令冲突，请更换指令名",
                        )
                        .await;
                        return None;
                    }
                    let mut snapshot = {
                        let mut guard = runtime
                            .quick_replies
                            .lock()
                            .unwrap_or_else(|err| err.into_inner());
                        guard.insert(key.clone(), text.clone());
                        guard.clone()
                    };
                    sort_quick_reply_map(&mut snapshot);
                    match persist_group_quick_replies(&runtime.group_id, &snapshot) {
                        Ok(()) => {
                            let msg = format!("已添加快捷回复：{}", key);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                        Err(err) => {
                            {
                                let mut guard = runtime
                                    .quick_replies
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                guard.remove(&key);
                            }
                            let msg = format!("添加快捷回复失败：{}", err);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                    }
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(
                    GlobalAction::QuickReplyDelete { key },
                )) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let key = key.trim().to_string();
                    if key.is_empty() {
                        send_group_text(out_tx, &chat_group_id, "错误：请提供要删除的快捷回复")
                            .await;
                        return None;
                    }
                    let removed_snapshot = {
                        let mut guard = runtime
                            .quick_replies
                            .lock()
                            .unwrap_or_else(|err| err.into_inner());
                        let removed = guard.remove(&key);
                        (removed, guard.clone())
                    };
                    if removed_snapshot.0.is_none() {
                        let msg = format!("快捷回复不存在：{}", key);
                        send_group_text(out_tx, &chat_group_id, &msg).await;
                        return None;
                    }
                    let mut sorted = removed_snapshot.1;
                    sort_quick_reply_map(&mut sorted);
                    match persist_group_quick_replies(&runtime.group_id, &sorted) {
                        Ok(()) => {
                            let msg = format!("已删除快捷回复：{}", key);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                        Err(err) => {
                            if let Some(removed_text) = removed_snapshot.0 {
                                {
                                    let mut guard = runtime
                                        .quick_replies
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    guard.insert(key.clone(), removed_text);
                                }
                            }
                            let msg = format!("删除快捷回复失败：{}", err);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                    }
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::ShortcutList)) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let list_text = build_shortcut_list_text(runtime);
                    send_group_text(out_tx, &chat_group_id, &list_text).await;
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::ShortcutAdd {
                    scope,
                    key,
                    definition,
                })) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let key = match validate_shortcut_name(&key) {
                        Ok(value) => value,
                        Err(err) => {
                            let msg = format!(
                                "错误：{}快捷指令名无效：{}",
                                shortcut_scope_label(scope),
                                err
                            );
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                            return None;
                        }
                    };
                    let definition = definition.trim().to_string();
                    let validate_result = match scope {
                        ShortcutScope::Review => {
                            let quick_reply_conflict = {
                                let guard = runtime
                                    .quick_replies
                                    .lock()
                                    .unwrap_or_else(|err| err.into_inner());
                                guard.contains_key(&key)
                            };
                            if quick_reply_conflict {
                                Err("审核快捷指令与快捷回复重名".to_string())
                            } else {
                                validate_review_shortcut_definition(&definition)
                            }
                        }
                        ShortcutScope::Global => validate_global_shortcut_definition(&definition),
                    };
                    if let Err(err) = validate_result {
                        let msg = format!(
                            "错误：{}快捷指令定义无效：{}",
                            shortcut_scope_label(scope),
                            err
                        );
                        send_group_text(out_tx, &chat_group_id, &msg).await;
                        return None;
                    }
                    let mut snapshot = {
                        let storage = shortcut_storage(runtime, scope);
                        let mut guard = storage.lock().unwrap_or_else(|err| err.into_inner());
                        guard.insert(key.clone(), definition.clone());
                        guard.clone()
                    };
                    sort_string_map(&mut snapshot);
                    match persist_group_string_map(
                        &runtime.group_id,
                        shortcut_field_name(scope),
                        &snapshot,
                    ) {
                        Ok(()) => {
                            let msg =
                                format!("已添加{}快捷指令：{}", shortcut_scope_label(scope), key);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                        Err(err) => {
                            {
                                let storage = shortcut_storage(runtime, scope);
                                let mut guard = storage.lock().unwrap_or_else(|e| e.into_inner());
                                guard.remove(&key);
                            }
                            let msg = format!("添加快捷指令失败：{}", err);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                    }
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(
                    GlobalAction::ShortcutDelete { scope, key },
                )) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let key = key.trim().to_string();
                    if key.is_empty() {
                        send_group_text(out_tx, &chat_group_id, "错误：请提供要删除的快捷指令")
                            .await;
                        return None;
                    }
                    let removed_snapshot = {
                        let storage = shortcut_storage(runtime, scope);
                        let mut guard = storage.lock().unwrap_or_else(|err| err.into_inner());
                        let removed = guard.remove(&key);
                        (removed, guard.clone())
                    };
                    if removed_snapshot.0.is_none() {
                        let msg = format!("{}快捷指令不存在：{}", shortcut_scope_label(scope), key);
                        send_group_text(out_tx, &chat_group_id, &msg).await;
                        return None;
                    }
                    let mut sorted = removed_snapshot.1;
                    sort_string_map(&mut sorted);
                    match persist_group_string_map(
                        &runtime.group_id,
                        shortcut_field_name(scope),
                        &sorted,
                    ) {
                        Ok(()) => {
                            let msg =
                                format!("已删除{}快捷指令：{}", shortcut_scope_label(scope), key);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                        Err(err) => {
                            if let Some(removed_definition) = removed_snapshot.0 {
                                let storage = shortcut_storage(runtime, scope);
                                {
                                    let mut guard =
                                        storage.lock().unwrap_or_else(|e| e.into_inner());
                                    guard.insert(key.clone(), removed_definition);
                                }
                            }
                            let msg = format!("删除快捷指令失败：{}", err);
                            send_group_text(out_tx, &chat_group_id, &msg).await;
                        }
                    }
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::SelfCheck)) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let report = {
                        let guard = state.lock().await;
                        build_selfcheck_report(runtime, &guard)
                    };
                    send_group_text(out_tx, &chat_group_id, &report).await;
                    return None;
                }
                AuditCommand::Global(ParsedGlobalAction::Builtin(action)) => {
                    {
                        let mut guard = state.lock().await;
                        if let Err(msg) = validate_global_action(&guard, &runtime.group_id, &action)
                        {
                            drop(guard);
                            send_group_text(out_tx, &chat_group_id, msg).await;
                            return None;
                        }
                        if let GlobalAction::Recall { review_code } = &action {
                            if let Some(review_id) = guard.review_by_code.get(review_code).copied()
                            {
                                guard.processed_reviews.remove(&review_id);
                            }
                        }
                    }
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    return Some(Command::GlobalAction(GlobalActionCommand {
                        group_id: runtime.group_id.clone(),
                        action,
                        operator_id: user_id.to_string(),
                        now_ms: timestamp_ms,
                        tz_offset_minutes: runtime.tz_offset_minutes,
                    }));
                }
                AuditCommand::Global(ParsedGlobalAction::Batch(actions)) => {
                    {
                        let mut guard = state.lock().await;
                        for action in &actions {
                            if let Err(msg) =
                                validate_global_action(&guard, &runtime.group_id, action)
                            {
                                drop(guard);
                                send_group_text(out_tx, &chat_group_id, msg).await;
                                return None;
                            }
                            if let GlobalAction::Recall { review_code } = action {
                                if let Some(review_id) =
                                    guard.review_by_code.get(review_code).copied()
                                {
                                    guard.processed_reviews.remove(&review_id);
                                }
                            }
                        }
                    }
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    return Some(Command::GlobalActionBatch(GlobalActionBatchCommand {
                        group_id: runtime.group_id.clone(),
                        actions,
                        operator_id: user_id.to_string(),
                        now_ms: timestamp_ms,
                        tz_offset_minutes: runtime.tz_offset_minutes,
                    }));
                }
                AuditCommand::Review {
                    review_code,
                    action,
                } => {
                    return parse_review_command(
                        runtime,
                        state,
                        out_tx,
                        &user_id,
                        &self_id,
                        &chat_group_id,
                        review_code,
                        action,
                        reply_id,
                        timestamp_ms,
                    )
                    .await;
                }
            }
        }
        if is_audit_group {
            return None;
        }
        debug_log!(
            "napcat inbound ignored group message for ingress: group_id={}",
            chat_group_id
        );
        return None;
    }

    if message_type == "private" {
        let raw_message = value.get("raw_message").and_then(|v| v.as_str());
        let raw_trimmed = raw_message.unwrap_or("").trim();

        if raw_trimmed == "#开始投稿" {
            {
                let mut guard = state.lock().await;
                guard.submission_sessions.insert(
                    user_id.clone(),
                    SubmissionSession {
                        messages: Vec::new(),
                        started_at_ms: timestamp_ms,
                        group_id: runtime.group_id.clone(),
                        confirming: false,
                    },
                );
            }
            send_private_text(
                out_tx,
                &user_id,
                "投稿会话已开始，请发送稿件内容。完成后发送 #结束投稿",
            )
            .await;
            return None;
        }

        {
            let mut guard = state.lock().await;
            if guard.submission_sessions.contains_key(&user_id) {
                if raw_trimmed == "#结束投稿" {
                    let count = guard
                        .submission_sessions
                        .get_mut(&user_id)
                        .map(|session| {
                            session.confirming = true;
                            session.messages.len()
                        })
                        .unwrap_or(0);
                    drop(guard);
                    send_private_text(
                        out_tx,
                        &user_id,
                        &format!(
                            "收到共 {} 条消息。\n发送 #确认 提交投稿\n发送 #追加 继续添加内容\n发送 #取消 放弃本次投稿",
                            count
                        ),
                    )
                    .await;
                    return None;
                }
                if raw_trimmed == "#取消" {
                    guard.submission_sessions.remove(&user_id);
                    drop(guard);
                    send_private_text(out_tx, &user_id, "投稿已取消。").await;
                    return None;
                }
                if raw_trimmed == "#确认" {
                    let Some(session_meta) = guard.submission_sessions.get(&user_id) else {
                        return None;
                    };
                    if !session_meta.confirming {
                        drop(guard);
                        send_private_text(out_tx, &user_id, "请先发送 #结束投稿 再确认。").await;
                        return None;
                    }
                    if session_meta.messages.is_empty() {
                        guard.submission_sessions.remove(&user_id);
                        drop(guard);
                        send_private_text(out_tx, &user_id, "没有可提交的内容。").await;
                        return None;
                    }

                    let session = guard
                        .submission_sessions
                        .remove(&user_id)
                        .expect("submission session exists");
                    let first_msg_id =
                        value_opt_to_string(session.messages[0].message.get("message_id"))
                            .unwrap_or_else(|| format!("submission-{}", session.started_at_ms));
                    let sender_name = extract_sender_name(&session.messages[0].message)
                        .or_else(|| Some(user_id.clone()));
                    let chat_id = format!("{}_submission_{}", user_id, session.started_at_ms);
                    let mut combined_text = String::new();
                    let mut combined_attachments = Vec::new();
                    let mut combined_summary = String::new();
                    for buffered in &session.messages {
                        let ExtractedMessage {
                            text,
                            summary_text,
                            attachments,
                        } = extract_message_lite(buffered.message.get("message"));
                        if !text.trim().is_empty() {
                            if !combined_text.is_empty() {
                                combined_text.push_str("\n\n");
                            }
                            combined_text.push_str(text.trim());
                        }
                        if !summary_text.trim().is_empty() {
                            if !combined_summary.is_empty() {
                                combined_summary.push_str("\n\n");
                            }
                            combined_summary.push_str(summary_text.trim());
                        }
                        combined_attachments.extend(attachments);
                    }
                    if combined_summary.is_empty() {
                        combined_summary = format!("[{} 条投稿消息]", session.messages.len());
                    }
                    let ingress_id = derive_ingress_id(&[
                        self_id.as_bytes(),
                        chat_id.as_bytes(),
                        user_id.as_bytes(),
                        first_msg_id.as_bytes(),
                    ]);
                    guard.pending_summary.insert(ingress_id, combined_summary);
                    let count = session.messages.len();
                    let group_id = session.group_id;
                    let started_at_ms = session.started_at_ms;
                    drop(guard);
                    send_private_text(
                        out_tx,
                        &user_id,
                        &format!("投稿已提交，共 {} 条消息，请等待审核。", count),
                    )
                    .await;
                    return Some(Command::Ingress(IngressCommand {
                        profile_id: self_id,
                        chat_id,
                        user_id: user_id.clone(),
                        sender_name,
                        group_id,
                        platform_msg_id: first_msg_id,
                        message: IngressMessage {
                            text: combined_text,
                            attachments: combined_attachments,
                        },
                        received_at_ms: started_at_ms,
                        close_immediately: true,
                    }));
                }
                if raw_trimmed == "#追加" {
                    if let Some(session) = guard.submission_sessions.get_mut(&user_id) {
                        session.confirming = false;
                    }
                    drop(guard);
                    send_private_text(
                        out_tx,
                        &user_id,
                        "继续投稿，请发送更多内容。完成后发送 #结束投稿",
                    )
                    .await;
                    return None;
                }
                if guard
                    .submission_sessions
                    .get(&user_id)
                    .map(|session| session.confirming)
                    .unwrap_or(false)
                {
                    drop(guard);
                    send_private_text(out_tx, &user_id, "请先发送 #确认、#取消 或 #追加。").await;
                    return None;
                }
                let count = guard
                    .submission_sessions
                    .get_mut(&user_id)
                    .map(|session| {
                        session.messages.push(BufferedMessage {
                            message: value.clone(),
                        });
                        session.messages.len()
                    })
                    .unwrap_or(0);
                drop(guard);
                send_private_text(
                    out_tx,
                    &user_id,
                    &format!("已收到第 {} 条消息。继续发送或发送 #结束投稿 完成。", count),
                )
                .await;
                return None;
            }
        }

        if let Some(raw_message) = raw_message {
            if is_auto_reply_message(raw_message) {
                debug_log!("napcat inbound ignored private system message");
                return None;
            }
        }
        let ExtractedMessage {
            text,
            summary_text,
            attachments,
        } = extract_message_lite(value.get("message"));
        if raw_message.map(|raw| raw.is_empty()).unwrap_or(true)
            && is_auto_reply_message(&summary_text)
        {
            debug_log!("napcat inbound ignored private system message");
            return None;
        }
        debug_log!(
            "napcat inbound private lite: text_len={} attachments={}",
            text.len(),
            attachments.len()
        );
        let ingress_id = derive_ingress_id(&[
            self_id.as_bytes(),
            user_id.as_bytes(),
            user_id.as_bytes(),
            message_id.as_bytes(),
        ]);
        let suppress_text = match raw_message {
            Some(raw) if !raw.is_empty() => raw,
            _ => summary_text.as_str(),
        };
        {
            let mut guard = state.lock().await;
            if should_suppress_private_message(
                &mut guard.friend_suppression,
                &user_id,
                suppress_text,
                now_ms(),
            ) {
                debug_log!("napcat inbound private suppressed after friend request");
                return None;
            }
            guard.pending_summary.insert(ingress_id, summary_text);
        }
        return Some(Command::Ingress(IngressCommand {
            profile_id: self_id,
            chat_id: user_id.clone(),
            user_id,
            sender_name,
            group_id: runtime.group_id.clone(),
            platform_msg_id: message_id,
            message: IngressMessage { text, attachments },
            received_at_ms: timestamp_ms,
            close_immediately: false,
        }));
    }

    None
}

async fn handle_friend_request(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    value: &Value,
) {
    let request_type = value.get("request_type").and_then(|v| v.as_str());
    if request_type != Some("friend") {
        return;
    }

    let user_id = value_opt_to_string(value.get("user_id")).unwrap_or_default();
    let flag = value_opt_to_string(value.get("flag")).unwrap_or_default();
    let self_id = value_opt_to_string(value.get("self_id")).unwrap_or_default();
    let comment = value
        .get("comment")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if !is_digits(&user_id, FRIEND_REQUEST_ID_MAX_LEN)
        || !is_digits(&flag, FRIEND_REQUEST_ID_MAX_LEN)
        || !is_digits(&self_id, FRIEND_REQUEST_ID_MAX_LEN)
    {
        debug_log!(
            "napcat friend request ignored: invalid identifiers user_id={} flag={} self_id={}",
            user_id,
            flag,
            self_id
        );
        return;
    }

    let window_ms = runtime.friend_request_window_sec.saturating_mul(1000) as i64;
    if window_ms > 0 {
        let now_ms = now_ms();
        let mut guard = state.lock().await;
        if !should_process_friend_request(&mut guard.friend_req_cache, &user_id, now_ms, window_ms)
        {
            debug_log!(
                "napcat friend request ignored: duplicate user_id={} window_sec={}",
                user_id,
                runtime.friend_request_window_sec
            );
            return;
        }
        if !comment.is_empty() {
            add_friend_request_suppression(
                &mut guard.friend_suppression,
                &user_id,
                &comment,
                now_ms,
                window_ms,
            );
        }
    }

    let approve_delay_sec = friend_request_delay_sec();
    let friend_add_message = runtime.friend_add_message.clone().and_then(|msg| {
        if msg.trim().is_empty() {
            None
        } else {
            Some(msg)
        }
    });
    let out_tx = out_tx.clone();
    tokio::spawn(async move {
        if approve_delay_sec > 0 {
            sleep(Duration::from_secs(approve_delay_sec)).await;
        }
        let approve_payload = serde_json::json!({
            "action": "set_friend_add_request",
            "params": {
                "flag": flag,
                "approve": true
            }
        });
        let _ = out_tx.send(approve_payload.to_string()).await;
        if let Some(text) = friend_add_message {
            sleep(Duration::from_secs(FRIEND_NOTIFY_DELAY_SEC)).await;
            let message_payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&text)
                }
            });
            let _ = out_tx.send(message_payload.to_string()).await;
        }
    });
}

fn is_auto_reply_message(text: &str) -> bool {
    text.contains("自动回复")
        || text.contains("请求添加你为好友")
        || text.contains("我们已成功添加为好友")
}

fn is_digits(value: &str, max_len: usize) -> bool {
    !value.is_empty() && value.len() <= max_len && value.chars().all(|ch| ch.is_ascii_digit())
}

fn is_digits_unbounded(value: &str) -> bool {
    is_digits(value, usize::MAX)
}

fn normalize_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        if FRIEND_SUPPRESS_REMOVE_CHARS.contains(ch) {
            continue;
        }
        out.push(ch);
    }
    out
}

fn should_process_friend_request(
    cache: &mut HashMap<String, i64>,
    user_id: &str,
    now_ms: i64,
    window_ms: i64,
) -> bool {
    if user_id.is_empty() || window_ms <= 0 {
        return true;
    }
    cache.retain(|_, exp| *exp > now_ms);
    if let Some(expire_at) = cache.get(user_id) {
        if *expire_at > now_ms {
            return false;
        }
    }
    cache.insert(user_id.to_string(), now_ms.saturating_add(window_ms));
    true
}

fn add_friend_request_suppression(
    cache: &mut HashMap<String, Vec<SuppressionEntry>>,
    user_id: &str,
    comment: &str,
    now_ms: i64,
    window_ms: i64,
) {
    if user_id.is_empty() || comment.is_empty() || window_ms <= 0 {
        return;
    }
    let normalized = normalize_text(comment);
    if normalized.is_empty() {
        return;
    }
    let entry = SuppressionEntry {
        comment_norm: normalized,
        expire_at_ms: now_ms.saturating_add(window_ms),
    };
    let list = cache.entry(user_id.to_string()).or_default();
    list.push(entry);
    list.retain(|item| item.expire_at_ms > now_ms);
}

fn should_suppress_private_message(
    cache: &mut HashMap<String, Vec<SuppressionEntry>>,
    user_id: &str,
    text: &str,
    now_ms: i64,
) -> bool {
    if user_id.is_empty() || text.is_empty() {
        return false;
    }
    let normalized = normalize_text(text);
    if normalized.is_empty() {
        return false;
    }
    let Some(list) = cache.get_mut(user_id) else {
        return false;
    };
    list.retain(|item| item.expire_at_ms > now_ms);
    list.iter().any(|item| item.comment_norm == normalized)
}

fn friend_request_delay_sec() -> u64 {
    if FRIEND_APPROVE_DELAY_MAX_SEC == 0 {
        return 0;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    (nanos % (FRIEND_APPROVE_DELAY_MAX_SEC as u128 + 1)) as u64
}

fn parse_notice_event(runtime: &NapCatRuntimeConfig, value: &Value) -> Option<Command> {
    let notice_type = value.get("notice_type").and_then(|v| v.as_str());
    let sub_type = value.get("sub_type").and_then(|v| v.as_str());
    if matches!(notice_type, Some("friend_recall")) || matches!(sub_type, Some("friend_recall")) {
        let user_id = value_opt_to_string(value.get("user_id")).or_else(|| {
            value
                .get("data")
                .and_then(|data| value_opt_to_string(data.get("user_id")))
        })?;
        let message_id = value_opt_to_string(value.get("message_id")).or_else(|| {
            value
                .get("data")
                .and_then(|data| value_opt_to_string(data.get("message_id")))
        })?;
        let profile_id =
            value_opt_to_string(value.get("self_id")).unwrap_or_else(|| "napcat".to_string());
        let ingress_id = derive_ingress_id(&[
            profile_id.as_bytes(),
            user_id.as_bytes(),
            user_id.as_bytes(),
            message_id.as_bytes(),
        ]);
        return Some(Command::DriverEvent(Event::Ingress(
            IngressEvent::MessageRecalled {
                ingress_id,
                recalled_at_ms: inbound_timestamp_ms(value),
            },
        )));
    }

    let is_input_status = (matches!(notice_type, Some("notify"))
        && matches!(sub_type, Some("input_status")))
        || matches!(notice_type, Some("input_status"))
        || matches!(sub_type, Some("input_status"));
    if !is_input_status {
        return None;
    }

    let user_id = value_opt_to_string(value.get("user_id")).or_else(|| {
        value
            .get("data")
            .and_then(|data| value_opt_to_string(data.get("user_id")))
    })?;
    let status_raw = value_opt_to_u8(value.get("event_type"))
        .or_else(|| {
            value
                .get("status")
                .and_then(|status| value_opt_to_u8(status.get("event_type")))
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| value_opt_to_u8(data.get("event_type")))
        })?;
    let status = match status_raw {
        0 => InputStatusKind::Speaking,
        1 => InputStatusKind::Typing,
        2 => InputStatusKind::Stopped,
        other => InputStatusKind::Unknown(other),
    };
    let profile_id =
        value_opt_to_string(value.get("self_id")).unwrap_or_else(|| "napcat".to_string());
    let timestamp_ms = inbound_timestamp_ms(value);

    Some(Command::DriverEvent(Event::Ingress(
        IngressEvent::InputStatusUpdated {
            profile_id,
            chat_id: user_id.clone(),
            user_id,
            group_id: runtime.group_id.clone(),
            status,
            received_at_ms: timestamp_ms,
        },
    )))
}

fn message_has_forward(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => items.iter().any(|item| {
            item.get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|kind| kind == "forward")
        }),
        _ => false,
    }
}

fn forward_placeholder(id: &str) -> String {
    if id.is_empty() {
        "[合并转发]".to_string()
    } else {
        format!("[合并转发:{}]", id)
    }
}

fn push_chunk(
    chunks: &mut Vec<MessageChunk>,
    text: &mut String,
    summary_text: &mut String,
    attachments: &mut Vec<IngressAttachment>,
) {
    let text_value = text.trim().to_string();
    let summary_value = summary_text.trim().to_string();
    let attachments_value = std::mem::take(attachments);
    if !text_value.is_empty() || !summary_value.is_empty() || !attachments_value.is_empty() {
        chunks.push(MessageChunk {
            text: text_value,
            summary_text: summary_value,
            attachments: attachments_value,
        });
    }
    text.clear();
    summary_text.clear();
}

fn extract_message_chunks<'a>(
    value: Option<&'a Value>,
    mut resolver: Option<&'a mut ForwardResolver>,
    depth: u32,
    capture_reply: bool,
) -> Pin<Box<dyn Future<Output = (Vec<MessageChunk>, Option<String>)> + Send + 'a>> {
    Box::pin(async move {
        let mut chunks = Vec::new();
        let mut text = String::new();
        let mut summary_text = String::new();
        let mut attachments = Vec::new();
        let mut reply_id = None;

        match value {
            Some(Value::String(s)) => {
                let extracted = extract_cq_faces(s);
                text.push_str(&extracted);
                summary_text.push_str(&extracted);
            }
            Some(Value::Array(items)) => {
                for item in items {
                    let segment_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let data = item.get("data");
                    match segment_type {
                        "text" => {
                            if let Some(segment) =
                                data.and_then(|d| d.get("text")).and_then(|v| v.as_str())
                            {
                                text.push_str(segment);
                                summary_text.push_str(segment);
                            }
                        }
                        "reply" => {
                            if capture_reply {
                                if let Some(id) =
                                    data.and_then(|d| d.get("id")).and_then(value_to_string)
                                {
                                    reply_id = Some(id);
                                }
                            }
                        }
                        "face" => {
                            if let Some(id) =
                                data.and_then(|d| d.get("id")).and_then(value_to_string)
                            {
                                let placeholder = face_inline_placeholder(&id)
                                    .unwrap_or_else(|| format!("[face:{}]", id));
                                text.push_str(&placeholder);
                                summary_text.push_str(&placeholder);
                            }
                        }
                        "json" => {
                            let raw = data
                                .and_then(|d| d.get("data"))
                                .and_then(value_to_string)
                                .or_else(|| data.and_then(|d| serde_json::to_string(d).ok()))
                                .unwrap_or_default();
                            text.push_str(&json_card_marker(&raw));
                            summary_text.push_str("[卡片]");
                        }
                        "forward" => {
                            let id = data
                                .and_then(|d| d.get("id"))
                                .and_then(value_to_string)
                                .unwrap_or_default();
                            push_chunk(&mut chunks, &mut text, &mut summary_text, &mut attachments);
                            if let Some(resolver) = resolver.as_mut() {
                                let mut resolved =
                                    resolve_forward_chunks(&id, resolver, depth).await;
                                chunks.append(&mut resolved);
                            } else {
                                let placeholder = forward_placeholder(&id);
                                chunks.push(MessageChunk {
                                    text: placeholder.clone(),
                                    summary_text: placeholder,
                                    attachments: Vec::new(),
                                });
                            }
                        }
                        "poke" => {
                            text.push_str(poke_marker());
                            summary_text.push_str("[戳一戳]");
                        }
                        "image" => {
                            let kind = image_kind_from_data(data);
                            if let Some(reference) = extract_reference(data) {
                                attachments.push(IngressAttachment {
                                    kind,
                                    name: attachment_name_from_data(data),
                                    reference,
                                    size_bytes: extract_attachment_size(data),
                                });
                            } else {
                                summary_text.push_str(attachment_placeholder(kind));
                            }
                        }
                        "video" | "file" | "record" => {
                            if let Some(reference) = extract_reference(data) {
                                attachments.push(IngressAttachment {
                                    kind: match segment_type {
                                        "video" => MediaKind::Video,
                                        "file" => file_segment_kind(data),
                                        "record" => MediaKind::Audio,
                                        _ => MediaKind::Other,
                                    },
                                    name: attachment_name_from_data(data),
                                    reference,
                                    size_bytes: extract_attachment_size(data),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        push_chunk(&mut chunks, &mut text, &mut summary_text, &mut attachments);
        (chunks, reply_id)
    })
}

async fn resolve_forward_chunks(
    forward_id: &str,
    resolver: &mut ForwardResolver,
    depth: u32,
) -> Vec<MessageChunk> {
    if forward_id.is_empty() || depth >= MAX_FORWARD_DEPTH {
        let placeholder = forward_placeholder(forward_id);
        return vec![MessageChunk {
            text: placeholder.clone(),
            summary_text: placeholder,
            attachments: Vec::new(),
        }];
    }

    if let Some(cached) = resolver.cache.get(forward_id) {
        return cached.clone();
    }
    if resolver.seen.contains(forward_id) {
        let placeholder = forward_placeholder(forward_id);
        return vec![MessageChunk {
            text: placeholder.clone(),
            summary_text: placeholder,
            attachments: Vec::new(),
        }];
    }
    resolver.seen.insert(forward_id.to_string());

    let resolved = match fetch_forward_messages(resolver, forward_id).await {
        Ok(messages) => forward_messages_to_chunks(&messages, resolver, depth + 1).await,
        Err(_err) => {
            debug_log!("forward resolve failed: id={} err={}", forward_id, _err);
            let placeholder = forward_placeholder(forward_id);
            vec![MessageChunk {
                text: placeholder.clone(),
                summary_text: placeholder,
                attachments: Vec::new(),
            }]
        }
    };
    resolver
        .cache
        .insert(forward_id.to_string(), resolved.clone());
    resolved
}

async fn fetch_forward_messages(
    resolver: &ForwardResolver,
    forward_id: &str,
) -> Result<Vec<Value>, String> {
    let body = napcat_ws_request(
        &resolver.account_id,
        "get_forward_msg",
        json!({ "message_id": forward_id }),
        Duration::from_secs(6),
    )
    .await?;
    let messages = body
        .get("data")
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing forward messages".to_string())?;
    Ok(messages.to_vec())
}

async fn forward_messages_to_chunks(
    messages: &[Value],
    resolver: &mut ForwardResolver,
    depth: u32,
) -> Vec<MessageChunk> {
    let mut chunks = Vec::new();
    for message in messages {
        let payload = message.get("message").or_else(|| message.get("content"));
        let (mut msg_chunks, _) =
            extract_message_chunks(payload, Some(&mut *resolver), depth, false).await;
        chunks.append(&mut msg_chunks);
    }
    chunks
}

async fn extract_message(
    value: Option<&Value>,
    resolver: &mut Option<ForwardResolver>,
) -> (ExtractedMessage, Option<String>) {
    let (chunks, reply_id) = extract_message_chunks(value, resolver.as_mut(), 0, true).await;
    let mut parts = Vec::new();
    let mut summary_parts = Vec::new();
    let mut attachments = Vec::new();
    for chunk in chunks {
        if !chunk.text.is_empty() {
            parts.push(chunk.text);
        }
        if !chunk.summary_text.is_empty() {
            summary_parts.push(chunk.summary_text);
        }
        attachments.extend(chunk.attachments);
    }
    let text = parts.join("\n\n");
    let summary_text = summary_parts.join("\n\n");
    (
        ExtractedMessage {
            text: text.trim().to_string(),
            summary_text: summary_text.trim().to_string(),
            attachments,
        },
        reply_id,
    )
}

pub(crate) fn extract_message_lite(value: Option<&Value>) -> ExtractedMessage {
    let mut text = String::new();
    let mut summary_text = String::new();
    let mut attachments = Vec::new();

    match value {
        Some(Value::String(s)) => {
            let extracted = extract_cq_faces(s);
            text.push_str(&extracted);
            summary_text.push_str(&extracted);
        }
        Some(Value::Array(items)) => {
            for item in items {
                let segment_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let data = item.get("data");
                match segment_type {
                    "text" => {
                        if let Some(segment) =
                            data.and_then(|d| d.get("text")).and_then(|v| v.as_str())
                        {
                            text.push_str(segment);
                            summary_text.push_str(segment);
                        }
                    }
                    "reply" => {
                        let id = data.and_then(|d| d.get("id")).and_then(value_to_string);
                        let body = id
                            .as_ref()
                            .map(|id| format!("引用的消息 ID: {}", id))
                            .unwrap_or_else(|| "引用的消息".to_string());
                        text.push_str(&reply_marker(&ReplyPreview {
                            id: id.clone(),
                            meta: None,
                            body,
                            missing: false,
                        }));
                        if let Some(id) = id {
                            summary_text.push_str(&format!("[回复:{}]", id));
                        } else {
                            summary_text.push_str("[回复]");
                        }
                    }
                    "face" => {
                        if let Some(id) = data.and_then(|d| d.get("id")).and_then(value_to_string) {
                            let placeholder = face_inline_placeholder(&id)
                                .unwrap_or_else(|| format!("[face:{}]", id));
                            text.push_str(&placeholder);
                            summary_text.push_str(&placeholder);
                        }
                    }
                    "json" => {
                        let raw = data
                            .and_then(|d| d.get("data"))
                            .and_then(value_to_string)
                            .or_else(|| data.and_then(|d| serde_json::to_string(d).ok()))
                            .unwrap_or_default();
                        text.push_str(&json_card_marker(&raw));
                        summary_text.push_str("[卡片]");
                    }
                    "forward" => {
                        if let Some(id) = data.and_then(|d| d.get("id")).and_then(value_to_string) {
                            text.push_str(&format!("[合并转发:{}]", id));
                            summary_text.push_str(&format!("[合并转发:{}]", id));
                        } else {
                            text.push_str("[合并转发]");
                            summary_text.push_str("[合并转发]");
                        }
                    }
                    "poke" => {
                        text.push_str(poke_marker());
                        summary_text.push_str("[戳一戳]");
                    }
                    "image" => {
                        let kind = image_kind_from_data(data);
                        if let Some(reference) = extract_reference(data) {
                            attachments.push(IngressAttachment {
                                kind,
                                name: attachment_name_from_data(data),
                                reference,
                                size_bytes: extract_attachment_size(data),
                            });
                        } else {
                            summary_text.push_str(attachment_placeholder(kind));
                        }
                    }
                    "video" | "file" | "record" => {
                        if segment_type == "record" {
                            text.push_str("[语音]");
                            summary_text.push_str("[语音]");
                        }
                        if let Some(reference) = extract_reference(data) {
                            attachments.push(IngressAttachment {
                                kind: match segment_type {
                                    "video" => MediaKind::Video,
                                    "file" => file_segment_kind(data),
                                    "record" => MediaKind::Audio,
                                    _ => MediaKind::Other,
                                },
                                name: attachment_name_from_data(data),
                                reference,
                                size_bytes: extract_attachment_size(data),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    ExtractedMessage {
        text: text.trim().to_string(),
        summary_text: summary_text.trim().to_string(),
        attachments,
    }
}

fn image_kind_from_data(data: Option<&Value>) -> MediaKind {
    match image_sub_type(data) {
        Some(0) => MediaKind::Image,
        Some(_) => MediaKind::Sticker,
        None => MediaKind::Sticker,
    }
}

fn image_sub_type(data: Option<&Value>) -> Option<i64> {
    let data = data?;
    value_opt_to_i64(
        data.get("sub_type")
            .or_else(|| data.get("subType"))
            .or_else(|| data.get("subtype")),
    )
}

fn file_segment_kind(data: Option<&Value>) -> MediaKind {
    let mime_is_image = data
        .and_then(|data| data.get("mime").or_else(|| data.get("mime_type")))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| value.starts_with("image/"));
    if mime_is_image
        || attachment_name_from_data(data)
            .as_deref()
            .is_some_and(is_image_filename)
    {
        MediaKind::Image
    } else {
        MediaKind::File
    }
}

fn attachment_name_from_data(data: Option<&Value>) -> Option<String> {
    let data = data?;
    ["name", "file_name", "filename", "file", "path", "url"]
        .iter()
        .find_map(|key| data.get(*key).and_then(value_to_string))
        .map(|value| filename_from_reference(&value))
        .filter(|value| !value.is_empty())
}

fn filename_from_reference(value: &str) -> String {
    let trimmed = value.trim().trim_start_matches("file://");
    let without_query = trimmed.split('?').next().unwrap_or(trimmed);
    without_query
        .rsplit('/')
        .next()
        .unwrap_or(without_query)
        .trim()
        .to_string()
}

fn is_image_filename(value: &str) -> bool {
    let normalized = filename_from_reference(value).to_ascii_lowercase();
    matches!(
        normalized.rsplit('.').next(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp")
    )
}

fn extract_reference(data: Option<&Value>) -> Option<MediaReference> {
    let data = data?;
    if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
        return Some(MediaReference::RemoteUrl {
            url: url.to_string(),
        });
    }
    if let Some(file) = data.get("file").and_then(|v| v.as_str()) {
        return Some(MediaReference::RemoteUrl {
            url: file.to_string(),
        });
    }
    if let Some(path) = data.get("path").and_then(|v| v.as_str()) {
        return Some(MediaReference::RemoteUrl {
            url: path.to_string(),
        });
    }
    None
}

fn extract_attachment_size(data: Option<&Value>) -> Option<u64> {
    let data = data?;
    let size = value_opt_to_i64(
        data.get("size")
            .or_else(|| data.get("file_size"))
            .or_else(|| data.get("filesize")),
    )?;
    u64::try_from(size).ok().filter(|value| *value > 0)
}

fn extract_cq_faces(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    let mut remaining = message;
    loop {
        let Some(start) = remaining.find("[CQ:face") else {
            output.push_str(remaining);
            break;
        };
        let (prefix, rest) = remaining.split_at(start);
        output.push_str(prefix);

        let Some(end) = rest.find(']') else {
            output.push_str(rest);
            break;
        };
        let segment = &rest[..=end];
        if let Some(face_id) = parse_cq_face_id(segment) {
            if let Some(placeholder) = face_inline_placeholder(&face_id) {
                output.push_str(&placeholder);
            } else {
                output.push_str(&format!("[face:{}]", face_id));
            }
            remaining = &rest[end + 1..];
            continue;
        }

        output.push_str(segment);
        remaining = &rest[end + 1..];
    }
    output
}

fn parse_cq_face_id(segment: &str) -> Option<String> {
    let trimmed = segment
        .strip_prefix('[')
        .unwrap_or(segment)
        .strip_suffix(']')
        .unwrap_or(segment);
    let params = trimmed.strip_prefix("CQ:face")?;
    let params = params.strip_prefix(',').unwrap_or(params);
    for part in params.split(',') {
        if let Some(value) = part.strip_prefix("id=") {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn face_inline_placeholder(face_id: &str) -> Option<String> {
    let face_id = normalize_face_id(face_id)?;
    let path = Path::new("res")
        .join("face")
        .join(format!("{}.png", face_id));
    if !path.exists() {
        return None;
    }
    Some(format!("[[face:{}]]", face_id))
}

fn normalize_face_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_string())
}

async fn parse_review_command(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    user_id: &str,
    _account_id: &str,
    group_id: &str,
    review_code: Option<ReviewCode>,
    action: ParsedReviewAction,
    reply_id: Option<String>,
    now_ms: i64,
) -> Option<Command> {
    let mut review_code = review_code;
    let mut review_id = None;
    let mut audit_msg_id = reply_id.clone();
    let mut is_processed = false;
    let mut reply_missing = false;

    {
        let guard = state.lock().await;
        if let Some(reply_id) = reply_id.as_ref() {
            if let Some(mapped) = guard.audit_msg_to_review.get(reply_id.as_str()) {
                review_id = Some(*mapped);
                review_code = None;
            } else {
                reply_missing = true;
            }
        }

        if review_id.is_none() {
            if let Some(code) = review_code {
                if let Some(mapped) = guard.review_by_code.get(&code).copied() {
                    review_id = Some(mapped);
                    review_code = None;
                    audit_msg_id = None;
                }
            }
        }
    }

    if reply_missing && review_id.is_none() && review_code.is_none() {
        send_group_text(out_tx, group_id, "找不到回复的消息").await;
        return None;
    }

    if review_id.is_none() && review_code.is_some() {
        send_group_text(out_tx, group_id, "找不到编号对应稿件").await;
        return None;
    }

    if review_id.is_none() && audit_msg_id.is_none() && review_code.is_none() {
        send_group_text(out_tx, group_id, "请回复审核消息或提供编号").await;
        return None;
    }

    if let Some(resolved_id) = review_id {
        let guard = state.lock().await;
        let Some(info) = guard.review_info.get(&resolved_id) else {
            send_group_text(out_tx, group_id, "找不到编号对应稿件").await;
            return None;
        };
        if info.group_id != runtime.group_id {
            send_group_text(out_tx, group_id, "无权限操作该稿件").await;
            return None;
        }
        is_processed = guard.processed_reviews.contains(&resolved_id);
    }

    if is_processed {
        send_group_text(out_tx, group_id, "此稿件已被处理").await;
        return None;
    }

    let command = match action {
        ParsedReviewAction::Builtin(action) => Command::ReviewAction(ReviewActionCommand {
            review_id,
            review_code,
            audit_msg_id,
            action,
            operator_id: user_id.to_string(),
            now_ms,
            tz_offset_minutes: runtime.tz_offset_minutes,
        }),
        ParsedReviewAction::Shortcut { key, args } => {
            let definition = {
                let guard = runtime
                    .review_shortcuts
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                guard.get(&key).cloned()
            };
            let Some(definition) = definition else {
                let msg = format!("审核快捷指令不存在：{}", key);
                send_group_text(out_tx, group_id, &msg).await;
                return None;
            };
            let (resolved_review_code, sender_id) = {
                let guard = state.lock().await;
                let resolved_review_code = review_id.and_then(|resolved_id| {
                    guard
                        .review_info
                        .get(&resolved_id)
                        .map(|info| info.review_code)
                });
                let sender_id = review_id.and_then(|resolved_id| {
                    resolve_review_submitter(&guard, resolved_id).map(|(_, sender)| sender)
                });
                (resolved_review_code, sender_id)
            };
            let actions = match parse_review_shortcut_actions(
                &definition,
                &ShortcutTemplateContext {
                    args: args.trim(),
                    review_code: resolved_review_code,
                    sender_id: sender_id.as_deref(),
                    group_id: &runtime.group_id,
                },
            ) {
                Ok(actions) => actions,
                Err(err) => {
                    let msg = format!("审核快捷指令展开失败：{}", err);
                    send_group_text(out_tx, group_id, &msg).await;
                    return None;
                }
            };
            if actions.len() == 1 {
                Command::ReviewAction(ReviewActionCommand {
                    review_id,
                    review_code,
                    audit_msg_id,
                    action: actions.into_iter().next()?,
                    operator_id: user_id.to_string(),
                    now_ms,
                    tz_offset_minutes: runtime.tz_offset_minutes,
                })
            } else {
                Command::ReviewActionBatch(ReviewActionBatchCommand {
                    review_id,
                    review_code,
                    audit_msg_id,
                    actions,
                    operator_id: user_id.to_string(),
                    now_ms,
                    tz_offset_minutes: runtime.tz_offset_minutes,
                })
            }
        }
    };

    send_group_text(out_tx, group_id, "已收到指令").await;
    Some(command)
}

fn message_mentions_self(value: Option<&Value>, self_id: &str) -> bool {
    if self_id.trim().is_empty() {
        return false;
    }
    match value {
        Some(Value::Array(items)) => items.iter().any(|item| {
            if item.get("type").and_then(|v| v.as_str()) != Some("at") {
                return false;
            }
            let at_target = item
                .get("data")
                .and_then(|data| data.get("qq"))
                .and_then(value_to_string)
                .unwrap_or_default();
            at_target.trim() == self_id
        }),
        Some(Value::String(raw)) => {
            let token = format!("qq={}", self_id);
            raw.contains("[CQ:at,") && raw.contains(&token)
        }
        _ => false,
    }
}

fn command_context_allowed(command: &AuditCommand, mentions_self: bool, reply_bound: bool) -> bool {
    match command {
        AuditCommand::Global(_) => mentions_self,
        AuditCommand::Review {
            review_code: Some(_),
            ..
        } => mentions_self,
        AuditCommand::Review {
            review_code: None, ..
        } => reply_bound,
    }
}

fn parse_audit_command(
    text: &str,
    has_reply: bool,
    runtime: &NapCatRuntimeConfig,
) -> Option<AuditCommand> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (first, rest) = split_first_token_with_rest(trimmed)?;

    if is_digits_unbounded(first) {
        let review_code = first.parse::<ReviewCode>().ok()?;
        let (command, args_text) = split_first_token_with_rest(rest)?;
        let args_text = args_text.trim_start();
        let action = parse_review_action(command, args_text, true, runtime)?;
        return Some(AuditCommand::Review {
            review_code: Some(review_code),
            action,
        });
    }

    if let Some(action) = parse_review_action(first, &rest, false, runtime) {
        return Some(AuditCommand::Review {
            review_code: None,
            action,
        });
    }

    if let Some(action) = parse_global_action(first, &rest, runtime) {
        return Some(AuditCommand::Global(action));
    }

    if has_reply {
        if let Some(action) = parse_review_action(first, &rest, true, runtime) {
            return Some(AuditCommand::Review {
                review_code: None,
                action,
            });
        }
    }

    None
}

fn split_first_token_with_rest(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let mut iter = input.splitn(2, char::is_whitespace);
    let first = iter.next().unwrap_or("");
    let rest = iter.next().unwrap_or("");
    if first.is_empty() {
        None
    } else {
        Some((first, rest))
    }
}

fn parse_review_action(
    command: &str,
    rest: &str,
    allow_quick_reply: bool,
    runtime: &NapCatRuntimeConfig,
) -> Option<ParsedReviewAction> {
    if command == RAW_BUILTIN_PREFIX {
        let (raw_command, raw_rest) = split_first_token_with_rest(rest)?;
        return parse_builtin_review_action(raw_command, raw_rest.trim_start(), false)
            .map(ParsedReviewAction::Builtin);
    }

    let has_shortcut = {
        let guard = runtime
            .review_shortcuts
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        guard.contains_key(command)
    };
    if has_shortcut {
        return Some(ParsedReviewAction::Shortcut {
            key: command.to_string(),
            args: rest.trim().to_string(),
        });
    }

    parse_builtin_review_action(command, rest.trim_start(), allow_quick_reply)
        .map(ParsedReviewAction::Builtin)
}

fn parse_global_action(
    command: &str,
    rest: &str,
    runtime: &NapCatRuntimeConfig,
) -> Option<ParsedGlobalAction> {
    if command == RAW_BUILTIN_PREFIX {
        let (raw_command, raw_rest) = split_first_token_with_rest(rest)?;
        return parse_builtin_global_action(raw_command, raw_rest).map(ParsedGlobalAction::Builtin);
    }

    let shortcut = {
        let guard = runtime
            .global_shortcuts
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        guard.get(command).cloned()
    };
    if let Some(definition) = shortcut {
        let actions = parse_global_shortcut_actions(
            &definition,
            &ShortcutTemplateContext {
                args: rest.trim(),
                review_code: None,
                sender_id: None,
                group_id: &runtime.group_id,
            },
        )
        .ok()?;
        return if actions.len() == 1 {
            Some(ParsedGlobalAction::Builtin(actions.into_iter().next()?))
        } else {
            Some(ParsedGlobalAction::Batch(actions))
        };
    }

    parse_builtin_global_action(command, rest).map(ParsedGlobalAction::Builtin)
}

fn validate_global_action(
    state: &NapCatState,
    group_id: &str,
    action: &GlobalAction,
) -> Result<(), &'static str> {
    match action {
        GlobalAction::Withdraw { review_code } => {
            validate_withdraw_action(state, group_id, *review_code)
        }
        _ => Ok(()),
    }
}

fn validate_withdraw_action(
    state: &NapCatState,
    group_id: &str,
    review_code: ReviewCode,
) -> Result<(), &'static str> {
    let Some(review_id) = state.review_by_code.get(&review_code) else {
        return Err("找不到编号对应稿件");
    };
    let Some(info) = state.review_info.get(review_id) else {
        return Err("找不到编号对应稿件");
    };
    if info.group_id != group_id {
        return Err("无权限操作该稿件");
    }
    let Some(plan) = state.send_plans.get(&info.post_id) else {
        return Err("该稿件不在暂存区");
    };
    if plan.group_id != group_id {
        return Err("无权限操作该稿件");
    }
    if !state.post_external_code.contains_key(&info.post_id) {
        return Err("该稿件缺少外部编号");
    }
    Ok(())
}

fn build_pending_list_text(state: &NapCatState, group_id: &str) -> String {
    let mut pending_reviews = state
        .review_info
        .iter()
        .filter_map(|(review_id, info)| {
            if info.group_id != group_id {
                return None;
            }
            if state.processed_reviews.contains(review_id) {
                return None;
            }
            Some(info.review_code)
        })
        .collect::<Vec<_>>();
    pending_reviews.sort_unstable();
    let pending_review_labels = pending_reviews
        .iter()
        .map(|code| format!("#{}", code))
        .collect::<Vec<_>>();

    let mut pending_send = state
        .send_plans
        .iter()
        .filter_map(|(post_id, plan)| {
            if plan.group_id != group_id {
                return None;
            }
            Some((
                plan.not_before_ms,
                plan.priority,
                plan.seq,
                post_label(state, *post_id),
            ))
        })
        .collect::<Vec<_>>();
    pending_send.sort_by(|a, b| (a.0, a.1, a.2, &a.3).cmp(&(b.0, b.1, b.2, &b.3)));
    let pending_send_labels = pending_send
        .into_iter()
        .map(|(_, _, _, label)| label)
        .collect::<Vec<_>>();

    let mut sending = state
        .sending
        .iter()
        .filter_map(|(post_id, info)| {
            if info.group_id != group_id {
                return None;
            }
            Some((info.started_at_ms, post_label(state, *post_id)))
        })
        .collect::<Vec<_>>();
    sending.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let sending_labels = sending
        .into_iter()
        .map(|(_, label)| label)
        .collect::<Vec<_>>();

    if pending_review_labels.is_empty()
        && pending_send_labels.is_empty()
        && sending_labels.is_empty()
    {
        return "待处理为空".to_string();
    }

    let mut lines = Vec::new();
    lines.push("待处理列表:".to_string());
    lines.push(format!(
        "待审核({}): {}",
        pending_review_labels.len(),
        format_list(&pending_review_labels),
    ));
    lines.push(format!(
        "待发送({}): {}",
        pending_send_labels.len(),
        format_list(&pending_send_labels),
    ));
    lines.push(format!(
        "发送中({}): {}",
        sending_labels.len(),
        format_list(&sending_labels),
    ));
    lines.join("\n")
}

fn build_blacklist_list_text(state: &NapCatState, group_id: &str) -> String {
    let Some(entries) = state.blacklist.get(group_id) else {
        return "黑名单为空".to_string();
    };
    if entries.is_empty() {
        return "黑名单为空".to_string();
    }
    let mut lines = entries
        .iter()
        .map(|(sender_id, reason)| {
            let reason = reason.as_deref().unwrap_or("无");
            format!("{} -> {}", sender_id, reason)
        })
        .collect::<Vec<_>>();
    lines.sort();
    let count = lines.len();
    lines.insert(0, format!("黑名单({}):", count));
    lines.join("\n")
}

fn build_quick_reply_list_text(runtime: &NapCatRuntimeConfig) -> String {
    let guard = runtime
        .quick_replies
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if guard.is_empty() {
        return "当前账号组未配置快捷回复".to_string();
    }
    let mut items = guard
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut lines = vec![format!("快捷回复列表（{}）:", items.len())];
    for (key, value) in items {
        lines.push(format!("{} = {}", key, value));
    }
    lines.join("\n")
}

fn build_shortcut_list_text(runtime: &NapCatRuntimeConfig) -> String {
    let review_lines = build_shortcut_scope_lines(runtime, ShortcutScope::Review);
    let global_lines = build_shortcut_scope_lines(runtime, ShortcutScope::Global);
    if review_lines.is_empty() && global_lines.is_empty() {
        return "当前账号组未配置快捷指令".to_string();
    }
    let mut lines = Vec::new();
    lines.push("快捷指令列表:".to_string());
    if review_lines.is_empty() {
        lines.push("审核快捷指令(0):".to_string());
        lines.push("（空）".to_string());
    } else {
        lines.push(format!("审核快捷指令({}):", review_lines.len()));
        lines.extend(review_lines);
    }
    if global_lines.is_empty() {
        lines.push("全局快捷指令(0):".to_string());
        lines.push("（空）".to_string());
    } else {
        lines.push(format!("全局快捷指令({}):", global_lines.len()));
        lines.extend(global_lines);
    }
    lines.join("\n")
}

fn build_shortcut_scope_lines(runtime: &NapCatRuntimeConfig, scope: ShortcutScope) -> Vec<String> {
    let guard = shortcut_storage(runtime, scope)
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let mut items = guard
        .iter()
        .map(|(k, v)| format!("{} = {}", k, v))
        .collect::<Vec<_>>();
    items.sort();
    items
}

fn build_selfcheck_report(runtime: &NapCatRuntimeConfig, state: &NapCatState) -> String {
    let pending_reviews = state
        .review_info
        .iter()
        .filter(|(review_id, info)| {
            info.group_id == runtime.group_id && !state.processed_reviews.contains(review_id)
        })
        .count();
    let pending_send = state
        .send_plans
        .values()
        .filter(|plan| plan.group_id == runtime.group_id)
        .count();
    let sending = state
        .sending
        .values()
        .filter(|sending| sending.group_id == runtime.group_id)
        .count();
    let blacklist = state
        .blacklist
        .get(&runtime.group_id)
        .map(|entries| entries.len())
        .unwrap_or(0);
    let quick_replies = runtime
        .quick_replies
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .len();
    let review_shortcuts = runtime
        .review_shortcuts
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .len();
    let global_shortcuts = runtime
        .global_shortcuts
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .len();
    let accounts_cfg = runtime.accounts.len();
    let online_accounts = group_accounts()
        .lock()
        .map(|m| m.get(&runtime.group_id).map(|list| list.len()).unwrap_or(0))
        .unwrap_or(0);
    let ws_base = base_url_for_log(&runtime.napcat.base_url);
    let ws_token = if runtime.napcat.access_token.is_some() {
        "已配置"
    } else {
        "未配置"
    };
    let audit_group = runtime
        .audit_group_id
        .clone()
        .unwrap_or_else(|| "未配置".to_string());
    let account_ids = if runtime.accounts.is_empty() {
        "无".to_string()
    } else {
        runtime.accounts.join(", ")
    };

    format!(
        "系统自检报告\n组: {}\n审核群: {}\nNapCat: {} (token {})\n账号: 配置 {} 个, 在线 {} 个\n账号列表: {}\n待审核: {}\n待发送: {}\n发送中: {}\n黑名单: {}\n快捷回复: {}\n审核快捷指令: {}\n全局快捷指令: {}\n队列策略: max_post_stack={}",
        runtime.group_id,
        audit_group,
        ws_base,
        ws_token,
        accounts_cfg,
        online_accounts,
        account_ids,
        pending_reviews,
        pending_send,
        sending,
        blacklist,
        quick_replies,
        review_shortcuts,
        global_shortcuts,
        runtime.max_queue
    )
}

fn quick_reply_key_conflicts(key: &str) -> bool {
    is_builtin_review_command_name(key)
}

fn sort_quick_reply_map(map: &mut HashMap<String, String>) {
    sort_string_map(map);
}

fn sort_string_map(map: &mut HashMap<String, String>) {
    let mut pairs = map
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    map.clear();
    for (k, v) in pairs {
        map.insert(k, v);
    }
}

fn persist_group_quick_replies(
    group_id: &str,
    quick_replies: &HashMap<String, String>,
) -> Result<(), String> {
    persist_group_string_map(group_id, "quick_replies", quick_replies)
}

fn persist_group_string_map(
    group_id: &str,
    field_name: &str,
    values: &HashMap<String, String>,
) -> Result<(), String> {
    let config_path = env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string());
    let data = fs::read_to_string(&config_path)
        .map_err(|err| format!("读取配置失败 {}: {}", config_path, err))?;
    let mut root: Value = serde_json::from_str(&data)
        .map_err(|err| format!("配置 JSON 解析失败 {}: {}", config_path, err))?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "配置根节点必须是对象".to_string())?;
    let mut qr_obj = serde_json::Map::new();
    let mut entries = values
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in entries {
        qr_obj.insert(k, Value::String(v));
    }
    if let Some(groups) = obj.get_mut("groups").and_then(|v| v.as_object_mut()) {
        let group = groups
            .get_mut(group_id)
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| format!("配置中不存在 groups.{}", group_id))?;
        group.insert(field_name.to_string(), Value::Object(qr_obj));
    } else {
        let group = obj
            .get_mut(group_id)
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| format!("配置中不存在组 {}", group_id))?;
        group.insert(field_name.to_string(), Value::Object(qr_obj));
    }
    let mut output =
        serde_json::to_string_pretty(&root).map_err(|err| format!("配置序列化失败: {}", err))?;
    output.push('\n');
    fs::write(&config_path, output).map_err(|err| format!("配置写入失败: {}", err))
}

fn shortcut_storage<'a>(
    runtime: &'a NapCatRuntimeConfig,
    scope: ShortcutScope,
) -> &'a Arc<std::sync::Mutex<HashMap<String, String>>> {
    match scope {
        ShortcutScope::Review => &runtime.review_shortcuts,
        ShortcutScope::Global => &runtime.global_shortcuts,
    }
}

fn collect_batch_post_ids_for_notify(
    state: &NapCatState,
    group_id: &str,
    leader: PostId,
    leader_priority: SendPriority,
    max_queue: usize,
) -> Vec<PostId> {
    if max_queue <= 1 || leader_priority != SendPriority::Normal {
        return vec![leader];
    }
    let mut queued = state
        .send_plans
        .iter()
        .filter(|(_, plan)| plan.group_id == group_id && plan.priority == leader_priority)
        .map(|(post_id, plan)| (plan.seq, *post_id))
        .collect::<Vec<_>>();
    queued.sort_by_key(|(seq, post_id)| (*seq, post_id.0));
    let mut out = Vec::with_capacity(queued.len().saturating_add(1));
    out.push(leader);
    for (_, post_id) in queued {
        if post_id != leader {
            out.push(post_id);
        }
    }
    out
}

fn post_batch_label(state: &NapCatState, post_ids: &[PostId]) -> String {
    if post_ids.is_empty() {
        return String::new();
    }
    post_ids
        .iter()
        .map(|post_id| post_label(state, *post_id))
        .collect::<Vec<_>>()
        .join(",")
}

fn post_label(state: &NapCatState, post_id: PostId) -> String {
    let review_code = state.post_review_code.get(&post_id).copied();
    let external_code = state.post_external_code.get(&post_id).copied();
    match (external_code, review_code) {
        (Some(external), Some(review)) => format!("#{}/{}", external, review),
        (Some(external), None) => format!("#{}", external),
        (None, Some(review)) => format!("#{}", review),
        (None, None) => format!("post:{}", id128_hex(post_id.0)),
    }
}

fn post_label_plain(state: &NapCatState, post_id: PostId) -> String {
    post_label(state, post_id)
        .trim_start_matches('#')
        .to_string()
}

fn post_code_text(state: &NapCatState, post_id: PostId) -> Option<String> {
    state
        .post_external_code
        .get(&post_id)
        .map(|code| code.to_string())
        .or_else(|| {
            state
                .post_review_code
                .get(&post_id)
                .map(|code| code.to_string())
        })
}

fn resolve_post_submitter(state: &NapCatState, post_id: PostId) -> Option<String> {
    let ingress_ids = state.post_ingress.get(&post_id)?;
    resolve_post_submitter_with_ingress(state, ingress_ids)
}

fn resolve_post_submitter_with_ingress(
    state: &NapCatState,
    ingress_ids: &[IngressId],
) -> Option<String> {
    ingress_ids.iter().find_map(|ingress_id| {
        let summary = state.ingress_summary.get(ingress_id)?;
        let trimmed = summary.user_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_review_submitter(state: &NapCatState, review_id: ReviewId) -> Option<(String, String)> {
    let info = state.review_info.get(&review_id)?;
    let user_id = state
        .review_submitter
        .get(&review_id)
        .cloned()
        .or_else(|| resolve_post_submitter(state, info.post_id))?;
    Some((info.group_id.clone(), user_id))
}

fn format_review_submitter_notice(kind: ReviewSubmitterNoticeKind, reason: Option<&str>) -> String {
    let reason = reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("无");
    match kind {
        ReviewSubmitterNoticeKind::Rejected => {
            format!("你的投稿已被拒，请修改后再发送。理由：{}", reason)
        }
        ReviewSubmitterNoticeKind::Deleted => String::new(),
    }
}

fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        "无".to_string()
    } else {
        items.join(" ")
    }
}

fn extract_sender_name(value: &Value) -> Option<String> {
    let sender = value.get("sender")?;
    let card = sender
        .get("card")
        .and_then(|v| v.as_str())
        .map(|s| s.trim());
    if let Some(card) = card {
        if !card.is_empty() {
            return Some(card.to_string());
        }
    }
    let nickname = sender
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.trim());
    nickname
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
}

const SUMMARY_LINE_MAX_CHARS: usize = 120;

fn build_audit_message(
    review_code: ReviewCode,
    post_id: PostId,
    ingress_ids: &[IngressId],
    ingress_map: &HashMap<IngressId, IngressSummary>,
    preview_images: Vec<String>,
    blob_paths: &HashMap<BlobId, String>,
    is_safe: bool,
) -> AuditMessage {
    let mut images = preview_images;
    if ingress_ids.is_empty() {
        return AuditMessage {
            text: format!("#{} post {}", review_code, post_id.0),
            images,
        };
    }

    let mut lines = Vec::new();
    let mut user_id = None;
    let mut sender_name = None;

    for ingress_id in ingress_ids {
        if let Some(summary) = ingress_map.get(ingress_id) {
            if user_id.is_none() {
                user_id = Some(summary.user_id.clone());
                sender_name = summary
                    .sender_name
                    .clone()
                    .filter(|name| !name.trim().is_empty());
            }

            if let Some(line) = sanitize_summary_line(&summary.text) {
                lines.push(line);
            }
            for attachment in &summary.attachments {
                if attachment.kind != MediaKind::Image {
                    lines.push(attachment_placeholder(attachment.kind).to_string());
                }
                if let Some(image) = image_source_from_attachment(attachment, blob_paths) {
                    images.push(image);
                }
            }
        }
    }

    let safety_text = if is_safe { "安全" } else { "不安全" };
    let header = match user_id {
        Some(user_id) => {
            let display_name = sender_name.unwrap_or_else(|| user_id.clone());
            format!(
                "#{} 来自 {}({}) 系统判断{}",
                review_code, display_name, user_id, safety_text
            )
        }
        None => format!(
            "#{} post {} 系统判断{}",
            review_code, post_id.0, safety_text
        ),
    };

    let mut text = String::new();
    text.push_str(&header);
    text.push('\n');
    text.push_str("消息概览：");
    if lines.is_empty() {
        text.push('\n');
        text.push_str(" （空）");
    } else {
        for line in lines {
            text.push('\n');
            text.push(' ');
            text.push_str(&line);
        }
    }
    if !images.is_empty() {
        text.push('\n');
        text.push_str("图片：");
    }

    AuditMessage { text, images }
}

fn sanitize_summary_line(text: &str) -> Option<String> {
    let with_cq = replace_face_placeholders_with_cq(text);
    let flattened = with_cq.replace('\n', " ");
    let normalized = flattened.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= SUMMARY_LINE_MAX_CHARS {
            break;
        }
        out.push(ch);
    }
    if trimmed.chars().count() > SUMMARY_LINE_MAX_CHARS {
        out.push_str("...");
    }
    Some(out)
}

fn replace_face_placeholders_with_cq(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'[' && bytes.get(idx + 1) == Some(&b'[') {
            let rest = &text[idx..];
            if rest.starts_with("[[face:") {
                let after_prefix = idx + "[[face:".len();
                if after_prefix <= text.len() {
                    if let Some(close) = text[after_prefix..].find("]]") {
                        let face_id = &text[after_prefix..after_prefix + close];
                        if !face_id.is_empty() && face_id.chars().all(|c| c.is_ascii_digit()) {
                            out.push_str("[CQ:face,id=");
                            out.push_str(face_id);
                            out.push(']');
                            idx = after_prefix + close + 2;
                            continue;
                        }
                    }
                }
            }
        }
        let ch = text[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

fn message_segments_from_text(text: &str) -> Vec<Value> {
    let mut segments = Vec::new();
    let mut buffer = String::new();
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'[' {
            let rest = &text[idx..];
            if let Some((face_id, consumed)) = parse_face_marker(rest) {
                flush_text_segment(&mut segments, &mut buffer);
                segments.push(serde_json::json!({
                    "type": "face",
                    "data": { "id": face_id }
                }));
                idx += consumed;
                continue;
            }
        }
        let ch = text[idx..].chars().next().unwrap();
        buffer.push(ch);
        idx += ch.len_utf8();
    }
    flush_text_segment(&mut segments, &mut buffer);
    segments
}

fn flush_text_segment(segments: &mut Vec<Value>, buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }
    segments.push(serde_json::json!({
        "type": "text",
        "data": { "text": buffer.clone() }
    }));
    buffer.clear();
}

fn parse_face_marker(rest: &str) -> Option<(String, usize)> {
    if let Some(found) = parse_face_placeholder(rest, "[[face:", "]]") {
        return Some(found);
    }
    if let Some(found) = parse_face_placeholder(rest, "[face:", "]") {
        return Some(found);
    }
    if rest.starts_with("[CQ:face") {
        let end = rest.find(']')?;
        let segment = &rest[..=end];
        let face_id = parse_cq_face_id(segment)?;
        let face_id = normalize_face_id(&face_id)?;
        return Some((face_id, end + 1));
    }
    None
}

fn parse_face_placeholder(rest: &str, prefix: &str, suffix: &str) -> Option<(String, usize)> {
    if !rest.starts_with(prefix) {
        return None;
    }
    let after_prefix = prefix.len();
    let close = rest[after_prefix..].find(suffix)?;
    let face_id = &rest[after_prefix..after_prefix + close];
    let face_id = normalize_face_id(face_id)?;
    Some((face_id, after_prefix + close + suffix.len()))
}

fn attachment_placeholder(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "[图片]",
        MediaKind::Video => "[视频]",
        MediaKind::File => "[文件]",
        MediaKind::Audio => "[音频]",
        MediaKind::Other => "[附件]",
        MediaKind::Sticker => "[表情]",
    }
}

fn image_source_from_attachment(
    attachment: &IngressAttachment,
    blob_paths: &HashMap<BlobId, String>,
) -> Option<String> {
    if attachment.kind != MediaKind::Image {
        return None;
    }
    match &attachment.reference {
        MediaReference::Blob { blob_id } => {
            if let Some(bytes) = blob_cache::get_bytes(*blob_id) {
                return Some(format!("base64://{}", STANDARD.encode(bytes.as_ref())));
            }
            blob_paths
                .get(blob_id)
                .map(|path| file_uri_from_path(Path::new(path)))
        }
        MediaReference::RemoteUrl { url } => {
            if url.starts_with("file://")
                || url.starts_with("data:")
                || url.starts_with("base64://")
            {
                return Some(url.clone());
            }
            if Path::new(url).exists() {
                return Some(file_uri_from_path(Path::new(url)));
            }
            None
        }
    }
}

fn rendered_png_blob_ids(state: &NapCatState, post_id: PostId) -> Vec<BlobId> {
    state
        .post_render_blobs
        .get(&post_id)
        .cloned()
        .unwrap_or_default()
}

fn rendered_png_previews(
    post_id: PostId,
    blob_ids: &[BlobId],
    blob_paths: &HashMap<BlobId, String>,
) -> Vec<String> {
    let mut ids = blob_ids.to_vec();
    if ids.is_empty() {
        ids.push(rendered_png_blob_id(post_id));
    }
    ids.into_iter()
        .filter_map(|blob_id| rendered_png_preview_for_blob(blob_id, blob_paths))
        .collect()
}

fn rendered_png_preview_for_blob(
    blob_id: BlobId,
    blob_paths: &HashMap<BlobId, String>,
) -> Option<String> {
    if let Some(bytes) = blob_cache::get_bytes(blob_id) {
        return Some(format!("base64://{}", STANDARD.encode(bytes.as_ref())));
    }
    if let Some(path) = blob_paths.get(&blob_id) {
        return Some(file_uri_from_path(Path::new(path)));
    }
    let path = rendered_png_path_for_blob(blob_id);
    let meta = fs::metadata(&path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    Some(file_uri_from_path(&path))
}

fn rendered_png_blob_id(post_id: PostId) -> BlobId {
    derive_blob_id(&[&post_id.to_be_bytes(), b"png"])
}

fn rendered_png_path_for_blob(blob_id: BlobId) -> PathBuf {
    let filename = format!("{}.png", id128_hex(blob_id.0));
    blob_root().join("png").join(filename)
}

fn blob_root() -> PathBuf {
    std::env::var("OQQWALL_BLOB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/blobs"))
}

fn file_uri_from_path(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    format!("file://{}", absolute.to_string_lossy())
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

fn base_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

const HELP_TEXT: &str = r#"全局指令:
这些是可以在任何时刻@本账号调用的指令
语法: @本账号/次要账号 指令

帮助:
查看这个帮助列表

调出:
调出曾经接收到过的投稿
用法：调出 <review_code>

撤回:
将暂存区中的稿件撤回到待处理，并重排后续待发送稿件的外部编号
用法：撤回 <review_code>

信息:
查询该编号的接收者、发送者、所属组、处理后信息
用法：信息 <review_code>

手动重新登录:
扫码登陆QQ空间

自动重新登录:
尝试自动登录QQ空间

待处理:
列出当前等待处理投稿（按账号组过滤）

删除待处理:
清空待处理列表，相当于对列表中的所有项目执行"删"审核指令

删除暂存区:
清空暂存区内容（仅清理待发送队列，不回滚外部编号）

发送暂存区:
将暂存区内容发送到QQ空间

清理发送中:
清理卡住的发送中状态，并重新入队

列出拉黑:
列出当前被拉黑账号列表

取消拉黑:
取消对某账号拉黑
用法：取消拉黑 <senderid>

设定编号:
设定下一条说说外部编号（纯数字）
用法：设定编号 <纯数字>

快捷回复:
查看当前账号组配置的快捷回复列表

快捷回复 添加:
添加快捷回复指令
用法：快捷回复 添加 指令名=内容
说明：会校验不与审核指令冲突，并写回配置文件

快捷回复 删除:
删除指定快捷回复指令
用法：快捷回复 删除 指令名
说明：删除后会写回配置文件

快捷指令:
查看当前账号组配置的快捷指令列表
用法：快捷指令

快捷指令 添加:
添加审核或全局快捷指令
用法：快捷指令 添加 审核 指令名=步骤1 | 步骤2
或：快捷指令 添加 全局 指令名=步骤1 | 步骤2
说明：步骤支持用 | 或换行分隔；审核快捷指令不能与快捷回复重名

快捷指令 删除:
删除指定审核或全局快捷指令
用法：快捷指令 删除 审核 指令名
或：快捷指令 删除 全局 指令名

自检:
系统与服务自检

系统修复:
重启服务并重建连接（谨慎使用）


审核指令:
这些指令仅在稿件审核流程中要求您发送指令时可用
语法: @本账号 review_code 指令
或 回复审核消息 指令

是:
发送，并给稿件发送者发送成功提示

否:
机器跳过此条，人工处理（常用于分段/匿名失败或含视频）

匿:
切换匿名状态，处理后会再次询问指令

等:
等待180秒，然后重新执行分段-渲染-审核流程

删:
此条不发送，也不用人工发送

拒:
拒绝稿件，并给发送者发送被拒提示

立即:
立刻发送暂存区全部投稿，并立即把当前投稿单发

刷新:
重新进行“聊天记录->图片”的过程

重渲染:
重新进行渲染，不重做分段

消息全选:
强制把本次投稿所有消息作为内容并重渲染

扩列审查:
扩列审核流程（抓等级/空间/名片/二维码等）

评论:
增加文本评论，处理后再次询问
用法：评论 <文本>

回复:
向投稿人发送一条信息
用法：回复 <文本>

展示:
展示稿件内容

拉黑:
不再接收来自此人的投稿
用法：拉黑 [理由]

快捷回复指令:
使用预设模板向投稿人发送消息
用法：回复审核消息 <快捷指令名>
或：@本账号 <review_code> <快捷指令名>

快捷指令:
审核快捷指令会优先于内置审核指令；全局快捷指令会优先于内置全局指令
如需执行被覆盖的原始内置指令，请在指令前加“原始”
示例：@本账号 123 原始 匿
示例：@本账号 原始 待处理"#;

fn is_admin_sender(value: &Value) -> bool {
    value
        .get("sender")
        .and_then(|sender| sender.get("role"))
        .and_then(|role| role.as_str())
        .map(|role| role == "admin" || role == "owner")
        .unwrap_or(false)
}

async fn send_group_text(out_tx: &mpsc::Sender<String>, group_id: &str, text: &str) {
    let payload = serde_json::json!({
        "action": "send_group_msg",
        "params": {
            "group_id": json_id(group_id),
            "message": [{"type": "text", "data": {"text": text}}]
        }
    });
    let _ = out_tx.send(payload.to_string()).await;
}

async fn send_private_text(out_tx: &mpsc::Sender<String>, user_id: &str, text: &str) {
    let payload = serde_json::json!({
        "action": "send_private_msg",
        "params": {
            "user_id": json_id(user_id),
            "message": [{"type": "text", "data": {"text": text}}]
        }
    });
    let _ = out_tx.send(payload.to_string()).await;
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn value_opt_to_string(value: Option<&Value>) -> Option<String> {
    value.and_then(value_to_string)
}

fn value_opt_to_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn value_opt_to_u8(value: Option<&Value>) -> Option<u8> {
    match value? {
        Value::Number(n) => n.as_u64().and_then(|v| u8::try_from(v).ok()),
        Value::String(s) => s.parse::<u8>().ok(),
        _ => None,
    }
}

fn inbound_timestamp_ms(value: &Value) -> i64 {
    value
        .get("time")
        .and_then(|v| v.as_i64())
        .map(|sec| sec.saturating_mul(1000))
        .unwrap_or_else(now_ms)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn next_echo(state: &mut NapCatState) -> String {
    state.next_echo = state.next_echo.saturating_add(1);
    format!("echo-{}", state.next_echo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock as StdOnceLock};

    fn global_test_lock() -> &'static StdMutex<()> {
        static LOCK: StdOnceLock<StdMutex<()>> = StdOnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
    }

    fn lock_globals_for_test() -> MutexGuard<'static, ()> {
        match global_test_lock().lock() {
            Ok(guard) => guard,
            Err(err) => err.into_inner(),
        }
    }

    fn mock_session() -> NapCatWsSession {
        let (out_tx, _out_rx) = mpsc::channel(1);
        NapCatWsSession {
            out_tx,
            state: Arc::new(Mutex::new(NapCatState::default())),
        }
    }

    fn test_runtime() -> NapCatRuntimeConfig {
        NapCatRuntimeConfig {
            napcat: NapCatConfig {
                base_url: "127.0.0.1:3001/oqqwall/ws".to_string(),
                access_token: None,
            },
            audit_group_id: Some("1".to_string()),
            group_id: "group-a".to_string(),
            accounts: vec!["100".to_string()],
            tz_offset_minutes: 0,
            friend_request_window_sec: 0,
            friend_add_message: None,
            max_queue: 1,
            quick_replies: Arc::new(std::sync::Mutex::new(HashMap::new())),
            review_shortcuts: Arc::new(std::sync::Mutex::new(HashMap::new())),
            global_shortcuts: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    fn parse_cmd(text: &str, has_reply: bool) -> Option<AuditCommand> {
        parse_audit_command(text, has_reply, &test_runtime())
    }

    fn clear_group_accounts_for_test(group_id: &str) {
        let mut guard = match group_accounts().lock() {
            Ok(guard) => guard,
            Err(err) => err.into_inner(),
        };
        guard.remove(group_id);
    }

    #[test]
    fn parse_help_and_review_with_code() {
        assert_eq!(
            parse_cmd("帮助", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::Help
            )))
        );
        assert_eq!(
            parse_cmd("help", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::Help
            )))
        );
        assert_eq!(
            parse_cmd("123 是", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Approve),
            })
        );
        assert_eq!(
            parse_cmd("123 删", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Delete { reason: None }),
            })
        );
        assert_eq!(
            parse_cmd("123 拒", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Reject { reason: None }),
            })
        );
        assert_eq!(
            parse_cmd("123 拒 广告", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Reject {
                    reason: Some("广告".to_string())
                }),
            })
        );
        assert_eq!(
            parse_cmd("123 合并 456", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Merge { review_code: 456 }),
            })
        );
    }

    #[test]
    fn parse_global_and_quick_reply_actions() {
        assert_eq!(
            parse_cmd("调出 42", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::Recall { review_code: 42 }
            )))
        );
        assert_eq!(
            parse_cmd("调出 #42", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::Recall { review_code: 42 }
            )))
        );
        assert_eq!(
            parse_cmd("撤回 42", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::Withdraw { review_code: 42 }
            )))
        );
        assert_eq!(
            parse_cmd("清理发送中", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::SendInFlightClear
            )))
        );
        assert_eq!(
            parse_cmd("快捷回复 添加 hi=hello", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::QuickReplyAdd {
                    key: "hi".to_string(),
                    text: "hello".to_string(),
                }
            )))
        );
        assert_eq!(
            parse_cmd("快捷指令 添加 审核 滚=拒 | 拉黑", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::ShortcutAdd {
                    scope: ShortcutScope::Review,
                    key: "滚".to_string(),
                    definition: "拒 | 拉黑".to_string(),
                }
            )))
        );
        assert_eq!(
            parse_cmd("快捷指令 删除 全局 待处理", false),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::ShortcutDelete {
                    scope: ShortcutScope::Global,
                    key: "待处理".to_string(),
                }
            )))
        );
    }

    #[test]
    fn parse_quick_reply_requires_reply_context() {
        assert_eq!(parse_cmd("谢谢", false), None);
        assert_eq!(
            parse_cmd("谢谢", true),
            Some(AuditCommand::Review {
                review_code: None,
                action: ParsedReviewAction::Builtin(ReviewAction::QuickReply {
                    key: "谢谢".to_string(),
                }),
            })
        );
    }

    #[test]
    fn parse_reply_text_preserves_spaces() {
        assert_eq!(
            parse_cmd("123 回复 hello world", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Reply {
                    text: "hello world".to_string(),
                }),
            })
        );
        assert_eq!(
            parse_cmd("123 回复  hello   world", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Reply {
                    text: "hello   world".to_string(),
                }),
            })
        );
        assert_eq!(
            parse_cmd("回复  你好  世界", true),
            Some(AuditCommand::Review {
                review_code: None,
                action: ParsedReviewAction::Builtin(ReviewAction::Reply {
                    text: "你好  世界".to_string(),
                }),
            })
        );
    }

    #[test]
    fn parse_review_shortcuts_override_builtin_and_support_raw_prefix() {
        let runtime = NapCatRuntimeConfig {
            review_shortcuts: Arc::new(std::sync::Mutex::new(HashMap::from([(
                "匿".to_string(),
                "匿 | 是".to_string(),
            )]))),
            ..test_runtime()
        };
        assert_eq!(
            parse_audit_command("123 匿", false, &runtime),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Shortcut {
                    key: "匿".to_string(),
                    args: String::new(),
                },
            })
        );
        assert_eq!(
            parse_audit_command("123 原始 匿", false, &runtime),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::ToggleAnonymous),
            })
        );
    }

    #[test]
    fn parse_global_shortcuts_override_builtin_and_expand() {
        let runtime = NapCatRuntimeConfig {
            global_shortcuts: Arc::new(std::sync::Mutex::new(HashMap::from([(
                "待处理".to_string(),
                "删除待处理 | 删除暂存区".to_string(),
            )]))),
            ..test_runtime()
        };
        assert_eq!(
            parse_audit_command("待处理", false, &runtime),
            Some(AuditCommand::Global(ParsedGlobalAction::Batch(vec![
                GlobalAction::PendingClear,
                GlobalAction::SendQueueClear,
            ])))
        );
        assert_eq!(
            parse_audit_command("原始 待处理", false, &runtime),
            Some(AuditCommand::Global(ParsedGlobalAction::Builtin(
                GlobalAction::PendingList
            )))
        );
    }

    #[test]
    fn parse_friend_recall_notice_to_driver_event() {
        let runtime = test_runtime();
        let payload = serde_json::json!({
            "post_type": "notice",
            "notice_type": "friend_recall",
            "self_id": "10001",
            "user_id": "20002",
            "message_id": "30003",
            "time": 1730000000
        });

        let command = parse_notice_event(&runtime, &payload);
        let expected_ingress = derive_ingress_id(&[b"10001", b"20002", b"20002", b"30003"]);
        assert!(matches!(
            command,
            Some(Command::DriverEvent(Event::Ingress(
                IngressEvent::MessageRecalled {
                    ingress_id,
                    recalled_at_ms,
                }
            ))) if ingress_id == expected_ingress && recalled_at_ms > 0
        ));
    }

    #[test]
    fn account_status_text_formats_online_and_offline() {
        assert_eq!(account_status_text("10001", true), "账号10001已上线");
        assert_eq!(account_status_text("10001", false), "账号10001已离线");
    }

    #[test]
    fn message_mentions_self_only_for_matching_at_segment() {
        let msg = serde_json::json!([
            {"type":"at","data":{"qq":"10001"}},
            {"type":"text","data":{"text":" 帮助"}}
        ]);
        assert!(message_mentions_self(Some(&msg), "10001"));
        assert!(!message_mentions_self(Some(&msg), "10002"));
        assert!(!message_mentions_self(
            Some(&serde_json::json!("帮助")),
            "10001"
        ));
    }

    #[test]
    fn command_context_requires_at_or_bound_reply() {
        let global = AuditCommand::Global(ParsedGlobalAction::Builtin(GlobalAction::Help));
        assert!(command_context_allowed(&global, true, false));
        assert!(!command_context_allowed(&global, false, true));

        let review_with_code = AuditCommand::Review {
            review_code: Some(42),
            action: ParsedReviewAction::Builtin(ReviewAction::Approve),
        };
        assert!(command_context_allowed(&review_with_code, true, false));
        assert!(!command_context_allowed(&review_with_code, false, true));

        let review_reply = AuditCommand::Review {
            review_code: None,
            action: ParsedReviewAction::Builtin(ReviewAction::Approve),
        };
        assert!(command_context_allowed(&review_reply, false, true));
        assert!(!command_context_allowed(&review_reply, true, false));
    }

    #[test]
    fn napcat_account_for_group_prefers_first_online_in_accounts_order() {
        let _guard = lock_globals_for_test();
        set_group_accounts("g-test", vec!["100".to_string(), "200".to_string()]);
        register_ws_session("200", mock_session());
        assert_eq!(napcat_account_for_group("g-test"), Some("200".to_string()));

        register_ws_session("100", mock_session());
        assert_eq!(napcat_account_for_group("g-test"), Some("100".to_string()));

        unregister_ws_session("100");
        unregister_ws_session("200");
        clear_group_accounts_for_test("g-test");
    }

    #[test]
    fn effective_primary_account_uses_accounts_order_with_online_fallback() {
        let _guard = lock_globals_for_test();
        let mut runtime = test_runtime();
        runtime.group_id = "g-test2".to_string();
        runtime.accounts = vec!["100".to_string(), "200".to_string()];

        register_ws_session("200", mock_session());
        assert_eq!(effective_primary_account(&runtime), Some("200".to_string()));
        assert!(is_effective_primary_account(&runtime, "200"));
        assert!(!is_effective_primary_account(&runtime, "100"));

        register_ws_session("100", mock_session());
        assert_eq!(effective_primary_account(&runtime), Some("100".to_string()));
        assert!(is_effective_primary_account(&runtime, "100"));
        assert!(!is_effective_primary_account(&runtime, "200"));

        unregister_ws_session("100");
        unregister_ws_session("200");
    }

    #[test]
    fn message_segments_from_text_parses_faces() {
        let segments = message_segments_from_text("a[[face:12]]b[face:34]c[CQ:face,id=56]!");
        assert_eq!(
            segments,
            vec![
                serde_json::json!({"type": "text", "data": {"text": "a"}}),
                serde_json::json!({"type": "face", "data": {"id": "12"}}),
                serde_json::json!({"type": "text", "data": {"text": "b"}}),
                serde_json::json!({"type": "face", "data": {"id": "34"}}),
                serde_json::json!({"type": "text", "data": {"text": "c"}}),
                serde_json::json!({"type": "face", "data": {"id": "56"}}),
                serde_json::json!({"type": "text", "data": {"text": "!"}}),
            ]
        );
    }

    #[test]
    fn rendered_png_previews_keep_all_rendered_pages() {
        let post_id = PostId::from_u128(42);
        let page_one = rendered_png_blob_id(post_id);
        let page_two = BlobId::from_u128(43);
        let mut blob_paths = HashMap::new();
        blob_paths.insert(page_one, "/tmp/page-one.png".to_string());
        blob_paths.insert(page_two, "/tmp/page-two.png".to_string());

        let previews = rendered_png_previews(post_id, &[page_one, page_two], &blob_paths);

        assert_eq!(
            previews,
            vec![
                "file:///tmp/page-one.png".to_string(),
                "file:///tmp/page-two.png".to_string(),
            ]
        );
    }

    #[test]
    fn file_segment_kind_treats_image_files_as_images() {
        let image_file = serde_json::json!({
            "file": "/tmp/photo.png",
            "file_size": 32
        });
        let image_mime = serde_json::json!({
            "name": "download.bin",
            "mime": "image/webp"
        });
        let text_file = serde_json::json!({
            "url": "file:///tmp/readme.txt",
            "file_size": 64
        });

        assert_eq!(file_segment_kind(Some(&image_file)), MediaKind::Image);
        assert_eq!(
            attachment_name_from_data(Some(&image_file)).as_deref(),
            Some("photo.png")
        );
        assert_eq!(file_segment_kind(Some(&image_mime)), MediaKind::Image);
        assert_eq!(file_segment_kind(Some(&text_file)), MediaKind::File);
        assert_eq!(
            attachment_name_from_data(Some(&text_file)).as_deref(),
            Some("readme.txt")
        );
    }

    #[test]
    fn collect_batch_post_ids_for_notify_matches_seq_order() {
        let leader = PostId::from_u128(1);
        let second = PostId::from_u128(2);
        let third = PostId::from_u128(3);
        let mut state = NapCatState::default();
        state.send_plans.insert(
            second,
            SendPlanInfo {
                group_id: "g".to_string(),
                not_before_ms: 0,
                priority: SendPriority::Normal,
                seq: 11,
            },
        );
        state.send_plans.insert(
            third,
            SendPlanInfo {
                group_id: "g".to_string(),
                not_before_ms: 0,
                priority: SendPriority::Normal,
                seq: 12,
            },
        );
        let batch = collect_batch_post_ids_for_notify(&state, "g", leader, SendPriority::Normal, 3);
        assert_eq!(batch, vec![leader, second, third]);
    }

    #[test]
    fn post_batch_label_joins_codes_without_spaces() {
        let first = PostId::from_u128(10);
        let second = PostId::from_u128(11);
        let mut state = NapCatState::default();
        state.post_external_code.insert(first, 1193);
        state.post_review_code.insert(first, 102);
        state.post_external_code.insert(second, 1094);
        state.post_review_code.insert(second, 103);

        let label = post_batch_label(&state, &[first, second]);
        assert_eq!(label, "#1193/102,#1094/103");
    }

    #[test]
    fn validate_withdraw_requires_queued_post_with_external_code() {
        let review_id = ReviewId::from_u128(10);
        let post_id = PostId::from_u128(20);
        let mut state = NapCatState::default();
        state.review_by_code.insert(42, review_id);
        state.review_info.insert(
            review_id,
            ReviewInfo {
                review_code: 42,
                post_id,
                group_id: "group-a".to_string(),
            },
        );

        assert_eq!(
            validate_withdraw_action(&state, "group-a", 42),
            Err("该稿件不在暂存区")
        );

        state.send_plans.insert(
            post_id,
            SendPlanInfo {
                group_id: "group-a".to_string(),
                not_before_ms: 0,
                priority: SendPriority::Normal,
                seq: 1,
            },
        );
        assert_eq!(
            validate_withdraw_action(&state, "group-a", 42),
            Err("该稿件缺少外部编号")
        );

        state.post_external_code.insert(post_id, 1001);
        assert_eq!(validate_withdraw_action(&state, "group-a", 42), Ok(()));
    }
}
