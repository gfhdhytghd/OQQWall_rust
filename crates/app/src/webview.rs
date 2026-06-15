use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use oqqwall_rust_core::draft::MediaReference;
use oqqwall_rust_core::state::{PostMeta, PostStage};
use oqqwall_rust_core::{Command, Id128, ReviewAction, ReviewActionCommand, StateView};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{AppConfig, WebviewAdminAccount, WebviewRole};
use crate::engine::EngineHandle;

include!(concat!(env!("OUT_DIR"), "/webview_assets.rs"));

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

const SESSION_COOKIE_NAME: &str = "oqqwall_webview_session";

#[derive(Clone)]
struct WebviewState {
    cmd_tx: tokio::sync::mpsc::Sender<Command>,
    state: Arc<RwLock<StateView>>,
    auth: Arc<RwLock<WebviewAuthStore>>,
    admin: Arc<RwLock<WebviewAdminStore>>,
    tz_offset_minutes: i32,
    session_ttl_sec: i64,
}

#[derive(Clone)]
struct WebviewIdentity {
    username: String,
    role: WebviewRole,
    groups: Vec<String>,
}

#[derive(Clone)]
struct WebviewSession {
    identity: WebviewIdentity,
    expires_at: i64,
}

#[derive(Default)]
struct WebviewAuthStore {
    users: HashMap<String, Vec<WebviewAdminAccount>>,
    sessions: HashMap<String, WebviewSession>,
}

#[derive(Default)]
struct WebviewAdminStore {
    audit_entries: Vec<WebviewAuditEntry>,
    saved_filters: HashMap<String, Vec<SavedFilterPreset>>,
}

#[derive(Clone)]
struct WebviewAuditEntry {
    audit_id: String,
    operator: String,
    action: String,
    target_type: String,
    target_id: String,
    group_id: Option<String>,
    summary: String,
    status: String,
    created_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedFilterQuery {
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    sort_by: Option<String>,
    #[serde(default)]
    sort_order: Option<String>,
    #[serde(default)]
    only_error: bool,
    #[serde(default)]
    only_actionable: bool,
    #[serde(default)]
    page_size: Option<usize>,
}

#[derive(Clone)]
struct SavedFilterPreset {
    preset_id: String,
    name: String,
    query: SavedFilterQuery,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Serialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Deserialize)]
struct WebviewLoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct WebviewLoginResponse {
    username: String,
    role: String,
    groups: Vec<String>,
    expires_at: i64,
}

#[derive(Serialize)]
struct WebviewMeResponse {
    username: String,
    role: String,
    groups: Vec<String>,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct ListPostsQuery {
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    cursor: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    date_from_ms: Option<i64>,
    #[serde(default)]
    date_to_ms: Option<i64>,
    #[serde(default)]
    sort_by: Option<String>,
    #[serde(default)]
    sort_order: Option<String>,
    #[serde(default)]
    active_only: Option<bool>,
    #[serde(default)]
    only_error: Option<bool>,
    #[serde(default)]
    actionable_only: Option<bool>,
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
    preview_text: Option<String>,
    preview_image_url: Option<String>,
    preview_image_urls: Vec<String>,
    preview_image_count: usize,
}

#[derive(Serialize)]
struct ListPostsResponse {
    items: Vec<PostListItem>,
    next_cursor: Option<usize>,
    total: usize,
}

#[derive(Serialize)]
struct ListReviewIdsResponse {
    review_ids: Vec<String>,
    total: usize,
}

#[derive(Serialize)]
struct StatsResponse {
    pending_count: usize,
    today_count: usize,
    total_count: usize,
    stage_breakdown: HashMap<String, usize>,
    actionable_count: usize,
    error_count: usize,
    daily_trend: Vec<DailyTrendItem>,
    hourly_distribution: Vec<HourlyDistributionItem>,
    avg_review_time_ms: Option<i64>,
}

#[derive(Serialize)]
struct GroupHealthResponse {
    items: Vec<GroupHealthItem>,
}

#[derive(Serialize)]
struct GroupHealthItem {
    group_id: String,
    total_count: usize,
    pending_count: usize,
    actionable_count: usize,
    error_count: usize,
    failed_count: usize,
    sent_count: usize,
    today_count: usize,
    avg_review_time_ms: Option<i64>,
    last_created_at_ms: Option<i64>,
}

#[derive(Serialize)]
struct FailureListResponse {
    summary: FailureSummary,
    items: Vec<FailureItem>,
}

#[derive(Serialize)]
struct FailureSummary {
    total_count: usize,
    stage_failed_count: usize,
    post_error_count: usize,
    render_error_count: usize,
    review_publish_error_count: usize,
}

#[derive(Serialize)]
struct FailureItem {
    post_id: String,
    review_id: Option<String>,
    review_code: Option<u32>,
    group_id: String,
    stage: String,
    source: String,
    error: String,
    created_at_ms: i64,
    sender_id: Option<String>,
    preview_text: Option<String>,
}

#[derive(Serialize)]
struct BlacklistListResponse {
    items: Vec<BlacklistItem>,
    total: usize,
}

#[derive(Serialize)]
struct BlacklistItem {
    group_id: String,
    sender_id: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct CreateBlacklistRequest {
    group_id: String,
    sender_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Serialize)]
struct PostCollectionResponse {
    items: Vec<PostListItem>,
    total: usize,
}

#[derive(Serialize)]
struct SimilarPostResponse {
    items: Vec<SimilarPostItem>,
    total: usize,
}

#[derive(Serialize)]
struct SimilarPostItem {
    post: PostListItem,
    similarity_reason: String,
}

#[derive(Serialize)]
struct AuditListResponse {
    items: Vec<AuditListItem>,
}

#[derive(Serialize)]
struct AuditListItem {
    audit_id: String,
    operator: String,
    action: String,
    target_type: String,
    target_id: String,
    group_id: Option<String>,
    summary: String,
    status: String,
    created_at_ms: i64,
}

#[derive(Serialize)]
struct SavedFilterListResponse {
    items: Vec<SavedFilterPresetResponse>,
}

#[derive(Clone, Serialize)]
struct SavedFilterPresetResponse {
    preset_id: String,
    name: String,
    query: SavedFilterQuery,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Serialize)]
struct DailyTrendItem {
    date: String,
    submitted: usize,
    approved: usize,
    rejected: usize,
    deleted: usize,
}

#[derive(Serialize)]
struct HourlyDistributionItem {
    hour: u8,
    count: usize,
}

#[derive(Serialize)]
struct PostDetailResponse {
    post_id: String,
    review_id: Option<String>,
    review_code: Option<u32>,
    decision_reason: Option<String>,
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
    timeline: Vec<PostTimelineItem>,
    sender_name: Option<String>,
}

#[derive(Serialize)]
struct PostTimelineItem {
    label: String,
    status: String,
    at_ms: Option<i64>,
    detail: Option<String>,
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

#[derive(Deserialize, Clone)]
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
    keyword: Option<String>,
}

