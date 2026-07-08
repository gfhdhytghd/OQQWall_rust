use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oqqwall_rust_core::event::{BlobEvent, DraftEvent, IngressEvent, MediaEvent, RenderEvent};
use oqqwall_rust_core::state::PostStage;
use oqqwall_rust_core::{
    Command, GlobalAction, GlobalActionCommand, Id128, IngressAttachment, IngressCommand,
    IngressMessage, IngressRouteMeta, MediaKind, MediaReference, ReplyPreview, ReviewAction,
    ReviewActionCommand, StateView, build_draft_from_messages, derive_blob_id, derive_ingress_id,
    derive_post_id, derive_session_id, forward_marker, json_card_marker, poke_marker, reply_marker,
};
use oqqwall_rust_drivers::avatar_cache;
use oqqwall_rust_drivers::napcat::{
    napcat_account_for_group, napcat_account_online, napcat_ws_request,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};

use crate::config::AppConfig;
use crate::engine::EngineHandle;

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

const FULL_PERMISSIONS: [&str; 7] = [
    "review.read",
    "review.write",
    "send.execute",
    "blacklist.read",
    "blacklist.write",
    "session.manage",
    "token.manage",
];
const DEFAULT_SESSION_TTL_SEC: i64 = 12 * 60 * 60;
const CREATE_REVIEW_CODE_WAIT_MS: i64 = 3_000;
const CREATE_REVIEW_CODE_POLL_MS: u64 = 100;
const MAX_CREATE_WARNINGS: usize = 20;
const MAX_SEGMENT_PLACEHOLDER_LEN: usize = 64;
const STRANGER_INFO_TIMEOUT_MS: u64 = 3_000;
const SEND_PRIVATE_TIMEOUT_MS: u64 = 5_000;
const AVATAR_WAIT_AFTER_FETCH_MS: i64 = 1_500;
const AVATAR_WAIT_POLL_MS: u64 = 100;

#[derive(Clone)]
struct ApiState {
    cmd_tx: tokio::sync::mpsc::Sender<Command>,
    state: Arc<RwLock<StateView>>,
    auth: Arc<RwLock<AuthStore>>,
    tz_offset_minutes: i32,
    account_group_by_account: HashMap<String, String>,
    account_groups_by_account: HashMap<String, Vec<String>>,
    known_groups: HashSet<String>,
}

#[derive(Debug, Clone)]
struct ApiToken {
    token_id: String,
    permissions: BTreeSet<String>,
    expire_at: Option<i64>,
    allowed_groups: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone)]
struct ApiSession {
    session_id: String,
    token_id: String,
    permissions: BTreeSet<String>,
    expires_at: i64,
    allowed_groups: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone)]
struct CreatePostCachedResponse {
    post_id: String,
    review_code: Option<u32>,
    accepted_messages: usize,
    normalization: CreatePostNormalization,
    warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct AuthStore {
    tokens: HashMap<String, ApiToken>,
    sessions: HashMap<String, ApiSession>,
    idempotency_seen: HashSet<String>,
    create_idempotency: HashMap<String, CreatePostCachedResponse>,
    next_token_seq: u64,
}

impl AuthStore {
    fn new(root_token: String) -> Self {
        let mut tokens = HashMap::new();
        let permissions = FULL_PERMISSIONS.iter().map(|v| (*v).to_string()).collect();
        tokens.insert(
            root_token,
            ApiToken {
                token_id: "root".to_string(),
                permissions,
                expire_at: None,
                allowed_groups: None,
            },
        );
        Self {
            tokens,
            sessions: HashMap::new(),
            idempotency_seen: HashSet::new(),
            create_idempotency: HashMap::new(),
            next_token_seq: 1,
        }
    }
}

#[derive(Debug, Clone)]
struct AuthContext {
    session_id: String,
    token_id: String,
    allowed_groups: Option<BTreeSet<String>>,
}

#[derive(Serialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
    request_id: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    token: String,
}

#[derive(Serialize)]
struct LoginResponse {
    session_id: String,
    expires_at: i64,
    permissions: Vec<String>,
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    permissions: Vec<String>,
    #[serde(default)]
    expire_at: Option<i64>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
}

#[derive(Serialize)]
struct CreateTokenResponse {
    token: String,
    token_id: String,
    expire_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_groups: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ListPostsQuery {
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    cursor: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct PostListItem {
    post_id: String,
    review_id: Option<String>,
    group_id: String,
    stage: String,
    external_code: Option<u64>,
    internal_code: Option<u32>,
    sender_id: Option<String>,
    created_at_ms: i64,
    last_error: Option<String>,
}

#[derive(Serialize)]
struct ListPostsResponse {
    items: Vec<PostListItem>,
    next_cursor: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct PostDetailResponse {
    post_id: String,
    review_id: Option<String>,
    review_code: Option<u32>,
    group_id: String,
    stage: String,
    external_code: Option<u64>,
    sender_id: Option<String>,
    session_id: String,
    created_at_ms: i64,
    is_anonymous: bool,
    is_safe: bool,
    blocks: Vec<PostBlock>,
    render_png_blob_id: Option<String>,
    last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind")]
enum PostBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "attachment")]
    Attachment {
        media_kind: String,
        reference_type: String,
        reference: String,
        size_bytes: Option<u64>,
    },
}

#[derive(Deserialize)]
struct ReviewDecisionRequest {
    action: String,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    delay_ms: Option<i64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    quick_reply_key: Option<String>,
    #[serde(default)]
    target_review_code: Option<u32>,
}

#[derive(Serialize)]
struct ReviewDecisionResponse {
    review_id: String,
    status: &'static str,
}

#[derive(Deserialize)]
struct BatchReviewDecisionRequest {
    review_ids: Vec<String>,
    action: String,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    delay_ms: Option<i64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    quick_reply_key: Option<String>,
    #[serde(default)]
    target_review_code: Option<u32>,
}

#[derive(Serialize)]
struct BatchReviewDecisionResponse {
    accepted: usize,
    failed: Vec<ReviewFailure>,
}

#[derive(Serialize)]
struct ReviewFailure {
    review_id: String,
    reason: String,
}

#[derive(Deserialize)]
struct ListBlacklistQuery {
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    cursor: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct BlacklistItem {
    group_id: String,
    sender_id: String,
    reason: Option<String>,
}

#[derive(Serialize)]
struct ListBlacklistResponse {
    items: Vec<BlacklistItem>,
    next_cursor: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct CreateBlacklistRequest {
    group_id: String,
    sender_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct SendPostsRequest {
    post_ids: Vec<String>,
    mode: String,
    #[serde(default)]
    schedule_at: Option<i64>,
}

#[derive(Deserialize)]
struct SendPrivateMessageRequest {
    target_account: String,
    #[serde(default)]
    group_id: Option<String>,
    user_id: Value,
    message: Vec<Value>,
}

#[derive(Serialize)]
struct SendPostsResponse {
    accepted: usize,
    failed: Vec<SendFailure>,
}

#[derive(Serialize)]
struct SendFailure {
    post_id: String,
    reason: String,
}

#[derive(Serialize)]
struct SendPrivateMessageResponse {
    request_id: String,
    status: &'static str,
    target_account: String,
    group_id: String,
    user_id: String,
    message_id: Option<String>,
    raw: Value,
}

#[derive(Debug, Deserialize)]
struct CreatePostRequest {
    target_account: String,
    sender_id: String,
    #[serde(default)]
    sender_name: Option<String>,
    #[serde(default)]
    sender_avatar_base64: Option<String>,
    #[serde(default)]
    source_webhook: Option<String>,
    #[serde(default)]
    tag: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_request_tags")]
    tags: Vec<String>,
    messages: Vec<CreatePostMessage>,
}

#[derive(Debug, Deserialize)]
struct CreateRenderedPostRequest {
    target_account: String,
    image_base64: String,
    image_mime: String,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    sender_name: Option<String>,
    #[serde(default)]
    source_webhook: Option<String>,
    #[serde(default)]
    tag: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_request_tags")]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CreatePostMessage {
    message_id: Value,
    time: Value,
    #[serde(default)]
    message: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct CreatePostNormalization {
    unknown_segments: usize,
    invalid_segments_folded: usize,
}

#[derive(Debug, Serialize)]
struct CreatePostResponse {
    request_id: String,
    post_id: String,
    review_code: Option<u32>,
    accepted_messages: usize,
    normalization: CreatePostNormalization,
    warnings: Vec<String>,
}

fn build_ingress_route_meta(
    source_webhook: Option<&str>,
    tags: &[String],
) -> Option<IngressRouteMeta> {
    let source_webhook = source_webhook
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let tags = tags
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .fold(Vec::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == value) {
                acc.push(value.to_string());
            }
            acc
        });
    if source_webhook.is_none() && tags.is_empty() {
        return None;
    }
    Some(IngressRouteMeta {
        source_webhook,
        tags,
    })
}

fn deserialize_request_tags<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Value>::deserialize(deserializer)?;
    Ok(parse_request_tags_value(raw.as_ref()))
}

fn parse_request_tags_value(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) = value {
        append_request_tags_value(&mut out, value);
    }
    out
}

fn append_request_tags_value(out: &mut Vec<String>, value: &Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                append_request_tags_value(out, item);
            }
        }
        Value::String(text) => {
            for item in split_request_tag_text(text) {
                push_request_tag(out, &item);
            }
        }
        Value::Number(number) => push_request_tag(out, &number.to_string()),
        Value::Bool(flag) => push_request_tag(out, if *flag { "true" } else { "false" }),
        _ => {}
    }
}

fn split_request_tag_text(value: &str) -> Vec<String> {
    value
        .split(|ch| matches!(ch, ',' | '，' | ';' | '；' | '|' | '\n' | '\r'))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn push_request_tag(out: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() || out.iter().any(|existing| existing == trimmed) {
        return;
    }
    out.push(trimmed.to_string());
}

fn merge_request_tags(tags: &[String], singular_tag: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        push_request_tag(&mut out, tag);
    }
    if let Some(value) = singular_tag {
        append_request_tags_value(&mut out, value);
    }
    out
}

pub fn spawn_web_api(handle: &EngineHandle, config: &AppConfig) {
    if !config.web_api_enabled {
        debug_log!("web api disabled by config");
        return;
    }
    let Some(root_token) = config.web_api_root_token.clone() else {
        debug_log!("web api disabled: missing api token");
        return;
    };
    if root_token.len() < 32 {
        debug_log!("web api disabled: api token too short");
        return;
    }
    let core_config = config.build_core_config();
    let mut account_group_by_account = HashMap::new();
    let mut account_groups_by_account: HashMap<String, Vec<String>> = HashMap::new();
    let mut known_groups = HashSet::new();
    for group in &config.groups {
        known_groups.insert(group.group_id.clone());
        if let Some(group_cfg) = core_config.group_config(&group.group_id) {
            for account in &group_cfg.accounts {
                account_group_by_account
                    .entry(account.clone())
                    .or_insert_with(|| group.group_id.clone());
                account_groups_by_account
                    .entry(account.clone())
                    .or_default()
                    .push(group.group_id.clone());
            }
        }
    }

    let state = ApiState {
        cmd_tx: handle.cmd_tx.clone(),
        state: handle.state(),
        auth: Arc::new(RwLock::new(AuthStore::new(root_token))),
        tz_offset_minutes: config.tz_offset_minutes,
        account_group_by_account,
        account_groups_by_account,
        known_groups,
    };

    let app = Router::new()
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route(
            "/v1/auth/sessions/{session_id}/revoke",
            post(revoke_session),
        )
        .route("/v1/auth/tokens", post(create_token))
        .route("/v1/posts", get(list_posts))
        .route("/v1/posts/create", post(create_post))
        .route("/v1/posts/create_rendered", post(create_rendered_post))
        .route("/v1/posts/{post_id}", get(get_post))
        .route("/v1/blobs/{blob_id}", get(get_blob))
        .route("/v1/reviews/{review_id}/decision", post(decide_review))
        .route("/v1/reviews/batch", post(decide_review_batch))
        .route("/v1/blacklist", get(list_blacklist).post(create_blacklist))
        .route(
            "/v1/blacklist/{group_id}/{sender_id}",
            delete(delete_blacklist),
        )
        .route("/v1/posts/send", post(send_posts))
        .route("/v1/messages/private/send", post(send_private_message))
        .with_state(state);

    let bind_addr = format!("0.0.0.0:{}", config.web_api_port);
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(listener) => listener,
            Err(_err) => {
                debug_log!("web api bind failed {}: {}", bind_addr, _err);
                return;
            }
        };
        debug_log!("web api started: {}", bind_addr);
        if let Err(_err) = axum::serve(listener, app).await {
            debug_log!("web api stopped: {}", _err);
        }
    });
}

async fn login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let now = now_sec();

    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    let Some(token) = guard.tokens.get(&req.token).cloned() else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid token",
            request_id,
        );
    };
    if token.expire_at.map(|ts| ts <= now).unwrap_or(false) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "token expired",
            request_id,
        );
    }

    let session_id = random_hex32();
    let expires_at = now + DEFAULT_SESSION_TTL_SEC;
    let session = ApiSession {
        session_id: session_id.clone(),
        token_id: token.token_id,
        permissions: token.permissions.clone(),
        expires_at,
        allowed_groups: token.allowed_groups.clone(),
    };
    guard.sessions.insert(session_id.clone(), session);

    let resp = LoginResponse {
        session_id,
        expires_at,
        permissions: token.permissions.into_iter().collect(),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

async fn logout(State(state): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, None, &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    guard.sessions.remove(&auth.session_id);
    StatusCode::NO_CONTENT.into_response()
}

async fn revoke_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("session.manage"), &request_id) {
        return resp;
    }
    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    guard.sessions.remove(&session_id);
    StatusCode::NO_CONTENT.into_response()
}

