use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::builder::{
    BooleanBuilder, Int16Builder, Int32Builder, Int64Builder, StringBuilder,
};
use arrow_schema::{DataType, Field, Schema};
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, TimeZone, Utc};
use parquet::arrow::ArrowWriter;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tokio::fs as tokio_fs;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

const SAMPLE_SCHEMA_VERSION: i32 = 1;
const MAX_UPLOAD_SAMPLES: usize = 2000;
const DEFAULT_MAX_BODY_MB: usize = 10;

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    object_dir: Arc<PathBuf>,
    export_dir: Arc<PathBuf>,
}

#[derive(Debug, Clone)]
struct AuthContext {
    token_id: String,
    permissions: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
    request_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UploadBatchRequest {
    batch_id: String,
    schema_version: i32,
    chat_objects: Vec<ChatObjectEntry>,
    samples: Vec<PendingSample>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatObjectEntry {
    chat_record_hash: String,
    codec: String,
    message_count: usize,
    payload: ChatRecord,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatRecord {
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatMessage {
    ingress_id: String,
    platform_msg_id: String,
    received_at_ms: i64,
    text: String,
    attachments: Vec<ChatAttachment>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatAttachment {
    kind: String,
    name: Option<String>,
    reference_type: String,
    reference: String,
    size_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PendingSample {
    sample_id: String,
    schema_version: i32,
    label: i16,
    augmentation: String,
    base_sample_id: Option<String>,
    label_source: String,
    decision_at_ms: i64,
    review_id: String,
    review_code: i32,
    post_id: String,
    group_id: String,
    sender_id: String,
    chat_record_hash: String,
    message_count: usize,
}

#[derive(Debug, Serialize)]
struct UploadBatchResponse {
    ingested: bool,
    duplicate: bool,
    batch_id: String,
    accepted_samples: usize,
    accepted_chat_objects: usize,
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct CursorQuery {
    #[serde(default)]
    cursor: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ListSamplesQuery {
    #[serde(default)]
    cursor: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    label: Option<i16>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    review_id: Option<String>,
    #[serde(default)]
    post_id: Option<String>,
    #[serde(default)]
    include_excluded: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ListBatchesResponse {
    items: Vec<BatchItem>,
    next_cursor: Option<i64>,
}

#[derive(Debug, Serialize)]
struct BatchItem {
    id: i64,
    batch_id: String,
    idempotency_key: String,
    request_sha256: String,
    schema_version: i32,
    sample_count: i32,
    chat_object_count: i32,
    token_id: String,
    received_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BatchDetailResponse {
    batch: BatchItem,
    sample_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ListSamplesResponse {
    items: Vec<SampleItem>,
    next_cursor: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SampleItem {
    id: i64,
    sample_id: String,
    schema_version: i32,
    label: i16,
    augmentation: String,
    base_sample_id: Option<String>,
    label_source: String,
    decision_at_ms: i64,
    review_id: String,
    review_code: i32,
    post_id: String,
    group_id: String,
    sender_id: String,
    chat_record_hash: String,
    message_count: i32,
    batch_id: String,
    excluded: bool,
    corrected_label: Option<i16>,
    note: Option<String>,
    ingested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct SampleDetailResponse {
    sample: SampleItem,
    mutations: Vec<SampleMutationItem>,
}

#[derive(Debug, Serialize)]
struct SampleMutationItem {
    id: i64,
    sample_id: String,
    actor_token_id: String,
    before_json: Value,
    after_json: Value,
    changed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct PatchSampleRequest {
    #[serde(default)]
    excluded: Option<bool>,
    #[serde(default)]
    corrected_label: Option<Option<i16>>,
    #[serde(default)]
    note: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
struct PatchSampleResponse {
    sample_id: String,
    excluded: bool,
    corrected_label: Option<i16>,
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateExportRequest {
    #[serde(default)]
    from_decision_at_ms: Option<i64>,
    #[serde(default)]
    to_decision_at_ms: Option<i64>,
    #[serde(default)]
    labels: Option<Vec<i16>>,
    #[serde(default)]
    include_excluded: Option<bool>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateExportResponse {
    job_id: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct ListExportsResponse {
    items: Vec<ExportJobItem>,
}

#[derive(Debug, Serialize)]
struct ExportJobItem {
    job_id: String,
    status: String,
    filters_json: Value,
    format: String,
    output_dir: String,
    row_count: Option<i64>,
    error: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct ExportManifest {
    job_id: String,
    schema_version: i32,
    created_at: DateTime<Utc>,
    row_count: i64,
    files: Vec<ExportFileEntry>,
    filters: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExportFileEntry {
    path: String,
    row_count: usize,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct CreateTokenRequest {
    #[serde(default)]
    token_id: Option<String>,
    permissions: Vec<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateTokenResponse {
    token_id: String,
    token: String,
    permissions: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeleteTokenPath {
    token_id: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    now: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ChatObjectResponse {
    chat_record_hash: String,
    codec: String,
    message_count: i32,
    payload: ChatRecord,
    first_seen_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportRow {
    sample_id: String,
    schema_version: i32,
    label: i16,
    augmentation: String,
    base_sample_id: Option<String>,
    label_source: String,
    decision_at_ms: i64,
    review_id: String,
    review_code: i32,
    post_id: String,
    group_id: String,
    sender_id: String,
    chat_record_hash: String,
    message_count: i32,
    batch_id: String,
    excluded: bool,
    corrected_label: Option<i16>,
    note: Option<String>,
    chat_record_json: String,
}

#[tokio::main]
async fn main() {
    init_tracing();

    let cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("config error: {}", err);
            std::process::exit(2);
        }
    };

    if let Err(err) = fs::create_dir_all(&cfg.object_dir) {
        eprintln!("create object dir failed: {}", err);
        std::process::exit(2);
    }
    if let Err(err) = fs::create_dir_all(&cfg.export_dir) {
        eprintln!("create export dir failed: {}", err);
        std::process::exit(2);
    }

    let pool = match PgPool::connect(&cfg.pg_dsn).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("connect postgres failed: {}", err);
            std::process::exit(2);
        }
    };
    if let Err(err) = init_schema(&pool).await {
        eprintln!("init schema failed: {}", err);
        std::process::exit(2);
    }
    if let Err(err) = ensure_root_token(&pool, &cfg.bootstrap_root_token).await {
        eprintln!("ensure root token failed: {}", err);
        std::process::exit(2);
    }

    let state = AppState {
        pool,
        object_dir: Arc::new(cfg.object_dir),
        export_dir: Arc::new(cfg.export_dir),
    };

    let app = Router::new()
        .route("/telemetry/v1/healthz", get(healthz))
        .route("/telemetry/v1/submission/batch", post(upload_batch))
        .route("/telemetry/v1/batches", get(list_batches))
        .route("/telemetry/v1/batches/{batch_id}", get(get_batch))
        .route("/telemetry/v1/samples", get(list_samples))
        .route(
            "/telemetry/v1/samples/{sample_id}",
            get(get_sample).patch(patch_sample),
        )
        .route(
            "/telemetry/v1/chat_objects/{chat_record_hash}",
            get(get_chat_object),
        )
        .route(
            "/telemetry/v1/exports",
            get(list_exports).post(create_export),
        )
        .route("/telemetry/v1/exports/{job_id}", get(get_export))
        .route(
            "/telemetry/v1/exports/{job_id}/manifest",
            get(get_export_manifest),
        )
        .route(
            "/telemetry/v1/exports/{job_id}/files/{name}",
            get(get_export_file),
        )
        .route("/telemetry/v1/admin/tokens", post(create_token))
        .route(
            "/telemetry/v1/admin/tokens/{token_id}",
            delete(delete_token),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            cfg.max_body_mb * 1024 * 1024,
        ))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_middleware));

    let listener = match tokio::net::TcpListener::bind(&cfg.http_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("bind {} failed: {}", cfg.http_addr, err);
            std::process::exit(2);
        }
    };
    info!("telemetry-collector listening on {}", cfg.http_addr);
    if let Err(err) = axum::serve(listener, app).await {
        error!("server stopped: {}", err);
    }
}

fn init_tracing() {
    let filter = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[derive(Debug)]
struct Config {
    http_addr: String,
    pg_dsn: String,
    object_dir: PathBuf,
    export_dir: PathBuf,
    bootstrap_root_token: String,
    max_body_mb: usize,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let http_addr = env::var("COLLECTOR_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:10925".into());
        let pg_dsn =
            env::var("COLLECTOR_PG_DSN").map_err(|_| "COLLECTOR_PG_DSN is required".to_string())?;
        let object_dir = PathBuf::from(
            env::var("COLLECTOR_OBJECT_DIR").unwrap_or_else(|_| "data/collector/objects".into()),
        );
        let export_dir = PathBuf::from(
            env::var("COLLECTOR_EXPORT_DIR").unwrap_or_else(|_| "data/collector/exports".into()),
        );
        let bootstrap_root_token = env::var("COLLECTOR_BOOTSTRAP_ROOT_TOKEN")
            .map_err(|_| "COLLECTOR_BOOTSTRAP_ROOT_TOKEN is required".to_string())?;
        if bootstrap_root_token.len() < 16 {
            return Err("COLLECTOR_BOOTSTRAP_ROOT_TOKEN must be at least 16 chars".to_string());
        }
        let max_body_mb = env::var("COLLECTOR_MAX_BODY_MB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_BODY_MB)
            .clamp(1, 256);

        Ok(Self {
            http_addr,
            pg_dsn,
            object_dir,
            export_dir,
            bootstrap_root_token,
            max_body_mb,
        })
    }
}

async fn init_schema(pool: &PgPool) -> Result<(), String> {
    let ddl = r#"
CREATE TABLE IF NOT EXISTS ingest_batches (
    id BIGSERIAL PRIMARY KEY,
    batch_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    request_sha256 TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    sample_count INTEGER NOT NULL,
    chat_object_count INTEGER NOT NULL,
    token_id TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_ingest_batches_received_at ON ingest_batches(received_at DESC);

CREATE TABLE IF NOT EXISTS chat_objects (
    chat_record_hash TEXT PRIMARY KEY,
    codec TEXT NOT NULL,
    message_count INTEGER NOT NULL,
    object_path TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS samples (
    id BIGSERIAL PRIMARY KEY,
    sample_id TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL,
    label SMALLINT NOT NULL,
    augmentation TEXT NOT NULL,
    base_sample_id TEXT NULL,
    label_source TEXT NOT NULL,
    decision_at_ms BIGINT NOT NULL,
    review_id TEXT NOT NULL,
    review_code INTEGER NOT NULL,
    post_id TEXT NOT NULL,
    group_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    chat_record_hash TEXT NOT NULL REFERENCES chat_objects(chat_record_hash),
    message_count INTEGER NOT NULL,
    batch_id TEXT NOT NULL,
    excluded BOOLEAN NOT NULL DEFAULT FALSE,
    corrected_label SMALLINT NULL,
    note TEXT NULL,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_samples_decision_at ON samples(decision_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_samples_label ON samples(label);
CREATE INDEX IF NOT EXISTS idx_samples_group_id ON samples(group_id);
CREATE INDEX IF NOT EXISTS idx_samples_review_id ON samples(review_id);
CREATE INDEX IF NOT EXISTS idx_samples_post_id ON samples(post_id);
CREATE INDEX IF NOT EXISTS idx_samples_chat_hash ON samples(chat_record_hash);

CREATE TABLE IF NOT EXISTS sample_mutations (
    id BIGSERIAL PRIMARY KEY,
    sample_id TEXT NOT NULL REFERENCES samples(sample_id),
    actor_token_id TEXT NOT NULL,
    before_json TEXT NOT NULL,
    after_json TEXT NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_sample_mutations_sample_id ON sample_mutations(sample_id, changed_at DESC);

CREATE TABLE IF NOT EXISTS api_tokens (
    token_id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    permissions_json TEXT NOT NULL,
    disabled BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    note TEXT NULL
);

CREATE TABLE IF NOT EXISTS export_jobs (
    job_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    filters_json TEXT NOT NULL,
    format TEXT NOT NULL,
    output_dir TEXT NOT NULL,
    row_count BIGINT NULL,
    error TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL
);
CREATE INDEX IF NOT EXISTS idx_export_jobs_created_at ON export_jobs(created_at DESC);
"#;

    for statement in ddl.split(';') {
        let s = statement.trim();
        if s.is_empty() {
            continue;
        }
        sqlx::query(s)
            .execute(pool)
            .await
            .map_err(|err| format!("exec schema statement failed: {}", err))?;
    }
    Ok(())
}

async fn ensure_root_token(pool: &PgPool, root_token: &str) -> Result<(), String> {
    let perms: Vec<String> = vec![
        "ingest.write".into(),
        "batches.read".into(),
        "samples.read".into(),
        "samples.write".into(),
        "exports.manage".into(),
        "tokens.manage".into(),
    ];
    let permissions_json = serde_json::to_string(&perms).map_err(|err| err.to_string())?;
    sqlx::query(
        r#"INSERT INTO api_tokens(token_id, token_hash, permissions_json, disabled, note)
           VALUES($1, $2, $3, false, $4)
           ON CONFLICT(token_id) DO UPDATE
           SET token_hash = EXCLUDED.token_hash,
               permissions_json = EXCLUDED.permissions_json,
               disabled = false,
               note = EXCLUDED.note"#,
    )
    .bind("root")
    .bind(hash_token(root_token))
    .bind(permissions_json)
    .bind(Some("bootstrap root token".to_string()))
    .execute(pool)
    .await
    .map_err(|err| format!("upsert root token failed: {}", err))?;
    Ok(())
}

async fn auth_middleware(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    if path == "/telemetry/v1/healthz" {
        return next.run(req).await;
    }
    let Some(token) = bearer_token(req.headers()) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "missing bearer token",
            request_id(req.headers()),
        );
    };
    let token_hash = hash_token(token);
    let row = match sqlx::query(
        "SELECT token_id, permissions_json, disabled, expires_at FROM api_tokens WHERE token_hash=$1",
    )
    .bind(token_hash)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("token query failed: {}", err),
                request_id(req.headers()),
            )
        }
    };

    let Some(row) = row else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid token",
            request_id(req.headers()),
        );
    };

    let disabled: bool = row.get("disabled");
    if disabled {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "token disabled",
            request_id(req.headers()),
        );
    }

    let expires_at: Option<DateTime<Utc>> = row.get("expires_at");
    if expires_at.map(|ts| ts <= Utc::now()).unwrap_or(false) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "token expired",
            request_id(req.headers()),
        );
    }

    let permissions_json: String = row.get("permissions_json");
    let permissions: Vec<String> = match serde_json::from_str(&permissions_json) {
        Ok(v) => v,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "token permissions invalid",
                request_id(req.headers()),
            );
        }
    };
    let ctx = AuthContext {
        token_id: row.get("token_id"),
        permissions: permissions.into_iter().collect(),
    };
    req.extensions_mut().insert(ctx);
    next.run(req).await
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(random_hex32)
}

fn require_permission(
    headers: &HeaderMap,
    ctx: &AuthContext,
    permission: &str,
) -> Result<(), Response> {
    if ctx.permissions.contains(permission) {
        return Ok(());
    }
    Err(error_response(
        StatusCode::FORBIDDEN,
        "FORBIDDEN",
        &format!("missing permission: {}", permission),
        request_id(headers),
    ))
}

async fn healthz() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        now: Utc::now(),
    })
}

async fn upload_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
    Json(req): Json<UploadBatchRequest>,
) -> Response {
    let req_id = request_id(&headers);
    if let Err(resp) = require_permission(&headers, &auth, "ingest.write") {
        return resp;
    }
    let Some(idempotency_key) = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "missing Idempotency-Key",
            req_id,
        );
    };

    if req.schema_version != SAMPLE_SCHEMA_VERSION {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "unsupported schema_version",
            req_id,
        );
    }
    if req.samples.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "samples is empty",
            req_id,
        );
    }
    if req.samples.len() > MAX_UPLOAD_SAMPLES {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "samples too large",
            req_id,
        );
    }

    let request_sha = match serde_json::to_vec(&req) {
        Ok(bytes) => hash_bytes(&bytes),
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("encode request failed: {}", err),
                req_id,
            );
        }
    };

    if let Err(resp) = check_idempotency(
        &state.pool,
        &idempotency_key,
        &request_sha,
        &req.batch_id,
        &req_id,
    )
    .await
    {
        return resp;
    }

    let mut object_map: HashMap<String, ChatObjectEntry> = HashMap::new();
    for obj in &req.chat_objects {
        if obj.codec != "json" {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "unsupported chat object codec",
                req_id,
            );
        }
        let calc_hash = hash_chat_record(&obj.payload);
        if calc_hash != obj.chat_record_hash {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("chat_record_hash mismatch: {}", obj.chat_record_hash),
                req_id,
            );
        }
        if obj.message_count != obj.payload.messages.len() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("message_count mismatch for {}", obj.chat_record_hash),
                req_id,
            );
        }
        object_map.insert(obj.chat_record_hash.clone(), obj.clone());
    }

    let mut missing_refs = HashSet::new();
    for sample in &req.samples {
        if sample.schema_version != SAMPLE_SCHEMA_VERSION {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "sample schema_version mismatch",
                req_id,
            );
        }
        if sample.label != 0 && sample.label != 1 {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("invalid label in sample {}", sample.sample_id),
                req_id,
            );
        }
        if sample.message_count == 0 {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("message_count must be >0 in sample {}", sample.sample_id),
                req_id,
            );
        }
        if !object_map.contains_key(&sample.chat_record_hash) {
            missing_refs.insert(sample.chat_record_hash.clone());
        }
    }

    for hash in &missing_refs {
        let exists = match sqlx::query("SELECT 1 FROM chat_objects WHERE chat_record_hash=$1")
            .bind(hash)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(v) => v.is_some(),
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    &format!("query chat object failed: {}", err),
                    req_id,
                );
            }
        };
        if !exists {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("missing chat object for hash {}", hash),
                req_id,
            );
        }
    }

    let mut persisted_objects: HashMap<String, String> = HashMap::new();
    for obj in object_map.values() {
        let bytes = match serde_json::to_vec(obj) {
            Ok(v) => v,
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    &format!("encode chat object failed: {}", err),
                    req_id,
                );
            }
        };
        let payload_sha = hash_bytes(&bytes);
        let path = state
            .object_dir
            .join(format!("{}.json", obj.chat_record_hash));
        if !path.exists() {
            let tmp = path.with_extension("json.tmp");
            let mut file = match tokio_fs::File::create(&tmp).await {
                Ok(file) => file,
                Err(err) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "INTERNAL",
                        &format!("create tmp object file failed: {}", err),
                        req_id,
                    );
                }
            };
            if let Err(err) = file.write_all(&bytes).await {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    &format!("write object failed: {}", err),
                    req_id,
                );
            }
            if let Err(err) = file.flush().await {
                warn!("flush object file failed: {}", err);
            }
            if let Err(err) = tokio_fs::rename(&tmp, &path).await {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    &format!("move object file failed: {}", err),
                    req_id,
                );
            }
        }
        persisted_objects.insert(obj.chat_record_hash.clone(), payload_sha);
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("begin tx failed: {}", err),
                req_id,
            );
        }
    };

    let insert_batch_res = sqlx::query(
        r#"INSERT INTO ingest_batches(batch_id, idempotency_key, request_sha256, schema_version, sample_count, chat_object_count, token_id)
           VALUES($1,$2,$3,$4,$5,$6,$7)"#,
    )
    .bind(&req.batch_id)
    .bind(&idempotency_key)
    .bind(&request_sha)
    .bind(req.schema_version)
    .bind(i32::try_from(req.samples.len()).unwrap_or(i32::MAX))
    .bind(i32::try_from(req.chat_objects.len()).unwrap_or(i32::MAX))
    .bind(&auth.token_id)
    .execute(&mut *tx)
    .await;

    if let Err(err) = insert_batch_res {
        if is_unique_violation(&err) {
            match tx.rollback().await {
                Ok(()) => {}
                Err(rollback_err) => warn!("rollback failed: {}", rollback_err),
            }
            return match check_idempotency(
                &state.pool,
                &idempotency_key,
                &request_sha,
                &req.batch_id,
                &req_id,
            )
            .await
            {
                Ok(()) => Json(UploadBatchResponse {
                    ingested: false,
                    duplicate: true,
                    batch_id: req.batch_id,
                    accepted_samples: 0,
                    accepted_chat_objects: 0,
                    request_id: req_id,
                })
                .into_response(),
                Err(resp) => resp,
            };
        }
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("insert ingest batch failed: {}", err),
            req_id,
        );
    }

    for obj in &req.chat_objects {
        let payload_sha = persisted_objects
            .get(&obj.chat_record_hash)
            .cloned()
            .unwrap_or_else(|| "".to_string());
        let object_path = format!("{}.json", obj.chat_record_hash);
        if let Err(err) = sqlx::query(
            r#"INSERT INTO chat_objects(chat_record_hash, codec, message_count, object_path, payload_sha256)
               VALUES($1,$2,$3,$4,$5)
               ON CONFLICT(chat_record_hash)
               DO UPDATE SET last_seen_at=NOW()"#,
        )
        .bind(&obj.chat_record_hash)
        .bind(&obj.codec)
        .bind(i32::try_from(obj.message_count).unwrap_or(i32::MAX))
        .bind(&object_path)
        .bind(payload_sha)
        .execute(&mut *tx)
        .await
        {
            let _ = tx.rollback().await;
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("upsert chat object failed: {}", err),
                req_id,
            );
        }
    }

    let mut accepted_samples = 0usize;
    for sample in &req.samples {
        let res = sqlx::query(
            r#"INSERT INTO samples(
                sample_id, schema_version, label, augmentation, base_sample_id, label_source,
                decision_at_ms, review_id, review_code, post_id, group_id, sender_id,
                chat_record_hash, message_count, batch_id
               ) VALUES(
                $1,$2,$3,$4,$5,$6,
                $7,$8,$9,$10,$11,$12,
                $13,$14,$15
               ) ON CONFLICT(sample_id) DO NOTHING"#,
        )
        .bind(&sample.sample_id)
        .bind(sample.schema_version)
        .bind(sample.label)
        .bind(&sample.augmentation)
        .bind(&sample.base_sample_id)
        .bind(&sample.label_source)
        .bind(sample.decision_at_ms)
        .bind(&sample.review_id)
        .bind(sample.review_code)
        .bind(&sample.post_id)
        .bind(&sample.group_id)
        .bind(&sample.sender_id)
        .bind(&sample.chat_record_hash)
        .bind(i32::try_from(sample.message_count).unwrap_or(i32::MAX))
        .bind(&req.batch_id)
        .execute(&mut *tx)
        .await;

        match res {
            Ok(done) => {
                if done.rows_affected() > 0 {
                    accepted_samples += 1;
                }
            }
            Err(err) => {
                let _ = tx.rollback().await;
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    &format!("insert sample failed: {}", err),
                    req_id,
                );
            }
        }
    }

    if let Err(err) = tx.commit().await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("commit ingest tx failed: {}", err),
            req_id,
        );
    }

    let response = UploadBatchResponse {
        ingested: true,
        duplicate: false,
        batch_id: req.batch_id,
        accepted_samples,
        accepted_chat_objects: req.chat_objects.len(),
        request_id: req_id,
    };
    (StatusCode::CREATED, Json(response)).into_response()
}