#[derive(Deserialize)]
struct ListFailuresQuery {
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct SenderPostsQuery {
    sender_id: String,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct SimilarPostsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    operator: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct SaveFilterPresetRequest {
    #[serde(default)]
    preset_id: Option<String>,
    name: String,
    query: SavedFilterQuery,
}

pub fn spawn_webview(handle: &EngineHandle, config: &AppConfig) {
    if !config.webview_enabled {
        debug_log!("webview disabled by config");
        return;
    }
    if config.webview_admins.is_empty() {
        debug_log!("webview disabled: no webview admins configured");
        return;
    }

    let mut users: HashMap<String, Vec<WebviewAdminAccount>> = HashMap::new();
    for user in &config.webview_admins {
        users
            .entry(user.username.clone())
            .or_default()
            .push(user.clone());
    }
    let state = WebviewState {
        cmd_tx: handle.cmd_tx.clone(),
        state: handle.state(),
        auth: Arc::new(RwLock::new(WebviewAuthStore {
            users,
            sessions: HashMap::new(),
        })),
        admin: Arc::new(RwLock::new(WebviewAdminStore::default())),
        tz_offset_minutes: config.tz_offset_minutes,
        session_ttl_sec: config.webview_session_ttl_sec,
    };

    let app = Router::new()
        .route("/auth/login", post(webview_login))
        .route("/auth/logout", post(webview_logout))
        .route("/auth/me", get(webview_me))
        .route("/api/stats", get(webview_get_stats))
        .route("/api/overview/groups", get(webview_get_group_health))
        .route("/api/failures", get(webview_list_failures))
        .route("/api/posts", get(webview_list_posts))
        .route("/api/posts/{post_id}", get(webview_get_post))
        .route("/api/posts/{post_id}/similar", get(webview_get_similar_posts))
        .route("/api/posts/by-sender", get(webview_list_sender_posts))
        .route("/api/blobs/{blob_id}", get(webview_get_blob))
        .route("/api/blacklist", get(webview_list_blacklist).post(webview_create_blacklist))
        .route(
            "/api/blacklist/{group_id}/{sender_id}",
            post(webview_delete_blacklist),
        )
        .route("/api/audit", get(webview_list_audit))
        .route(
            "/api/filter-presets",
            get(webview_list_filter_presets).post(webview_save_filter_preset),
        )
        .route("/api/reviews/ids", get(webview_list_review_ids))
        .route(
            "/api/reviews/{review_id}/decision",
            post(webview_decide_review),
        )
        .route("/api/reviews/batch", post(webview_decide_review_batch))
        .route("/", get(webview_index))
        .route("/{*path}", get(webview_static))
        .with_state(state);

    let bind_addr = format!("{}:{}", config.webview_host, config.webview_port);
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(listener) => listener,
            Err(_err) => {
                debug_log!("webview bind failed {}: {}", bind_addr, _err);
                return;
            }
        };
        debug_log!("webview started: {}", bind_addr);
        if let Err(_err) = axum::serve(listener, app).await {
            debug_log!("webview stopped: {}", _err);
        }
    });
}

async fn webview_login(
    State(state): State<WebviewState>,
    Json(req): Json<WebviewLoginRequest>,
) -> impl IntoResponse {
    let username = req.username.trim();
    if username.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "username required");
    }
    let password = req.password.trim();
    if password.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "password required");
    }

    let now = now_sec();
    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
            );
        }
    };
    let Some(candidates) = guard.users.get(username).cloned() else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid credential",
        );
    };

    let mut chosen: Option<WebviewIdentity> = None;
    for candidate in candidates {
        if verify_password(password, &candidate.password_hash) {
            chosen = Some(WebviewIdentity {
                username: candidate.username,
                role: candidate.role,
                groups: candidate.groups,
            });
            if matches!(
                chosen.as_ref().map(|v| &v.role),
                Some(WebviewRole::GlobalAdmin)
            ) {
                break;
            }
        }
    }
    let Some(identity) = chosen else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid credential",
        );
    };

    let session_id = random_hex32();
    let expires_at = now + state.session_ttl_sec;
    guard.sessions.insert(
        session_id.clone(),
        WebviewSession {
            identity: identity.clone(),
            expires_at,
        },
    );
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_COOKIE_NAME, session_id, state.session_ttl_sec
    );
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.insert(SET_COOKIE, value);
    }
    (
        StatusCode::OK,
        headers,
        Json(WebviewLoginResponse {
            username: identity.username,
            role: role_to_string(&identity.role).to_string(),
            groups: identity.groups,
            expires_at,
        }),
    )
        .into_response()
}

async fn webview_logout(
    State(state): State<WebviewState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(session_id) = session_cookie(&headers) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    if let Ok(mut guard) = state.auth.write() {
        guard.sessions.remove(&session_id);
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        SET_COOKIE,
        HeaderValue::from_static(
            "oqqwall_webview_session=deleted; Path=/; Max-Age=0; HttpOnly; SameSite=Lax",
        ),
    );
    (StatusCode::NO_CONTENT, response_headers).into_response()
}

async fn webview_me(State(state): State<WebviewState>, headers: HeaderMap) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    (
        StatusCode::OK,
        Json(WebviewMeResponse {
            username: session.identity.username,
            role: role_to_string(&session.identity.role).to_string(),
            groups: session.identity.groups,
            expires_at: session.expires_at,
        }),
    )
        .into_response()
}

async fn webview_get_stats(
    State(state): State<WebviewState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };

    let mut pending_count = 0;
    let mut today_count = 0;
    let mut total_count = 0;
    let mut actionable_count = 0;
    let mut error_count = 0;
    let mut stage_breakdown = HashMap::new();
    let mut daily_trend: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut hourly_distribution = [0usize; 24];
    let mut total_review_time_ms = 0i64;
    let mut total_reviewed_count = 0usize;

    let now_ms = now_ms();
    let day_start_ms = local_day_start_ms(now_ms, state.tz_offset_minutes);

    for (_, meta) in &guard.posts {
        if !can_access_group(allowed_groups.as_ref(), &meta.group_id) {
            continue;
        }
        total_count += 1;
        if meta.stage == PostStage::ReviewPending {
            pending_count += 1;
        }
        if meta.review_id.is_some() {
            actionable_count += 1;
        }
        if meta.last_error.is_some() {
            error_count += 1;
        }
        if meta.created_at_ms >= day_start_ms {
            today_count += 1;
        }
        let stage_str = stage_to_string(meta.stage);
        *stage_breakdown.entry(stage_str).or_insert(0) += 1;

        let date = ms_to_date_string(meta.created_at_ms, state.tz_offset_minutes);
        let trend = daily_trend.entry(date).or_insert((0, 0, 0, 0));
        trend.0 += 1;
        let hour = local_hour(meta.created_at_ms, state.tz_offset_minutes);
        hourly_distribution[usize::from(hour)] += 1;

        if let Some(review_id) = meta.review_id {
            if let Some(review) = guard.reviews.get(&review_id) {
                if let Some(decided_at_ms) = review.decided_at_ms {
                    if decided_at_ms >= meta.created_at_ms {
                        let elapsed = decided_at_ms.saturating_sub(meta.created_at_ms);
                        if elapsed > 0 {
                            total_review_time_ms = total_review_time_ms.saturating_add(elapsed);
                            total_reviewed_count = total_reviewed_count.saturating_add(1);
                        }
                    }
                }
                if let Some(decision) = review.decision {
                    let trend = daily_trend
                        .entry(ms_to_date_string(
                            meta.created_at_ms,
                            state.tz_offset_minutes,
                        ))
                        .or_insert((0, 0, 0, 0));
                    match decision {
                        oqqwall_rust_core::event::ReviewDecision::Approved => trend.1 += 1,
                        oqqwall_rust_core::event::ReviewDecision::Rejected => trend.2 += 1,
                        oqqwall_rust_core::event::ReviewDecision::Deleted => trend.3 += 1,
                        _ => {}
                    }
                }
            }
        }
    }

    let mut daily_trend = daily_trend
        .into_iter()
        .map(
            |(date, (submitted, approved, rejected, deleted))| DailyTrendItem {
                date,
                submitted,
                approved,
                rejected,
                deleted,
            },
        )
        .collect::<Vec<_>>();
    daily_trend.sort_by(|a, b| a.date.cmp(&b.date));
    if daily_trend.len() > 14 {
        daily_trend = daily_trend.split_off(daily_trend.len() - 14);
    }
    let hourly_distribution = hourly_distribution
        .into_iter()
        .enumerate()
        .map(|(hour, count)| HourlyDistributionItem {
            hour: hour as u8,
            count,
        })
        .collect::<Vec<_>>();
    let avg_review_time_ms = if total_reviewed_count > 0 {
        Some(total_review_time_ms / total_reviewed_count as i64)
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(StatsResponse {
            pending_count,
            today_count,
            total_count,
            stage_breakdown,
            actionable_count,
            error_count,
            daily_trend,
            hourly_distribution,
            avg_review_time_ms,
        }),
    )
        .into_response()
}