async fn create_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("token.manage"), &request_id) {
        return resp;
    }

    let mut permissions = BTreeSet::new();
    for permission in req.permissions {
        if !FULL_PERMISSIONS.contains(&permission.as_str()) {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("unsupported permission: {}", permission),
                request_id,
            );
        }
        permissions.insert(permission);
    }
    if permissions.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "permissions cannot be empty",
            request_id,
        );
    }
    let allowed_groups = match normalize_allowed_groups(req.allowed_groups, &state.known_groups) {
        Ok(value) => value,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", &message, request_id);
        }
    };

    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    guard.next_token_seq = guard.next_token_seq.saturating_add(1);
    let token_id = format!("tok_{}", guard.next_token_seq);
    let token = random_hex32();
    guard.tokens.insert(
        token.clone(),
        ApiToken {
            token_id: token_id.clone(),
            permissions,
            expire_at: req.expire_at,
            allowed_groups: allowed_groups.clone(),
        },
    );

    (
        StatusCode::OK,
        Json(CreateTokenResponse {
            token,
            token_id,
            expire_at: req.expire_at,
            allowed_groups: allowed_groups
                .as_ref()
                .map(|groups| groups.iter().cloned().collect()),
        }),
    )
        .into_response()
}

async fn list_posts(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListPostsQuery>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.read"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let stage_filter = match query.stage.as_deref() {
        Some(value) => match parse_stage(value) {
            Some(stage) => Some(stage),
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "BAD_REQUEST",
                    "invalid stage",
                    request_id,
                );
            }
        },
        None => None,
    };

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let mut denied = 0usize;
    let mut items = guard
        .posts
        .values()
        .filter(|meta| {
            let stage_ok = stage_filter
                .map(|stage| meta.stage == stage)
                .unwrap_or(true);
            if !stage_ok {
                return false;
            }
            if group_allowed(&auth, &meta.group_id) {
                true
            } else {
                denied = denied.saturating_add(1);
                false
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.post_id.cmp(&a.post_id))
    });

    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let page = items.iter().skip(cursor).take(limit);
    let mut out = Vec::new();
    for meta in page {
        let review_code = meta
            .review_id
            .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));
        let sender_id = guard
            .session_ingress
            .get(&meta.session_id)
            .and_then(|ids| ids.first())
            .and_then(|id| guard.ingress_meta.get(id))
            .map(|ingress| ingress.user_id.clone());

        out.push(PostListItem {
            post_id: id_to_string(meta.post_id),
            review_id: meta.review_id.map(id_to_string),
            group_id: meta.group_id.clone(),
            stage: stage_to_string(meta.stage),
            external_code: guard.external_code_by_post.get(&meta.post_id).copied(),
            internal_code: review_code,
            sender_id,
            created_at_ms: meta.created_at_ms,
            last_error: meta.last_error.clone(),
        });
    }

    let next_cursor = if cursor + out.len() < items.len() {
        Some(cursor + out.len())
    } else {
        None
    };

    let mut warnings = Vec::new();
    if denied > 0 {
        warnings.push("results filtered by allowed_groups".to_string());
    }

    (
        StatusCode::OK,
        Json(ListPostsResponse {
            items: out,
            next_cursor,
            warnings,
        }),
    )
        .into_response()
}

async fn create_post(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreatePostRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let target_account = req.target_account.trim();
    let sender_id = req.sender_id.trim();
    let mut sender_name = req
        .sender_name
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if target_account.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "target_account cannot be empty",
            request_id,
        );
    }
    if sender_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "sender_id cannot be empty",
            request_id,
        );
    }
    if req.messages.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "messages cannot be empty",
            request_id,
        );
    }
    let Some(mapped_group) = state.account_group_by_account.get(target_account) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "unknown target_account",
            request_id,
        );
    };
    if !state.known_groups.contains(mapped_group) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "target_account mapped group is invalid",
            request_id,
        );
    }
    let group_id = mapped_group.as_str();
    if let Err(resp) = ensure_group_allowed(&auth, group_id, &request_id) {
        return resp;
    }

    let dedup_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|key| {
            format!(
                "create:{}:{}:{}:{}",
                auth.session_id, group_id, target_account, key
            )
        });
    if let Some(key) = dedup_key.as_ref() {
        let cached = match state.auth.write() {
            Ok(guard) => guard.create_idempotency.get(key).cloned(),
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "auth store unavailable",
                    request_id,
                );
            }
        };
        if let Some(cached) = cached {
            return (
                StatusCode::OK,
                Json(CreatePostResponse {
                    request_id,
                    post_id: cached.post_id,
                    review_code: cached.review_code,
                    accepted_messages: cached.accepted_messages,
                    normalization: cached.normalization,
                    warnings: cached.warnings,
                }),
            )
                .into_response();
        }
    }

    let mut warnings = Vec::new();
    let numeric_sender = is_digits_only(sender_id);
    let mut sender_name_from_stranger = false;
    if sender_name.is_none() && numeric_sender {
        match fetch_sender_name_from_stranger_info(target_account, sender_id).await {
            Ok(Some(name)) => {
                sender_name = Some(name);
                sender_name_from_stranger = true;
            }
            Ok(None) => push_warning(
                &mut warnings,
                "sender_name fallback failed: get_stranger_info returned empty nickname"
                    .to_string(),
            ),
            Err(err) => push_warning(
                &mut warnings,
                format!("sender_name fallback failed: {}", err),
            ),
        }
    }
    if sender_name.is_none() {
        sender_name = Some("未知".to_string());
    }

    let avatar_from_payload = match req.sender_avatar_base64.as_ref() {
        Some(raw) => match decode_sender_avatar_base64(raw) {
            Ok(Some(bytes)) => {
                avatar_cache::insert_avatar_bytes(sender_id, Arc::from(bytes));
                true
            }
            Ok(None) => false,
            Err(reason) => {
                push_warning(
                    &mut warnings,
                    format!("sender_avatar_base64 invalid, fallback applied: {}", reason),
                );
                false
            }
        },
        None => false,
    };
    if !avatar_from_payload {
        if numeric_sender && sender_name_from_stranger {
            if !trigger_avatar_fetch_and_wait(&state, sender_id).await {
                push_warning(
                    &mut warnings,
                    "avatar fallback failed, using default anonymous avatar".to_string(),
                );
            }
        } else {
            push_warning(
                &mut warnings,
                "avatar fallback skipped, using default anonymous avatar".to_string(),
            );
        }
    }

    let profile_id = format!("api_post_create:{}", target_account);
    let chat_id = dedup_key
        .as_ref()
        .map(|key| format!("api:create:{}:{}:{}", group_id, target_account, key))
        .unwrap_or_else(|| {
            format!(
                "api:create:{}:{}:{}",
                group_id,
                target_account,
                random_hex32()
            )
        });
    let mut commands = Vec::new();
    let mut normalization = CreatePostNormalization::default();
    let route_tags = merge_request_tags(&req.tags, req.tag.as_ref());
    let route_meta = build_ingress_route_meta(req.source_webhook.as_deref(), &route_tags);
    let mut first_platform_msg_id: Option<String> = None;
    let mut platform_msg_ids = HashSet::new();
    for (idx, message) in req.messages.iter().enumerate() {
        let mut platform_msg_id = match parse_message_id(&message.message_id) {
            Some(value) => value,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "BAD_REQUEST",
                    &format!("messages[{}].message_id is required", idx),
                    request_id,
                );
            }
        };
        if !platform_msg_ids.insert(platform_msg_id.clone()) {
            let original = platform_msg_id.clone();
            platform_msg_id = format!("{}#{}", platform_msg_id, idx);
            platform_msg_ids.insert(platform_msg_id.clone());
            push_warning(
                &mut warnings,
                format!(
                    "messages[{}].message_id duplicated, rewritten from {} to {}",
                    idx, original, platform_msg_id
                ),
            );
        }
        if first_platform_msg_id.is_none() {
            first_platform_msg_id = Some(platform_msg_id.clone());
        }
        let received_at_ms = match parse_received_at_ms(&message.time) {
            Some(value) => value,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "BAD_REQUEST",
                    &format!("messages[{}].time is required", idx),
                    request_id,
                );
            }
        };
        let normalized_message =
            normalize_segments(&message.message, &mut normalization, &mut warnings, idx);
        commands.push(IngressCommand {
            profile_id: profile_id.clone(),
            chat_id: chat_id.clone(),
            user_id: sender_id.to_string(),
            sender_name: sender_name.clone(),
            group_id: group_id.to_string(),
            platform_msg_id,
            message: normalized_message,
            route_meta: route_meta.clone(),
            received_at_ms,
            close_immediately: false,
        });
    }
    let Some(first_platform_msg_id) = first_platform_msg_id else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "messages cannot be empty",
            request_id,
        );
    };

    let first_ingress_id = derive_ingress_id(&[
        profile_id.as_bytes(),
        chat_id.as_bytes(),
        sender_id.as_bytes(),
        first_platform_msg_id.as_bytes(),
    ]);
    let first_ingress_bytes = first_ingress_id.to_be_bytes();
    let session_id = derive_session_id(&[
        chat_id.as_bytes(),
        sender_id.as_bytes(),
        group_id.as_bytes(),
        &first_ingress_bytes,
    ]);
    let post_id = derive_post_id(&[&session_id.to_be_bytes()]);

    for command in commands {
        if state.cmd_tx.send(Command::Ingress(command)).await.is_err() {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "UNAVAILABLE",
                "engine command channel closed",
                request_id,
            );
        }
    }

    let review_code = wait_review_code(&state, post_id).await;
    let accepted_messages = req.messages.len();
    let post_id_text = id_to_string(post_id);
    if let Some(key) = dedup_key {
        let mut guard = match state.auth.write() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "auth store unavailable",
                    request_id,
                );
            }
        };
        guard.create_idempotency.insert(
            key,
            CreatePostCachedResponse {
                post_id: post_id_text.clone(),
                review_code,
                accepted_messages,
                normalization: normalization.clone(),
                warnings: warnings.clone(),
            },
        );
    }

    (
        StatusCode::OK,
        Json(CreatePostResponse {
            request_id,
            post_id: post_id_text,
            review_code,
            accepted_messages,
            normalization,
            warnings,
        }),
    )
        .into_response()
}

async fn create_rendered_post(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateRenderedPostRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let target_account = req.target_account.trim();
    if target_account.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "target_account cannot be empty",
            request_id,
        );
    }
    let Some(mapped_group) = state.account_group_by_account.get(target_account) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "unknown target_account",
            request_id,
        );
    };
    if !state.known_groups.contains(mapped_group) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "target_account mapped group is invalid",
            request_id,
        );
    }
    let group_id = mapped_group.as_str();
    if let Err(resp) = ensure_group_allowed(&auth, group_id, &request_id) {
        return resp;
    }

    let image_bytes = match decode_required_base64_payload(&req.image_base64) {
        Ok(bytes) => bytes,
        Err(reason) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("image_base64 invalid: {}", reason),
                request_id,
            );
        }
    };
    let ext = match normalize_rendered_image_extension(&req.image_mime) {
        Some(ext) => ext,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "image_mime must be one of image/png,image/jpeg,image/jpg,image/webp",
                request_id,
            );
        }
    };

    let dedup_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|key| {
            format!(
                "create_rendered:{}:{}:{}:{}",
                auth.session_id, group_id, target_account, key
            )
        });
    if let Some(key) = dedup_key.as_ref() {
        let cached = match state.auth.write() {
            Ok(guard) => guard.create_idempotency.get(key).cloned(),
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "auth store unavailable",
                    request_id,
                );
            }
        };
        if let Some(cached) = cached {
            return (
                StatusCode::OK,
                Json(CreatePostResponse {
                    request_id,
                    post_id: cached.post_id,
                    review_code: cached.review_code,
                    accepted_messages: cached.accepted_messages,
                    normalization: cached.normalization,
                    warnings: cached.warnings,
                }),
            )
                .into_response();
        }
    }

    let mut warnings = Vec::new();
    let sender_id_input = req
        .sender_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut sender_name_input = req
        .sender_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let (sender_id, sender_name, is_anonymous) = if let Some(sender_id) = sender_id_input {
        if is_digits_only(sender_id) {
            if sender_name_input.is_none() {
                match fetch_sender_name_from_stranger_info(target_account, sender_id).await {
                    Ok(Some(name)) => sender_name_input = Some(name),
                    Ok(None) => push_warning(
                        &mut warnings,
                        "sender_name fallback failed: get_stranger_info returned empty nickname"
                            .to_string(),
                    ),
                    Err(err) => push_warning(
                        &mut warnings,
                        format!("sender_name fallback failed: {}", err),
                    ),
                }
            }
            if sender_name_input.is_none() {
                sender_name_input = Some("未知".to_string());
            }
            (sender_id.to_string(), sender_name_input, false)
        } else {
            push_warning(
                &mut warnings,
                "sender_id is not numeric, mention disabled for this post".to_string(),
            );
            if sender_name_input.is_some() {
                push_warning(
                    &mut warnings,
                    "sender_name ignored because sender_id is not numeric".to_string(),
                );
            }
            (sender_id.to_string(), None, false)
        }
    } else {
        if sender_name_input.is_some() {
            push_warning(
                &mut warnings,
                "sender_name ignored because sender_id is missing (anonymous post)".to_string(),
            );
        }
        ("unknown".to_string(), None, true)
    };

    let profile_id = format!("api_post_rendered:{}", target_account);
    let chat_id = dedup_key
        .as_ref()
        .map(|key| {
            format!(
                "api:create_rendered:{}:{}:{}",
                group_id, target_account, key
            )
        })
        .unwrap_or_else(|| {
            format!(
                "api:create_rendered:{}:{}:{}",
                group_id,
                target_account,
                random_hex32()
            )
        });
    let platform_msg_id = "rendered_image".to_string();
    let ingress_id = derive_ingress_id(&[
        profile_id.as_bytes(),
        chat_id.as_bytes(),
        sender_id.as_bytes(),
        platform_msg_id.as_bytes(),
    ]);
    let ingress_bytes = ingress_id.to_be_bytes();
    let session_id = derive_session_id(&[
        chat_id.as_bytes(),
        sender_id.as_bytes(),
        group_id.as_bytes(),
        &ingress_bytes,
    ]);
    let post_id = derive_post_id(&[&session_id.to_be_bytes()]);
    let blob_id = derive_blob_id(&[&post_id.to_be_bytes(), b"rendered"]);
    let persisted_path = match persist_rendered_blob(blob_id, ext, &image_bytes) {
        Ok(path) => path,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("persist rendered image failed: {}", err),
                request_id,
            );
        }
    };

    let attachment_size = u64::try_from(image_bytes.len()).unwrap_or(u64::MAX);
    let received_at_ms = now_ms();
    let route_tags = merge_request_tags(&req.tags, req.tag.as_ref());
    let route_meta = build_ingress_route_meta(req.source_webhook.as_deref(), &route_tags);
    let ingress_message = IngressMessage {
        text: String::new(),
        attachments: vec![IngressAttachment {
            kind: MediaKind::Image,
            name: Some(format!("rendered.{}", ext)),
            reference: MediaReference::Blob { blob_id },
            size_bytes: Some(attachment_size),
        }],
    };
    let draft = oqqwall_rust_core::draft::Draft {
        blocks: vec![oqqwall_rust_core::draft::DraftBlock::Attachment {
            kind: oqqwall_rust_core::draft::MediaKind::Image,
            name: Some(format!("rendered.{}", ext)),
            reference: oqqwall_rust_core::draft::MediaReference::Blob { blob_id },
            size_bytes: Some(attachment_size),
        }],
    };
    let driver_events = [
        oqqwall_rust_core::Event::Blob(BlobEvent::BlobRegistered {
            blob_id,
            size_bytes: attachment_size,
        }),
        oqqwall_rust_core::Event::Blob(BlobEvent::BlobPersisted {
            blob_id,
            path: persisted_path,
        }),
        oqqwall_rust_core::Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id,
            profile_id: profile_id.clone(),
            chat_id: chat_id.clone(),
            user_id: sender_id.clone(),
            sender_name: sender_name.clone(),
            group_id: group_id.to_string(),
            platform_msg_id,
            route_meta,
            received_at_ms,
            message: ingress_message,
        }),
        oqqwall_rust_core::Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: group_id.to_string(),
            ingress_ids: vec![ingress_id],
            is_anonymous,
            is_safe: true,
            draft,
            created_at_ms: received_at_ms,
        }),
        oqqwall_rust_core::Event::Render(RenderEvent::PngReady { post_id, blob_id }),
    ];
    for event in driver_events {
        if state
            .cmd_tx
            .send(Command::DriverEvent(event))
            .await
            .is_err()
        {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "UNAVAILABLE",
                "engine command channel closed",
                request_id,
            );
        }
    }

    let review_code = wait_review_code(&state, post_id).await;
    let accepted_messages = 1usize;
    let normalization = CreatePostNormalization::default();
    let post_id_text = id_to_string(post_id);
    if let Some(key) = dedup_key {
        let mut guard = match state.auth.write() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "auth store unavailable",
                    request_id,
                );
            }
        };
        guard.create_idempotency.insert(
            key,
            CreatePostCachedResponse {
                post_id: post_id_text.clone(),
                review_code,
                accepted_messages,
                normalization: normalization.clone(),
                warnings: warnings.clone(),
            },
        );
    }

    (
        StatusCode::OK,
        Json(CreatePostResponse {
            request_id,
            post_id: post_id_text,
            review_code,
            accepted_messages,
            normalization,
            warnings,
        }),
    )
        .into_response()
}