async fn check_idempotency(
    pool: &PgPool,
    idempotency_key: &str,
    request_sha: &str,
    batch_id: &str,
    request_id: &str,
) -> Result<(), Response> {
    let row = sqlx::query("SELECT request_sha256 FROM ingest_batches WHERE idempotency_key=$1")
        .bind(idempotency_key)
        .fetch_optional(pool)
        .await
        .map_err(|err| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("idempotency query failed: {}", err),
                request_id.to_string(),
            )
        })?;

    if let Some(row) = row {
        let existing_sha: String = row.get("request_sha256");
        if existing_sha == request_sha {
            let resp = UploadBatchResponse {
                ingested: false,
                duplicate: true,
                batch_id: batch_id.to_string(),
                accepted_samples: 0,
                accepted_chat_objects: 0,
                request_id: request_id.to_string(),
            };
            return Err((StatusCode::OK, Json(resp)).into_response());
        }
        return Err(error_response(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_CONFLICT",
            "idempotency key already used with different payload",
            request_id.to_string(),
        ));
    }

    Ok(())
}

async fn list_batches(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CursorQuery>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "batches.read") {
        return resp;
    }
    let cursor = query.cursor.unwrap_or(i64::MAX);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let rows = match sqlx::query(
        r#"SELECT id, batch_id, idempotency_key, request_sha256, schema_version,
                  sample_count, chat_object_count, token_id, received_at
           FROM ingest_batches
           WHERE id < $1
           ORDER BY id DESC
           LIMIT $2"#,
    )
    .bind(cursor)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("list batches failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(BatchItem {
            id: row.get("id"),
            batch_id: row.get("batch_id"),
            idempotency_key: row.get("idempotency_key"),
            request_sha256: row.get("request_sha256"),
            schema_version: row.get("schema_version"),
            sample_count: row.get("sample_count"),
            chat_object_count: row.get("chat_object_count"),
            token_id: row.get("token_id"),
            received_at: row.get("received_at"),
        });
    }
    let next_cursor = items.last().map(|x| x.id);
    Json(ListBatchesResponse { items, next_cursor }).into_response()
}