async fn webview_get_group_health(
    State(state): State<WebviewState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };

    let mut groups: HashMap<String, GroupHealthItem> = HashMap::new();
    let mut review_elapsed: HashMap<String, (i64, usize)> = HashMap::new();

    for (_, meta) in &guard.posts {
        if !can_access_group(allowed_groups.as_ref(), &meta.group_id) {
            continue;
        }
        let entry = groups.entry(meta.group_id.clone()).or_insert(GroupHealthItem {
            group_id: meta.group_id.clone(),
            total_count: 0,
            pending_count: 0,
            actionable_count: 0,
            error_count: 0,
            failed_count: 0,
            sent_count: 0,
            today_count: 0,
            avg_review_time_ms: None,
            last_created_at_ms: None,
        });
        entry.total_count = entry.total_count.saturating_add(1);
        if meta.stage == PostStage::ReviewPending {
            entry.pending_count = entry.pending_count.saturating_add(1);
        }
        if meta.review_id.is_some() {
            entry.actionable_count = entry.actionable_count.saturating_add(1);
        }
        if meta.last_error.is_some() {
            entry.error_count = entry.error_count.saturating_add(1);
        }
        if meta.stage == PostStage::Failed {
            entry.failed_count = entry.failed_count.saturating_add(1);
        }
        if meta.stage == PostStage::Sent {
            entry.sent_count = entry.sent_count.saturating_add(1);
        }
        if meta.created_at_ms >= local_day_start_ms(now_ms(), state.tz_offset_minutes) {
            entry.today_count = entry.today_count.saturating_add(1);
        }
        entry.last_created_at_ms = Some(
            entry
                .last_created_at_ms
                .map(|current| current.max(meta.created_at_ms))
                .unwrap_or(meta.created_at_ms),
        );

        if let Some(review_id) = meta.review_id {
            if let Some(review) = guard.reviews.get(&review_id) {
                if let Some(decided_at_ms) = review.decided_at_ms {
                    let elapsed = decided_at_ms.saturating_sub(meta.created_at_ms);
                    let slot = review_elapsed.entry(meta.group_id.clone()).or_insert((0, 0));
                    slot.0 = slot.0.saturating_add(elapsed.max(0));
                    slot.1 = slot.1.saturating_add(1);
                }
            }
        }
    }

    for item in groups.values_mut() {
        if let Some((elapsed, count)) = review_elapsed.get(&item.group_id) {
            if *count > 0 {
                item.avg_review_time_ms = Some(*elapsed / *count as i64);
            }
        }
    }

    let mut items = groups.into_values().collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.pending_count
            .cmp(&a.pending_count)
            .then_with(|| b.error_count.cmp(&a.error_count))
            .then_with(|| a.group_id.cmp(&b.group_id))
    });

    (StatusCode::OK, Json(GroupHealthResponse { items })).into_response()
}

async fn webview_list_failures(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<ListFailuresQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let keyword = query
        .keyword
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let group_filter = query
        .group_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let source_filter = query
        .source
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let limit = query.limit.unwrap_or(100).clamp(1, 300);

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };

    let mut summary = FailureSummary {
        total_count: 0,
        stage_failed_count: 0,
        post_error_count: 0,
        render_error_count: 0,
        review_publish_error_count: 0,
    };
    let mut items = Vec::new();

    for (post_id, meta) in &guard.posts {
        if !can_access_group(allowed_groups.as_ref(), &meta.group_id) {
            continue;
        }
        if group_filter.map(|value| value != meta.group_id).unwrap_or(false) {
            continue;
        }

        let sender_id = primary_sender_id(&guard, meta);
        let preview_text = post_preview_text(&guard, *post_id);
        let review_code = meta
            .review_id
            .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));

        let mut push_item = |source: &str, error: String| {
            if source_filter
                .as_deref()
                .map(|value| value != source)
                .unwrap_or(false)
            {
                return;
            }
            let haystack = format!(
                "{} {} {} {}",
                meta.group_id,
                sender_id.clone().unwrap_or_default(),
                preview_text.clone().unwrap_or_default(),
                error
            )
            .to_ascii_lowercase();
            if keyword
                .as_deref()
                .map(|needle| !haystack.contains(needle))
                .unwrap_or(false)
            {
                return;
            }
            summary.total_count = summary.total_count.saturating_add(1);
            items.push(FailureItem {
                post_id: id_to_string(*post_id),
                review_id: meta.review_id.map(id_to_string),
                review_code,
                group_id: meta.group_id.clone(),
                stage: stage_to_string(meta.stage),
                source: source.to_string(),
                error,
                created_at_ms: meta.created_at_ms,
                sender_id: sender_id.clone(),
                preview_text: preview_text.clone(),
            });
        };

        if meta.stage == PostStage::Failed {
            summary.stage_failed_count = summary.stage_failed_count.saturating_add(1);
        }
        if let Some(error) = meta.last_error.clone() {
            summary.post_error_count = summary.post_error_count.saturating_add(1);
            push_item("post", error);
        }
        if let Some(render) = guard.render.get(post_id) {
            if let Some(error) = render.last_error.clone() {
                summary.render_error_count = summary.render_error_count.saturating_add(1);
                push_item("render", error);
            }
        }
        if let Some(review_id) = meta.review_id {
            if let Some(review) = guard.reviews.get(&review_id) {
                if let Some(error) = review.publish_last_error.clone() {
                    summary.review_publish_error_count =
                        summary.review_publish_error_count.saturating_add(1);
                    push_item("review_publish", error);
                }
            }
        }
    }

    items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    items.truncate(limit);

    (
        StatusCode::OK,
        Json(FailureListResponse { summary, items }),
    )
        .into_response()
}