async fn get_post(
    State(state): State<ApiState>,
    Path(post_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.read"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let Some(post_id) = parse_id128(&post_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "invalid post_id",
            request_id,
        );
    };

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let Some(meta) = guard.posts.get(&post_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "post not found",
            request_id,
        );
    };
    if let Err(resp) = ensure_group_allowed(&auth, &meta.group_id, &request_id) {
        return resp;
    }

    let review_code = meta
        .review_id
        .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));
    let sender_id = guard
        .session_ingress
        .get(&meta.session_id)
        .and_then(|ids| ids.first())
        .and_then(|id| guard.ingress_meta.get(id))
        .map(|ingress| ingress.user_id.clone());

    let blocks = guard
        .drafts
        .get(&post_id)
        .map(|draft| {
            draft
                .blocks
                .iter()
                .map(|block| match block {
                    oqqwall_rust_core::draft::DraftBlock::Paragraph { text } => {
                        PostBlock::Text { text: text.clone() }
                    }
                    oqqwall_rust_core::draft::DraftBlock::Attachment {
                        kind,
                        reference,
                        size_bytes,
                        ..
                    } => {
                        let (reference_type, reference) = match reference {
                            oqqwall_rust_core::draft::MediaReference::RemoteUrl { url } => {
                                ("remote_url".to_string(), url.clone())
                            }
                            oqqwall_rust_core::draft::MediaReference::Blob { blob_id } => {
                                ("blob_id".to_string(), id_to_string(*blob_id))
                            }
                        };
                        PostBlock::Attachment {
                            media_kind: media_kind_to_string(*kind),
                            reference_type,
                            reference,
                            size_bytes: *size_bytes,
                        }
                    }
                    oqqwall_rust_core::draft::DraftBlock::Reply { preview } => PostBlock::Text {
                        text: format!("[回复] {}", preview.body),
                    },
                    oqqwall_rust_core::draft::DraftBlock::Poke => PostBlock::Text {
                        text: "[戳一戳]".to_string(),
                    },
                    oqqwall_rust_core::draft::DraftBlock::JsonCard { .. } => PostBlock::Text {
                        text: "[卡片]".to_string(),
                    },
                    oqqwall_rust_core::draft::DraftBlock::Forward { items } => PostBlock::Text {
                        text: format!("[合并转发:{}条]", items.len()),
                    },
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let render_png_blob_id = guard
        .render
        .get(&post_id)
        .and_then(|render| render.png_blob)
        .map(id_to_string);

    (
        StatusCode::OK,
        Json(PostDetailResponse {
            post_id: id_to_string(meta.post_id),
            review_id: meta.review_id.map(id_to_string),
            review_code,
            group_id: meta.group_id.clone(),
            stage: stage_to_string(meta.stage),
            external_code: guard.external_code_by_post.get(&meta.post_id).copied(),
            sender_id,
            session_id: id_to_string(meta.session_id),
            created_at_ms: meta.created_at_ms,
            is_anonymous: meta.is_anonymous,
            is_safe: meta.is_safe,
            blocks,
            render_png_blob_id,
            last_error: meta.last_error.clone(),
        }),
    )
        .into_response()
}

async fn get_blob(
    State(state): State<ApiState>,
    Path(blob_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.read"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let Some(blob_id) = parse_id128(&blob_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "invalid blob_id",
            request_id,
        );
    };

    let path = {
        let guard = match state.state.read() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "state unavailable",
                    request_id,
                );
            }
        };
        let Some(meta) = guard.blobs.get(&blob_id) else {
            return error_response(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "blob not found",
                request_id,
            );
        };
        let groups = collect_blob_groups(&guard, blob_id);
        if auth.allowed_groups.is_some() && !groups.is_empty() && !groups_allowed(&auth, &groups) {
            return error_response(
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "group is not allowed for current token",
                request_id,
            );
        }
        if auth.allowed_groups.is_some() && groups.is_empty() {
            return error_response(
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "blob group is not allowed for current token",
                request_id,
            );
        }
        let Some(path) = meta.persisted_path.clone() else {
            return error_response(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "blob has no persisted path",
                request_id,
            );
        };
        path
    };

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "blob file missing",
                request_id,
            );
        }
    };

    let mime = match path.rsplit('.').next().map(|ext| ext.to_ascii_lowercase()) {
        Some(ext) if ext == "png" => "image/png",
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ext) if ext == "gif" => "image/gif",
        Some(ext) if ext == "webp" => "image/webp",
        Some(ext) if ext == "mp4" => "video/mp4",
        Some(ext) if ext == "mp3" => "audio/mpeg",
        Some(ext) if ext == "wav" => "audio/wav",
        _ => "application/octet-stream",
    };
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=60"),
    );
    (StatusCode::OK, response_headers, bytes).into_response()
}

async fn decide_review(
    State(state): State<ApiState>,
    Path(review_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ReviewDecisionRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let Some(review_id) = parse_id128(&review_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "invalid review_id",
            request_id,
        );
    };
    {
        let snapshot = match state.state.read() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "state unavailable",
                    request_id,
                );
            }
        };
        if let Some(group_id) = group_id_of_review(&snapshot, review_id) {
            if let Err(resp) = ensure_group_allowed(&auth, group_id, &request_id) {
                return resp;
            }
        }
    }

    if let Some(key) = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        let dedup_key = format!("review:{}:{}:{}", auth.session_id, review_id.0, key);
        let mut auth_guard = match state.auth.write() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "auth store unavailable",
                    request_id,
                );
            }
        };
        if auth_guard.idempotency_seen.contains(&dedup_key) {
            return (
                StatusCode::OK,
                Json(ReviewDecisionResponse {
                    review_id: id_to_string(review_id),
                    status: "applied",
                }),
            )
                .into_response();
        }
        auth_guard.idempotency_seen.insert(dedup_key);
    }

    let action = match parse_review_action(&req) {
        Ok(action) => action,
        Err(reason) => {
            return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", reason, request_id);
        }
    };

    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: Some(review_id),
        review_code: None,
        audit_msg_id: None,
        action,
        operator_id: format!("api:{}", auth.token_id),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });

    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
            request_id,
        );
    }

    (
        StatusCode::OK,
        Json(ReviewDecisionResponse {
            review_id: id_to_string(review_id),
            status: "applied",
        }),
    )
        .into_response()
}

async fn decide_review_batch(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<BatchReviewDecisionRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let action_template = ReviewDecisionRequest {
        action: req.action,
        comment: req.comment,
        delay_ms: req.delay_ms,
        text: req.text,
        quick_reply_key: req.quick_reply_key,
        target_review_code: req.target_review_code,
    };
    let action = match parse_review_action(&action_template) {
        Ok(action) => action,
        Err(reason) => {
            return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", reason, request_id);
        }
    };

    let snapshot = match state.state.read() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let mut accepted = 0usize;
    let mut failed = Vec::new();
    for raw_review_id in req.review_ids {
        let Some(review_id) = parse_id128(&raw_review_id) else {
            failed.push(ReviewFailure {
                review_id: raw_review_id,
                reason: "invalid review_id".to_string(),
            });
            continue;
        };
        let Some(group_id) = group_id_of_review(&snapshot, review_id) else {
            failed.push(ReviewFailure {
                review_id: id_to_string(review_id),
                reason: "review not found".to_string(),
            });
            continue;
        };
        if !group_allowed(&auth, group_id) {
            failed.push(ReviewFailure {
                review_id: id_to_string(review_id),
                reason: "permission denied for group".to_string(),
            });
            continue;
        }
        let cmd = Command::ReviewAction(ReviewActionCommand {
            review_id: Some(review_id),
            review_code: None,
            audit_msg_id: None,
            action: action.clone(),
            operator_id: format!("api:{}", auth.token_id),
            now_ms: now_ms(),
            tz_offset_minutes: state.tz_offset_minutes,
        });
        if state.cmd_tx.send(cmd).await.is_err() {
            failed.push(ReviewFailure {
                review_id: id_to_string(review_id),
                reason: "engine command channel closed".to_string(),
            });
            continue;
        }
        accepted = accepted.saturating_add(1);
    }

    (
        StatusCode::OK,
        Json(BatchReviewDecisionResponse { accepted, failed }),
    )
        .into_response()
}

async fn list_blacklist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListBlacklistQuery>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("blacklist.read"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let selected_group = query
        .group_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let mut warnings = Vec::new();
    if let Some(group_id) = selected_group {
        if !group_allowed(&auth, group_id) {
            warnings.push("results filtered by allowed_groups".to_string());
            return (
                StatusCode::OK,
                Json(ListBlacklistResponse {
                    items: Vec::new(),
                    next_cursor: None,
                    warnings,
                }),
            )
                .into_response();
        }
    }

    let mut rows = Vec::new();
    let mut denied = 0usize;
    for (group_id, group) in &guard.blacklist {
        if selected_group
            .map(|selected| selected != group_id)
            .unwrap_or(false)
        {
            continue;
        }
        if !group_allowed(&auth, group_id) {
            denied = denied.saturating_add(1);
            continue;
        }
        for (sender_id, reason) in group {
            rows.push(BlacklistItem {
                group_id: group_id.clone(),
                sender_id: sender_id.clone(),
                reason: reason.clone(),
            });
        }
    }
    rows.sort_by(|a, b| {
        a.group_id
            .cmp(&b.group_id)
            .then_with(|| a.sender_id.cmp(&b.sender_id))
    });

    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = rows
        .iter()
        .skip(cursor)
        .take(limit)
        .map(|row| BlacklistItem {
            group_id: row.group_id.clone(),
            sender_id: row.sender_id.clone(),
            reason: row.reason.clone(),
        })
        .collect::<Vec<_>>();
    let next_cursor = if cursor + items.len() < rows.len() {
        Some(cursor + items.len())
    } else {
        None
    };
    if denied > 0 {
        warnings.push("results filtered by allowed_groups".to_string());
    }

    (
        StatusCode::OK,
        Json(ListBlacklistResponse {
            items,
            next_cursor,
            warnings,
        }),
    )
        .into_response()
}

