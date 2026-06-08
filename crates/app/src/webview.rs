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
        "skipped" => Some(PostStage::Skipped),
        "manual" => Some(PostStage::Manual),
        "failed" => Some(PostStage::Failed),
        _ => None,
    }
}

fn is_active_stage(stage: PostStage) -> bool {
    !matches!(
        stage,
        PostStage::Rejected | PostStage::Skipped | PostStage::Failed
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
        PostStage::Skipped => "skipped",
        PostStage::Manual => "manual",
        PostStage::Failed => "failed",
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