async fn webview_list_posts(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<ListPostsQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let stage_filter = query.stage.as_deref().and_then(parse_stage);
    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let allowed_groups = allowed_groups(&session.identity);
    let keyword_lower = query
        .keyword
        .as_ref()
        .map(|keyword| keyword.trim().to_ascii_lowercase())
        .filter(|keyword| !keyword.is_empty());
    let group_filter = query
        .group_id
        .as_ref()
        .map(|group| group.trim())
        .filter(|group| !group.is_empty());
    let sort_by = query.sort_by.as_deref().unwrap_or("created_at");
    let sort_asc = matches!(query.sort_order.as_deref(), Some("asc"));

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };
    let mut rows = guard
        .posts
        .iter()
        .filter(|(_, meta)| can_access_group(allowed_groups.as_ref(), &meta.group_id))
        .filter(|(_, meta)| {
            stage_filter
                .map(|stage| stage == meta.stage)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| {
            if query.active_only.unwrap_or(false) {
                is_active_stage(meta.stage)
            } else {
                true
            }
        })
        .filter(|(_, meta)| {
            group_filter
                .map(|group| group == meta.group_id)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| {
            if query.only_error.unwrap_or(false) {
                meta.last_error.is_some()
            } else {
                true
            }
        })
        .filter(|(_, meta)| {
            if query.actionable_only.unwrap_or(false) {
                meta.review_id.is_some()
            } else {
                true
            }
        })
        .filter(|(_, meta)| {
            query
                .date_from_ms
                .map(|from| meta.created_at_ms >= from)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| {
            query
                .date_to_ms
                .map(|to| meta.created_at_ms <= to)
                .unwrap_or(true)
        })
        .filter(|(post_id, meta)| {
            keyword_lower
                .as_deref()
                .map(|keyword| post_keyword_matches(&guard, post_id, meta, keyword))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    sort_post_rows(&guard, &mut rows, sort_by, sort_asc);
    let total = rows.len();

    let items = rows
        .iter()
        .skip(cursor)
        .take(limit)
        .map(|(_, meta)| {
            let sender_id = guard
                .session_ingress
                .get(&meta.session_id)
                .and_then(|ids| ids.first())
                .and_then(|id| guard.ingress_meta.get(id))
                .map(|ingress| ingress.user_id.clone());
            let review_code = meta
                .review_id
                .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));

            let draft = guard.drafts.get(&meta.post_id);
            let preview_text = draft.and_then(|d| {
                d.blocks.iter().find_map(|b| match b {
                    oqqwall_rust_core::draft::DraftBlock::Paragraph { text } => {
                        Some(text.chars().take(100).collect::<String>())
                    }
                    _ => None,
                })
            });

            let draft_image_urls = draft
                .map(|d| {
                    d.blocks
                        .iter()
                        .filter_map(|b| match b {
                            oqqwall_rust_core::draft::DraftBlock::Attachment {
                                reference,
                                kind: oqqwall_rust_core::draft::MediaKind::Image,
                                ..
                            } => match reference {
                                MediaReference::Blob { blob_id } => {
                                    Some(format!("/api/blobs/{}", id_to_string(*blob_id)))
                                }
                                MediaReference::RemoteUrl { url } => Some(url.clone()),
                            },
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let render_blob_ids = guard
                .render
                .get(&meta.post_id)
                .map(|render| {
                    if render.png_blobs.is_empty() {
                        render.png_blob.into_iter().collect::<Vec<_>>()
                    } else {
                        render.png_blobs.clone()
                    }
                })
                .unwrap_or_default();
            let mut preview_image_urls = render_blob_ids
                .into_iter()
                .map(|blob_id| format!("/api/blobs/{}", id_to_string(blob_id)))
                .collect::<Vec<_>>();
            preview_image_urls.extend(draft_image_urls);
            let preview_image_url = preview_image_urls.first().cloned();
            let preview_image_count = preview_image_urls.len();

            PostListItem {
                post_id: id_to_string(meta.post_id),
                review_id: meta.review_id.map(id_to_string),
                group_id: meta.group_id.clone(),
                stage: stage_to_string(meta.stage),
                external_code: guard.external_code_by_post.get(&meta.post_id).copied(),
                internal_code: review_code,
                sender_id,
                created_at_ms: meta.created_at_ms,
                last_error: meta.last_error.clone(),
                preview_text,
                preview_image_url,
                preview_image_urls,
                preview_image_count,
            }
        })
        .collect::<Vec<_>>();
    let next_cursor = if cursor + items.len() < rows.len() {
        Some(cursor + items.len())
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(ListPostsResponse {
            items,
            next_cursor,
            total,
        }),
    )
        .into_response()
}

async fn webview_list_review_ids(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<ListPostsQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let stage_filter = query.stage.as_deref().and_then(parse_stage);
    let allowed_groups = allowed_groups(&session.identity);
    let keyword_lower = query
        .keyword
        .as_ref()
        .map(|keyword| keyword.trim().to_ascii_lowercase())
        .filter(|keyword| !keyword.is_empty());
    let group_filter = query
        .group_id
        .as_ref()
        .map(|group| group.trim())
        .filter(|group| !group.is_empty());

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };

    let review_ids = guard
        .posts
        .iter()
        .filter(|(_, meta)| can_access_group(allowed_groups.as_ref(), &meta.group_id))
        .filter(|(_, meta)| {
            stage_filter
                .map(|stage| stage == meta.stage)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| {
            if query.active_only.unwrap_or(false) {
                is_active_stage(meta.stage)
            } else {
                true
            }
        })
        .filter(|(_, meta)| {
            group_filter
                .map(|group| group == meta.group_id)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| {
            if query.only_error.unwrap_or(false) {
                meta.last_error.is_some()
            } else {
                true
            }
        })
        .filter(|(_, meta)| {
            if query.actionable_only.unwrap_or(false) {
                meta.review_id.is_some()
            } else {
                true
            }
        })
        .filter(|(_, meta)| {
            query
                .date_from_ms
                .map(|from| meta.created_at_ms >= from)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| {
            query
                .date_to_ms
                .map(|to| meta.created_at_ms <= to)
                .unwrap_or(true)
        })
        .filter(|(post_id, meta)| {
            keyword_lower
                .as_deref()
                .map(|keyword| post_keyword_matches(&guard, post_id, meta, keyword))
                .unwrap_or(true)
        })
        .filter_map(|(_, meta)| meta.review_id.map(id_to_string))
        .collect::<Vec<_>>();
    let total = review_ids.len();

    (
        StatusCode::OK,
        Json(ListReviewIdsResponse { review_ids, total }),
    )
        .into_response()
}

async fn webview_get_post(
    State(state): State<WebviewState>,
    Path(post_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let Some(post_id) = parse_id128(&post_id) else {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "invalid post_id");
    };
    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };
    let Some(meta) = guard.posts.get(&post_id) else {
        return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "post not found");
    };
    if !can_access_group(allowed_groups.as_ref(), &meta.group_id) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "permission denied",
        );
    }

    let review_code = meta
        .review_id
        .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));
    let decision_reason = meta.review_id.and_then(|id| {
        guard
            .reviews
            .get(&id)
            .and_then(|review| review.decision_reason.clone())
    });
    let sender_id = primary_sender_id(&guard, meta);
    let sender_name = primary_sender_name(&guard, meta);
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
        .and_then(|render| {
            render
                .png_blob
                .or_else(|| render.png_blobs.first().copied())
        })
        .map(id_to_string);
    let timeline = build_post_timeline(&guard, meta);

    (
        StatusCode::OK,
        Json(PostDetailResponse {
            post_id: id_to_string(meta.post_id),
            review_id: meta.review_id.map(id_to_string),
            review_code,
            decision_reason,
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
            timeline,
            sender_name,
        }),
    )
        .into_response()
}

async fn webview_get_blob(
    State(state): State<WebviewState>,
    Path(blob_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let Some(blob_id) = parse_id128(&blob_id) else {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "invalid blob_id");
    };
    let path = {
        let guard = match state.state.read() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "state unavailable",
                );
            }
        };
        if !can_access_blob(&guard, allowed_groups.as_ref(), blob_id) {
            return error_response(
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "permission denied",
            );
        }
        let Some(meta) = guard.blobs.get(&blob_id) else {
            return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "blob not found");
        };
        let Some(path) = meta.persisted_path.clone() else {
            return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "blob not available");
        };
        path
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "blob file missing"),
    };

    let mut response_headers = HeaderMap::new();
    let mime = detect_mime_from_path(&path);
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