async fn create_blacklist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateBlacklistRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("blacklist.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let sender_id = req.sender_id.trim();
    let group_id = req.group_id.trim();
    if sender_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "sender_id cannot be empty",
            request_id,
        );
    }
    if group_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "group_id cannot be empty",
            request_id,
        );
    }
    if let Err(resp) = ensure_group_allowed(&auth, group_id, &request_id) {
        return resp;
    }
    let cmd = Command::GlobalAction(GlobalActionCommand {
        group_id: group_id.to_string(),
        action: GlobalAction::BlacklistAdd {
            sender_id: sender_id.to_string(),
            reason: req
                .reason
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        },
        operator_id: format!("api:{}", auth.token_id),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
            request_id,
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn delete_blacklist(
    State(state): State<ApiState>,
    Path((group_id, sender_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("blacklist.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    if let Err(resp) = ensure_group_allowed(&auth, &group_id, &request_id) {
        return resp;
    }

    let cmd = Command::GlobalAction(GlobalActionCommand {
        group_id,
        action: GlobalAction::BlacklistRemove {
            sender_id: sender_id.clone(),
        },
        operator_id: format!("api:{}", auth.token_id),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
            request_id,
        );
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn send_posts(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<SendPostsRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("send.execute"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    if req.mode == "scheduled" && req.schedule_at.is_some() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNSUPPORTED",
            "schedule_at is not supported by current command model",
            request_id,
        );
    }

    let mut failed = Vec::new();
    let mut accepted = 0usize;
    let snapshot = match state.state.read() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    for raw_post_id in req.post_ids {
        let Some(post_id) = parse_id128(&raw_post_id) else {
            failed.push(SendFailure {
                post_id: raw_post_id,
                reason: "invalid post_id".to_string(),
            });
            continue;
        };
        let Some(post) = snapshot.posts.get(&post_id) else {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "post not found".to_string(),
            });
            continue;
        };
        if !group_allowed(&auth, &post.group_id) {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "permission denied for group".to_string(),
            });
            continue;
        }
        let Some(review_id) = post.review_id else {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "post has no review_id".to_string(),
            });
            continue;
        };

        let action = match req.mode.as_str() {
            "immediate" => ReviewAction::Immediate,
            "scheduled" => ReviewAction::Approve,
            _ => {
                failed.push(SendFailure {
                    post_id: id_to_string(post_id),
                    reason: "unsupported mode".to_string(),
                });
                continue;
            }
        };

        let cmd = Command::ReviewAction(ReviewActionCommand {
            review_id: Some(review_id),
            review_code: None,
            audit_msg_id: None,
            action,
            operator_id: format!("api:{}", auth.token_id),
            now_ms: now_ms(),
            tz_offset_minutes: state.tz_offset_minutes,
        });
        if state.cmd_tx.send(cmd).await.is_err() {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "engine command channel closed".to_string(),
            });
            continue;
        }
        accepted = accepted.saturating_add(1);
    }

    (StatusCode::OK, Json(SendPostsResponse { accepted, failed })).into_response()
}

async fn send_private_message(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<SendPrivateMessageRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("send.execute"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let target_account = req.target_account.trim();
    if target_account.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "target_account cannot be empty",
            request_id,
        );
    }
    if !state.account_groups_by_account.contains_key(target_account) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "unknown target_account",
            request_id,
        );
    }
    if req.message.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "message cannot be empty",
            request_id,
        );
    }
    if req.message.iter().any(|segment| !segment.is_object()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "message segments must be objects",
            request_id,
        );
    }
    let Some(user_id) = value_to_string(&req.user_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "user_id is required",
            request_id,
        );
    };
    let user_id = user_id.trim().to_string();
    if user_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "user_id is required",
            request_id,
        );
    }

    let group_id = match resolve_group_for_target_account(
        &state,
        &auth,
        target_account,
        req.group_id.as_deref(),
        &request_id,
    ) {
        Ok(group_id) => group_id,
        Err(resp) => return resp,
    };

    let response = match napcat_ws_request(
        target_account,
        "send_private_msg",
        json!({
            "user_id": user_id,
            "message": req.message,
        }),
        Duration::from_millis(SEND_PRIVATE_TIMEOUT_MS),
    )
    .await
    {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "UNAVAILABLE",
                &format!("send_private_msg failed: {}", err),
                request_id,
            );
        }
    };

    let message_id = response
        .get("data")
        .and_then(|value| value.get("message_id"))
        .and_then(value_to_string);

    (
        StatusCode::OK,
        Json(SendPrivateMessageResponse {
            request_id,
            status: "ok",
            target_account: target_account.to_string(),
            group_id,
            user_id,
            message_id,
            raw: response,
        }),
    )
        .into_response()
}

fn resolve_group_for_target_account(
    state: &ApiState,
    auth: &AuthContext,
    target_account: &str,
    requested_group_id: Option<&str>,
    request_id: &str,
) -> Result<String, axum::response::Response> {
    let Some(candidate_groups) = state.account_groups_by_account.get(target_account) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "unknown target_account",
            request_id.to_string(),
        ));
    };

    if let Some(raw_group_id) = requested_group_id {
        let group_id = raw_group_id.trim();
        if group_id.is_empty() {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "group_id cannot be empty",
                request_id.to_string(),
            ));
        }
        if !state.known_groups.contains(group_id) {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "unknown group_id",
                request_id.to_string(),
            ));
        }
        if !candidate_groups.iter().any(|group| group == group_id) {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "target_account is not configured in group_id",
                request_id.to_string(),
            ));
        }
        if !group_allowed(auth, group_id) {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "group is not allowed for current token",
                request_id.to_string(),
            ));
        }
        if !napcat_account_online(target_account) {
            return Err(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "UNAVAILABLE",
                "target_account is not online",
                request_id.to_string(),
            ));
        }
        return Ok(group_id.to_string());
    }

    let allowed_candidates = candidate_groups
        .iter()
        .filter(|group_id| group_allowed(auth, group_id))
        .cloned()
        .collect::<Vec<_>>();
    if allowed_candidates.is_empty() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "target_account is not in allowed_groups",
            request_id.to_string(),
        ));
    }
    if !napcat_account_online(target_account) {
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "target_account is not online",
            request_id.to_string(),
        ));
    }
    if allowed_candidates.len() == 1 {
        return Ok(allowed_candidates[0].clone());
    }
    let primary_matches = allowed_candidates
        .into_iter()
        .filter(|group_id| napcat_account_for_group(group_id).as_deref() == Some(target_account))
        .collect::<Vec<_>>();
    match primary_matches.len() {
        1 => Ok(primary_matches[0].clone()),
        0 => Err(error_response(
            StatusCode::CONFLICT,
            "CONFLICT",
            "target_account matches multiple allowed groups, please provide group_id",
            request_id.to_string(),
        )),
        _ => Err(error_response(
            StatusCode::CONFLICT,
            "CONFLICT",
            "target_account matches multiple online groups, please provide group_id",
            request_id.to_string(),
        )),
    }
}

fn normalize_allowed_groups(
    input: Option<Vec<String>>,
    known_groups: &HashSet<String>,
) -> Result<Option<BTreeSet<String>>, String> {
    let Some(input) = input else {
        return Ok(None);
    };
    let mut groups = BTreeSet::new();
    for raw in input {
        let group = raw.trim();
        if group.is_empty() {
            return Err("allowed_groups contains empty value".to_string());
        }
        if !known_groups.contains(group) {
            return Err(format!("allowed_groups contains unknown group {}", group));
        }
        groups.insert(group.to_string());
    }
    if groups.is_empty() {
        return Err("allowed_groups cannot be empty".to_string());
    }
    Ok(Some(groups))
}

fn group_allowed(auth: &AuthContext, group_id: &str) -> bool {
    auth.allowed_groups
        .as_ref()
        .map(|groups| groups.contains(group_id))
        .unwrap_or(true)
}

fn groups_allowed(auth: &AuthContext, groups: &HashSet<String>) -> bool {
    groups.iter().any(|group_id| group_allowed(auth, group_id))
}

fn ensure_group_allowed(
    auth: &AuthContext,
    group_id: &str,
    request_id: &str,
) -> Result<(), axum::response::Response> {
    if group_allowed(auth, group_id) {
        Ok(())
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "group is not allowed for current token",
            request_id.to_string(),
        ))
    }
}

fn group_id_of_review(state: &StateView, review_id: Id128) -> Option<&str> {
    let review = state.reviews.get(&review_id)?;
    let post = state.posts.get(&review.post_id)?;
    Some(post.group_id.as_str())
}

fn collect_blob_groups(state: &StateView, blob_id: Id128) -> HashSet<String> {
    let mut groups = HashSet::new();
    for (post_id, render) in &state.render {
        if render.png_blob == Some(blob_id) {
            if let Some(post) = state.posts.get(post_id) {
                groups.insert(post.group_id.clone());
            }
        }
    }
    for (post_id, draft) in &state.drafts {
        let linked = draft.blocks.iter().any(|block| {
            matches!(
                block,
                oqqwall_rust_core::draft::DraftBlock::Attachment {
                    reference: oqqwall_rust_core::draft::MediaReference::Blob {
                        blob_id: draft_blob_id
                    },
                    ..
                } if *draft_blob_id == blob_id
            )
        });
        if linked {
            if let Some(post) = state.posts.get(post_id) {
                groups.insert(post.group_id.clone());
            }
        }
    }
    groups
}

fn is_digits_only(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn decode_sender_avatar_base64(raw: &str) -> Result<Option<Vec<u8>>, &'static str> {
    let payload = raw.trim();
    if payload.is_empty() {
        return Ok(None);
    }
    let decoded = STANDARD
        .decode(payload)
        .map_err(|_| "invalid base64 payload")?;
    if decoded.is_empty() {
        return Ok(None);
    }
    Ok(Some(decoded))
}

fn decode_required_base64_payload(raw: &str) -> Result<Vec<u8>, &'static str> {
    let payload = raw.trim();
    if payload.is_empty() {
        return Err("empty payload");
    }
    let decoded = STANDARD
        .decode(payload)
        .map_err(|_| "invalid base64 payload")?;
    if decoded.is_empty() {
        return Err("empty decoded payload");
    }
    Ok(decoded)
}

fn normalize_rendered_image_extension(raw_mime: &str) -> Option<&'static str> {
    let mime = raw_mime.trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn persist_rendered_blob(blob_id: Id128, ext: &str, bytes: &[u8]) -> Result<String, String> {
    let dir = blob_root().join(ext);
    fs::create_dir_all(&dir).map_err(|err| format!("create dir failed: {}", err))?;
    let filename = format!("{}.{}", id128_hex(blob_id.0), ext);
    let path = dir.join(filename);
    fs::write(&path, bytes).map_err(|err| format!("write file failed: {}", err))?;
    Ok(path.to_string_lossy().to_string())
}

fn blob_root() -> PathBuf {
    std::env::var("OQQWALL_BLOB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/blobs"))
}

async fn fetch_sender_name_from_stranger_info(
    target_account: &str,
    sender_id: &str,
) -> Result<Option<String>, String> {
    let response = napcat_ws_request(
        target_account,
        "get_stranger_info",
        json!({ "user_id": sender_id }),
        Duration::from_millis(STRANGER_INFO_TIMEOUT_MS),
    )
    .await?;
    let data = response
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "missing data in get_stranger_info response".to_string())?;
    let nickname = data
        .get("nickname")
        .and_then(value_to_string)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(nickname)
}

async fn trigger_avatar_fetch_and_wait(state: &ApiState, sender_id: &str) -> bool {
    if avatar_cache::has_avatar(sender_id) {
        return true;
    }
    let event = oqqwall_rust_core::Event::Media(MediaEvent::AvatarFetchRequested {
        user_id: sender_id.to_string(),
    });
    if state
        .cmd_tx
        .send(Command::DriverEvent(event))
        .await
        .is_err()
    {
        return false;
    }
    wait_avatar_cached(sender_id, AVATAR_WAIT_AFTER_FETCH_MS).await
}

async fn wait_avatar_cached(sender_id: &str, timeout_ms: i64) -> bool {
    let timeout_ms = timeout_ms.max(0);
    let deadline = now_ms().saturating_add(timeout_ms);
    loop {
        if avatar_cache::has_avatar(sender_id) {
            return true;
        }
        if now_ms() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(AVATAR_WAIT_POLL_MS)).await;
    }
}

fn parse_message_id(value: &Value) -> Option<String> {
    value_to_string(value)
}

fn parse_received_at_ms(value: &Value) -> Option<i64> {
    let raw = match value {
        Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                i as i128
            } else if let Some(u) = v.as_u64() {
                u as i128
            } else {
                return None;
            }
        }
        Value::String(v) => v.trim().parse::<i128>().ok()?,
        _ => return None,
    };
    if raw <= 0 {
        return None;
    }
    let ms = if raw >= 100_000_000_000i128 {
        raw
    } else {
        raw.saturating_mul(1000)
    };
    Some(ms.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(v) => Some(v.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        _ => None,
    }
}

fn parse_name(data: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    let data = data?;
    ["name", "file_name", "filename", "file", "path", "url"]
        .iter()
        .find_map(|key| data.get(*key).and_then(value_to_string))
        .map(|value| filename_from_reference(&value))
        .filter(|v| !v.is_empty())
}

fn media_kind_for_segment(
    segment_type: &str,
    data: Option<&serde_json::Map<String, Value>>,
) -> MediaKind {
    match segment_type {
        "video" => MediaKind::Video,
        "record" => MediaKind::Audio,
        "file" => {
            let mime_is_image = data
                .and_then(|map| map.get("mime").or_else(|| map.get("mime_type")))
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|value| value.starts_with("image/"));
            if mime_is_image || parse_name(data).as_deref().is_some_and(is_image_filename) {
                MediaKind::Image
            } else {
                MediaKind::File
            }
        }
        _ => MediaKind::Other,
    }
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

