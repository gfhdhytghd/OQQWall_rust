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
use oqqwall_rust_core::{
    Command, Id128, ReviewAction, ReviewActionBatchCommand, ReviewActionCommand, StateView,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::config::{
    AppConfig, WebviewAdminAccount, WebviewRole, load_config_root_for_edit,
    load_group_user_notification_settings, parse_agent_commands, resolve_config_path,
    save_config_root_for_edit, save_group_user_notification_settings,
};
use crate::engine::EngineHandle;
use oqqwall_rust_drivers::napcat::{
    AgentCommandBlock, AgentCommandConfig, AgentCommandTrigger, TagValueMappingGroup,
    UserNotificationSettings, UserNotificationTemplate, normalize_agent_command_config,
    update_group_agent_command_admins, update_group_agent_commands,
    update_group_user_notification_settings, validate_agent_command_config,
    validate_agent_command_name,
};

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
    group_ids: Vec<String>,
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
struct DailyTrendItem {
    date: String,
    submitted: usize,
    approved: usize,
    rejected: usize,
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

#[derive(Debug, Deserialize)]
struct UserNotificationSettingsQuery {
    #[serde(default)]
    group_id: Option<String>,
}

#[derive(Serialize)]
struct UserNotificationVariableInfo {
    key: &'static str,
    label: &'static str,
    description: &'static str,
    example: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserNotificationTemplatePayload {
    enabled: bool,
    include_post_tags: bool,
    #[serde(default)]
    text_template: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    images: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MappingEntryPayload {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TagValueMappingGroupPayload {
    #[serde(default)]
    tag: String,
    #[serde(default)]
    mappings: Vec<TagValueMappingEntryPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TagValueMappingEntryPayload {
    #[serde(default)]
    source: String,
    #[serde(default)]
    target: String,
}

#[derive(Serialize)]
struct UserNotificationSettingsResponse {
    group_id: String,
    available_groups: Vec<String>,
    queue_entered: UserNotificationTemplatePayload,
    review_queued: UserNotificationTemplatePayload,
    send_succeeded: UserNotificationTemplatePayload,
    rejected: UserNotificationTemplatePayload,
    webhook_tag_map: Vec<MappingEntryPayload>,
    tag_value_map: Vec<MappingEntryPayload>,
    tag_value_maps: Vec<TagValueMappingGroupPayload>,
    variables: Vec<UserNotificationVariableInfo>,
}

#[derive(Debug, Deserialize)]
struct UpdateUserNotificationSettingsRequest {
    group_id: String,
    queue_entered: UserNotificationTemplatePayload,
    review_queued: UserNotificationTemplatePayload,
    send_succeeded: UserNotificationTemplatePayload,
    rejected: UserNotificationTemplatePayload,
    #[serde(default)]
    webhook_tag_map: Vec<MappingEntryPayload>,
    #[serde(default)]
    tag_value_map: Vec<MappingEntryPayload>,
    #[serde(default)]
    tag_value_maps: Vec<TagValueMappingGroupPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigAdminPayload {
    #[serde(default)]
    id: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    password_set: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfigAgentCommandPayload {
    #[serde(default)]
    name: String,
    enabled: bool,
    #[serde(default)]
    admin_only: bool,
    #[serde(default)]
    trigger: AgentCommandTrigger,
    #[serde(default)]
    description: String,
    #[serde(default)]
    blocks: Vec<AgentCommandBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfigCommonPayload {
    web_api_enabled: bool,
    web_api_port: u16,
    #[serde(default)]
    web_api_root_token: String,
    webview_enabled: bool,
    #[serde(default)]
    webview_host: String,
    webview_port: u16,
    webview_session_ttl_sec: i64,
    telemetry_enabled: bool,
    #[serde(default)]
    telemetry_local_dir: String,
    telemetry_upload_enabled: bool,
    telemetry_upload_interval_sec: u64,
    telemetry_upload_batch_size: usize,
    telemetry_max_append_messages: usize,
    process_waittime_sec: u64,
    min_interval_ms: u64,
    max_image_number_one_post: u64,
    send_timeout_ms: u64,
    send_max_attempts: u32,
    tz_offset_minutes: i32,
    max_cache_mb: u64,
    #[serde(default)]
    napcat_base_url: String,
    #[serde(default)]
    napcat_access_token: String,
    at_unprived_sender: bool,
    friend_request_window_sec: u32,
    #[serde(default)]
    friend_add_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfigGroupPayload {
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    audit_group_id: String,
    #[serde(default)]
    accounts: Vec<String>,
    #[serde(default)]
    napcat_base_url: String,
    #[serde(default)]
    napcat_access_token: String,
    process_waittime_sec: u64,
    min_interval_ms: u64,
    max_post_stack: u64,
    max_image_number_one_post: u64,
    send_timeout_ms: u64,
    send_max_attempts: u32,
    #[serde(default)]
    send_schedule: Vec<String>,
    individual_image_in_posts: bool,
    #[serde(default)]
    watermark_text: String,
    #[serde(default)]
    friend_add_message: String,
    friend_request_window_sec: u32,
    #[serde(default)]
    quick_replies: Vec<MappingEntryPayload>,
    #[serde(default)]
    review_shortcuts: Vec<MappingEntryPayload>,
    #[serde(default)]
    global_shortcuts: Vec<MappingEntryPayload>,
    #[serde(default)]
    agent_commands: Vec<AppConfigAgentCommandPayload>,
    #[serde(default)]
    agent_command_admins: Vec<String>,
    #[serde(default)]
    webview_admins: Vec<ConfigAdminPayload>,
}

#[derive(Serialize)]
struct AppConfigSettingsResponse {
    config_path: String,
    common: AppConfigCommonPayload,
    global_admins: Vec<ConfigAdminPayload>,
    groups: Vec<AppConfigGroupPayload>,
    agent_command_variables: Vec<UserNotificationVariableInfo>,
}

#[derive(Debug, Deserialize)]
struct UpdateAppConfigSettingsRequest {
    common: AppConfigCommonPayload,
    #[serde(default)]
    global_admins: Vec<ConfigAdminPayload>,
    #[serde(default)]
    groups: Vec<AppConfigGroupPayload>,
}

#[derive(Serialize)]
struct ReviewFailure {
    review_id: String,
    reason: String,
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
        group_ids: config
            .groups
            .iter()
            .map(|group| group.group_id.clone())
            .collect(),
        tz_offset_minutes: config.tz_offset_minutes,
        session_ttl_sec: config.webview_session_ttl_sec,
    };

    let app = Router::new()
        .route("/auth/login", post(webview_login))
        .route("/auth/logout", post(webview_logout))
        .route("/auth/me", get(webview_me))
        .route("/api/stats", get(webview_get_stats))
        .route("/api/posts", get(webview_list_posts))
        .route("/api/posts/{post_id}", get(webview_get_post))
        .route("/api/blobs/{blob_id}", get(webview_get_blob))
        .route("/api/reviews/ids", get(webview_list_review_ids))
        .route(
            "/api/settings/config",
            get(webview_get_app_config_settings).post(webview_update_app_config_settings),
        )
        .route(
            "/api/settings/user-notifications",
            get(webview_get_user_notification_settings)
                .post(webview_update_user_notification_settings),
        )
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
    let mut daily_trend: HashMap<String, (usize, usize, usize)> = HashMap::new();
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
        let trend = daily_trend.entry(date).or_insert((0, 0, 0));
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
                        .or_insert((0, 0, 0));
                    match decision {
                        oqqwall_rust_core::event::ReviewDecision::Approved => trend.1 += 1,
                        oqqwall_rust_core::event::ReviewDecision::Rejected => trend.2 += 1,
                        _ => {}
                    }
                }
            }
        }
    }

    let mut daily_trend = daily_trend
        .into_iter()
        .map(|(date, (submitted, approved, rejected))| DailyTrendItem {
            date,
            submitted,
            approved,
            rejected,
        })
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

            let render_image_url = guard
                .render
                .get(&meta.post_id)
                .and_then(|r| r.png_blob)
                .map(|blob_id| format!("/api/blobs/{}", id_to_string(blob_id)));
            let mut preview_image_urls = Vec::new();
            if let Some(url) = render_image_url {
                preview_image_urls.push(url);
            }
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

async fn webview_get_app_config_settings(
    State(state): State<WebviewState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    if session.identity.role != WebviewRole::GlobalAdmin {
        return error_response(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "only global admin can edit config",
        );
    }
    let root = match load_config_root_for_edit() {
        Ok(root) => root,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("failed to load config: {}", err),
            );
        }
    };
    (
        StatusCode::OK,
        Json(build_app_config_settings_response(&root)),
    )
        .into_response()
}

async fn webview_update_app_config_settings(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Json(req): Json<UpdateAppConfigSettingsRequest>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    if session.identity.role != WebviewRole::GlobalAdmin {
        return error_response(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "only global admin can edit config",
        );
    }

    let current_root = match load_config_root_for_edit() {
        Ok(root) => root,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("failed to load config: {}", err),
            );
        }
    };
    let updated_root = match apply_app_config_settings_update(&current_root, req) {
        Ok(root) => root,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", &err),
    };
    let saved_root = match save_config_root_for_edit(&updated_root) {
        Ok(root) => root,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", &err),
    };
    refresh_runtime_agent_commands(&saved_root);
    (
        StatusCode::OK,
        Json(build_app_config_settings_response(&saved_root)),
    )
        .into_response()
}

fn refresh_runtime_agent_commands(root: &Value) {
    for (group_id, group_obj) in collect_config_group_objects(root) {
        let commands = match parse_agent_commands(group_obj.get("agent_commands")) {
            Ok(commands) => commands,
            Err(_err) => {
                debug_log!(
                    "skip runtime agent_commands refresh: group_id={} err={}",
                    group_id,
                    _err
                );
                continue;
            }
        };
        if let Err(_err) = update_group_agent_commands(&group_id, commands) {
            debug_log!(
                "runtime agent_commands refresh failed: group_id={} err={}",
                group_id,
                _err
            );
        }
        if let Err(_err) = update_group_agent_command_admins(
            &group_id,
            cfg_string_list(group_obj.get("agent_command_admins")),
        ) {
            debug_log!(
                "runtime agent_command_admins refresh failed: group_id={} err={}",
                group_id,
                _err
            );
        }
    }
}

async fn webview_get_user_notification_settings(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Query(query): Query<UserNotificationSettingsQuery>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let available_groups = accessible_group_ids(&state, &session.identity);
    let group_id = match resolve_settings_group(&available_groups, query.group_id.as_deref()) {
        Ok(group_id) => group_id,
        Err(resp) => return resp,
    };
    let config = match load_group_user_notification_settings(&group_id) {
        Ok(config) => config,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("failed to load user_notifications: {}", err),
            );
        }
    };
    (
        StatusCode::OK,
        Json(build_user_notification_settings_response(
            group_id,
            available_groups,
            config,
        )),
    )
        .into_response()
}