async fn webview_list_blacklist(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<ListBlacklistQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let keyword = query
        .keyword
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let group_filter = query
        .group_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };

    let mut items = Vec::new();
    for (group_id, group) in &guard.blacklist {
        if !can_access_group(allowed_groups.as_ref(), group_id) {
            continue;
        }
        if group_filter.map(|value| value != group_id).unwrap_or(false) {
            continue;
        }
        for (sender_id, reason) in group {
            let haystack = format!(
                "{} {} {}",
                group_id,
                sender_id,
                reason.clone().unwrap_or_default()
            )
            .to_ascii_lowercase();
            if keyword
                .as_deref()
                .map(|needle| !haystack.contains(needle))
                .unwrap_or(false)
            {
                continue;
            }
            items.push(BlacklistItem {
                group_id: group_id.clone(),
                sender_id: sender_id.clone(),
                reason: reason.clone(),
            });
        }
    }
    items.sort_by(|a, b| {
        a.group_id
            .cmp(&b.group_id)
            .then_with(|| a.sender_id.cmp(&b.sender_id))
    });

    let total = items.len();
    (
        StatusCode::OK,
        Json(BlacklistListResponse { items, total }),
    )
        .into_response()
}

async fn webview_create_blacklist(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Json(req): Json<CreateBlacklistRequest>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed = allowed_groups(&session.identity);
    if !can_access_group(allowed.as_ref(), &req.group_id) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "permission denied",
        );
    }
    let cmd = Command::GlobalAction(oqqwall_rust_core::GlobalActionCommand {
        group_id: req.group_id.clone(),
        action: oqqwall_rust_core::GlobalAction::BlacklistAdd {
            sender_id: req.sender_id.clone(),
            reason: req.reason.clone(),
        },
        operator_id: format!("webview:{}", session.identity.username),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
        );
    }
    append_audit_entry(
        &state,
        WebviewAuditEntry {
            audit_id: random_hex32(),
            operator: session.identity.username,
            action: "blacklist_add".to_string(),
            target_type: "sender".to_string(),
            target_id: req.sender_id,
            group_id: Some(req.group_id),
            summary: "已从后台加入黑名单".to_string(),
            status: "submitted".to_string(),
            created_at_ms: now_ms(),
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn webview_delete_blacklist(
    State(state): State<WebviewState>,
    Path((group_id, sender_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed = allowed_groups(&session.identity);
    if !can_access_group(allowed.as_ref(), &group_id) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "permission denied",
        );
    }
    let audit_group_id = group_id.clone();
    let cmd = Command::GlobalAction(oqqwall_rust_core::GlobalActionCommand {
        group_id,
        action: oqqwall_rust_core::GlobalAction::BlacklistRemove {
            sender_id: sender_id.clone(),
        },
        operator_id: format!("webview:{}", session.identity.username),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
        );
    }
    append_audit_entry(
        &state,
        WebviewAuditEntry {
            audit_id: random_hex32(),
            operator: session.identity.username,
            action: "blacklist_remove".to_string(),
            target_type: "sender".to_string(),
            target_id: sender_id,
            group_id: Some(audit_group_id),
            summary: "已从后台移出黑名单".to_string(),
            status: "submitted".to_string(),
            created_at_ms: now_ms(),
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn webview_list_sender_posts(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<SenderPostsQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let sender_id = query.sender_id.trim();
    if sender_id.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "sender_id required");
    }
    let group_filter = query
        .group_id
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };

    let mut items = guard
        .posts
        .iter()
        .filter(|(_, meta)| can_access_group(allowed_groups.as_ref(), &meta.group_id))
        .filter(|(_, meta)| {
            group_filter
                .map(|value| value == meta.group_id)
                .unwrap_or(true)
        })
        .filter(|(_, meta)| sender_matches(&guard, meta, sender_id))
        .map(|(post_id, meta)| build_post_list_item(&guard, *post_id, meta))
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    let total = items.len();
    items.truncate(limit);

    (
        StatusCode::OK,
        Json(PostCollectionResponse { items, total }),
    )
        .into_response()
}

async fn webview_get_similar_posts(
    State(state): State<WebviewState>,
    Path(post_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<SimilarPostsQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let Some(post_id) = parse_id128(&post_id) else {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "invalid post_id");
    };
    let limit = query.limit.unwrap_or(10).clamp(1, 30);
    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
            );
        }
    };
    let Some(base_meta) = guard.posts.get(&post_id) else {
        return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "post not found");
    };
    if !can_access_group(allowed_groups.as_ref(), &base_meta.group_id) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "permission denied",
        );
    }

    let base_sender = primary_sender_id(&guard, base_meta);
    let base_text = post_preview_text(&guard, post_id);
    let base_group = base_meta.group_id.clone();
    let mut items = guard
        .posts
        .iter()
        .filter(|(candidate_id, _candidate_meta)| **candidate_id != post_id)
        .filter(|(_, candidate_meta)| can_access_group(allowed_groups.as_ref(), &candidate_meta.group_id))
        .filter(|(_, candidate_meta)| {
            candidate_meta.group_id == base_group
                || sender_matches(&guard, candidate_meta, base_sender.as_deref().unwrap_or(""))
        })
        .map(|(candidate_id, candidate_meta)| {
            let mut score = 0usize;
            let mut reason = Vec::new();
            if candidate_meta.group_id == base_group {
                score = score.saturating_add(2);
                reason.push("同组");
            }
            let candidate_sender = primary_sender_id(&guard, candidate_meta);
            if base_sender.is_some() && candidate_sender == base_sender {
                score = score.saturating_add(3);
                reason.push("同投稿人");
            }
            if let (Some(base), Some(candidate)) = (&base_text, post_preview_text(&guard, *candidate_id)) {
                if text_similarity(base, &candidate) {
                    score = score.saturating_add(4);
                    reason.push("内容相近");
                }
            }
            (
                score,
                SimilarPostItem {
                    post: build_post_list_item(&guard, *candidate_id, candidate_meta),
                    similarity_reason: if reason.is_empty() {
                        "弱相关".to_string()
                    } else {
                        reason.join("，")
                    },
                },
            )
        })
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.0.cmp(&a.0));
    let total = items.len();
    let items = items
        .into_iter()
        .take(limit)
        .map(|(_, item)| item)
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(SimilarPostResponse { items, total }),
    )
        .into_response()
}

async fn webview_list_audit(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let allowed_groups = allowed_groups(&session.identity);
    let keyword = query
        .operator
        .as_ref()
        .or(query.group_id.as_ref())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let limit = query.limit.unwrap_or(100).clamp(1, 300);
    let guard = match state.admin.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "audit store unavailable",
            );
        }
    };

    let mut items = guard
        .audit_entries
        .iter()
        .filter(|entry| {
            if let Some(group_id) = query.group_id.as_deref() {
                entry.group_id.as_deref() == Some(group_id)
            } else {
                true
            }
        })
        .filter(|entry| {
            if let Some(ref keyword) = keyword {
                entry.summary.to_ascii_lowercase().contains(keyword)
                    || entry.operator.to_ascii_lowercase().contains(keyword)
            } else {
                true
            }
        })
        .filter(|entry| {
            if let Some(groups) = allowed_groups.as_ref() {
                entry
                    .group_id
                    .as_deref()
                    .map(|group_id| groups.contains(group_id))
                    .unwrap_or(true)
            } else {
                true
            }
        })
        .map(|entry| AuditListItem {
            audit_id: entry.audit_id.clone(),
            operator: entry.operator.clone(),
            action: entry.action.clone(),
            target_type: entry.target_type.clone(),
            target_id: entry.target_id.clone(),
            group_id: entry.group_id.clone(),
            summary: entry.summary.clone(),
            status: entry.status.clone(),
            created_at_ms: entry.created_at_ms,
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    items.truncate(limit);

    (
        StatusCode::OK,
        Json(AuditListResponse { items }),
    )
        .into_response()
}

async fn webview_list_filter_presets(
    State(state): State<WebviewState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let guard = match state.admin.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "filter store unavailable",
            );
        }
    };
    let items = guard
        .saved_filters
        .get(&session.identity.username)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|preset| SavedFilterPresetResponse {
            preset_id: preset.preset_id,
            name: preset.name,
            query: preset.query,
            created_at_ms: preset.created_at_ms,
            updated_at_ms: preset.updated_at_ms,
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(SavedFilterListResponse { items })).into_response()
}