fn normalize_segments(
    segments: &[Value],
    normalization: &mut CreatePostNormalization,
    warnings: &mut Vec<String>,
    message_index: usize,
) -> IngressMessage {
    let mut text = String::new();
    let mut attachments = Vec::new();

    for (segment_index, segment) in segments.iter().enumerate() {
        let Some(segment_obj) = segment.as_object() else {
            fold_invalid_segment(
                normalization,
                warnings,
                &mut text,
                message_index,
                segment_index,
                "invalid_segment",
                "segment is not object",
            );
            continue;
        };
        let segment_type = segment_obj
            .get("type")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown");
        let data = segment_obj.get("data").and_then(|value| value.as_object());
        match segment_type {
            "text" => {
                let value = data
                    .and_then(|map| map.get("text"))
                    .and_then(value_to_string);
                if let Some(value) = value {
                    text.push_str(&value);
                } else {
                    fold_invalid_segment(
                        normalization,
                        warnings,
                        &mut text,
                        message_index,
                        segment_index,
                        "text",
                        "missing data.text",
                    );
                }
            }
            "face" => {
                let face_id = data
                    .and_then(|map| map.get("id"))
                    .and_then(value_to_string)
                    .filter(|value| value.chars().all(|c| c.is_ascii_digit()));
                if let Some(face_id) = face_id {
                    text.push_str(&format!("[[face:{}]]", face_id));
                } else {
                    fold_invalid_segment(
                        normalization,
                        warnings,
                        &mut text,
                        message_index,
                        segment_index,
                        "face",
                        "missing valid face id",
                    );
                }
            }
            "image" => {
                let Some(data) = data else {
                    fold_invalid_segment(
                        normalization,
                        warnings,
                        &mut text,
                        message_index,
                        segment_index,
                        "image",
                        "missing data object",
                    );
                    continue;
                };
                match build_data_uri_reference(data, "image/jpeg") {
                    Ok((reference, size_bytes)) => attachments.push(IngressAttachment {
                        kind: MediaKind::Image,
                        name: parse_name(Some(data)),
                        reference,
                        size_bytes: Some(size_bytes),
                    }),
                    Err(reason) => fold_invalid_segment(
                        normalization,
                        warnings,
                        &mut text,
                        message_index,
                        segment_index,
                        "image",
                        reason,
                    ),
                }
            }
            "video" | "file" | "record" => {
                if segment_type == "record" {
                    text.push_str("[语音]");
                }
                let Some(data) = data else {
                    fold_invalid_segment(
                        normalization,
                        warnings,
                        &mut text,
                        message_index,
                        segment_index,
                        segment_type,
                        "missing data object",
                    );
                    continue;
                };
                let fallback_mime = match segment_type {
                    "video" => "video/mp4",
                    "record" => "audio/mpeg",
                    _ => "application/octet-stream",
                };
                match extract_media_reference(data, fallback_mime) {
                    Ok((reference, size_bytes)) => attachments.push(IngressAttachment {
                        kind: media_kind_for_segment(segment_type, Some(data)),
                        name: parse_name(Some(data)),
                        reference,
                        size_bytes,
                    }),
                    Err(reason) => fold_invalid_segment(
                        normalization,
                        warnings,
                        &mut text,
                        message_index,
                        segment_index,
                        segment_type,
                        reason,
                    ),
                }
            }
            "reply" => {
                let reply_id = data
                    .and_then(|map| map.get("id"))
                    .and_then(value_to_string)
                    .filter(|value| !value.trim().is_empty());
                let body = reply_id
                    .as_ref()
                    .map(|id| format!("引用的消息 ID: {}", id))
                    .unwrap_or_else(|| "引用的消息".to_string());
                text.push_str(&reply_marker(&ReplyPreview {
                    id: reply_id,
                    meta: None,
                    body,
                    missing: false,
                }));
            }
            "forward" => {
                if let Some(items) = data.and_then(normalize_forward_items) {
                    text.push_str(&forward_marker(&items));
                } else if let Some(forward_id) =
                    data.and_then(|map| map.get("id")).and_then(value_to_string)
                {
                    text.push_str(&format!("[合并转发:{}]", forward_id));
                } else {
                    text.push_str("[合并转发]");
                }
            }
            "json" => {
                let raw = data
                    .and_then(|map| map.get("data"))
                    .and_then(value_to_string)
                    .or_else(|| {
                        data.and_then(|map| serde_json::to_string(&Value::Object(map.clone())).ok())
                    })
                    .unwrap_or_default();
                text.push_str(&json_card_marker(&raw));
            }
            "poke" => text.push_str(poke_marker()),
            other => fold_unknown_segment(
                normalization,
                warnings,
                &mut text,
                message_index,
                segment_index,
                other,
            ),
        }
    }

    IngressMessage {
        text: text.trim().to_string(),
        attachments,
    }
}

fn normalize_forward_items(
    data: &serde_json::Map<String, Value>,
) -> Option<Vec<oqqwall_rust_core::ForwardItem>> {
    let list = data
        .get("messages")
        .or_else(|| data.get("content"))
        .and_then(Value::as_array)?;
    let mut out = Vec::new();
    for item in list {
        let Some(item_obj) = item.as_object() else {
            continue;
        };
        let sender_name = item_obj
            .get("sender")
            .and_then(Value::as_object)
            .and_then(|sender| {
                sender
                    .get("nickname")
                    .or_else(|| sender.get("card"))
                    .and_then(value_to_string)
            })
            .or_else(|| item_obj.get("sender_name").and_then(value_to_string))
            .or_else(|| item_obj.get("nickname").and_then(value_to_string));
        let Some(message) = item_obj.get("message").and_then(Value::as_array) else {
            continue;
        };
        let mut normalization = CreatePostNormalization::default();
        let mut warnings = Vec::new();
        let ingress = normalize_segments(message, &mut normalization, &mut warnings, 0);
        let draft = build_draft_from_messages(&[ingress]);
        out.push(oqqwall_rust_core::ForwardItem {
            sender_name,
            blocks: draft.blocks,
        });
    }
    (!out.is_empty()).then_some(out)
}

fn extract_media_reference(
    data: &serde_json::Map<String, Value>,
    fallback_mime: &str,
) -> Result<(MediaReference, Option<u64>), &'static str> {
    if data
        .get("base64")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        let (reference, size_bytes) = build_data_uri_reference(data, fallback_mime)?;
        return Ok((reference, Some(size_bytes)));
    }
    if let Some(url) = data.get("url").and_then(value_to_string) {
        return Ok((MediaReference::RemoteUrl { url }, None));
    }
    if let Some(file) = data.get("file").and_then(value_to_string) {
        return Ok((MediaReference::RemoteUrl { url: file }, None));
    }
    if let Some(path) = data.get("path").and_then(value_to_string) {
        return Ok((MediaReference::RemoteUrl { url: path }, None));
    }
    Err("missing media reference")
}

fn build_data_uri_reference(
    data: &serde_json::Map<String, Value>,
    fallback_mime: &str,
) -> Result<(MediaReference, u64), &'static str> {
    let payload = data
        .get("base64")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("missing data.base64")?;
    let decoded = STANDARD
        .decode(payload)
        .map_err(|_| "invalid base64 payload")?;
    let mime = data
        .get("mime")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.contains('/'))
        .unwrap_or(fallback_mime);
    let url = format!("data:{};base64,{}", mime, STANDARD.encode(&decoded));
    Ok((MediaReference::RemoteUrl { url }, decoded.len() as u64))
}

fn fold_unknown_segment(
    normalization: &mut CreatePostNormalization,
    warnings: &mut Vec<String>,
    text: &mut String,
    message_index: usize,
    segment_index: usize,
    segment_type: &str,
) {
    normalization.unknown_segments = normalization.unknown_segments.saturating_add(1);
    let short = shorten_marker(segment_type);
    text.push_str(&format!("[未知段:{}]", short));
    push_warning(
        warnings,
        format!(
            "messages[{}].message[{}] unknown type {}",
            message_index, segment_index, short
        ),
    );
}

fn fold_invalid_segment(
    normalization: &mut CreatePostNormalization,
    warnings: &mut Vec<String>,
    text: &mut String,
    message_index: usize,
    segment_index: usize,
    segment_type: &str,
    reason: &str,
) {
    normalization.invalid_segments_folded = normalization.invalid_segments_folded.saturating_add(1);
    let short = shorten_marker(segment_type);
    text.push_str(&format!("[{}:invalid]", short));
    push_warning(
        warnings,
        format!(
            "messages[{}].message[{}] {} folded: {}",
            message_index, segment_index, short, reason
        ),
    );
}

fn shorten_marker(value: &str) -> String {
    let mut out = value.trim().to_string();
    if out.is_empty() {
        out = "segment".to_string();
    }
    if out.len() > MAX_SEGMENT_PLACEHOLDER_LEN {
        out.truncate(MAX_SEGMENT_PLACEHOLDER_LEN);
    }
    out
}

fn push_warning(warnings: &mut Vec<String>, warning: String) {
    if warnings.len() < MAX_CREATE_WARNINGS {
        warnings.push(warning);
    }
}

async fn wait_review_code(state: &ApiState, post_id: Id128) -> Option<u32> {
    let deadline = now_ms().saturating_add(CREATE_REVIEW_CODE_WAIT_MS);
    loop {
        let review_code = {
            let guard = state.state.read().ok()?;
            let review_id = guard.posts.get(&post_id).and_then(|meta| meta.review_id);
            review_id.and_then(|id| guard.reviews.get(&id).map(|meta| meta.review_code))
        };
        if review_code.is_some() || now_ms() >= deadline {
            return review_code;
        }
        sleep(Duration::from_millis(CREATE_REVIEW_CODE_POLL_MS)).await;
    }
}