async fn webview_update_user_notification_settings(
    State(state): State<WebviewState>,
    headers: HeaderMap,
    Json(req): Json<UpdateUserNotificationSettingsRequest>,
) -> impl IntoResponse {
    let session = match authenticate_webview(&state, &headers) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    let available_groups = accessible_group_ids(&state, &session.identity);
    let group_id = req.group_id.trim().to_string();
    if group_id.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", "group_id required");
    }
    if !available_groups.iter().any(|item| item == &group_id) {
        return error_response(StatusCode::FORBIDDEN, "FORBIDDEN", "group not allowed");
    }

    let queue_entered = normalize_user_notification_template_payload(req.queue_entered);
    let review_queued = normalize_user_notification_template_payload(req.review_queued);
    let send_succeeded = normalize_user_notification_template_payload(req.send_succeeded);
    let rejected = normalize_user_notification_template_payload(req.rejected);
    for (stage_key, template) in [
        ("queue_entered", &queue_entered),
        ("review_queued", &review_queued),
        ("send_succeeded", &send_succeeded),
        ("rejected", &rejected),
    ] {
        if template.enabled
            && !template.include_post_tags
            && template.text_template.trim().is_empty()
            && template.tags.is_empty()
            && template.images.is_empty()
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!(
                    "{} must include text, tags, images, or post tags when enabled",
                    stage_key
                ),
            );
        }
    }

    let config = UserNotificationSettings {
        queue_entered: user_notification_template_from_payload(queue_entered),
        review_queued: user_notification_template_from_payload(review_queued),
        send_succeeded: user_notification_template_from_payload(send_succeeded),
        rejected: user_notification_template_from_payload(rejected),
        webhook_tag_map: normalize_mapping_entries(req.webhook_tag_map),
        tag_value_maps: normalize_tag_value_mapping_groups(if req.tag_value_maps.is_empty() {
            req.tag_value_map
                .into_iter()
                .map(|entry| TagValueMappingGroupPayload {
                    tag: entry.value.clone(),
                    mappings: vec![TagValueMappingEntryPayload {
                        source: entry.key,
                        target: entry.value,
                    }],
                })
                .collect()
        } else {
            req.tag_value_maps
        }),
    };
    if let Err(err) = save_group_user_notification_settings(&group_id, &config) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("failed to save user_notifications: {}", err),
        );
    }
    if let Err(err) = update_group_user_notification_settings(&group_id, config.clone()) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("failed to update runtime user_notifications: {}", err),
        );
    }
    (
        StatusCode::OK,
        Json(build_user_notification_settings_response(
            group_id,
            available_groups,
            config,
        )),
    )
        .into_response()
}

fn build_user_notification_settings_response(
    group_id: String,
    available_groups: Vec<String>,
    config: UserNotificationSettings,
) -> UserNotificationSettingsResponse {
    UserNotificationSettingsResponse {
        group_id,
        available_groups,
        queue_entered: payload_from_user_notification_template(&config.queue_entered),
        review_queued: payload_from_user_notification_template(&config.review_queued),
        send_succeeded: payload_from_user_notification_template(&config.send_succeeded),
        rejected: payload_from_user_notification_template(&config.rejected),
        webhook_tag_map: payload_entries_from_map(&config.webhook_tag_map),
        tag_value_map: payload_entries_from_tag_value_maps(&config.tag_value_maps),
        tag_value_maps: payload_groups_from_tag_value_maps(&config.tag_value_maps),
        variables: user_notification_variables(),
    }
}