async fn get_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(batch_id): AxumPath<String>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "batches.read") {
        return resp;
    }
    let row = match sqlx::query(
        r#"SELECT id, batch_id, idempotency_key, request_sha256, schema_version,
                  sample_count, chat_object_count, token_id, received_at
           FROM ingest_batches WHERE batch_id=$1"#,
    )
    .bind(&batch_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("get batch failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let Some(row) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "batch not found",
            request_id(&headers),
        );
    };

    let batch = BatchItem {
        id: row.get("id"),
        batch_id: row.get("batch_id"),
        idempotency_key: row.get("idempotency_key"),
        request_sha256: row.get("request_sha256"),
        schema_version: row.get("schema_version"),
        sample_count: row.get("sample_count"),
        chat_object_count: row.get("chat_object_count"),
        token_id: row.get("token_id"),
        received_at: row.get("received_at"),
    };

    let sample_rows =
        match sqlx::query("SELECT sample_id FROM samples WHERE batch_id=$1 ORDER BY id ASC")
            .bind(&batch_id)
            .fetch_all(&state.pool)
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    &format!("query batch samples failed: {}", err),
                    request_id(&headers),
                );
            }
        };
    let sample_ids = sample_rows
        .into_iter()
        .map(|row| row.get("sample_id"))
        .collect();
    Json(BatchDetailResponse { batch, sample_ids }).into_response()
}