async fn webview_save_filter_preset(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Json(req): Json<SaveFilterPresetRequest>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let now = now_ms();
    let mut guard = match state.admin.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "filter store unavailable",
            );
        }
    };
    let items = guard
        .saved_filters
        .entry(session.identity.username.clone())
        .or_default();
    let preset_id = req.preset_id.unwrap_or_else(random_hex32);
    if let Some(existing) = items.iter_mut().find(|preset| preset.preset_id == preset_id) {
        existing.name = req.name.clone();
        existing.query = req.query.clone();
        existing.updated_at_ms = now;
    } else {
        items.push(SavedFilterPreset {
            preset_id: preset_id.clone(),
            name: req.name.clone(),
            query: req.query.clone(),
            created_at_ms: now,
            updated_at_ms: now,
        });
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    drop(guard);
    append_audit_entry(
        &state,
        WebviewAuditEntry {
            audit_id: random_hex32(),
            operator: session.identity.username,
            action: "filter_preset_save".to_string(),
            target_type: "filter_preset".to_string(),
            target_id: preset_id,
            group_id: req.query.group_id.clone(),
            summary: format!("已保存筛选器：{}", req.name),
            status: "saved".to_string(),
            created_at_ms: now,
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn webview_decide_review(
    State(state): State<WebviewState>,
    Path(review_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ReviewDecisionRequest>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let Some(review_id) = parse_id128(&review_id) else {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "invalid review_id");
    };
    let action = match parse_review_action(&req) {
        Ok(action) => action,
        Err(reason) => return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", reason),
    };
    if !can_access_review(&state, &session.identity, review_id) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "permission denied",
        );
    }
    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: Some(review_id),
        review_code: None,
        audit_msg_id: None,
        action,
        operator_id: format!("webview:{}", session.identity.username),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
        );
    }
    let summary = format!(
        "执行稿件操作：{} #{}",
        req.action,
        id_to_string(review_id)
    );
    let audit_group_id = {
        let Ok(guard) = state.state.read() else {
            None
        };
        guard
            .reviews
            .get(&review_id)
            .and_then(|review| guard.posts.get(&review.post_id))
            .map(|post| post.group_id.clone())
    };
    append_audit_entry(
        &state,
        WebviewAuditEntry {
            audit_id: random_hex32(),
            operator: session.identity.username,
            action: req.action.clone(),
            target_type: "review".to_string(),
            target_id: id_to_string(review_id),
            group_id: audit_group_id,
            summary,
            status: "applied".to_string(),
            created_at_ms: now_ms(),
        },
    );
    (
        StatusCode::OK,
        Json(ReviewDecisionResponse {
            review_id: id_to_string(review_id),
            status: "applied",
        }),
    )
        .into_response()
}

async fn webview_decide_review_batch(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Json(req): Json<BatchReviewDecisionRequest>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let action_req = ReviewDecisionRequest {
        action: req.action,
        comment: req.comment,
        delay_ms: req.delay_ms,
        text: req.text,
        quick_reply_key: req.quick_reply_key,
        target_review_code: req.target_review_code,
    };
    let action = match parse_review_action(&action_req) {
        Ok(action) => action,
        Err(reason) => return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", reason),
    };
    let mut accepted = 0usize;
    let mut failed = Vec::new();
    let requested_action = req.action.clone();
    let requested_count = req.review_ids.len();
    let mut batch_group_ids = HashSet::new();
    for raw_review_id in req.review_ids {
        let Some(review_id) = parse_id128(&raw_review_id) else {
            failed.push(ReviewFailure {
                review_id: raw_review_id,
                reason: "invalid review_id".to_string(),
            });
            continue;
        };
        if let Ok(guard) = state.state.read() {
            if let Some(review) = guard.reviews.get(&review_id) {
                if let Some(post) = guard.posts.get(&review.post_id) {
                    batch_group_ids.insert(post.group_id.clone());
                }
            }
        }
        if !can_access_review(&state, &session.identity, review_id) {
            failed.push(ReviewFailure {
                review_id: id_to_string(review_id),
                reason: "permission denied".to_string(),
            });
            continue;
        }
        let cmd = Command::ReviewAction(ReviewActionCommand {
            review_id: Some(review_id),
            review_code: None,
            audit_msg_id: None,
            action: action.clone(),
            operator_id: format!("webview:{}", session.identity.username),
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
    append_audit_entry(
        &state,
        WebviewAuditEntry {
            audit_id: random_hex32(),
            operator: session.identity.username,
            action: format!("batch:{}", requested_action),
            target_type: "review_batch".to_string(),
            target_id: format!("accepted:{} requested:{}", accepted, requested_count),
            group_id: if batch_group_ids.len() == 1 {
                batch_group_ids.iter().next().cloned()
            } else {
                None
            },
            summary: format!("批量执行 {}，成功 {} 条，失败 {} 条", requested_action, accepted, failed.len()),
            status: if failed.is_empty() {
                "applied".to_string()
            } else {
                "partial".to_string()
            },
            created_at_ms: now_ms(),
        },
    );

    (
        StatusCode::OK,
        Json(BatchReviewDecisionResponse { accepted, failed }),
    )
        .into_response()
}

async fn webview_index(State(_state): State<WebviewState>) -> impl IntoResponse {
    serve_static_path("/index.html")
}

async fn webview_static(
    State(_state): State<WebviewState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let req_path = format!("/{}", path.trim_start_matches('/'));
    serve_static_path(&req_path)
}

fn serve_static_path(req_path: &str) -> axum::response::Response {
    let asset = find_asset(req_path).or_else(|| {
        if req_path == "/" || !req_path.contains('.') {
            find_asset("/index.html")
        } else {
            None
        }
    });
    if let Some(asset) = asset {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_str(asset.content_type)
                .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
        );
        let cache = if req_path.starts_with("/assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_str(cache).unwrap_or_else(|_| HeaderValue::from_static("no-cache")),
        );
        return (StatusCode::OK, headers, asset.bytes).into_response();
    }
    if req_path.starts_with("/assets/") || req_path.contains('.') {
        return (
            StatusCode::NOT_FOUND,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            )],
            "asset not found",
        )
            .into_response();
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        "<!doctype html><meta charset='utf-8'><title>Webview UI Missing</title><h1>webview-ui dist not found</h1><p>Run bun run build in crates/app/webview-ui.</p>",
    )
        .into_response()
}

fn authenticate_webview(
    state: &WebviewState,
    headers: &HeaderMap,
) -> Result<WebviewSession, axum::response::Response> {
    let Some(session_id) = session_cookie(headers) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "missing session",
        ));
    };
    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
            ));
        }
    };
    let now = now_sec();
    guard.sessions.retain(|_, session| session.expires_at > now);
    let Some(session) = guard.sessions.get(&session_id).cloned() else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid session",
        ));
    };
    Ok(session)
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let mut iter = pair.trim().splitn(2, '=');
        let key = iter.next()?.trim();
        let value = iter.next()?.trim();
        if key == SESSION_COOKIE_NAME && !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn allowed_groups(identity: &WebviewIdentity) -> Option<HashSet<String>> {
    if identity.role == WebviewRole::GlobalAdmin {
        return None;
    }
    Some(identity.groups.iter().cloned().collect())
}