fn build_app_config_settings_response(root: &Value) -> AppConfigSettingsResponse {
    let common_obj = root.get("common").and_then(|value| value.as_object());
    let web_api_obj =
        common_obj.and_then(|obj| obj.get("web_api").and_then(|value| value.as_object()));
    let webview_obj =
        common_obj.and_then(|obj| obj.get("webview").and_then(|value| value.as_object()));
    let telemetry_obj =
        common_obj.and_then(|obj| obj.get("telemetry").and_then(|value| value.as_object()));

    let common = AppConfigCommonPayload {
        web_api_enabled: cfg_bool(web_api_obj.and_then(|obj| obj.get("enabled"))).unwrap_or(false),
        web_api_port: cfg_u16(web_api_obj.and_then(|obj| obj.get("port"))).unwrap_or(10923),
        web_api_root_token: cfg_string(web_api_obj.and_then(|obj| obj.get("root_token")))
            .unwrap_or_default(),
        webview_enabled: cfg_bool(webview_obj.and_then(|obj| obj.get("enabled"))).unwrap_or(false),
        webview_host: cfg_string(webview_obj.and_then(|obj| obj.get("host")))
            .unwrap_or_else(|| "0.0.0.0".to_string()),
        webview_port: cfg_u16(webview_obj.and_then(|obj| obj.get("port"))).unwrap_or(10924),
        webview_session_ttl_sec: cfg_i64(webview_obj.and_then(|obj| obj.get("session_ttl_sec")))
            .unwrap_or(12 * 60 * 60),
        telemetry_enabled: cfg_bool(telemetry_obj.and_then(|obj| obj.get("enabled")))
            .unwrap_or(true),
        telemetry_local_dir: cfg_string(telemetry_obj.and_then(|obj| obj.get("local_dir")))
            .unwrap_or_else(|| "telemetry".to_string()),
        telemetry_upload_enabled: cfg_bool(telemetry_obj.and_then(|obj| obj.get("upload_enabled")))
            .unwrap_or(true),
        telemetry_upload_interval_sec: cfg_u64(
            telemetry_obj.and_then(|obj| obj.get("upload_interval_sec")),
        )
        .unwrap_or(30),
        telemetry_upload_batch_size: cfg_usize(
            telemetry_obj.and_then(|obj| obj.get("upload_batch_size")),
        )
        .unwrap_or(20),
        telemetry_max_append_messages: cfg_usize(
            telemetry_obj.and_then(|obj| obj.get("max_append_messages")),
        )
        .unwrap_or(2),
        process_waittime_sec: cfg_u64(common_obj.and_then(|obj| obj.get("process_waittime_sec")))
            .unwrap_or(20),
        min_interval_ms: cfg_u64(common_obj.and_then(|obj| obj.get("min_interval_ms")))
            .unwrap_or(0),
        max_image_number_one_post: cfg_u64(
            common_obj.and_then(|obj| obj.get("max_image_number_one_post")),
        )
        .unwrap_or(30),
        send_timeout_ms: cfg_u64(common_obj.and_then(|obj| obj.get("send_timeout_ms")))
            .unwrap_or(300_000),
        send_max_attempts: cfg_u32(common_obj.and_then(|obj| obj.get("send_max_attempts")))
            .unwrap_or(3),
        tz_offset_minutes: cfg_i64(common_obj.and_then(|obj| obj.get("tz_offset_minutes")))
            .unwrap_or(0) as i32,
        max_cache_mb: cfg_u64(common_obj.and_then(|obj| obj.get("max_cache_mb"))).unwrap_or(256),
        napcat_base_url: cfg_string(common_obj.and_then(|obj| obj.get("napcat_base_url")))
            .unwrap_or_default(),
        napcat_access_token: cfg_string(common_obj.and_then(|obj| obj.get("napcat_access_token")))
            .unwrap_or_default(),
        at_unprived_sender: cfg_bool(common_obj.and_then(|obj| obj.get("at_unprived_sender")))
            .unwrap_or(false),
        friend_request_window_sec: cfg_u32(
            common_obj.and_then(|obj| obj.get("friend_request_window_sec")),
        )
        .unwrap_or(300),
        friend_add_message: cfg_string(common_obj.and_then(|obj| obj.get("friend_add_message")))
            .unwrap_or_default(),
    };

    let groups = collect_config_group_objects(root)
        .into_iter()
        .map(|(group_id, group_obj)| AppConfigGroupPayload {
            group_id: group_id.clone(),
            audit_group_id: cfg_string(group_obj.get("mangroupid")).unwrap_or_default(),
            accounts: cfg_string_list(group_obj.get("accounts")),
            napcat_base_url: cfg_string(group_obj.get("napcat_base_url"))
                .unwrap_or_else(|| common.napcat_base_url.clone()),
            napcat_access_token: cfg_string(group_obj.get("napcat_access_token"))
                .unwrap_or_else(|| common.napcat_access_token.clone()),
            process_waittime_sec: cfg_u64(group_obj.get("process_waittime_sec"))
                .unwrap_or(common.process_waittime_sec),
            min_interval_ms: cfg_u64(group_obj.get("min_interval_ms"))
                .unwrap_or(common.min_interval_ms),
            max_post_stack: cfg_u64(group_obj.get("max_post_stack")).unwrap_or(1),
            max_image_number_one_post: cfg_u64(group_obj.get("max_image_number_one_post"))
                .unwrap_or(common.max_image_number_one_post),
            send_timeout_ms: cfg_u64(group_obj.get("send_timeout_ms"))
                .unwrap_or(common.send_timeout_ms),
            send_max_attempts: cfg_u32(group_obj.get("send_max_attempts"))
                .unwrap_or(common.send_max_attempts),
            send_schedule: cfg_schedule_list(group_obj.get("send_schedule")),
            individual_image_in_posts: cfg_bool(group_obj.get("individual_image_in_posts"))
                .unwrap_or(true),
            watermark_text: cfg_string(group_obj.get("watermark_text")).unwrap_or_default(),
            friend_add_message: cfg_string(group_obj.get("friend_add_message"))
                .unwrap_or_else(|| common.friend_add_message.clone()),
            friend_request_window_sec: cfg_u32(group_obj.get("friend_request_window_sec"))
                .unwrap_or(common.friend_request_window_sec),
            quick_replies: payload_entries_from_map(&cfg_string_map(
                group_obj.get("quick_replies"),
            )),
            review_shortcuts: payload_entries_from_map(&cfg_string_map(
                group_obj.get("review_shortcuts"),
            )),
            global_shortcuts: payload_entries_from_map(&cfg_string_map(
                group_obj.get("global_shortcuts"),
            )),
            agent_commands: payload_agent_commands(group_obj.get("agent_commands")),
            agent_command_admins: cfg_string_list(group_obj.get("agent_command_admins")),
            webview_admins: build_config_admin_payloads(
                group_obj.get("webview_admins"),
                &format!("group:{}", group_id),
            ),
        })
        .collect();

    AppConfigSettingsResponse {
        config_path: resolve_config_path(),
        common,
        global_admins: build_config_admin_payloads(
            root.as_object()
                .and_then(|obj| obj.get("webview_global_admins")),
            "root",
        ),
        groups,
        agent_command_variables: agent_command_variables(),
    }
}