async fn list_samples(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListSamplesQuery>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "samples.read") {
        return resp;
    }

    let cursor = query.cursor.unwrap_or(i64::MAX);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let include_excluded = query.include_excluded.unwrap_or(false);

    let rows = match sqlx::query(
        r#"SELECT id, sample_id, schema_version, label, augmentation, base_sample_id,
                  label_source, decision_at_ms, review_id, review_code, post_id,
                  group_id, sender_id, chat_record_hash, message_count, batch_id,
                  excluded, corrected_label, note, ingested_at
           FROM samples
           WHERE id < $1
             AND ($2::SMALLINT IS NULL OR label = $2)
             AND ($3::TEXT IS NULL OR group_id = $3)
             AND ($4::TEXT IS NULL OR review_id = $4)
             AND ($5::TEXT IS NULL OR post_id = $5)
             AND ($6::BOOLEAN OR excluded = false)
           ORDER BY id DESC
           LIMIT $7"#,
    )
    .bind(cursor)
    .bind(query.label)
    .bind(query.group_id)
    .bind(query.review_id)
    .bind(query.post_id)
    .bind(include_excluded)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("list samples failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(row_to_sample_item(&row));
    }
    let next_cursor = items.last().map(|x| x.id);
    Json(ListSamplesResponse { items, next_cursor }).into_response()
}