fn can_access_group(allowed_groups: Option<&HashSet<String>>, group_id: &str) -> bool {
    allowed_groups
        .map(|groups| groups.contains(group_id))
        .unwrap_or(true)
}

fn can_access_review(state: &WebviewState, identity: &WebviewIdentity, review_id: Id128) -> bool {
    let allowed = allowed_groups(identity);
    if allowed.is_none() {
        return true;
    }
    let Ok(guard) = state.state.read() else {
        return false;
    };
    let Some(review) = guard.reviews.get(&review_id) else {
        return false;
    };
    let Some(post) = guard.posts.get(&review.post_id) else {
        return false;
    };
    can_access_group(allowed.as_ref(), &post.group_id)
}

fn can_access_blob(
    snapshot: &StateView,
    allowed_groups: Option<&HashSet<String>>,
    blob_id: Id128,
) -> bool {
    if allowed_groups.is_none() {
        return true;
    }
    for (post_id, post_meta) in &snapshot.posts {
        if !can_access_group(allowed_groups, &post_meta.group_id) {
            continue;
        }
        if snapshot
            .render
            .get(post_id)
            .is_some_and(|meta| meta.png_blob == Some(blob_id) || meta.png_blobs.contains(&blob_id))
        {
            return true;
        }
        let Some(draft) = snapshot.drafts.get(post_id) else {
            continue;
        };
        for block in &draft.blocks {
            if let oqqwall_rust_core::draft::DraftBlock::Attachment { reference, .. } = block {
                if let MediaReference::Blob { blob_id: bid } = reference {
                    if *bid == blob_id {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn sort_post_rows<'a>(
    snapshot: &StateView,
    rows: &mut Vec<(&'a Id128, &'a PostMeta)>,
    sort_by: &str,
    sort_asc: bool,
) {
    rows.sort_by(|(post_a, meta_a), (post_b, meta_b)| {
        let ordering = match sort_by {
            "stage" => stage_to_string(meta_a.stage).cmp(&stage_to_string(meta_b.stage)),
            "group_id" | "group" => meta_a.group_id.cmp(&meta_b.group_id),
            "code" | "external_code" => {
                let code_a = snapshot
                    .external_code_by_post
                    .get(post_a)
                    .copied()
                    .or_else(|| {
                        meta_a.review_id.and_then(|id| {
                            snapshot
                                .reviews
                                .get(&id)
                                .map(|review| review.review_code as u64)
                        })
                    })
                    .unwrap_or(0);
                let code_b = snapshot
                    .external_code_by_post
                    .get(post_b)
                    .copied()
                    .or_else(|| {
                        meta_b.review_id.and_then(|id| {
                            snapshot
                                .reviews
                                .get(&id)
                                .map(|review| review.review_code as u64)
                        })
                    })
                    .unwrap_or(0);
                code_a.cmp(&code_b)
            }
            _ => meta_a.created_at_ms.cmp(&meta_b.created_at_ms),
        };
        if sort_asc {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

fn post_keyword_matches(
    snapshot: &StateView,
    post_id: &Id128,
    meta: &PostMeta,
    keyword_lower: &str,
) -> bool {
    if meta.group_id.to_ascii_lowercase().contains(keyword_lower) {
        return true;
    }
    if meta
        .last_error
        .as_ref()
        .map(|error| error.to_ascii_lowercase().contains(keyword_lower))
        .unwrap_or(false)
    {
        return true;
    }
    if snapshot
        .external_code_by_post
        .get(post_id)
        .map(|code| code.to_string().contains(keyword_lower))
        .unwrap_or(false)
    {
        return true;
    }
    if meta
        .review_id
        .and_then(|id| snapshot.reviews.get(&id).map(|review| review.review_code))
        .map(|code| code.to_string().contains(keyword_lower))
        .unwrap_or(false)
    {
        return true;
    }
    if snapshot
        .session_ingress
        .get(&meta.session_id)
        .and_then(|ids| ids.first())
        .and_then(|id| snapshot.ingress_meta.get(id))
        .map(|ingress| {
            ingress.user_id.to_ascii_lowercase().contains(keyword_lower)
                || ingress
                    .sender_name
                    .as_ref()
                    .map(|name| name.to_ascii_lowercase().contains(keyword_lower))
                    .unwrap_or(false)
        })
        .unwrap_or(false)
    {
        return true;
    }
    snapshot
        .drafts
        .get(post_id)
        .map(|draft| {
            draft.blocks.iter().any(|block| match block {
                oqqwall_rust_core::draft::DraftBlock::Paragraph { text } => {
                    text.to_ascii_lowercase().contains(keyword_lower)
                }
                oqqwall_rust_core::draft::DraftBlock::Attachment { kind, .. } => {
                    media_kind_to_string(*kind).contains(keyword_lower)
                }
                oqqwall_rust_core::draft::DraftBlock::Reply { preview } => {
                    preview.body.to_ascii_lowercase().contains(keyword_lower)
                }
                oqqwall_rust_core::draft::DraftBlock::Poke => "戳一戳".contains(keyword_lower),
                oqqwall_rust_core::draft::DraftBlock::JsonCard { raw } => {
                    raw.to_ascii_lowercase().contains(keyword_lower)
                }
                oqqwall_rust_core::draft::DraftBlock::Forward { items } => {
                    items.iter().any(|item| {
                        item.sender_name
                            .as_deref()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains(keyword_lower)
                    })
                }
            })
        })
        .unwrap_or(false)
}

fn build_post_list_item(snapshot: &StateView, post_id: Id128, meta: &PostMeta) -> PostListItem {
    let sender_id = primary_sender_id(snapshot, meta);
    let review_code = meta
        .review_id
        .and_then(|id| snapshot.reviews.get(&id).map(|review| review.review_code));
    let preview_text = post_preview_text(snapshot, post_id);
    let preview_image_urls = post_preview_images(snapshot, post_id);
    let preview_image_url = preview_image_urls.first().cloned();
    let preview_image_count = preview_image_urls.len();

    PostListItem {
        post_id: id_to_string(meta.post_id),
        review_id: meta.review_id.map(id_to_string),
        group_id: meta.group_id.clone(),
        stage: stage_to_string(meta.stage),
        external_code: snapshot.external_code_by_post.get(&meta.post_id).copied(),
        internal_code: review_code,
        sender_id,
        created_at_ms: meta.created_at_ms,
        last_error: meta.last_error.clone(),
        preview_text,
        preview_image_url,
        preview_image_urls,
        preview_image_count,
    }
}

fn primary_sender_id(snapshot: &StateView, meta: &PostMeta) -> Option<String> {
    snapshot
        .session_ingress
        .get(&meta.session_id)
        .and_then(|ids| ids.first())
        .and_then(|id| snapshot.ingress_meta.get(id))
        .map(|ingress| ingress.user_id.clone())
}

fn primary_sender_name(snapshot: &StateView, meta: &PostMeta) -> Option<String> {
    snapshot
        .session_ingress
        .get(&meta.session_id)
        .and_then(|ids| ids.first())
        .and_then(|id| snapshot.ingress_meta.get(id))
        .and_then(|ingress| ingress.sender_name.clone())
}

fn sender_matches(snapshot: &StateView, meta: &PostMeta, sender_id: &str) -> bool {
    if sender_id.is_empty() {
        return false;
    }
    primary_sender_id(snapshot, meta)
        .map(|value| value == sender_id)
        .unwrap_or(false)
}

fn post_preview_text(snapshot: &StateView, post_id: Id128) -> Option<String> {
    snapshot.drafts.get(&post_id).and_then(|draft| {
        draft.blocks.iter().find_map(|block| match block {
            oqqwall_rust_core::draft::DraftBlock::Paragraph { text } => {
                Some(text.chars().take(100).collect::<String>())
            }
            oqqwall_rust_core::draft::DraftBlock::Reply { preview } => {
                Some(format!("[回复] {}", preview.body.chars().take(80).collect::<String>()))
            }
            _ => None,
        })
    })
}

fn post_preview_images(snapshot: &StateView, post_id: Id128) -> Vec<String> {
    let draft_image_urls = snapshot
        .drafts
        .get(&post_id)
        .map(|draft| {
            draft
                .blocks
                .iter()
                .filter_map(|block| match block {
                    oqqwall_rust_core::draft::DraftBlock::Attachment {
                        reference,
                        kind: oqqwall_rust_core::draft::MediaKind::Image,
                        ..
                    } => match reference {
                        MediaReference::Blob { blob_id } => {
                            Some(format!("/api/blobs/{}", id_to_string(*blob_id)))
                        }
                        MediaReference::RemoteUrl { url } => Some(url.clone()),
                    },
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let render_blob_ids = snapshot
        .render
        .get(&post_id)
        .map(|render| {
            if render.png_blobs.is_empty() {
                render.png_blob.into_iter().collect::<Vec<_>>()
            } else {
                render.png_blobs.clone()
            }
        })
        .unwrap_or_default();

    let mut preview_image_urls = render_blob_ids
        .into_iter()
        .map(|blob_id| format!("/api/blobs/{}", id_to_string(blob_id)))
        .collect::<Vec<_>>();
    preview_image_urls.extend(draft_image_urls);
    preview_image_urls
}

fn build_post_timeline(snapshot: &StateView, meta: &PostMeta) -> Vec<PostTimelineItem> {
    let mut items = vec![PostTimelineItem {
        label: "稿件创建".to_string(),
        status: "done".to_string(),
        at_ms: Some(meta.created_at_ms),
        detail: Some(stage_to_string(meta.stage)),
    }];

    if let Some(render) = snapshot.render.get(&meta.post_id) {
        items.push(PostTimelineItem {
            label: "渲染阶段".to_string(),
            status: if render.last_error.is_some() {
                "error".to_string()
            } else if render.png_blob.is_some() || !render.png_blobs.is_empty() {
                "done".to_string()
            } else {
                "waiting".to_string()
            },
            at_ms: None,
            detail: render.last_error.clone(),
        });
    }

    if let Some(review_id) = meta.review_id {
        if let Some(review) = snapshot.reviews.get(&review_id) {
            items.push(PostTimelineItem {
                label: "审核发布".to_string(),
                status: if review.publish_last_error.is_some() {
                    "error".to_string()
                } else if review.audit_msg_id.is_some() {
                    "done".to_string()
                } else {
                    "waiting".to_string()
                },
                at_ms: None,
                detail: review.publish_last_error.clone(),
            });
            items.push(PostTimelineItem {
                label: "审核决策".to_string(),
                status: if review.decision.is_some() {
                    "done".to_string()
                } else {
                    "waiting".to_string()
                },
                at_ms: review.decided_at_ms,
                detail: review
                    .decision_reason
                    .clone()
                    .or_else(|| review.decided_by.clone()),
            });
        }
    }

    if meta.stage == PostStage::Sent {
        items.push(PostTimelineItem {
            label: "发送完成".to_string(),
            status: "done".to_string(),
            at_ms: snapshot.last_ts_ms,
            detail: None,
        });
    } else if meta.stage == PostStage::Failed || meta.last_error.is_some() {
        items.push(PostTimelineItem {
            label: "异常状态".to_string(),
            status: "error".to_string(),
            at_ms: snapshot.last_ts_ms,
            detail: meta.last_error.clone(),
        });
    }

    items
}

fn text_similarity(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let left_short = left.chars().take(24).collect::<String>();
    let right_short = right.chars().take(24).collect::<String>();
    left.contains(&right_short) || right.contains(&left_short)
}

fn append_audit_entry(state: &WebviewState, entry: WebviewAuditEntry) {
    if let Ok(mut guard) = state.admin.write() {
        guard.audit_entries.push(entry);
        if guard.audit_entries.len() > 500 {
            let overflow = guard.audit_entries.len() - 500;
            guard.audit_entries.drain(0..overflow);
        }
    }
}

fn parse_review_action(req: &ReviewDecisionRequest) -> Result<ReviewAction, &'static str> {
    match req.action.as_str() {
        "approve" => Ok(ReviewAction::Approve),
        "reject" => Ok(ReviewAction::Reject {
            reason: req.comment.clone(),
        }),
        "delete" => Ok(ReviewAction::Delete {
            reason: req.comment.clone(),
        }),
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

fn parse_id128(value: &str) -> Option<Id128> {
    value.parse::<u128>().ok().map(Id128)
}

fn detect_mime_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next().map(|ext| ext.to_ascii_lowercase()) {
        Some(ext) if ext == "png" => "image/png",
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ext) if ext == "gif" => "image/gif",
        Some(ext) if ext == "webp" => "image/webp",
        Some(ext) if ext == "mp4" => "video/mp4",
        Some(ext) if ext == "mp3" => "audio/mpeg",
        Some(ext) if ext == "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn role_to_string(role: &WebviewRole) -> &'static str {
    match role {
        WebviewRole::GlobalAdmin => "global_admin",
        WebviewRole::GroupAdmin => "group_admin",
    }
}

fn verify_password(password: &str, password_hash: &str) -> bool {
    if let Some(hex) = password_hash.strip_prefix("sha256:") {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        let digest = hasher.finalize();
        return format!("{:x}", digest) == hex.to_ascii_lowercase();
    }
    password == password_hash
}

fn find_asset(path: &str) -> Option<&'static EmbeddedWebAsset> {
    EMBEDDED_WEB_ASSETS.iter().find(|asset| asset.path == path)
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

fn is_active_stage(stage: PostStage) -> bool {
    !matches!(
        stage,
        PostStage::Rejected
            | PostStage::Deleted
            | PostStage::Withdrawn
            | PostStage::Skipped
            | PostStage::Failed
    )
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

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &str,
) -> axum::response::Response {
    (
        status,
        Json(ApiError {
            error: ApiErrorBody {
                code,
                message: message.to_string(),
            },
        }),
    )
        .into_response()
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

fn local_day_start_ms(ms: i64, tz_offset_minutes: i32) -> i64 {
    let offset_ms = i64::from(tz_offset_minutes).saturating_mul(60_000);
    let local = ms.saturating_add(offset_ms);
    local
        .saturating_sub(local.rem_euclid(86_400_000))
        .saturating_sub(offset_ms)
}

fn local_hour(ms: i64, tz_offset_minutes: i32) -> u8 {
    let offset_ms = i64::from(tz_offset_minutes).saturating_mul(60_000);
    let local = ms.saturating_add(offset_ms);
    ((local.div_euclid(3_600_000)).rem_euclid(24)) as u8
}

fn ms_to_date_string(ms: i64, tz_offset_minutes: i32) -> String {
    let offset_ms = i64::from(tz_offset_minutes).saturating_mul(60_000);
    let days = ms.saturating_add(offset_ms).div_euclid(86_400_000);
    civil_from_days(days)
}

fn civil_from_days(days_since_epoch: i64) -> String {
    // Howard Hinnant's civil calendar algorithm, adjusted for Unix epoch.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02}")
}

fn random_hex32() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