fn apply_app_config_settings_update(
    current_root: &Value,
    req: UpdateAppConfigSettingsRequest,
) -> Result<Value, String> {
    if req.groups.is_empty() {
        return Err("至少保留一个分组配置。".to_string());
    }
    if req.common.webview_enabled
        && !req
            .global_admins
            .iter()
            .chain(
                req.groups
                    .iter()
                    .flat_map(|group| group.webview_admins.iter()),
            )
            .any(|admin| {
                !admin.username.trim().is_empty()
                    || !admin.password.trim().is_empty()
                    || admin.password_set
            })
    {
        return Err("启用 Web 审核面板前，至少配置一个 WebUI 管理员。".to_string());
    }

    let mut seen_group_ids = HashSet::new();
    for group in &req.groups {
        let group_id = group.group_id.trim();
        if group_id.is_empty() {
            return Err("分组标识不能为空。".to_string());
        }
        if !seen_group_ids.insert(group_id.to_string()) {
            return Err(format!("分组 '{}' 重复。", group_id));
        }
    }

    let existing_group_objects = collect_config_group_objects(current_root)
        .into_iter()
        .collect::<HashMap<_, _>>();
    let inline_group_ids = collect_inline_group_ids(current_root);
    let existing_global_admin_passwords = collect_config_admin_passwords(
        current_root
            .as_object()
            .and_then(|obj| obj.get("webview_global_admins")),
        "root",
    );

    let mut root = current_root.clone();
    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| "config root must be a json object".to_string())?;

    {
        let common_obj = ensure_object_field(root_obj, "common");
        apply_common_config_payload(common_obj, &req.common);
    }

    write_config_admin_list(
        root_obj,
        "webview_global_admins",
        &req.global_admins,
        &existing_global_admin_passwords,
        "全局 WebUI 管理员",
    )?;

    let mut groups_obj = Map::new();
    for group in req.groups {
        let group_id = group.group_id.trim().to_string();
        let existing_passwords = existing_group_objects
            .get(&group_id)
            .map(|group_obj| {
                collect_config_admin_passwords(
                    group_obj.get("webview_admins"),
                    &format!("group:{}", group_id),
                )
            })
            .unwrap_or_default();
        let mut group_obj = existing_group_objects
            .get(&group_id)
            .cloned()
            .unwrap_or_default();
        apply_group_config_payload(&mut group_obj, &group, &existing_passwords)?;
        groups_obj.insert(group_id, Value::Object(group_obj));
    }
    root_obj.insert("groups".to_string(), Value::Object(groups_obj));
    for key in inline_group_ids {
        root_obj.remove(&key);
    }
    root_obj.insert("schema_version".to_string(), Value::from(1_u64));

    Ok(root)
}

fn apply_common_config_payload(obj: &mut Map<String, Value>, payload: &AppConfigCommonPayload) {
    set_u64_field(obj, "process_waittime_sec", payload.process_waittime_sec);
    set_u64_field(obj, "min_interval_ms", payload.min_interval_ms);
    set_u64_field(
        obj,
        "max_image_number_one_post",
        payload.max_image_number_one_post,
    );
    set_u64_field(obj, "send_timeout_ms", payload.send_timeout_ms);
    set_u32_field(obj, "send_max_attempts", payload.send_max_attempts);
    set_i64_field(
        obj,
        "tz_offset_minutes",
        i64::from(payload.tz_offset_minutes),
    );
    set_u64_field(obj, "max_cache_mb", payload.max_cache_mb);
    set_string_field(obj, "napcat_base_url", &payload.napcat_base_url);
    set_string_field(obj, "napcat_access_token", &payload.napcat_access_token);
    set_bool_field(obj, "at_unprived_sender", payload.at_unprived_sender);
    set_u32_field(
        obj,
        "friend_request_window_sec",
        payload.friend_request_window_sec,
    );
    set_string_field(obj, "friend_add_message", &payload.friend_add_message);

    {
        let web_api = ensure_object_field(obj, "web_api");
        set_bool_field(web_api, "enabled", payload.web_api_enabled);
        set_u16_field(web_api, "port", payload.web_api_port);
        set_string_field(web_api, "root_token", &payload.web_api_root_token);
    }
    {
        let webview = ensure_object_field(obj, "webview");
        set_bool_field(webview, "enabled", payload.webview_enabled);
        set_string_field(webview, "host", &payload.webview_host);
        set_u16_field(webview, "port", payload.webview_port);
        set_i64_field(webview, "session_ttl_sec", payload.webview_session_ttl_sec);
    }
    {
        let telemetry = ensure_object_field(obj, "telemetry");
        set_bool_field(telemetry, "enabled", payload.telemetry_enabled);
        set_string_field(telemetry, "local_dir", &payload.telemetry_local_dir);
        set_bool_field(
            telemetry,
            "upload_enabled",
            payload.telemetry_upload_enabled,
        );
        set_u64_field(
            telemetry,
            "upload_interval_sec",
            payload.telemetry_upload_interval_sec,
        );
        set_usize_field(
            telemetry,
            "upload_batch_size",
            payload.telemetry_upload_batch_size,
        );
        set_usize_field(
            telemetry,
            "max_append_messages",
            payload.telemetry_max_append_messages,
        );
    }
}

fn apply_group_config_payload(
    obj: &mut Map<String, Value>,
    payload: &AppConfigGroupPayload,
    existing_admin_passwords: &HashMap<String, String>,
) -> Result<(), String> {
    let group_id = payload.group_id.trim();
    if group_id.is_empty() {
        return Err("分组标识不能为空。".to_string());
    }
    let audit_group_id = payload.audit_group_id.trim();
    if audit_group_id.is_empty() {
        return Err(format!("分组 '{}' 的审核群号不能为空。", group_id));
    }
    let accounts = normalize_string_values(&payload.accounts);
    if accounts.is_empty() {
        return Err(format!("分组 '{}' 至少需要一个发送账号。", group_id));
    }

    set_string_field(obj, "mangroupid", audit_group_id);
    set_string_list_field(obj, "accounts", &accounts);
    set_string_field(obj, "napcat_base_url", &payload.napcat_base_url);
    set_string_field(obj, "napcat_access_token", &payload.napcat_access_token);
    set_u64_field(obj, "process_waittime_sec", payload.process_waittime_sec);
    set_u64_field(obj, "min_interval_ms", payload.min_interval_ms);
    set_u64_field(obj, "max_post_stack", payload.max_post_stack);
    set_u64_field(
        obj,
        "max_image_number_one_post",
        payload.max_image_number_one_post,
    );
    set_u64_field(obj, "send_timeout_ms", payload.send_timeout_ms);
    set_u32_field(obj, "send_max_attempts", payload.send_max_attempts);
    set_string_list_field(
        obj,
        "send_schedule",
        &normalize_string_values(&payload.send_schedule),
    );
    set_bool_field(
        obj,
        "individual_image_in_posts",
        payload.individual_image_in_posts,
    );
    set_string_field(obj, "watermark_text", &payload.watermark_text);
    set_string_field(obj, "friend_add_message", &payload.friend_add_message);
    set_u32_field(
        obj,
        "friend_request_window_sec",
        payload.friend_request_window_sec,
    );
    set_mapping_entries_field(obj, "quick_replies", &payload.quick_replies);
    set_mapping_entries_field(obj, "review_shortcuts", &payload.review_shortcuts);
    set_mapping_entries_field(obj, "global_shortcuts", &payload.global_shortcuts);
    set_agent_commands_field(obj, "agent_commands", &payload.agent_commands)?;
    set_string_list_field(
        obj,
        "agent_command_admins",
        &normalize_string_values(&payload.agent_command_admins),
    );
    write_config_admin_list(
        obj,
        "webview_admins",
        &payload.webview_admins,
        existing_admin_passwords,
        &format!("分组 '{}' 的 WebUI 管理员", group_id),
    )?;
    Ok(())
}