async fn get_sample(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(sample_id): AxumPath<String>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "samples.read") {
        return resp;
    }

    let row = match sqlx::query(
        r#"SELECT id, sample_id, schema_version, label, augmentation, base_sample_id,
                  label_source, decision_at_ms, review_id, review_code, post_id,
                  group_id, sender_id, chat_record_hash, message_count, batch_id,
                  excluded, corrected_label, note, ingested_at
           FROM samples WHERE sample_id=$1"#,
    )
    .bind(&sample_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("get sample failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let Some(row) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "sample not found",
            request_id(&headers),
        );
    };

    let sample = row_to_sample_item(&row);

    let mutations_rows = match sqlx::query(
        "SELECT id, sample_id, actor_token_id, before_json, after_json, changed_at FROM sample_mutations WHERE sample_id=$1 ORDER BY id DESC",
    )
    .bind(&sample_id)
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("query sample mutations failed: {}", err),
                request_id(&headers),
            )
        }
    };
    let mutations = mutations_rows
        .into_iter()
        .map(|row| SampleMutationItem {
            id: row.get("id"),
            sample_id: row.get("sample_id"),
            actor_token_id: row.get("actor_token_id"),
            before_json: serde_json::from_str::<Value>(&row.get::<String, _>("before_json"))
                .unwrap_or(Value::Null),
            after_json: serde_json::from_str::<Value>(&row.get::<String, _>("after_json"))
                .unwrap_or(Value::Null),
            changed_at: row.get("changed_at"),
        })
        .collect();

    Json(SampleDetailResponse { sample, mutations }).into_response()
}

async fn patch_sample(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(sample_id): AxumPath<String>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
    Json(req): Json<PatchSampleRequest>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "samples.write") {
        return resp;
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("begin tx failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let existing = match sqlx::query(
        "SELECT excluded, corrected_label, note FROM samples WHERE sample_id=$1 FOR UPDATE",
    )
    .bind(&sample_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("query sample failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let Some(existing) = existing else {
        let _ = tx.rollback().await;
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "sample not found",
            request_id(&headers),
        );
    };

    let old_excluded: bool = existing.get("excluded");
    let old_corrected_label: Option<i16> = existing.get("corrected_label");
    let old_note: Option<String> = existing.get("note");

    let new_excluded = req.excluded.unwrap_or(old_excluded);
    let new_corrected_label = req.corrected_label.unwrap_or(old_corrected_label);
    let new_note = req.note.unwrap_or(old_note.clone());

    if let Some(value) = new_corrected_label {
        if value != 0 && value != 1 {
            let _ = tx.rollback().await;
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "corrected_label must be 0 or 1",
                request_id(&headers),
            );
        }
    }

    if let Err(err) = sqlx::query(
        "UPDATE samples SET excluded=$1, corrected_label=$2, note=$3 WHERE sample_id=$4",
    )
    .bind(new_excluded)
    .bind(new_corrected_label)
    .bind(&new_note)
    .bind(&sample_id)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("update sample failed: {}", err),
            request_id(&headers),
        );
    }

    let before_json = json!({
        "excluded": old_excluded,
        "corrected_label": old_corrected_label,
        "note": old_note,
    });
    let after_json = json!({
        "excluded": new_excluded,
        "corrected_label": new_corrected_label,
        "note": new_note,
    });

    if let Err(err) = sqlx::query(
        "INSERT INTO sample_mutations(sample_id, actor_token_id, before_json, after_json) VALUES($1,$2,$3,$4)",
    )
    .bind(&sample_id)
    .bind(&auth.token_id)
    .bind(before_json.to_string())
    .bind(after_json.to_string())
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("insert mutation failed: {}", err),
            request_id(&headers),
        );
    }

    if let Err(err) = tx.commit().await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("commit sample mutation failed: {}", err),
            request_id(&headers),
        );
    }

    Json(PatchSampleResponse {
        sample_id,
        excluded: new_excluded,
        corrected_label: new_corrected_label,
        note: new_note,
    })
    .into_response()
}

async fn get_chat_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(chat_record_hash): AxumPath<String>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "samples.read") {
        return resp;
    }

    let row = match sqlx::query(
        "SELECT codec, message_count, object_path, first_seen_at, last_seen_at FROM chat_objects WHERE chat_record_hash=$1",
    )
    .bind(&chat_record_hash)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("query chat object failed: {}", err),
                request_id(&headers),
            )
        }
    };

    let Some(row) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "chat object not found",
            request_id(&headers),
        );
    };

    let object_path: String = row.get("object_path");
    let path = state.object_dir.join(&object_path);
    let bytes = match tokio_fs::read(&path).await {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("read object payload failed: {}", err),
                request_id(&headers),
            );
        }
    };
    let entry: ChatObjectEntry = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("decode object payload failed: {}", err),
                request_id(&headers),
            );
        }
    };

    Json(ChatObjectResponse {
        chat_record_hash,
        codec: row.get("codec"),
        message_count: row.get("message_count"),
        payload: entry.payload,
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
    })
    .into_response()
}