fn authenticate(
    state: &ApiState,
    headers: &HeaderMap,
    required_permission: Option<&str>,
    request_id: &str,
) -> Result<AuthContext, axum::response::Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "missing bearer session",
            request_id.to_string(),
        ));
    };

    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id.to_string(),
            ));
        }
    };

    let now = now_sec();
    guard.sessions.retain(|_, session| session.expires_at > now);

    let Some(session) = guard.sessions.get(&token).cloned() else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid or expired session",
            request_id.to_string(),
        ));
    };

    if let Some(permission) = required_permission {
        if !session.permissions.contains(permission) {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "permission denied",
                request_id.to_string(),
            ));
        }
    }

    Ok(AuthContext {
        session_id: session.session_id,
        token_id: session.token_id,
        allowed_groups: session.allowed_groups,
    })
}

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &str,
    request_id: String,
) -> axum::response::Response {
    let body = ApiError {
        error: ApiErrorBody {
            code,
            message: message.to_string(),
            request_id,
        },
    };
    (status, Json(body)).into_response()
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("X-Request-Id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(random_hex32)
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("Authorization")?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

fn parse_id128(value: &str) -> Option<Id128> {
    value.parse::<u128>().ok().map(Id128)
}

fn parse_review_action(req: &ReviewDecisionRequest) -> Result<ReviewAction, &'static str> {
    match req.action.as_str() {
        "approve" => Ok(ReviewAction::Approve),
        "reject" => Ok(ReviewAction::Reject),
        "delete" => Ok(ReviewAction::Delete),
        "defer" => Ok(ReviewAction::Defer {
            delay_ms: req.delay_ms.unwrap_or(0),
        }),
        "skip" => Ok(ReviewAction::Skip),
        "immediate" => Ok(ReviewAction::Immediate),
        "refresh" => Ok(ReviewAction::Refresh),
        "rerender" => Ok(ReviewAction::Rerender),
        "select_all" => Ok(ReviewAction::SelectAllMessages),
        "toggle_anonymous" => Ok(ReviewAction::ToggleAnonymous),
        "expand_audit" => Ok(ReviewAction::ExpandAudit),
        "show" => Ok(ReviewAction::Show),
        "comment" => {
            let text = req
                .text
                .as_deref()
                .or(req.comment.as_deref())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or("comment requires text")?;
            Ok(ReviewAction::Comment {
                text: text.to_string(),
            })
        }
        "reply" => {
            let text = req
                .text
                .as_deref()
                .or(req.comment.as_deref())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or("reply requires text")?;
            Ok(ReviewAction::Reply {
                text: text.to_string(),
            })
        }
        "blacklist" => Ok(ReviewAction::Blacklist {
            reason: req
                .comment
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }),
        "quick_reply" => {
            let key = req
                .quick_reply_key
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or("quick_reply requires quick_reply_key")?;
            Ok(ReviewAction::QuickReply {
                key: key.to_string(),
            })
        }
        "merge" => {
            let code = req
                .target_review_code
                .ok_or("merge requires target_review_code")?;
            Ok(ReviewAction::Merge { review_code: code })
        }
        _ => Err("unsupported action"),
    }
}

fn parse_stage(value: &str) -> Option<PostStage> {
    match value {
        "drafted" => Some(PostStage::Drafted),
        "render_requested" => Some(PostStage::RenderRequested),
        "rendered" => Some(PostStage::Rendered),
        "review_pending" => Some(PostStage::ReviewPending),
        "reviewed" => Some(PostStage::Reviewed),
        "scheduled" => Some(PostStage::Scheduled),
        "sending" => Some(PostStage::Sending),
        "sent" => Some(PostStage::Sent),
        "rejected" => Some(PostStage::Rejected),
        "deleted" => Some(PostStage::Deleted),
        "skipped" => Some(PostStage::Skipped),
        "manual" => Some(PostStage::Manual),
        "failed" => Some(PostStage::Failed),
        "withdrawn" => Some(PostStage::Withdrawn),
        _ => None,
    }
}

fn stage_to_string(stage: PostStage) -> String {
    match stage {
        PostStage::Drafted => "drafted",
        PostStage::RenderRequested => "render_requested",
        PostStage::Rendered => "rendered",
        PostStage::ReviewPending => "review_pending",
        PostStage::Reviewed => "reviewed",
        PostStage::Scheduled => "scheduled",
        PostStage::Sending => "sending",
        PostStage::Sent => "sent",
        PostStage::Rejected => "rejected",
        PostStage::Deleted => "deleted",
        PostStage::Skipped => "skipped",
        PostStage::Manual => "manual",
        PostStage::Failed => "failed",
        PostStage::Withdrawn => "withdrawn",
    }
    .to_string()
}

fn media_kind_to_string(kind: oqqwall_rust_core::draft::MediaKind) -> String {
    match kind {
        oqqwall_rust_core::draft::MediaKind::Image => "image",
        oqqwall_rust_core::draft::MediaKind::Video => "video",
        oqqwall_rust_core::draft::MediaKind::File => "file",
        oqqwall_rust_core::draft::MediaKind::Audio => "audio",
        oqqwall_rust_core::draft::MediaKind::Other => "other",
        oqqwall_rust_core::draft::MediaKind::Sticker => "sticker",
    }
    .to_string()
}

fn id_to_string(id: Id128) -> String {
    id.0.to_string()
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn now_sec() -> i64 {
    now_ms() / 1000
}

fn random_hex32() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::header::AUTHORIZATION;
    use oqqwall_rust_core::event::ReviewDecision;
    use oqqwall_rust_core::state::{BlobMeta, PostMeta, PostStage, RenderMeta, ReviewMeta};
    use serde_json::json;
    use std::sync::OnceLock;
    use tokio::sync::mpsc;

    fn ensure_test_blob_dir() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            let dir = std::env::temp_dir().join("oqqwall_rust_web_api_test_blobs");
            let _ = std::fs::create_dir_all(&dir);
            unsafe {
                std::env::set_var("OQQWALL_BLOB_DIR", &dir);
            }
        });
    }

    #[test]
    fn parse_stage_roundtrip() {
        let value = parse_stage("review_pending").expect("stage");
        assert_eq!(stage_to_string(value), "review_pending");
    }

    #[test]
    fn random_token_has_32_hex_chars() {
        let token = random_hex32();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn is_digits_only_works() {
        assert!(is_digits_only("123456"));
        assert!(!is_digits_only("abc123"));
        assert!(!is_digits_only(""));
    }

    #[test]
    fn decode_sender_avatar_base64_accepts_valid_data() {
        let out = decode_sender_avatar_base64("aGVsbG8=").expect("decode");
        assert_eq!(out, Some(b"hello".to_vec()));
    }

    #[test]
    fn decode_sender_avatar_base64_rejects_invalid_data() {
        let out = decode_sender_avatar_base64("!!!");
        assert!(out.is_err());
    }

    #[test]
    fn normalize_segments_accepts_base64_image() {
        let mut normalization = CreatePostNormalization::default();
        let mut warnings = Vec::new();
        let message = normalize_segments(
            &[json!({
                "type": "image",
                "data": {
                    "base64": "aGVsbG8=",
                    "mime": "image/png",
                    "name": "a.png"
                }
            })],
            &mut normalization,
            &mut warnings,
            0,
        );
        assert_eq!(message.attachments.len(), 1);
        assert!(warnings.is_empty());
        assert_eq!(normalization.invalid_segments_folded, 0);
        assert_eq!(normalization.unknown_segments, 0);
    }

    #[test]
    fn normalize_segments_treats_image_file_as_image() {
        let mut normalization = CreatePostNormalization::default();
        let mut warnings = Vec::new();
        let message = normalize_segments(
            &[
                json!({
                    "type": "file",
                    "data": {
                        "url": "file:///tmp/photo.webp",
                        "file_size": 32
                    }
                }),
                json!({
                    "type": "file",
                    "data": {
                        "url": "file:///tmp/readme.txt",
                        "file_size": 64
                    }
                }),
            ],
            &mut normalization,
            &mut warnings,
            0,
        );
        assert_eq!(message.attachments.len(), 2);
        assert_eq!(message.attachments[0].kind, MediaKind::Image);
        assert_eq!(message.attachments[0].name.as_deref(), Some("photo.webp"));
        assert_eq!(message.attachments[1].kind, MediaKind::File);
        assert_eq!(message.attachments[1].name.as_deref(), Some("readme.txt"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn normalize_segments_folds_invalid_image() {
        let mut normalization = CreatePostNormalization::default();
        let mut warnings = Vec::new();
        let message = normalize_segments(
            &[json!({
                "type": "image",
                "data": {
                    "base64": "!!!"
                }
            })],
            &mut normalization,
            &mut warnings,
            0,
        );
        assert!(message.attachments.is_empty());
        assert!(message.text.contains("[image:invalid]"));
        assert_eq!(normalization.invalid_segments_folded, 1);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn normalize_allowed_groups_rejects_unknown() {
        let mut known_groups = HashSet::new();
        known_groups.insert("10001".to_string());
        let result = normalize_allowed_groups(Some(vec!["missing".to_string()]), &known_groups);
        assert!(result.is_err());
    }

    fn build_test_state(
        allowed_groups: Option<Vec<&str>>,
    ) -> (ApiState, mpsc::Receiver<Command>, String) {
        ensure_test_blob_dir();
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let mut auth = AuthStore::new("0123456789abcdef0123456789abcdef".to_string());
        let session_id = "sess_test".to_string();
        let permissions = FULL_PERMISSIONS
            .iter()
            .map(|value| (*value).to_string())
            .collect::<BTreeSet<_>>();
        let allowed_groups = allowed_groups.map(|values| {
            values
                .into_iter()
                .map(|value| value.to_string())
                .collect::<BTreeSet<_>>()
        });
        auth.sessions.insert(
            session_id.clone(),
            ApiSession {
                session_id: session_id.clone(),
                token_id: "tok_test".to_string(),
                permissions,
                expires_at: now_sec() + 3600,
                allowed_groups,
            },
        );
        let mut account_group_by_account = HashMap::new();
        account_group_by_account.insert("acc10001".to_string(), "10001".to_string());
        account_group_by_account.insert("acc20002".to_string(), "20002".to_string());
        account_group_by_account.insert("acc_shared".to_string(), "10001".to_string());
        let mut account_groups_by_account = HashMap::new();
        account_groups_by_account.insert("acc10001".to_string(), vec!["10001".to_string()]);
        account_groups_by_account.insert("acc20002".to_string(), vec!["20002".to_string()]);
        account_groups_by_account.insert(
            "acc_shared".to_string(),
            vec!["10001".to_string(), "20002".to_string()],
        );
        let known_groups = ["10001", "20002"]
            .iter()
            .map(|value| value.to_string())
            .collect::<HashSet<_>>();
        (
            ApiState {
                cmd_tx,
                state: Arc::new(RwLock::new(StateView::default())),
                auth: Arc::new(RwLock::new(auth)),
                tz_offset_minutes: 0,
                account_group_by_account,
                account_groups_by_account,
                known_groups,
            },
            cmd_rx,
            session_id,
        )
    }

    fn build_headers(session_id: &str, idempotency_key: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", session_id)).expect("header"),
        );
        if let Some(key) = idempotency_key {
            headers.insert(
                "Idempotency-Key",
                HeaderValue::from_str(key).expect("idem header"),
            );
        }
        headers
    }

    fn expected_post_id(
        session_id: &str,
        group_id: &str,
        target_account: &str,
        sender_id: &str,
        message_id: &str,
        idempotency_key: &str,
    ) -> Id128 {
        let profile_id = format!("api_post_create:{}", target_account);
        let dedup_key = format!(
            "create:{}:{}:{}:{}",
            session_id, group_id, target_account, idempotency_key
        );
        let chat_id = format!("api:create:{}:{}:{}", group_id, target_account, dedup_key);
        let ingress_id = derive_ingress_id(&[
            profile_id.as_bytes(),
            chat_id.as_bytes(),
            sender_id.as_bytes(),
            message_id.as_bytes(),
        ]);
        let ingress_bytes = ingress_id.to_be_bytes();
        let session_id = derive_session_id(&[
            chat_id.as_bytes(),
            sender_id.as_bytes(),
            group_id.as_bytes(),
            &ingress_bytes,
        ]);
        derive_post_id(&[&session_id.to_be_bytes()])
    }

    fn expected_rendered_post_id(
        session_id: &str,
        group_id: &str,
        target_account: &str,
        sender_id: &str,
        idempotency_key: &str,
    ) -> Id128 {
        let profile_id = format!("api_post_rendered:{}", target_account);
        let dedup_key = format!(
            "create_rendered:{}:{}:{}:{}",
            session_id, group_id, target_account, idempotency_key
        );
        let chat_id = format!(
            "api:create_rendered:{}:{}:{}",
            group_id, target_account, dedup_key
        );
        let platform_msg_id = "rendered_image";
        let ingress_id = derive_ingress_id(&[
            profile_id.as_bytes(),
            chat_id.as_bytes(),
            sender_id.as_bytes(),
            platform_msg_id.as_bytes(),
        ]);
        let ingress_bytes = ingress_id.to_be_bytes();
        let session_id = derive_session_id(&[
            chat_id.as_bytes(),
            sender_id.as_bytes(),
            group_id.as_bytes(),
            &ingress_bytes,
        ]);
        derive_post_id(&[&session_id.to_be_bytes()])
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("bytes");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[test]
    fn create_post_request_deserializes_string_and_singular_tags() {
        let req: CreatePostRequest = serde_json::from_value(json!({
            "target_account": "acc10001",
            "sender_id": "user_a",
            "source_webhook": "hook://newsroom",
            "tag": ["manual", "3344846508"],
            "tags": "manual, 3344846508\n编辑部",
            "messages": [{
                "message_id": "m1",
                "time": 1767094033,
                "message": []
            }]
        }))
        .expect("request");

        assert_eq!(
            merge_request_tags(&req.tags, req.tag.as_ref()),
            vec![
                "manual".to_string(),
                "3344846508".to_string(),
                "编辑部".to_string()
            ]
        );
    }

    #[test]
    fn create_rendered_post_request_deserializes_numeric_tag() {
        let req: CreateRenderedPostRequest = serde_json::from_value(json!({
            "target_account": "acc10001",
            "image_base64": "aGVsbG8=",
            "image_mime": "image/png",
            "tag": 3344846508u64,
            "tags": ["manual", "3344846508"]
        }))
        .expect("request");

        assert_eq!(
            merge_request_tags(&req.tags, req.tag.as_ref()),
            vec!["manual".to_string(), "3344846508".to_string()]
        );
    }

    #[tokio::test]
    async fn create_post_rejects_unknown_target_account() {
        let (state, _rx, session_id) = build_test_state(None);
        let headers = build_headers(&session_id, None);
        let req = CreatePostRequest {
            target_account: "missing".to_string(),
            sender_id: "user_a".to_string(),
            sender_name: Some("Alice".to_string()),
            sender_avatar_base64: None,
            messages: vec![CreatePostMessage {
                message_id: json!("m1"),
                time: json!(1767094033),
                message: vec![json!({"type":"text","data":{"text":"hello"}})],
            }],
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
        assert_eq!(body["error"]["message"], "unknown target_account");
    }

    #[tokio::test]
    async fn create_post_rejects_group_not_allowed() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["20002"]));
        let headers = build_headers(&session_id, None);
        let req = CreatePostRequest {
            target_account: "acc10001".to_string(),
            sender_id: "user_a".to_string(),
            sender_name: Some("Alice".to_string()),
            sender_avatar_base64: None,
            messages: vec![CreatePostMessage {
                message_id: json!("m1"),
                time: json!(1767094033),
                message: vec![json!({"type":"text","data":{"text":"hello"}})],
            }],
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn create_post_success_returns_review_code_and_sends_ingress() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_ok";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post = expected_post_id(
            &session_id,
            "10001",
            "acc10001",
            "abc_sender",
            "m1",
            idem_key,
        );
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998001);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9001),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 42,
                    decision: Some(ReviewDecision::Deferred),
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreatePostRequest {
            target_account: "acc10001".to_string(),
            sender_id: "abc_sender".to_string(),
            sender_name: Some("Alice".to_string()),
            sender_avatar_base64: Some("aGVsbG8=".to_string()),
            messages: vec![CreatePostMessage {
                message_id: json!("m1"),
                time: json!(1767094033),
                message: vec![json!({"type":"text","data":{"text":"hello"}})],
            }],
            source_webhook: Some("hook://newsroom".to_string()),
            tag: None,
            tags: vec!["3344846508".to_string(), "manual".to_string()],
        };
        let response = create_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["post_id"], json!(expected_post.0.to_string()));
        assert_eq!(body["review_code"], 42);
        assert_eq!(body["accepted_messages"], 1);
        assert!(avatar_cache::has_avatar("abc_sender"));
        let sent = rx.try_recv().expect("ingress cmd");
        match sent {
            Command::Ingress(cmd) => {
                assert_eq!(cmd.group_id, "10001");
                assert_eq!(cmd.sender_name, Some("Alice".to_string()));
                let route_meta = cmd.route_meta.expect("route_meta");
                assert_eq!(
                    route_meta.source_webhook.as_deref(),
                    Some("hook://newsroom")
                );
                assert_eq!(
                    route_meta.tags,
                    vec!["3344846508".to_string(), "manual".to_string()]
                );
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[tokio::test]
    async fn create_post_non_numeric_sender_falls_back_to_unknown_name() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_unknown";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post =
            expected_post_id(&session_id, "10001", "acc10001", "sender_x", "m2", idem_key);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998002);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9002),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 43,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreatePostRequest {
            target_account: "acc10001".to_string(),
            sender_id: "sender_x".to_string(),
            sender_name: None,
            sender_avatar_base64: None,
            messages: vec![CreatePostMessage {
                message_id: json!("m2"),
                time: json!(1767094034),
                message: vec![json!({"type":"text","data":{"text":"hello"}})],
            }],
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["review_code"], 43);
        let warnings = body["warnings"].as_array().expect("warnings");
        assert!(warnings.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("avatar fallback skipped"))
        }));
        let sent = rx.try_recv().expect("ingress cmd");
        match sent {
            Command::Ingress(cmd) => {
                assert_eq!(cmd.sender_name, Some("未知".to_string()));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[tokio::test]
    async fn create_post_numeric_sender_stranger_lookup_failed_uses_unknown_name() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_numeric";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post =
            expected_post_id(&session_id, "10001", "acc10001", "123456", "m3", idem_key);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998003);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9003),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 44,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreatePostRequest {
            target_account: "acc10001".to_string(),
            sender_id: "123456".to_string(),
            sender_name: None,
            sender_avatar_base64: None,
            messages: vec![CreatePostMessage {
                message_id: json!("m3"),
                time: json!(1767094035),
                message: vec![json!({"type":"text","data":{"text":"hello"}})],
            }],
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["review_code"], 44);
        let warnings = body["warnings"].as_array().expect("warnings");
        assert!(warnings.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("sender_name fallback failed"))
        }));
        assert!(warnings.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("avatar fallback skipped"))
        }));
        let sent = rx.try_recv().expect("ingress cmd");
        match sent {
            Command::Ingress(cmd) => {
                assert_eq!(cmd.sender_name, Some("未知".to_string()));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[tokio::test]
    async fn create_post_numeric_sender_with_given_name_skips_avatar_fetch() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_given_name";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post =
            expected_post_id(&session_id, "10001", "acc10001", "123457", "m4", idem_key);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998004);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9004),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 45,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreatePostRequest {
            target_account: "acc10001".to_string(),
            sender_id: "123457".to_string(),
            sender_name: Some("Alice".to_string()),
            sender_avatar_base64: None,
            messages: vec![CreatePostMessage {
                message_id: json!("m4"),
                time: json!(1767094036),
                message: vec![json!({"type":"text","data":{"text":"hello"}})],
            }],
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["review_code"], 45);
        let warnings = body["warnings"].as_array().expect("warnings");
        assert!(warnings.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("avatar fallback skipped"))
        }));
        let sent = rx.try_recv().expect("ingress cmd");
        match sent {
            Command::Ingress(cmd) => {
                assert_eq!(cmd.sender_name, Some("Alice".to_string()));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[tokio::test]
    async fn create_rendered_post_rejects_invalid_mime() {
        let (state, mut rx, session_id) = build_test_state(None);
        let headers = build_headers(&session_id, None);
        let req = CreateRenderedPostRequest {
            target_account: "acc10001".to_string(),
            image_base64: "aGVsbG8=".to_string(),
            image_mime: "image/gif".to_string(),
            sender_id: None,
            sender_name: None,
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_rendered_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|text| text.contains("image_mime"))
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn create_rendered_post_success_anonymous_emits_driver_events() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_rendered_anon";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post =
            expected_rendered_post_id(&session_id, "10001", "acc10001", "unknown", idem_key);
        let expected_blob = derive_blob_id(&[&expected_post.to_be_bytes(), b"rendered"]);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998101);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9101),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: true,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 501,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreateRenderedPostRequest {
            target_account: "acc10001".to_string(),
            image_base64: "aGVsbG8=".to_string(),
            image_mime: "image/png".to_string(),
            sender_id: None,
            sender_name: None,
            source_webhook: Some("hook://render".to_string()),
            tag: None,
            tags: vec!["rendered".to_string(), "3344846508".to_string()],
        };
        let response = create_rendered_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["post_id"], json!(expected_post.0.to_string()));
        assert_eq!(body["review_code"], 501);
        assert_eq!(body["accepted_messages"], 1);

        let mut saw_blob_registered = false;
        let mut saw_blob_persisted = false;
        let mut saw_ingress = false;
        let mut saw_draft = false;
        let mut saw_render = false;
        for _ in 0..5 {
            let cmd = rx.try_recv().expect("driver event");
            match cmd {
                Command::DriverEvent(event) => match event {
                    oqqwall_rust_core::Event::Blob(BlobEvent::BlobRegistered {
                        blob_id,
                        size_bytes,
                    }) => {
                        assert_eq!(blob_id, expected_blob);
                        assert_eq!(size_bytes, 5);
                        saw_blob_registered = true;
                    }
                    oqqwall_rust_core::Event::Blob(BlobEvent::BlobPersisted { blob_id, path }) => {
                        assert_eq!(blob_id, expected_blob);
                        assert!(path.ends_with(".png"));
                        saw_blob_persisted = true;
                    }
                    oqqwall_rust_core::Event::Ingress(IngressEvent::MessageAccepted {
                        user_id,
                        sender_name,
                        group_id,
                        route_meta,
                        ..
                    }) => {
                        assert_eq!(user_id, "unknown");
                        assert_eq!(sender_name, None);
                        assert_eq!(group_id, "10001");
                        let route_meta = route_meta.expect("route_meta");
                        assert_eq!(route_meta.source_webhook.as_deref(), Some("hook://render"));
                        assert_eq!(
                            route_meta.tags,
                            vec!["rendered".to_string(), "3344846508".to_string()]
                        );
                        saw_ingress = true;
                    }
                    oqqwall_rust_core::Event::Draft(DraftEvent::PostDraftCreated {
                        post_id,
                        is_anonymous,
                        ..
                    }) => {
                        assert_eq!(post_id, expected_post);
                        assert!(is_anonymous);
                        saw_draft = true;
                    }
                    oqqwall_rust_core::Event::Render(RenderEvent::PngReady {
                        post_id,
                        blob_id,
                    }) => {
                        assert_eq!(post_id, expected_post);
                        assert_eq!(blob_id, expected_blob);
                        saw_render = true;
                    }
                    other => panic!("unexpected event: {:?}", other),
                },
                other => panic!("unexpected command: {:?}", other),
            }
        }
        assert!(saw_blob_registered);
        assert!(saw_blob_persisted);
        assert!(saw_ingress);
        assert!(saw_draft);
        assert!(saw_render);
    }

    #[tokio::test]
    async fn create_rendered_post_numeric_sender_keeps_name() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_rendered_numeric";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post =
            expected_rendered_post_id(&session_id, "10001", "acc10001", "123456", idem_key);
        let expected_blob = derive_blob_id(&[&expected_post.to_be_bytes(), b"rendered"]);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998102);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9102),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 502,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreateRenderedPostRequest {
            target_account: "acc10001".to_string(),
            image_base64: "aGVsbG8=".to_string(),
            image_mime: "image/jpeg".to_string(),
            sender_id: Some("123456".to_string()),
            sender_name: Some("Alice".to_string()),
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_rendered_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["post_id"], json!(expected_post.0.to_string()));
        assert_eq!(body["review_code"], 502);

        let mut saw_ingress = false;
        let mut saw_draft = false;
        let mut saw_jpg_path = false;
        for _ in 0..5 {
            let cmd = rx.try_recv().expect("driver event");
            if let Command::DriverEvent(event) = cmd {
                match event {
                    oqqwall_rust_core::Event::Blob(BlobEvent::BlobPersisted { blob_id, path }) => {
                        assert_eq!(blob_id, expected_blob);
                        assert!(path.ends_with(".jpg"));
                        saw_jpg_path = true;
                    }
                    oqqwall_rust_core::Event::Ingress(IngressEvent::MessageAccepted {
                        user_id,
                        sender_name,
                        ..
                    }) => {
                        assert_eq!(user_id, "123456");
                        assert_eq!(sender_name, Some("Alice".to_string()));
                        saw_ingress = true;
                    }
                    oqqwall_rust_core::Event::Draft(DraftEvent::PostDraftCreated {
                        is_anonymous,
                        ..
                    }) => {
                        assert!(!is_anonymous);
                        saw_draft = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_jpg_path);
        assert!(saw_ingress);
        assert!(saw_draft);
    }

    #[tokio::test]
    async fn create_rendered_post_non_numeric_sender_disables_mention() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_rendered_nonnumeric";
        let headers = build_headers(&session_id, Some(idem_key));
        let expected_post =
            expected_rendered_post_id(&session_id, "10001", "acc10001", "abc_sender", idem_key);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998103);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9103),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 503,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let req = CreateRenderedPostRequest {
            target_account: "acc10001".to_string(),
            image_base64: "aGVsbG8=".to_string(),
            image_mime: "image/webp".to_string(),
            sender_id: Some("abc_sender".to_string()),
            sender_name: Some("Alice".to_string()),
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let response = create_rendered_post(State(state), headers, Json(req))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["review_code"], 503);
        let warnings = body["warnings"].as_array().expect("warnings");
        assert!(warnings.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("mention disabled"))
        }));

        for _ in 0..5 {
            let cmd = rx.try_recv().expect("driver event");
            if let Command::DriverEvent(oqqwall_rust_core::Event::Ingress(
                IngressEvent::MessageAccepted { sender_name, .. },
            )) = cmd
            {
                assert_eq!(sender_name, None);
            }
        }
    }

    #[tokio::test]
    async fn create_rendered_post_idempotency_reuses_cached_response() {
        let (state, mut rx, session_id) = build_test_state(None);
        let idem_key = "idem_rendered_reuse";
        let expected_post =
            expected_rendered_post_id(&session_id, "10001", "acc10001", "unknown", idem_key);
        let shared_state = state.state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let review_id = Id128(998104);
            let mut guard = shared_state.write().expect("lock");
            guard.posts.insert(
                expected_post,
                PostMeta {
                    post_id: expected_post,
                    session_id: Id128(9104),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: now_ms(),
                    is_anonymous: true,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id: expected_post,
                    review_code: 504,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        });
        let headers = build_headers(&session_id, Some(idem_key));
        let req = CreateRenderedPostRequest {
            target_account: "acc10001".to_string(),
            image_base64: "aGVsbG8=".to_string(),
            image_mime: "image/png".to_string(),
            sender_id: None,
            sender_name: None,
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let first = create_rendered_post(State(state.clone()), headers.clone(), Json(req))
            .await
            .into_response();
        assert_eq!(first.status(), StatusCode::OK);
        let body_first = response_json(first).await;
        assert_eq!(body_first["post_id"], json!(expected_post.0.to_string()));
        assert_eq!(body_first["review_code"], 504);
        for _ in 0..5 {
            let _ = rx.try_recv().expect("driver event");
        }

        let second_req = CreateRenderedPostRequest {
            target_account: "acc10001".to_string(),
            image_base64: "aGVsbG8=".to_string(),
            image_mime: "image/png".to_string(),
            sender_id: None,
            sender_name: None,
            source_webhook: None,
            tag: None,
            tags: Vec::new(),
        };
        let second = create_rendered_post(State(state), headers, Json(second_req))
            .await
            .into_response();
        assert_eq!(second.status(), StatusCode::OK);
        let body_second = response_json(second).await;
        assert_eq!(body_second["post_id"], body_first["post_id"]);
        assert_eq!(body_second["review_code"], body_first["review_code"]);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn list_posts_filters_by_allowed_groups_and_adds_warning() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                Id128(70001),
                PostMeta {
                    post_id: Id128(70001),
                    session_id: Id128(1),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 10,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                Id128(70002),
                PostMeta {
                    post_id: Id128(70002),
                    session_id: Id128(2),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 11,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let headers = build_headers(&session_id, None);
        let response = list_posts(
            State(state),
            headers,
            Query(ListPostsQuery {
                stage: None,
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["items"][0]["group_id"], "10001");
        assert!(
            body["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|item| item
                    .as_str()
                    .is_some_and(|text| text.contains("filtered by allowed_groups")))
        );
    }

    #[tokio::test]
    async fn get_post_rejects_group_not_allowed() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["20002"]));
        let post_id = Id128(71001);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_id,
                PostMeta {
                    post_id,
                    session_id: Id128(3),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 12,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let headers = build_headers(&session_id, None);
        let response = get_post(State(state), Path(post_id.0.to_string()), headers)
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn decide_review_rejects_group_not_allowed() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let post_id = Id128(72001);
        let review_id = Id128(72011);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_id,
                PostMeta {
                    post_id,
                    session_id: Id128(4),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_id),
                    created_at_ms: 13,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_id,
                ReviewMeta {
                    review_id,
                    post_id,
                    review_code: 81,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        }
        let headers = build_headers(&session_id, None);
        let response = decide_review(
            State(state),
            Path(review_id.0.to_string()),
            headers,
            Json(ReviewDecisionRequest {
                action: "approve".to_string(),
                comment: None,
                delay_ms: None,
                text: None,
                quick_reply_key: None,
                target_review_code: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn decide_review_batch_partially_applies_group_allowed_items() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let post_allow = Id128(73001);
        let review_allow = Id128(73011);
        let post_deny = Id128(73002);
        let review_deny = Id128(73012);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_allow,
                PostMeta {
                    post_id: post_allow,
                    session_id: Id128(5),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_allow),
                    created_at_ms: 14,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_allow,
                ReviewMeta {
                    review_id: review_allow,
                    post_id: post_allow,
                    review_code: 82,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
            guard.posts.insert(
                post_deny,
                PostMeta {
                    post_id: post_deny,
                    session_id: Id128(6),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_deny),
                    created_at_ms: 15,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.reviews.insert(
                review_deny,
                ReviewMeta {
                    review_id: review_deny,
                    post_id: post_deny,
                    review_code: 83,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
        }
        let headers = build_headers(&session_id, None);
        let response = decide_review_batch(
            State(state),
            headers,
            Json(BatchReviewDecisionRequest {
                review_ids: vec![review_allow.0.to_string(), review_deny.0.to_string()],
                action: "approve".to_string(),
                comment: None,
                delay_ms: None,
                text: None,
                quick_reply_key: None,
                target_review_code: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["accepted"], 1);
        assert_eq!(body["failed"].as_array().map(Vec::len), Some(1));
        assert!(
            body["failed"][0]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("permission denied"))
        );
        let sent = rx.try_recv().expect("review cmd");
        match sent {
            Command::ReviewAction(cmd) => {
                assert_eq!(cmd.review_id, Some(review_allow));
            }
            other => panic!("unexpected command: {:?}", other),
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn list_blacklist_filters_by_allowed_groups_and_adds_warning() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        {
            let mut guard = state.state.write().expect("lock");
            guard.blacklist.insert(
                "10001".to_string(),
                HashMap::from([("u1".to_string(), Some("r1".to_string()))]),
            );
            guard.blacklist.insert(
                "20002".to_string(),
                HashMap::from([("u2".to_string(), Some("r2".to_string()))]),
            );
        }
        let headers = build_headers(&session_id, None);
        let response = list_blacklist(
            State(state),
            headers,
            Query(ListBlacklistQuery {
                group_id: None,
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["items"][0]["group_id"], "10001");
        assert!(
            body["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|item| item
                    .as_str()
                    .is_some_and(|text| text.contains("filtered by allowed_groups")))
        );
    }

    #[tokio::test]
    async fn list_blacklist_query_unauthorized_group_returns_empty_with_warning() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        {
            let mut guard = state.state.write().expect("lock");
            guard.blacklist.insert(
                "20002".to_string(),
                HashMap::from([("u2".to_string(), Some("r2".to_string()))]),
            );
        }
        let headers = build_headers(&session_id, None);
        let response = list_blacklist(
            State(state),
            headers,
            Query(ListBlacklistQuery {
                group_id: Some("20002".to_string()),
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(0));
        assert!(
            body["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|item| item
                    .as_str()
                    .is_some_and(|text| text.contains("filtered by allowed_groups")))
        );
    }

    #[tokio::test]
    async fn create_blacklist_rejects_group_not_allowed() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let headers = build_headers(&session_id, None);
        let response = create_blacklist(
            State(state),
            headers,
            Json(CreateBlacklistRequest {
                group_id: "20002".to_string(),
                sender_id: "user-x".to_string(),
                reason: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn delete_blacklist_rejects_group_not_allowed() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let headers = build_headers(&session_id, None);
        let response = delete_blacklist(
            State(state),
            Path(("20002".to_string(), "user-y".to_string())),
            headers,
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn send_posts_partially_applies_group_allowed_items() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let post_allow = Id128(74001);
        let review_allow = Id128(74011);
        let post_deny = Id128(74002);
        let review_deny = Id128(74012);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_allow,
                PostMeta {
                    post_id: post_allow,
                    session_id: Id128(7),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_allow),
                    created_at_ms: 16,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                post_deny,
                PostMeta {
                    post_id: post_deny,
                    session_id: Id128(8),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_deny),
                    created_at_ms: 17,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let headers = build_headers(&session_id, None);
        let response = send_posts(
            State(state),
            headers,
            Json(SendPostsRequest {
                post_ids: vec![post_allow.0.to_string(), post_deny.0.to_string()],
                mode: "immediate".to_string(),
                schedule_at: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["accepted"], 1);
        assert_eq!(body["failed"].as_array().map(Vec::len), Some(1));
        assert!(
            body["failed"][0]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("permission denied"))
        );
        let sent = rx.try_recv().expect("review cmd");
        match sent {
            Command::ReviewAction(cmd) => {
                assert_eq!(cmd.review_id, Some(review_allow));
            }
            other => panic!("unexpected command: {:?}", other),
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn get_blob_rejects_group_not_allowed() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        let post_id = Id128(75001);
        let blob_id = Id128(75091);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_id,
                PostMeta {
                    post_id,
                    session_id: Id128(9),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 18,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.render.insert(
                post_id,
                RenderMeta {
                    png_blob: Some(blob_id),
                    png_blobs: Vec::new(),
                    last_error: None,
                    last_attempt: 0,
                    retry_at_ms: None,
                },
            );
            guard.blobs.insert(
                blob_id,
                BlobMeta {
                    blob_id,
                    size_bytes: 4,
                    persisted_path: Some("/tmp/not_exists_for_test.png".to_string()),
                    ref_count: 1,
                },
            );
        }
        let headers = build_headers(&session_id, None);
        let response = get_blob(State(state), Path(blob_id.0.to_string()), headers)
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn list_posts_without_group_scope_returns_all_without_warning() {
        let (state, _rx, session_id) = build_test_state(None);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                Id128(76001),
                PostMeta {
                    post_id: Id128(76001),
                    session_id: Id128(11),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 19,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                Id128(76002),
                PostMeta {
                    post_id: Id128(76002),
                    session_id: Id128(12),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 20,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let response = list_posts(
            State(state),
            build_headers(&session_id, None),
            Query(ListPostsQuery {
                stage: None,
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(2));
        assert!(body.get("warnings").is_none());
    }

    #[tokio::test]
    async fn list_posts_stage_filter_respects_group_scope() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                Id128(76101),
                PostMeta {
                    post_id: Id128(76101),
                    session_id: Id128(13),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 21,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                Id128(76102),
                PostMeta {
                    post_id: Id128(76102),
                    session_id: Id128(14),
                    group_id: "10001".to_string(),
                    stage: PostStage::Sent,
                    review_id: None,
                    created_at_ms: 22,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                Id128(76103),
                PostMeta {
                    post_id: Id128(76103),
                    session_id: Id128(15),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 23,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let response = list_posts(
            State(state),
            build_headers(&session_id, None),
            Query(ListPostsQuery {
                stage: Some("review_pending".to_string()),
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["items"][0]["post_id"], "76101");
        assert!(
            body["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|item| item
                    .as_str()
                    .is_some_and(|text| text.contains("filtered by allowed_groups")))
        );
    }

    #[tokio::test]
    async fn get_blob_scoped_token_without_blob_group_mapping_rejected() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        let blob_id = Id128(76291);
        {
            let mut guard = state.state.write().expect("lock");
            guard.blobs.insert(
                blob_id,
                BlobMeta {
                    blob_id,
                    size_bytes: 3,
                    persisted_path: Some("/tmp/blob_unmapped.bin".to_string()),
                    ref_count: 1,
                },
            );
        }
        let response = get_blob(
            State(state),
            Path(blob_id.0.to_string()),
            build_headers(&session_id, None),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|msg| msg.contains("blob group"))
        );
    }

    #[tokio::test]
    async fn get_blob_allowed_group_then_missing_file_returns_not_found() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        let post_id = Id128(76301);
        let blob_id = Id128(76391);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_id,
                PostMeta {
                    post_id,
                    session_id: Id128(16),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 24,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.render.insert(
                post_id,
                RenderMeta {
                    png_blob: Some(blob_id),
                    png_blobs: Vec::new(),
                    last_error: None,
                    last_attempt: 0,
                    retry_at_ms: None,
                },
            );
            guard.blobs.insert(
                blob_id,
                BlobMeta {
                    blob_id,
                    size_bytes: 4,
                    persisted_path: Some("/tmp/not_exists_blob_test_76391.png".to_string()),
                    ref_count: 1,
                },
            );
        }
        let response = get_blob(
            State(state),
            Path(blob_id.0.to_string()),
            build_headers(&session_id, None),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "blob file missing");
    }

    #[tokio::test]
    async fn decide_review_batch_reports_review_not_found() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let response = decide_review_batch(
            State(state),
            build_headers(&session_id, None),
            Json(BatchReviewDecisionRequest {
                review_ids: vec!["77001".to_string()],
                action: "approve".to_string(),
                comment: None,
                delay_ms: None,
                text: None,
                quick_reply_key: None,
                target_review_code: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["accepted"], 0);
        assert_eq!(body["failed"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["failed"][0]["reason"], "review not found");
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn send_posts_reports_permission_denied_and_missing_review() {
        let (state, mut rx, session_id) = build_test_state(Some(vec!["10001"]));
        let post_denied = Id128(77101);
        let post_no_review = Id128(77102);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_denied,
                PostMeta {
                    post_id: post_denied,
                    session_id: Id128(17),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(Id128(77111)),
                    created_at_ms: 25,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                post_no_review,
                PostMeta {
                    post_id: post_no_review,
                    session_id: Id128(18),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 26,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let response = send_posts(
            State(state),
            build_headers(&session_id, None),
            Json(SendPostsRequest {
                post_ids: vec![post_denied.0.to_string(), post_no_review.0.to_string()],
                mode: "immediate".to_string(),
                schedule_at: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["accepted"], 0);
        assert_eq!(body["failed"].as_array().map(Vec::len), Some(2));
        let reasons = body["failed"]
            .as_array()
            .expect("failed")
            .iter()
            .filter_map(|item| item["reason"].as_str().map(|text| text.to_string()))
            .collect::<Vec<_>>();
        assert!(reasons.iter().any(|r| r.contains("permission denied")));
        assert!(reasons.iter().any(|r| r.contains("post has no review_id")));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn create_blacklist_rejects_empty_group_id() {
        let (state, mut rx, session_id) = build_test_state(None);
        let response = create_blacklist(
            State(state),
            build_headers(&session_id, None),
            Json(CreateBlacklistRequest {
                group_id: "   ".to_string(),
                sender_id: "u".to_string(),
                reason: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "group_id cannot be empty");
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn list_blacklist_query_allowed_group_has_no_warning() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        {
            let mut guard = state.state.write().expect("lock");
            guard.blacklist.insert(
                "10001".to_string(),
                HashMap::from([("u3".to_string(), Some("r3".to_string()))]),
            );
        }
        let response = list_blacklist(
            State(state),
            build_headers(&session_id, None),
            Query(ListBlacklistQuery {
                group_id: Some("10001".to_string()),
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        assert!(body.get("warnings").is_none());
    }

    #[tokio::test]
    async fn send_posts_unscoped_token_allows_cross_groups() {
        let (state, mut rx, session_id) = build_test_state(None);
        let post_a = Id128(77201);
        let review_a = Id128(77211);
        let post_b = Id128(77202);
        let review_b = Id128(77212);
        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                post_a,
                PostMeta {
                    post_id: post_a,
                    session_id: Id128(19),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_a),
                    created_at_ms: 27,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                post_b,
                PostMeta {
                    post_id: post_b,
                    session_id: Id128(20),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: Some(review_b),
                    created_at_ms: 28,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let response = send_posts(
            State(state),
            build_headers(&session_id, None),
            Json(SendPostsRequest {
                post_ids: vec![post_a.0.to_string(), post_b.0.to_string()],
                mode: "immediate".to_string(),
                schedule_at: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["accepted"], 2);
        assert_eq!(body["failed"].as_array().map(Vec::len), Some(0));
        let cmd1 = rx.try_recv().expect("first cmd");
        let cmd2 = rx.try_recv().expect("second cmd");
        assert!(matches!(cmd1, Command::ReviewAction(_)));
        assert!(matches!(cmd2, Command::ReviewAction(_)));
    }

    #[tokio::test]
    async fn send_private_message_rejects_empty_message() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: None,
                user_id: json!("123456"),
                message: Vec::new(),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "message cannot be empty");
    }

    #[tokio::test]
    async fn send_private_message_rejects_group_without_target_account() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: Some("20002".to_string()),
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|msg| msg.contains("not configured in group_id"))
        );
    }

    #[tokio::test]
    async fn send_private_message_rejects_when_target_account_not_in_allowed_groups() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc20002".to_string(),
                group_id: None,
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn send_private_message_requires_online_target_account() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: Some("10001".to_string()),
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response_json(response).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|msg| msg.contains("not online"))
        );
    }

    #[tokio::test]
    async fn send_private_message_rejects_empty_target_account() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "   ".to_string(),
                group_id: None,
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "target_account cannot be empty");
    }

    #[tokio::test]
    async fn send_private_message_rejects_unknown_target_account() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc_missing".to_string(),
                group_id: None,
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "unknown target_account");
    }

    #[tokio::test]
    async fn send_private_message_rejects_non_object_segment() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: None,
                user_id: json!("123456"),
                message: vec![json!("plain-text-segment")],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "message segments must be objects");
    }

    #[tokio::test]
    async fn send_private_message_rejects_missing_user_id() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: None,
                user_id: Value::Null,
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "user_id is required");
    }

    #[tokio::test]
    async fn send_private_message_rejects_blank_group_id() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: Some("   ".to_string()),
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "group_id cannot be empty");
    }

    #[tokio::test]
    async fn send_private_message_rejects_unknown_group_id() {
        let (state, _rx, session_id) = build_test_state(None);
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc10001".to_string(),
                group_id: Some("99999".to_string()),
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "unknown group_id");
    }

    #[tokio::test]
    async fn send_private_message_rejects_explicit_group_without_permission() {
        let (state, _rx, session_id) = build_test_state(Some(vec!["10001"]));
        let response = send_private_message(
            State(state),
            build_headers(&session_id, None),
            Json(SendPrivateMessageRequest {
                target_account: "acc20002".to_string(),
                group_id: Some("20002".to_string()),
                user_id: json!("123456"),
                message: vec![json!({"type":"text","data":{"text":"ping"}})],
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn create_token_and_login_scoped_session_filters_posts() {
        let (state, _rx, session_id) = build_test_state(None);
        let create_resp = create_token(
            State(state.clone()),
            build_headers(&session_id, None),
            Json(CreateTokenRequest {
                permissions: vec!["review.read".to_string()],
                expire_at: None,
                allowed_groups: Some(vec!["10001".to_string()]),
            }),
        )
        .await
        .into_response();
        assert_eq!(create_resp.status(), StatusCode::OK);
        let create_body = response_json(create_resp).await;
        let token = create_body["token"].as_str().expect("token").to_string();

        let login_resp = login(
            State(state.clone()),
            HeaderMap::new(),
            Json(LoginRequest { token }),
        )
        .await
        .into_response();
        assert_eq!(login_resp.status(), StatusCode::OK);
        let login_body = response_json(login_resp).await;
        let scoped_session = login_body["session_id"]
            .as_str()
            .expect("session")
            .to_string();

        {
            let mut guard = state.state.write().expect("lock");
            guard.posts.insert(
                Id128(77301),
                PostMeta {
                    post_id: Id128(77301),
                    session_id: Id128(21),
                    group_id: "10001".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 29,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
            guard.posts.insert(
                Id128(77302),
                PostMeta {
                    post_id: Id128(77302),
                    session_id: Id128(22),
                    group_id: "20002".to_string(),
                    stage: PostStage::ReviewPending,
                    review_id: None,
                    created_at_ms: 30,
                    is_anonymous: false,
                    is_safe: true,
                    last_error: None,
                },
            );
        }
        let list_resp = list_posts(
            State(state),
            build_headers(&scoped_session, None),
            Query(ListPostsQuery {
                stage: None,
                cursor: None,
                limit: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(list_resp.status(), StatusCode::OK);
        let list_body = response_json(list_resp).await;
        assert_eq!(list_body["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(list_body["items"][0]["group_id"], "10001");
    }
}