fn build_config_admin_payloads(value: Option<&Value>, scope: &str) -> Vec<ConfigAdminPayload> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let obj = item.as_object()?;
            let username = cfg_string(obj.get("username"))?;
            if username.trim().is_empty() {
                return None;
            }
            let password = cfg_string(obj.get("password")).unwrap_or_default();
            Some(ConfigAdminPayload {
                id: format!("{}:{}", scope, index),
                username,
                password: String::new(),
                password_set: !password.trim().is_empty(),
            })
        })
        .collect()
}

fn collect_config_admin_passwords(value: Option<&Value>, scope: &str) -> HashMap<String, String> {
    let Some(Value::Array(items)) = value else {
        return HashMap::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let obj = item.as_object()?;
            let username = cfg_string(obj.get("username"))?;
            let password = cfg_string(obj.get("password"))?;
            if username.trim().is_empty() || password.trim().is_empty() {
                return None;
            }
            Some((format!("{}:{}", scope, index), password))
        })
        .collect()
}

fn write_config_admin_list(
    obj: &mut Map<String, Value>,
    key: &str,
    entries: &[ConfigAdminPayload],
    existing_passwords: &HashMap<String, String>,
    label: &str,
) -> Result<(), String> {
    let mut items = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let username = entry.username.trim();
        let password_input = entry.password.trim();
        if username.is_empty() && password_input.is_empty() {
            continue;
        }
        if username.is_empty() {
            return Err(format!("{}第 {} 项缺少用户名。", label, index + 1));
        }
        let password = if !password_input.is_empty() {
            password_input.to_string()
        } else if let Some(existing) = existing_passwords.get(entry.id.trim()) {
            existing.clone()
        } else {
            return Err(format!("{}第 {} 项未填写密码。", label, index + 1));
        };
        items.push(serde_json::json!({
            "username": username,
            "password": password,
        }));
    }
    if items.is_empty() {
        obj.remove(key);
    } else {
        obj.insert(key.to_string(), Value::Array(items));
    }
    Ok(())
}

fn set_mapping_entries_field(
    obj: &mut Map<String, Value>,
    key: &str,
    entries: &[MappingEntryPayload],
) {
    let map = normalize_mapping_entries(entries.to_vec());
    if map.is_empty() {
        obj.remove(key);
    } else {
        obj.insert(key.to_string(), config_string_map_to_value(&map));
    }
}

fn set_agent_commands_field(
    obj: &mut Map<String, Value>,
    key: &str,
    commands: &[AppConfigAgentCommandPayload],
) -> Result<(), String> {
    let normalized = normalize_agent_command_payloads(commands)?;
    if normalized.is_empty() {
        obj.remove(key);
        return Ok(());
    }
    let mut entries = normalized
        .into_iter()
        .map(|entry| {
            let config = AgentCommandConfig {
                enabled: entry.enabled,
                admin_only: entry.admin_only,
                trigger: entry.trigger,
                description: entry.description,
                blocks: entry.blocks,
            };
            let value = serde_json::to_value(config).map_err(|err| {
                format!(
                    "failed to serialize agent command '{}': {}",
                    entry.name, err
                )
            })?;
            Ok((entry.name, value))
        })
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    let map = entries.into_iter().collect::<Map<String, Value>>();
    obj.insert(key.to_string(), Value::Object(map));
    Ok(())
}

fn ensure_object_field<'a>(
    obj: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !obj.get(key).map(|value| value.is_object()).unwrap_or(false) {
        obj.insert(key.to_string(), Value::Object(Map::new()));
    }
    obj.get_mut(key)
        .and_then(|value| value.as_object_mut())
        .expect("object field must exist")
}

fn set_string_field(obj: &mut Map<String, Value>, key: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        obj.remove(key);
    } else {
        obj.insert(key.to_string(), Value::String(trimmed.to_string()));
    }
}

fn set_bool_field(obj: &mut Map<String, Value>, key: &str, value: bool) {
    obj.insert(key.to_string(), Value::Bool(value));
}

fn set_u16_field(obj: &mut Map<String, Value>, key: &str, value: u16) {
    obj.insert(key.to_string(), Value::from(value));
}

fn set_u32_field(obj: &mut Map<String, Value>, key: &str, value: u32) {
    obj.insert(key.to_string(), Value::from(value));
}

fn set_u64_field(obj: &mut Map<String, Value>, key: &str, value: u64) {
    obj.insert(key.to_string(), Value::from(value));
}

fn set_usize_field(obj: &mut Map<String, Value>, key: &str, value: usize) {
    obj.insert(key.to_string(), Value::from(value as u64));
}

fn set_i64_field(obj: &mut Map<String, Value>, key: &str, value: i64) {
    obj.insert(key.to_string(), Value::from(value));
}

fn set_string_list_field(obj: &mut Map<String, Value>, key: &str, values: &[String]) {
    if values.is_empty() {
        obj.remove(key);
    } else {
        obj.insert(
            key.to_string(),
            Value::Array(values.iter().cloned().map(Value::String).collect()),
        );
    }
}

fn normalize_string_values(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_string();
        if !out.iter().any(|existing| existing == &normalized) {
            out.push(normalized);
        }
    }
    out
}

fn collect_config_group_objects(root: &Value) -> Vec<(String, Map<String, Value>)> {
    let Some(obj) = root.as_object() else {
        return Vec::new();
    };
    let mut groups = if let Some(nested) = obj.get("groups").and_then(|value| value.as_object()) {
        nested
            .iter()
            .filter_map(|(group_id, value)| {
                value
                    .as_object()
                    .map(|group| (group_id.clone(), group.clone()))
            })
            .collect::<Vec<_>>()
    } else {
        obj.iter()
            .filter_map(|(group_id, value)| {
                if group_id == "common"
                    || group_id == "schema_version"
                    || group_id == "webview_global_admins"
                {
                    return None;
                }
                value
                    .as_object()
                    .map(|group| (group_id.clone(), group.clone()))
            })
            .collect::<Vec<_>>()
    };
    groups.sort_by(|(left, _), (right, _)| left.cmp(right));
    groups
}

fn collect_inline_group_ids(root: &Value) -> Vec<String> {
    let Some(obj) = root.as_object() else {
        return Vec::new();
    };
    if obj
        .get("groups")
        .and_then(|value| value.as_object())
        .is_some()
    {
        return Vec::new();
    }
    obj.iter()
        .filter_map(|(key, value)| {
            if key == "common" || key == "schema_version" || key == "webview_global_admins" {
                return None;
            }
            value.as_object().map(|_| key.clone())
        })
        .collect()
}

fn cfg_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn cfg_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(value) => Some(value.as_i64().unwrap_or(0) != 0),
        _ => None,
    }
}

fn cfg_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn cfg_u16(value: Option<&Value>) -> Option<u16> {
    cfg_i64(value).and_then(|value| u16::try_from(value).ok())
}

fn cfg_u32(value: Option<&Value>) -> Option<u32> {
    cfg_i64(value).and_then(|value| u32::try_from(value).ok())
}

fn cfg_u64(value: Option<&Value>) -> Option<u64> {
    cfg_i64(value).and_then(|value| u64::try_from(value).ok())
}

fn cfg_usize(value: Option<&Value>) -> Option<usize> {
    cfg_i64(value).and_then(|value| usize::try_from(value).ok())
}

fn cfg_string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| cfg_string(Some(item)))
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        Some(Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect(),
        Some(Value::Number(value)) => vec![value.to_string()],
        _ => Vec::new(),
    }
}