async fn create_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
    Json(req): Json<CreateExportRequest>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "exports.manage") {
        return resp;
    }

    let format = req.format.clone().unwrap_or_else(|| "parquet".to_string());
    if format != "parquet" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "only parquet export is supported",
            request_id(&headers),
        );
    }

    let job_id = format!("exp_{}", random_hex16());
    let job_dir = state.export_dir.join(&job_id);
    if let Err(err) = fs::create_dir_all(&job_dir) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("create export dir failed: {}", err),
            request_id(&headers),
        );
    }

    let filters_json = match serde_json::to_string(&req) {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("encode export filters failed: {}", err),
                request_id(&headers),
            );
        }
    };

    if let Err(err) = sqlx::query(
        r#"INSERT INTO export_jobs(job_id, status, filters_json, format, output_dir)
           VALUES($1, 'pending', $2, $3, $4)"#,
    )
    .bind(&job_id)
    .bind(&filters_json)
    .bind(&format)
    .bind(job_dir.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("insert export job failed: {}", err),
            request_id(&headers),
        );
    }

    let state_for_task = state.clone();
    let request_for_task = req;
    let job_id_for_task = job_id.clone();
    tokio::spawn(async move {
        let state_for_fail = state_for_task.clone();
        if let Err(err) = run_export_job(state_for_task, &job_id_for_task, request_for_task).await {
            error!("export job {} failed: {}", job_id_for_task, err);
            if let Err(update_err) = sqlx::query(
                "UPDATE export_jobs SET status='failed', error=$2, finished_at=NOW() WHERE job_id=$1",
            )
            .bind(&job_id_for_task)
            .bind(err)
            .execute(&state_for_fail.pool)
            .await
            {
                error!(
                    "failed to mark export job {} as failed: {}",
                    job_id_for_task, update_err
                );
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(CreateExportResponse {
            job_id,
            status: "pending".to_string(),
        }),
    )
        .into_response()
}

async fn run_export_job(
    state: AppState,
    job_id: &str,
    req: CreateExportRequest,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE export_jobs SET status='running', started_at=NOW(), error=NULL WHERE job_id=$1",
    )
    .bind(job_id)
    .execute(&state.pool)
    .await
    .map_err(|err| format!("set export running failed: {}", err))?;

    let include_excluded = req.include_excluded.unwrap_or(false);
    let rows = sqlx::query(
        r#"SELECT s.sample_id, s.schema_version, s.label, s.augmentation, s.base_sample_id,
                  s.label_source, s.decision_at_ms, s.review_id, s.review_code, s.post_id,
                  s.group_id, s.sender_id, s.chat_record_hash, s.message_count, s.batch_id,
                  s.excluded, s.corrected_label, s.note, c.object_path
           FROM samples s
           JOIN chat_objects c ON c.chat_record_hash = s.chat_record_hash
           WHERE ($1::BIGINT IS NULL OR s.decision_at_ms >= $1)
             AND ($2::BIGINT IS NULL OR s.decision_at_ms <= $2)
             AND ($3::TEXT IS NULL OR s.group_id = $3)
             AND ($4::BOOLEAN OR s.excluded=false)
           ORDER BY s.id ASC"#,
    )
    .bind(req.from_decision_at_ms)
    .bind(req.to_decision_at_ms)
    .bind(req.group_id.clone())
    .bind(include_excluded)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| format!("query export rows failed: {}", err))?;

    let label_filter: Option<HashSet<i16>> = req.labels.clone().map(|v| v.into_iter().collect());
    let mut grouped: BTreeMap<(String, i16), Vec<ExportRow>> = BTreeMap::new();

    for row in rows {
        let label: i16 = row.get("label");
        if let Some(filter) = &label_filter {
            if !filter.contains(&label) {
                continue;
            }
        }
        let chat_record_hash: String = row.get("chat_record_hash");
        let object_path: String = row.get("object_path");
        let payload_bytes = tokio_fs::read(state.object_dir.join(object_path))
            .await
            .map_err(|err| format!("read chat object for export failed: {}", err))?;
        let object_entry: ChatObjectEntry = serde_json::from_slice(&payload_bytes)
            .map_err(|err| format!("decode chat object for export failed: {}", err))?;
        let chat_record_json = serde_json::to_string(&object_entry.payload)
            .map_err(|err| format!("encode chat record json failed: {}", err))?;

        let decision_at_ms: i64 = row.get("decision_at_ms");
        let date = ms_to_utc_date(decision_at_ms)?;
        let partition = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());

        grouped
            .entry((partition, label))
            .or_default()
            .push(ExportRow {
                sample_id: row.get("sample_id"),
                schema_version: row.get("schema_version"),
                label,
                augmentation: row.get("augmentation"),
                base_sample_id: row.get("base_sample_id"),
                label_source: row.get("label_source"),
                decision_at_ms,
                review_id: row.get("review_id"),
                review_code: row.get("review_code"),
                post_id: row.get("post_id"),
                group_id: row.get("group_id"),
                sender_id: row.get("sender_id"),
                chat_record_hash,
                message_count: row.get("message_count"),
                batch_id: row.get("batch_id"),
                excluded: row.get("excluded"),
                corrected_label: row.get("corrected_label"),
                note: row.get("note"),
                chat_record_json,
            });
    }

    let root = state.export_dir.join(job_id);
    fs::create_dir_all(&root).map_err(|err| format!("create export root failed: {}", err))?;

    let mut files = Vec::new();
    let mut total_rows = 0i64;
    for ((date, label), rows) in grouped {
        if rows.is_empty() {
            continue;
        }
        total_rows += i64::try_from(rows.len()).unwrap_or(0);
        let partition_dir = root.join(format!("decision_date={}/label={}", date, label));
        fs::create_dir_all(&partition_dir)
            .map_err(|err| format!("create export partition dir failed: {}", err))?;
        let file_name = format!("part-{}.parquet", random_hex16());
        let file_path = partition_dir.join(&file_name);

        write_parquet_file(&file_path, &rows)?;

        let bytes = fs::read(&file_path)
            .map_err(|err| format!("read parquet file to hash failed: {}", err))?;
        let rel = file_path
            .strip_prefix(&root)
            .map_err(|err| format!("strip export prefix failed: {}", err))?
            .to_string_lossy()
            .to_string();
        files.push(ExportFileEntry {
            path: rel,
            row_count: rows.len(),
            sha256: hash_bytes(&bytes),
        });
    }

    let manifest = ExportManifest {
        job_id: job_id.to_string(),
        schema_version: SAMPLE_SCHEMA_VERSION,
        created_at: Utc::now(),
        row_count: total_rows,
        files,
        filters: serde_json::to_value(&req).unwrap_or(Value::Null),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| format!("encode export manifest failed: {}", err))?;
    fs::write(root.join("manifest.json"), manifest_bytes)
        .map_err(|err| format!("write export manifest failed: {}", err))?;

    sqlx::query(
        "UPDATE export_jobs SET status='completed', row_count=$2, finished_at=NOW() WHERE job_id=$1",
    )
    .bind(job_id)
    .bind(total_rows)
    .execute(&state.pool)
    .await
    .map_err(|err| format!("mark export completed failed: {}", err))?;

    Ok(())
}

fn write_parquet_file(path: &Path, rows: &[ExportRow]) -> Result<(), String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("sample_id", DataType::Utf8, false),
        Field::new("schema_version", DataType::Int32, false),
        Field::new("label", DataType::Int16, false),
        Field::new("augmentation", DataType::Utf8, false),
        Field::new("base_sample_id", DataType::Utf8, true),
        Field::new("label_source", DataType::Utf8, false),
        Field::new("decision_at_ms", DataType::Int64, false),
        Field::new("review_id", DataType::Utf8, false),
        Field::new("review_code", DataType::Int32, false),
        Field::new("post_id", DataType::Utf8, false),
        Field::new("group_id", DataType::Utf8, false),
        Field::new("sender_id", DataType::Utf8, false),
        Field::new("chat_record_hash", DataType::Utf8, false),
        Field::new("message_count", DataType::Int32, false),
        Field::new("batch_id", DataType::Utf8, false),
        Field::new("excluded", DataType::Boolean, false),
        Field::new("corrected_label", DataType::Int16, true),
        Field::new("note", DataType::Utf8, true),
        Field::new("chat_record_json", DataType::Utf8, false),
    ]));

    let mut sample_id = StringBuilder::new();
    let mut schema_version = Int32Builder::new();
    let mut label = Int16Builder::new();
    let mut augmentation = StringBuilder::new();
    let mut base_sample_id = StringBuilder::new();
    let mut label_source = StringBuilder::new();
    let mut decision_at_ms = Int64Builder::new();
    let mut review_id = StringBuilder::new();
    let mut review_code = Int32Builder::new();
    let mut post_id = StringBuilder::new();
    let mut group_id = StringBuilder::new();
    let mut sender_id = StringBuilder::new();
    let mut chat_record_hash = StringBuilder::new();
    let mut message_count = Int32Builder::new();
    let mut batch_id = StringBuilder::new();
    let mut excluded = BooleanBuilder::new();
    let mut corrected_label = Int16Builder::new();
    let mut note = StringBuilder::new();
    let mut chat_record_json = StringBuilder::new();

    for row in rows {
        sample_id.append_value(&row.sample_id);
        schema_version.append_value(row.schema_version);
        label.append_value(row.label);
        augmentation.append_value(&row.augmentation);
        if let Some(value) = &row.base_sample_id {
            base_sample_id.append_value(value);
        } else {
            base_sample_id.append_null();
        }
        label_source.append_value(&row.label_source);
        decision_at_ms.append_value(row.decision_at_ms);
        review_id.append_value(&row.review_id);
        review_code.append_value(row.review_code);
        post_id.append_value(&row.post_id);
        group_id.append_value(&row.group_id);
        sender_id.append_value(&row.sender_id);
        chat_record_hash.append_value(&row.chat_record_hash);
        message_count.append_value(row.message_count);
        batch_id.append_value(&row.batch_id);
        excluded.append_value(row.excluded);
        if let Some(v) = row.corrected_label {
            corrected_label.append_value(v);
        } else {
            corrected_label.append_null();
        }
        if let Some(v) = &row.note {
            note.append_value(v);
        } else {
            note.append_null();
        }
        chat_record_json.append_value(&row.chat_record_json);
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(sample_id.finish()),
            Arc::new(schema_version.finish()),
            Arc::new(label.finish()),
            Arc::new(augmentation.finish()),
            Arc::new(base_sample_id.finish()),
            Arc::new(label_source.finish()),
            Arc::new(decision_at_ms.finish()),
            Arc::new(review_id.finish()),
            Arc::new(review_code.finish()),
            Arc::new(post_id.finish()),
            Arc::new(group_id.finish()),
            Arc::new(sender_id.finish()),
            Arc::new(chat_record_hash.finish()),
            Arc::new(message_count.finish()),
            Arc::new(batch_id.finish()),
            Arc::new(excluded.finish()),
            Arc::new(corrected_label.finish()),
            Arc::new(note.finish()),
            Arc::new(chat_record_json.finish()),
        ],
    )
    .map_err(|err| format!("build record batch failed: {}", err))?;

    let file =
        fs::File::create(path).map_err(|err| format!("create parquet file failed: {}", err))?;
    let mut writer = ArrowWriter::try_new(file, schema, None)
        .map_err(|err| format!("create parquet writer failed: {}", err))?;
    writer
        .write(&batch)
        .map_err(|err| format!("write parquet batch failed: {}", err))?;
    writer
        .close()
        .map_err(|err| format!("close parquet writer failed: {}", err))?;
    Ok(())
}

fn ms_to_utc_date(ms: i64) -> Result<NaiveDate, String> {
    match Utc.timestamp_millis_opt(ms) {
        LocalResult::Single(dt) => Ok(dt.date_naive()),
        _ => Err(format!("invalid timestamp ms: {}", ms)),
    }
}

async fn list_exports(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "exports.manage") {
        return resp;
    }

    let rows = match sqlx::query(
        "SELECT job_id, status, filters_json, format, output_dir, row_count, error, created_at, started_at, finished_at FROM export_jobs ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("list exports failed: {}", err),
                request_id(&headers),
            )
        }
    };

    let items = rows.into_iter().map(row_to_export_item).collect();
    Json(ListExportsResponse { items }).into_response()
}

async fn get_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "exports.manage") {
        return resp;
    }
    let row = match sqlx::query(
        "SELECT job_id, status, filters_json, format, output_dir, row_count, error, created_at, started_at, finished_at FROM export_jobs WHERE job_id=$1",
    )
    .bind(&job_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("get export failed: {}", err),
                request_id(&headers),
            )
        }
    };

    let Some(row) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "export job not found",
            request_id(&headers),
        );
    };
    Json(row_to_export_item(row)).into_response()
}

async fn get_export_manifest(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "exports.manage") {
        return resp;
    }

    let row = match sqlx::query("SELECT output_dir FROM export_jobs WHERE job_id=$1")
        .bind(&job_id)
        .fetch_optional(&state.pool)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("query export job failed: {}", err),
                request_id(&headers),
            );
        }
    };
    let Some(row) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "export job not found",
            request_id(&headers),
        );
    };
    let output_dir: String = row.get("output_dir");
    let path = PathBuf::from(output_dir).join("manifest.json");
    let bytes = match tokio_fs::read(path).await {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                &format!("manifest not found: {}", err),
                request_id(&headers),
            );
        }
    };

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        Body::from(bytes),
    )
        .into_response()
}