fn cfg_schedule_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| cfg_string(Some(item)))
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        Some(Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn cfg_string_map(value: Option<&Value>) -> HashMap<String, String> {
    let Some(Value::Object(obj)) = value else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (key, value) in obj {
        let Some(value) = cfg_string(Some(value)) else {
            continue;
        };
        let normalized_key = key.trim();
        let normalized_value = value.trim();
        if normalized_key.is_empty() || normalized_value.is_empty() {
            continue;
        }
        out.insert(normalized_key.to_string(), normalized_value.to_string());
    }
    out
}

fn payload_agent_commands(value: Option<&Value>) -> Vec<AppConfigAgentCommandPayload> {
    let Some(Value::Object(obj)) = value else {
        return Vec::new();
    };
    let mut out = obj
        .iter()
        .filter_map(|(name, raw)| {
            let parsed = serde_json::from_value::<AgentCommandConfig>(raw.clone()).ok()?;
            let normalized = normalize_agent_command_config(&parsed);
            Some(AppConfigAgentCommandPayload {
                name: name.trim().to_string(),
                enabled: normalized.enabled,
                admin_only: normalized.admin_only,
                trigger: normalized.trigger,
                description: normalized.description,
                blocks: normalized.blocks,
            })
        })
        .collect::<Vec<_>>();
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

fn config_string_map_to_value(map: &HashMap<String, String>) -> Value {
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_by(|(left_key, _), (right_key, _)| left_key.cmp(right_key));
    let obj = entries
        .into_iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect::<Map<String, Value>>();
    Value::Object(obj)
}

fn payload_from_user_notification_template(
    template: &UserNotificationTemplate,
) -> UserNotificationTemplatePayload {
    UserNotificationTemplatePayload {
        enabled: template.enabled,
        include_post_tags: template.include_post_tags,
        text_template: template.text_template.clone(),
        tags: template.tags.clone(),
        images: template.images.clone(),
    }
}

fn user_notification_template_from_payload(
    payload: UserNotificationTemplatePayload,
) -> UserNotificationTemplate {
    UserNotificationTemplate {
        enabled: payload.enabled,
        include_post_tags: payload.include_post_tags,
        text_template: payload.text_template,
        tags: payload.tags,
        images: payload.images,
    }
}

fn normalize_user_notification_template_payload(
    payload: UserNotificationTemplatePayload,
) -> UserNotificationTemplatePayload {
    UserNotificationTemplatePayload {
        enabled: payload.enabled,
        include_post_tags: payload.include_post_tags,
        text_template: payload.text_template.replace("\r\n", "\n"),
        tags: payload
            .tags
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .collect(),
        images: payload
            .images
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .collect(),
    }
}

fn normalize_mapping_entries(entries: Vec<MappingEntryPayload>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for entry in entries {
        let key = entry.key.trim();
        let value = entry.value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        out.insert(key.to_string(), value.to_string());
    }
    out
}

fn normalize_tag_value_mapping_groups(
    groups: Vec<TagValueMappingGroupPayload>,
) -> Vec<TagValueMappingGroup> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for group in groups {
        let tag = group.tag.trim().to_string();
        if tag.is_empty() {
            continue;
        }
        let mappings = group
            .mappings
            .into_iter()
            .filter_map(|mapping| {
                let source = mapping.source.trim().to_string();
                let target = mapping.target.trim().to_string();
                if source.is_empty() || target.is_empty() {
                    None
                } else {
                    Some(oqqwall_rust_drivers::napcat::TagValueMappingEntry { source, target })
                }
            })
            .collect::<Vec<_>>();
        if mappings.is_empty() {
            continue;
        }
        let dedup_key = tag.to_ascii_lowercase();
        if !seen.insert(dedup_key) {
            continue;
        }
        out.push(TagValueMappingGroup {
            tag,
            mappings,
            sources: Vec::new(),
        });
    }
    out
}

fn normalize_agent_command_payloads(
    commands: &[AppConfigAgentCommandPayload],
) -> Result<Vec<AppConfigAgentCommandPayload>, String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for entry in commands {
        let name = entry.name.trim().trim_start_matches('#').trim().to_string();
        let has_content =
            !name.is_empty() || !entry.description.trim().is_empty() || !entry.blocks.is_empty();
        if !has_content {
            continue;
        }
        let normalized_name = validate_agent_command_name(&name)?;
        if !seen.insert(normalized_name.clone()) {
            return Err(format!("agent 指令重复：{}", normalized_name));
        }
        let config = normalize_agent_command_config(&AgentCommandConfig {
            enabled: entry.enabled,
            admin_only: entry.admin_only,
            trigger: entry.trigger,
            description: entry.description.clone(),
            blocks: entry.blocks.clone(),
        });
        validate_agent_command_config(&normalized_name, &config)?;
        normalized.push(AppConfigAgentCommandPayload {
            name: normalized_name,
            enabled: config.enabled,
            admin_only: config.admin_only,
            trigger: config.trigger,
            description: config.description,
            blocks: config.blocks,
        });
    }
    Ok(normalized)
}