async fn get_export_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((job_id, name)): AxumPath<(String, String)>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "exports.manage") {
        return resp;
    }
    if name.contains('/') || name.contains("..") {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "invalid file name",
            request_id(&headers),
        );
    }

    let row = match sqlx::query("SELECT output_dir FROM export_jobs WHERE job_id=$1")
        .bind(&job_id)
        .fetch_optional(&state.pool)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("query export job failed: {}", err),
                request_id(&headers),
            );
        }
    };
    let Some(row) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "export job not found",
            request_id(&headers),
        );
    };

    let output_dir: String = row.get("output_dir");
    let root = PathBuf::from(output_dir);
    let target = find_file_by_name(&root, &name);
    let Some(path) = target else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "export file not found",
            request_id(&headers),
        );
    };
    let bytes = match tokio_fs::read(path).await {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("read export file failed: {}", err),
                request_id(&headers),
            );
        }
    };

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        Body::from(bytes),
    )
        .into_response()
}

fn find_file_by_name(root: &Path, name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_by_name(&path, name) {
                return Some(found);
            }
            continue;
        }
        if path.file_name().and_then(|v| v.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

async fn create_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
    Json(req): Json<CreateTokenRequest>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "tokens.manage") {
        return resp;
    }

    let allowed = vec![
        "ingest.write",
        "batches.read",
        "samples.read",
        "samples.write",
        "exports.manage",
        "tokens.manage",
    ];
    let mut perms = BTreeSet::new();
    for p in req.permissions {
        if !allowed.contains(&p.as_str()) {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("unsupported permission: {}", p),
                request_id(&headers),
            );
        }
        perms.insert(p);
    }
    if perms.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "permissions cannot be empty",
            request_id(&headers),
        );
    }

    let token_id = req
        .token_id
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| format!("tok_{}", random_hex8()));
    if token_id == "root" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "token_id root is reserved",
            request_id(&headers),
        );
    }

    let token = format!("t_{}{}", random_hex16(), random_hex16());
    let token_hash = hash_token(&token);
    let permissions_json = match serde_json::to_string(&perms.iter().cloned().collect::<Vec<_>>()) {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                &format!("encode permissions failed: {}", err),
                request_id(&headers),
            );
        }
    };

    let insert = sqlx::query(
        "INSERT INTO api_tokens(token_id, token_hash, permissions_json, expires_at, note) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(&token_id)
    .bind(token_hash)
    .bind(permissions_json)
    .bind(req.expires_at)
    .bind(req.note.clone())
    .execute(&state.pool)
    .await;

    if let Err(err) = insert {
        if is_unique_violation(&err) {
            return error_response(
                StatusCode::CONFLICT,
                "CONFLICT",
                "token_id already exists",
                request_id(&headers),
            );
        }
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("insert token failed: {}", err),
            request_id(&headers),
        );
    }

    Json(CreateTokenResponse {
        token_id,
        token,
        permissions: perms.into_iter().collect(),
        expires_at: req.expires_at,
        note: req.note,
    })
    .into_response()
}

async fn delete_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(DeleteTokenPath { token_id }): AxumPath<DeleteTokenPath>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthContext>,
) -> Response {
    if let Err(resp) = require_permission(&headers, &auth, "tokens.manage") {
        return resp;
    }
    if token_id == "root" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "cannot delete root token",
            request_id(&headers),
        );
    }

    let result = sqlx::query("DELETE FROM api_tokens WHERE token_id=$1")
        .bind(token_id)
        .execute(&state.pool)
        .await;
    match result {
        Ok(done) => {
            if done.rows_affected() == 0 {
                return error_response(
                    StatusCode::NOT_FOUND,
                    "NOT_FOUND",
                    "token not found",
                    request_id(&headers),
                );
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            &format!("delete token failed: {}", err),
            request_id(&headers),
        ),
    }
}

fn row_to_export_item(row: sqlx::postgres::PgRow) -> ExportJobItem {
    let filters_json: String = row.get("filters_json");
    ExportJobItem {
        job_id: row.get("job_id"),
        status: row.get("status"),
        filters_json: serde_json::from_str(&filters_json).unwrap_or(Value::Null),
        format: row.get("format"),
        output_dir: row.get("output_dir"),
        row_count: row.get("row_count"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}

fn row_to_sample_item(row: &sqlx::postgres::PgRow) -> SampleItem {
    SampleItem {
        id: row.get("id"),
        sample_id: row.get("sample_id"),
        schema_version: row.get("schema_version"),
        label: row.get("label"),
        augmentation: row.get("augmentation"),
        base_sample_id: row.get("base_sample_id"),
        label_source: row.get("label_source"),
        decision_at_ms: row.get("decision_at_ms"),
        review_id: row.get("review_id"),
        review_code: row.get("review_code"),
        post_id: row.get("post_id"),
        group_id: row.get("group_id"),
        sender_id: row.get("sender_id"),
        chat_record_hash: row.get("chat_record_hash"),
        message_count: row.get("message_count"),
        batch_id: row.get("batch_id"),
        excluded: row.get("excluded"),
        corrected_label: row.get("corrected_label"),
        note: row.get("note"),
        ingested_at: row.get("ingested_at"),
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => db.code().as_deref() == Some("23505"),
        _ => false,
    }
}

fn hash_chat_record(chat_record: &ChatRecord) -> String {
    let bytes = serde_json::to_vec(chat_record).unwrap_or_default();
    hash_bytes(&bytes)
}

fn hash_token(token: &str) -> String {
    hash_bytes(token.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn random_hex32() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn random_hex16() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn random_hex8() -> String {
    let mut bytes = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get("Authorization")?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        return None;
    }
    Some(token)
}

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &str,
    request_id: String,
) -> Response {
    (
        status,
        Json(ApiError {
            error: ApiErrorBody {
                code,
                message: message.to_string(),
                request_id,
            },
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_record() -> ChatRecord {
        ChatRecord {
            messages: vec![ChatMessage {
                ingress_id: "1".to_string(),
                platform_msg_id: "m1".to_string(),
                received_at_ms: 1700000000000,
                text: "hello".to_string(),
                attachments: vec![ChatAttachment {
                    kind: "image".to_string(),
                    name: Some("a.png".to_string()),
                    reference_type: "blob_id".to_string(),
                    reference: "abc".to_string(),
                    size_bytes: Some(123),
                }],
            }],
        }
    }

    #[test]
    fn hash_chat_record_is_stable() {
        let rec = sample_record();
        let h1 = hash_chat_record(&rec);
        let h2 = hash_chat_record(&rec);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn ms_to_utc_date_checks_range() {
        assert!(ms_to_utc_date(0).is_ok());
        assert!(ms_to_utc_date(i64::MAX).is_err());
    }

    #[test]
    fn find_file_by_name_walks_subdirs() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("now")
            .as_nanos();
        let root = env::temp_dir().join(format!("collector_test_{}", suffix));
        let nested = root.join("a").join("b");
        fs::create_dir_all(&nested).expect("create nested");
        let target = nested.join("part-1.parquet");
        fs::write(&target, b"ok").expect("write file");

        let found = find_file_by_name(&root, "part-1.parquet");
        assert_eq!(found.as_deref(), Some(target.as_path()));

        let _ = fs::remove_dir_all(root);
    }
}