fn payload_entries_from_map(map: &HashMap<String, String>) -> Vec<MappingEntryPayload> {
    let mut entries = map
        .iter()
        .map(|(key, value)| MappingEntryPayload {
            key: key.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries
}

fn payload_groups_from_tag_value_maps(
    groups: &[oqqwall_rust_drivers::napcat::TagValueMappingGroup],
) -> Vec<TagValueMappingGroupPayload> {
    let mut out = groups
        .iter()
        .map(|group| TagValueMappingGroupPayload {
            tag: group.tag.clone(),
            mappings: group
                .mappings
                .iter()
                .map(|entry| TagValueMappingEntryPayload {
                    source: entry.source.clone(),
                    target: entry.target.clone(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    out.sort_by(|left, right| left.tag.cmp(&right.tag));
    out
}

fn payload_entries_from_tag_value_maps(
    groups: &[oqqwall_rust_drivers::napcat::TagValueMappingGroup],
) -> Vec<MappingEntryPayload> {
    let mut entries = Vec::new();
    for group in groups {
        for mapping in &group.mappings {
            entries.push(MappingEntryPayload {
                key: mapping.source.clone(),
                value: mapping.target.clone(),
            });
        }
    }
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries
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
    let actions = match parse_review_actions(&req) {
        Ok(actions) => actions,
        Err(reason) => return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", reason),
    };
    if !can_access_review(&state, &session.identity, review_id) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "permission denied",
        );
    }
    let cmd = build_review_command(
        review_id,
        actions,
        format!("webview:{}", session.identity.username),
        now_ms(),
        state.tz_offset_minutes,
    );
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
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
    let actions = match parse_review_actions(&action_req) {
        Ok(actions) => actions,
        Err(reason) => return error_response(StatusCode::BAD_REQUEST, "BAD_REQUEST", reason),
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
        if !can_access_review(&state, &session.identity, review_id) {
            failed.push(ReviewFailure {
                review_id: id_to_string(review_id),
                reason: "permission denied".to_string(),
            });
            continue;
        }
        let cmd = build_review_command(
            review_id,
            actions.clone(),
            format!("webview:{}", session.identity.username),
            now_ms(),
            state.tz_offset_minutes,
        );
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
    let asset = find_asset(&req_path).or_else(|| find_asset("/index.html"));
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

fn accessible_group_ids(state: &WebviewState, identity: &WebviewIdentity) -> Vec<String> {
    let mut groups = if identity.role == WebviewRole::GlobalAdmin {
        state.group_ids.clone()
    } else {
        identity.groups.clone()
    };
    groups.sort();
    groups.dedup();
    groups
}

fn resolve_settings_group(
    available_groups: &[String],
    requested_group_id: Option<&str>,
) -> Result<String, axum::response::Response> {
    if available_groups.is_empty() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "no groups available",
        ));
    }
    if let Some(group_id) = requested_group_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if available_groups.iter().any(|value| value == group_id) {
            return Ok(group_id.to_string());
        }
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "group not allowed",
        ));
    }
    Ok(available_groups[0].clone())
}

fn user_notification_variables() -> Vec<UserNotificationVariableInfo> {
    vec![
        UserNotificationVariableInfo {
            key: "stage",
            label: "通知阶段",
            description: "当前回链所在的阶段标识。",
            example: "<stage>",
        },
        UserNotificationVariableInfo {
            key: "code",
            label: "显示编号",
            description: "优先使用外部编号，缺失时回退到内部编号。",
            example: "<code>",
        },
        UserNotificationVariableInfo {
            key: "external_code",
            label: "外部编号",
            description: "对外展示的稿件编号。",
            example: "<external_code>",
        },
        UserNotificationVariableInfo {
            key: "internal_code",
            label: "内部编号",
            description: "系统内部使用的审核编号。",
            example: "<internal_code>",
        },
        UserNotificationVariableInfo {
            key: "post_id",
            label: "稿件 ID",
            description: "Rust 内部的 post_id 值。",
            example: "<post_id>",
        },
        UserNotificationVariableInfo {
            key: "review_id",
            label: "审核 ID",
            description: "Rust 内部的 review_id 值。",
            example: "<review_id>",
        },
        UserNotificationVariableInfo {
            key: "group_id",
            label: "分组标识",
            description: "稿件所属的 group_id。",
            example: "<group_id>",
        },
        UserNotificationVariableInfo {
            key: "sender_id",
            label: "投稿人 QQ",
            description: "系统识别到的投稿人 user_id。",
            example: "<sender_id>",
        },
        UserNotificationVariableInfo {
            key: "account_id",
            label: "发送账号",
            description: "实际执行发稿的账号 QQ。",
            example: "<account_id>",
        },
        UserNotificationVariableInfo {
            key: "send_time",
            label: "发送时间",
            description: "发稿成功时按当前时区格式化后的时间。",
            example: "<send_time>",
        },
        UserNotificationVariableInfo {
            key: "send_timestamp_ms",
            label: "发送时间戳",
            description: "发稿成功时的毫秒时间戳。",
            example: "<send_timestamp_ms>",
        },
        UserNotificationVariableInfo {
            key: "reviewer",
            label: "审核人",
            description: "最后一次处理该稿件的操作人，通常来自 WebUI 用户名。",
            example: "<reviewer>",
        },
        UserNotificationVariableInfo {
            key: "reviewed_at",
            label: "审核时间",
            description: "最后一次审核决策发生的格式化时间。",
            example: "<reviewed_at>",
        },
        UserNotificationVariableInfo {
            key: "queue_time",
            label: "入队时间",
            description: "稿件进入发送队列时的格式化时间。",
            example: "<queue_time>",
        },
        UserNotificationVariableInfo {
            key: "queue_timestamp_ms",
            label: "入队时间戳",
            description: "稿件进入发送队列时的毫秒时间戳。",
            example: "<queue_timestamp_ms>",
        },
        UserNotificationVariableInfo {
            key: "scheduled_for",
            label: "计划发送时间",
            description: "按当前时区格式化后的计划发送时间。",
            example: "<scheduled_for>",
        },
        UserNotificationVariableInfo {
            key: "scheduled_timestamp_ms",
            label: "计划发送时间戳",
            description: "计划发送时刻的毫秒时间戳。",
            example: "<scheduled_timestamp_ms>",
        },
        UserNotificationVariableInfo {
            key: "source_webhook",
            label: "来源 Webhook",
            description: "从 Web API / webhook 入口记录到的 webhook 标识。",
            example: "<source_webhook>",
        },
        UserNotificationVariableInfo {
            key: "source_webhook_tag",
            label: "Webhook 标签",
            description: "根据 webhook 映射表得到的标签值。",
            example: "<source_webhook_tag>",
        },
        UserNotificationVariableInfo {
            key: "raw_tag_list",
            label: "原始标签列表",
            description: "映射前收到的原始标签列表。",
            example: "<raw_tag_list>",
        },
        UserNotificationVariableInfo {
            key: "tag_list",
            label: "实际标签列表",
            description: "经过映射后的标签列表，使用逗号拼接。",
            example: "<tag_list>",
        },
        UserNotificationVariableInfo {
            key: "tag_count",
            label: "标签数量",
            description: "稿件当前实际标签的数量。",
            example: "<tag_count>",
        },
    ]
}

fn agent_command_variables() -> Vec<UserNotificationVariableInfo> {
    vec![
        UserNotificationVariableInfo {
            key: "command_name",
            label: "指令名",
            description: "当前命中的 agent 指令名，不带 # 前缀。",
            example: "<command_name>",
        },
        UserNotificationVariableInfo {
            key: "command_args",
            label: "指令参数",
            description: "用户在 #指令 后面继续输入的参数文本。",
            example: "<command_args>",
        },
        UserNotificationVariableInfo {
            key: "command_text",
            label: "完整指令",
            description: "用户发送的完整命令文本，通常形如 #帮助 参数。",
            example: "<command_text>",
        },
        UserNotificationVariableInfo {
            key: "raw_message",
            label: "原始消息",
            description: "NapCat 上报的 raw_message 原文。",
            example: "<raw_message>",
        },
        UserNotificationVariableInfo {
            key: "message_text",
            label: "提取文本",
            description: "从消息段提取后的纯文本内容。",
            example: "<message_text>",
        },
        UserNotificationVariableInfo {
            key: "sender_id",
            label: "发送者 QQ",
            description: "触发这条 agent 指令的用户 QQ 号。",
            example: "<sender_id>",
        },
        UserNotificationVariableInfo {
            key: "sender_name",
            label: "发送者昵称",
            description: "NapCat 上报到的发送者昵称或备注名。",
            example: "<sender_name>",
        },
        UserNotificationVariableInfo {
            key: "group_id",
            label: "分组 ID",
            description: "当前账号所在的 OQQWall 分组标识。",
            example: "<group_id>",
        },
        UserNotificationVariableInfo {
            key: "account_id",
            label: "账号 QQ",
            description: "接收到这条私聊指令的机器人账号 QQ。",
            example: "<account_id>",
        },
        UserNotificationVariableInfo {
            key: "received_at",
            label: "接收时间",
            description: "按当前时区格式化后的指令接收时间。",
            example: "<received_at>",
        },
        UserNotificationVariableInfo {
            key: "received_timestamp_ms",
            label: "接收时间戳",
            description: "这条指令的毫秒时间戳。",
            example: "<received_timestamp_ms>",
        },
        UserNotificationVariableInfo {
            key: "submission_session_active",
            label: "投稿会话状态",
            description: "当前用户是否已经处于私聊投稿会话中，值为 true 或 false。",
            example: "<submission_session_active>",
        },
        UserNotificationVariableInfo {
            key: "submission_session_message_count",
            label: "会话消息数",
            description: "当前私聊投稿会话里已经缓存的消息条数。",
            example: "<submission_session_message_count>",
        },
        UserNotificationVariableInfo {
            key: "submission_post_id",
            label: "触发投稿 ID",
            description: "收到新投稿触发时的当前 post_id，私聊指令触发时为空。",
            example: "<submission_post_id>",
        },
        UserNotificationVariableInfo {
            key: "submission_sender_id",
            label: "投稿人 QQ",
            description: "收到新投稿触发时的投稿人 QQ 号，私聊指令触发时为空。",
            example: "<submission_sender_id>",
        },
        UserNotificationVariableInfo {
            key: "submission_sender_name",
            label: "投稿人昵称",
            description: "收到新投稿触发时的投稿人昵称或备注名。",
            example: "<submission_sender_name>",
        },
        UserNotificationVariableInfo {
            key: "submission_message_count",
            label: "投稿消息数",
            description: "收到新投稿触发时，构成当前稿件的原始消息条数。",
            example: "<submission_message_count>",
        },
        UserNotificationVariableInfo {
            key: "submission_image_count",
            label: "投稿图片数",
            description: "收到新投稿触发时，当前稿件里的图片附件数量。",
            example: "<submission_image_count>",
        },
        UserNotificationVariableInfo {
            key: "submission_text_message_count",
            label: "投稿文本消息数",
            description: "收到新投稿触发时，含文本内容的原始消息条数。",
            example: "<submission_text_message_count>",
        },
        UserNotificationVariableInfo {
            key: "submission_is_multi_image_single_text",
            label: "多图单文",
            description: "当前投稿是否为多张图片加一条文本消息，值为 true 或 false。",
            example: "<submission_is_multi_image_single_text>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_id",
            label: "上一条投稿 ID",
            description: "当前用户在本分组里最近一条投稿的 post_id。",
            example: "<previous_post_id>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_code",
            label: "上一条投稿编号",
            description: "当前用户最近一条投稿的显示编号，优先外部编号，没有则回退到内部编号。",
            example: "<previous_post_code>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_external_code",
            label: "上一条投稿外部编号",
            description: "当前用户最近一条投稿的外部编号。",
            example: "<previous_post_external_code>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_internal_code",
            label: "上一条投稿内部编号",
            description: "当前用户最近一条投稿的内部审核编号。",
            example: "<previous_post_internal_code>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_info",
            label: "上一条投稿信息",
            description: "当前用户最近一条投稿的摘要信息，适合直接插入回复文案。",
            example: "<previous_post_info>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_created_at",
            label: "上一条投稿时间",
            description: "当前用户最近一条投稿的格式化创建时间。",
            example: "<previous_post_created_at>",
        },
        UserNotificationVariableInfo {
            key: "previous_post_created_timestamp_ms",
            label: "上一条投稿时间戳",
            description: "当前用户最近一条投稿创建时间的毫秒时间戳。",
            example: "<previous_post_created_timestamp_ms>",
        },
    ]
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
            .and_then(|meta| meta.png_blob)
            .map(|id| id == blob_id)
            .unwrap_or(false)
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

fn build_review_command(
    review_id: Id128,
    actions: Vec<ReviewAction>,
    operator_id: String,
    now_ms: i64,
    tz_offset_minutes: i32,
) -> Command {
    if actions.len() == 1 {
        return Command::ReviewAction(ReviewActionCommand {
            review_id: Some(review_id),
            review_code: None,
            audit_msg_id: None,
            action: actions.into_iter().next().expect("single action exists"),
            operator_id,
            now_ms,
            tz_offset_minutes,
        });
    }
    Command::ReviewActionBatch(ReviewActionBatchCommand {
        review_id: Some(review_id),
        review_code: None,
        audit_msg_id: None,
        actions,
        operator_id,
        now_ms,
        tz_offset_minutes,
    })
}

fn optional_request_text(req: &ReviewDecisionRequest) -> Option<String> {
    req.text
        .as_deref()
        .or(req.comment.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn review_note_reply_text(text: String) -> String {
    if text.starts_with("审核备注：") || text.starts_with("审核备注:") {
        text
    } else {
        format!("审核备注：{}", text)
    }
}

fn parse_review_actions(req: &ReviewDecisionRequest) -> Result<Vec<ReviewAction>, &'static str> {
    match req.action.as_str() {
        "approve" => Ok(vec![ReviewAction::Approve]),
        "reject" => {
            let mut actions = vec![ReviewAction::Reject];
            if let Some(text) = optional_request_text(req) {
                actions.push(ReviewAction::Reply {
                    text: review_note_reply_text(text),
                });
            }
            Ok(actions)
        }
        "delete" => Ok(vec![ReviewAction::Delete]),
        "defer" => Ok(vec![ReviewAction::Defer {
            delay_ms: req.delay_ms.unwrap_or(0),
        }]),
        "skip" => Ok(vec![ReviewAction::Skip]),
        "immediate" => Ok(vec![ReviewAction::Immediate]),
        "refresh" => Ok(vec![ReviewAction::Refresh]),
        "rerender" => Ok(vec![ReviewAction::Rerender]),
        "select_all" => Ok(vec![ReviewAction::SelectAllMessages]),
        "toggle_anonymous" => Ok(vec![ReviewAction::ToggleAnonymous]),
        "expand_audit" => Ok(vec![ReviewAction::ExpandAudit]),
        "show" => Ok(vec![ReviewAction::Show]),
        "comment" => {
            let text = optional_request_text(req).ok_or("comment requires text")?;
            Ok(vec![ReviewAction::Comment { text }])
        }
        "reply" => {
            let text = optional_request_text(req).ok_or("reply requires text")?;
            Ok(vec![ReviewAction::Reply { text }])
        }
        "blacklist" => Ok(vec![ReviewAction::Blacklist {
            reason: req
                .comment
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }]),
        "quick_reply" => {
            let key = req
                .quick_reply_key
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or("quick_reply requires quick_reply_key")?;
            Ok(vec![ReviewAction::QuickReply {
                key: key.to_string(),
            }])
        }
        "merge" => {
            let code = req
                .target_review_code
                .ok_or("merge requires target_review_code")?;
            Ok(vec![ReviewAction::Merge { review_code: code }])
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
            | PostStage::Skipped
            | PostStage::Sent
            | PostStage::Failed
            | PostStage::Withdrawn
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_stage_excludes_sent_and_terminal_review_states() {
        assert!(is_active_stage(PostStage::ReviewPending));
        assert!(is_active_stage(PostStage::Scheduled));
        assert!(!is_active_stage(PostStage::Sent));
        assert!(!is_active_stage(PostStage::Rejected));
        assert!(!is_active_stage(PostStage::Deleted));
        assert!(!is_active_stage(PostStage::Skipped));
        assert!(!is_active_stage(PostStage::Failed));
        assert!(!is_active_stage(PostStage::Withdrawn));
    }

    #[test]
    fn parse_reject_with_comment_sends_reply_action_batch() {
        let actions = parse_review_actions(&ReviewDecisionRequest {
            action: "reject".to_string(),
            comment: Some("内容不合适".to_string()),
            delay_ms: None,
            text: None,
            quick_reply_key: None,
            target_review_code: None,
        })
        .expect("actions");

        assert_eq!(
            actions,
            vec![
                ReviewAction::Reject,
                ReviewAction::Reply {
                    text: "审核备注：内容不合适".to_string()
                }
            ]
        );
    }
}
