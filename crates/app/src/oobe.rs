use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, oneshot};

use crate::config::AppConfig;

const DEFAULT_OOBE_PORT: u16 = 5000;
const DEFAULT_GROUP_ID: &str = "default";
const DEFAULT_NAPCAT_BASE_URL: &str = "0.0.0.0:3001/oqqwall/ws";
const DEFAULT_WEBVIEW_HOST: &str = "0.0.0.0";
const DEFAULT_WEBVIEW_PORT: u16 = 5999;
const DEFAULT_PROCESS_WAITTIME_SEC: u64 = 20;
const DEFAULT_TZ_OFFSET_MINUTES: i64 = 480;
const DEFAULT_MAX_CACHE_MB: u64 = 256;
const DEFAULT_MAX_POST_STACK: u64 = 1;
const DEFAULT_MAX_IMAGE_NUMBER_ONE_POST: u64 = 30;
const DEFAULT_WEBVIEW_SESSION_TTL_SEC: u64 = 12 * 60 * 60;
const OOBE_PAGE_HTML: &str = include_str!("oobe_page.html");

#[derive(Debug, Clone)]
struct OobeCliOptions {
    config_path: String,
    force: bool,
}

#[derive(Debug, Clone)]
struct OobeLaunchOptions {
    config_path: String,
    force_overwrite: bool,
    invalid_reason: Option<String>,
}

#[derive(Clone)]
struct OobeState {
    config_path: String,
    listen_port: u16,
    overwrite_required: bool,
    invalid_reason: Option<String>,
    completion_tx: Arc<Mutex<Option<oneshot::Sender<OobeCompletion>>>>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Debug, Clone)]
struct OobeCompletion {
    config_path: String,
    review_url: String,
    reverse_ws_urls: Vec<ReverseWsHint>,
}

#[derive(Debug, Clone, Serialize)]
struct ReverseWsHint {
    account_id: String,
    url: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct OobeMetaResponse {
    config_path: String,
    listen_port: u16,
    overwrite_required: bool,
    invalid_reason: Option<String>,
    defaults: OobeDefaults,
}

#[derive(Debug, Serialize)]
struct OobeDefaults {
    group_id: &'static str,
    napcat_base_url: &'static str,
    webview_host: &'static str,
    webview_port: u16,
    process_waittime_sec: u64,
    tz_offset_minutes: i64,
    max_cache_mb: u64,
    max_post_stack: u64,
    max_image_number_one_post: u64,
    individual_image_in_posts: bool,
    at_unprived_sender: bool,
}

#[derive(Debug, Deserialize)]
struct OobeSubmitRequest {
    #[serde(default)]
    confirm_overwrite: bool,
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    audit_group_id: String,
    #[serde(default)]
    accounts_text: String,
    #[serde(default)]
    napcat_base_url: String,
    #[serde(default)]
    napcat_access_token: String,
    process_waittime_sec: u64,
    tz_offset_minutes: i64,
    max_cache_mb: u64,
    at_unprived_sender: bool,
    max_post_stack: u64,
    max_image_number_one_post: u64,
    individual_image_in_posts: bool,
    #[serde(default)]
    send_schedule_text: String,
    #[serde(default)]
    webview_host: String,
    webview_port: u16,
    #[serde(default)]
    admin_username: String,
    #[serde(default)]
    admin_password: String,
}

#[derive(Debug, Serialize)]
struct OobeSubmitResponse {
    message: String,
    config_path: String,
    review_url: String,
    reverse_ws_urls: Vec<ReverseWsHint>,
}

#[derive(Debug, Serialize)]
struct OobeApiError {
    error: OobeApiErrorBody,
}

#[derive(Debug, Serialize)]
struct OobeApiErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug, Clone)]
struct OobeConfigDraft {
    group_id: String,
    audit_group_id: String,
    account_ids: Vec<String>,
    napcat_base_url: String,
    napcat_access_token: String,
    process_waittime_sec: u64,
    tz_offset_minutes: i64,
    max_cache_mb: u64,
    at_unprived_sender: bool,
    max_post_stack: u64,
    max_image_number_one_post: u64,
    individual_image_in_posts: bool,
    send_schedule: Vec<String>,
    webview_host: String,
    webview_port: u16,
    admin_username: String,
    admin_password_hash: String,
}

pub async fn run(args: &[String]) -> Result<(), String> {
    let Some(options) = parse_oobe_args(args)? else {
        return Ok(());
    };
    let completion = launch_web_oobe(OobeLaunchOptions {
        config_path: options.config_path,
        force_overwrite: options.force,
        invalid_reason: None,
    })
    .await?;
    print_completion(&completion);
    Ok(())
}

pub async fn run_auto(config_path: String, invalid_reason: Option<String>) -> Result<(), String> {
    let completion = launch_web_oobe(OobeLaunchOptions {
        config_path,
        force_overwrite: true,
        invalid_reason,
    })
    .await?;
    print_completion(&completion);
    Ok(())
}

fn parse_oobe_args(args: &[String]) -> Result<Option<OobeCliOptions>, String> {
    let mut config_path = env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string());
    let mut force = false;

    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "oobe" | "--oobe" => {}
            "--config" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "missing value for --config".to_string())?;
                config_path = value.to_string();
            }
            "--force" => {
                force = true;
            }
            "-h" | "--help" => {
                print_oobe_help();
                return Ok(None);
            }
            other => {
                return Err(format!("unknown oobe argument: {}", other));
            }
        }
    }

    Ok(Some(OobeCliOptions { config_path, force }))
}

async fn launch_web_oobe(options: OobeLaunchOptions) -> Result<OobeCompletion, String> {
    let config_exists = Path::new(&options.config_path).exists();
    let listener = bind_first_available(DEFAULT_OOBE_PORT).await?;
    let listen_port = listener
        .local_addr()
        .map_err(|err| format!("failed to inspect OOBE listener: {}", err))?
        .port();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let state = OobeState {
        config_path: options.config_path.clone(),
        listen_port,
        overwrite_required: config_exists && !options.force_overwrite,
        invalid_reason: options.invalid_reason.clone(),
        completion_tx: Arc::new(Mutex::new(Some(completion_tx))),
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
    };

    let app = Router::new()
        .route("/", get(oobe_index))
        .route("/api/meta", get(oobe_meta))
        .route("/api/config", post(oobe_submit))
        .with_state(state);

    println!();
    println!("OQQWall Web OOBE is ready.");
    println!("OOBE URL: http://127.0.0.1:{}/", listen_port);
    println!(
        "This temporary setup page will stay online until config '{}' is saved.",
        options.config_path
    );
    if let Some(reason) = &options.invalid_reason {
        println!("Current config is invalid and will be replaced after setup:");
        println!("{}", reason);
    } else if config_exists && !options.force_overwrite {
        println!(
            "An existing config file was detected. Confirm overwrite in the WebUI to continue."
        );
    }
    println!();

    let server = tokio::spawn(async move {
        let shutdown = async {
            let _ = shutdown_rx.await;
        };
        if let Err(err) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
        {
            eprintln!("OOBE server stopped with error: {}", err);
        }
    });

    let completion = completion_rx
        .await
        .map_err(|_| "OOBE exited before a config was submitted".to_string())?;
    let _ = server
        .await
        .map_err(|err| format!("OOBE server join failed: {}", err))?;
    Ok(completion)
}

async fn bind_first_available(start_port: u16) -> Result<tokio::net::TcpListener, String> {
    let mut last_err: Option<String> = None;
    for port in start_port..=u16::MAX {
        match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => return Ok(listener),
            Err(err) => {
                last_err = Some(err.to_string());
            }
        }
    }
    Err(format!(
        "failed to bind an OOBE port at or above {}: {}",
        start_port,
        last_err.unwrap_or_else(|| "no free port found".to_string())
    ))
}

async fn oobe_index() -> Html<&'static str> {
    Html(OOBE_PAGE_HTML)
}

async fn oobe_meta(State(state): State<OobeState>) -> Json<OobeMetaResponse> {
    Json(OobeMetaResponse {
        config_path: state.config_path,
        listen_port: state.listen_port,
        overwrite_required: state.overwrite_required,
        invalid_reason: state.invalid_reason,
        defaults: OobeDefaults {
            group_id: DEFAULT_GROUP_ID,
            napcat_base_url: DEFAULT_NAPCAT_BASE_URL,
            webview_host: DEFAULT_WEBVIEW_HOST,
            webview_port: DEFAULT_WEBVIEW_PORT,
            process_waittime_sec: DEFAULT_PROCESS_WAITTIME_SEC,
            tz_offset_minutes: DEFAULT_TZ_OFFSET_MINUTES,
            max_cache_mb: DEFAULT_MAX_CACHE_MB,
            max_post_stack: DEFAULT_MAX_POST_STACK,
            max_image_number_one_post: DEFAULT_MAX_IMAGE_NUMBER_ONE_POST,
            individual_image_in_posts: true,
            at_unprived_sender: false,
        },
    })
}

async fn oobe_submit(
    State(state): State<OobeState>,
    Json(req): Json<OobeSubmitRequest>,
) -> impl IntoResponse {
    if state.overwrite_required && !req.confirm_overwrite {
        return oobe_error(
            StatusCode::BAD_REQUEST,
            "OVERWRITE_REQUIRED",
            "existing config detected; confirm overwrite before continuing",
        );
    }

    let draft = match OobeConfigDraft::from_submit(req) {
        Ok(draft) => draft,
        Err(err) => {
            return oobe_error(StatusCode::BAD_REQUEST, "VALIDATION", &err);
        }
    };
    let root = draft.to_config_value();
    if let Err(err) = AppConfig::from_value(&root) {
        return oobe_error(StatusCode::BAD_REQUEST, "CONFIG_INVALID", &err);
    }
    if let Err(err) = write_config_file(&state.config_path, &root) {
        return oobe_error(StatusCode::INTERNAL_SERVER_ERROR, "WRITE_FAILED", &err);
    }

    let completion = draft.to_completion(&state.config_path);
    if let Some(tx) = state.completion_tx.lock().await.take() {
        let _ = tx.send(completion.clone());
    }
    if let Some(tx) = state.shutdown_tx.lock().await.take() {
        let _ = tx.send(());
    }

    (
        StatusCode::OK,
        Json(OobeSubmitResponse {
            message: "Config saved. OQQWall will continue startup now.".to_string(),
            config_path: completion.config_path,
            review_url: completion.review_url,
            reverse_ws_urls: completion.reverse_ws_urls,
        }),
    )
        .into_response()
}

impl OobeConfigDraft {
    fn from_submit(req: OobeSubmitRequest) -> Result<Self, String> {
        let group_id = trim_required(&req.group_id, "group_id")?;
        let audit_group_id = trim_required(&req.audit_group_id, "audit_group_id")?;
        if !is_numeric(&audit_group_id) {
            return Err("audit_group_id must contain only digits".to_string());
        }

        let account_ids = parse_numeric_list(&req.accounts_text, "accounts")?;
        if account_ids.is_empty() {
            return Err("accounts must include at least one QQ account id".to_string());
        }

        let napcat_base_url = trim_required(&req.napcat_base_url, "napcat_base_url")?;
        let napcat_access_token =
            nonempty(req.napcat_access_token.trim()).unwrap_or_else(generate_access_token);

        if req.max_post_stack == 0 {
            return Err("max_post_stack must be at least 1".to_string());
        }
        if req.max_image_number_one_post == 0 {
            return Err("max_image_number_one_post must be at least 1".to_string());
        }
        if req.webview_port == 0 {
            return Err("webview_port must be between 1 and 65535".to_string());
        }

        let webview_host = trim_required(&req.webview_host, "webview_host")?;
        let admin_username = trim_required(&req.admin_username, "admin_username")?;
        let admin_password = trim_required(&req.admin_password, "admin_password")?;
        let send_schedule = parse_schedule_entries(&req.send_schedule_text)?;

        Ok(Self {
            group_id,
            audit_group_id,
            account_ids,
            napcat_base_url,
            napcat_access_token,
            process_waittime_sec: req.process_waittime_sec,
            tz_offset_minutes: req.tz_offset_minutes,
            max_cache_mb: req.max_cache_mb,
            at_unprived_sender: req.at_unprived_sender,
            max_post_stack: req.max_post_stack,
            max_image_number_one_post: req.max_image_number_one_post,
            individual_image_in_posts: req.individual_image_in_posts,
            send_schedule,
            webview_host,
            webview_port: req.webview_port,
            admin_username,
            admin_password_hash: hash_password(&admin_password),
        })
    }

    fn to_config_value(&self) -> Value {
        let send_schedule = if self.send_schedule.is_empty() {
            Value::Array(Vec::new())
        } else {
            Value::Array(
                self.send_schedule
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            )
        };
        json!({
            "schema_version": 1,
            "common": {
                "process_waittime_sec": self.process_waittime_sec,
                "tz_offset_minutes": self.tz_offset_minutes,
                "max_cache_mb": self.max_cache_mb,
                "at_unprived_sender": self.at_unprived_sender,
                "max_post_stack": self.max_post_stack,
                "max_image_number_one_post": self.max_image_number_one_post,
                "webview": {
                    "enabled": true,
                    "host": self.webview_host,
                    "port": self.webview_port,
                    "session_ttl_sec": DEFAULT_WEBVIEW_SESSION_TTL_SEC,
                },
            },
            "groups": {
                self.group_id.clone(): {
                    "mangroupid": self.audit_group_id,
                    "accounts": self.account_ids,
                    "napcat_base_url": self.napcat_base_url,
                    "napcat_access_token": self.napcat_access_token,
                    "max_post_stack": self.max_post_stack,
                    "max_image_number_one_post": self.max_image_number_one_post,
                    "individual_image_in_posts": self.individual_image_in_posts,
                    "send_schedule": send_schedule,
                }
            },
            "webview_global_admins": [
                {
                    "username": self.admin_username,
                    "password": self.admin_password_hash,
                    "role": "global_admin"
                }
            ]
        })
    }

    fn to_completion(&self, config_path: &str) -> OobeCompletion {
        let token = self.napcat_access_token.clone();
        let reverse_ws_urls = self
            .account_ids
            .iter()
            .map(|account_id| ReverseWsHint {
                account_id: account_id.clone(),
                url: build_reverse_ws_url(&self.napcat_base_url, account_id),
                token: token.clone(),
            })
            .collect();
        OobeCompletion {
            config_path: config_path.to_string(),
            review_url: build_review_url(&self.webview_host, self.webview_port),
            reverse_ws_urls,
        }
    }
}

fn trim_required(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{} is required", field_name));
    }
    Ok(trimmed.to_string())
}

fn parse_numeric_list(input: &str, field_name: &str) -> Result<Vec<String>, String> {
    let values = split_text_entries(input);
    if let Some(invalid) = values.iter().find(|value| !is_numeric(value)) {
        return Err(format!(
            "{} contains a non-numeric value: {}",
            field_name, invalid
        ));
    }
    Ok(values)
}

fn split_text_entries(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in input.split([',', '\n', '\r', ';']) {
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = trimmed.to_string();
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn parse_schedule_entries(input: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for piece in split_text_entries(input) {
        if !is_valid_schedule(&piece) {
            return Err(format!(
                "send_schedule contains an invalid time: {} (expected HH:MM)",
                piece
            ));
        }
        out.push(piece);
    }
    Ok(out)
}

fn is_valid_schedule(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    if hour.len() != 2 || minute.len() != 2 {
        return false;
    }
    let Ok(hour) = hour.parse::<u8>() else {
        return false;
    };
    let Ok(minute) = minute.parse::<u8>() else {
        return false;
    };
    hour < 24 && minute < 60
}

fn build_reverse_ws_url(base_url: &str, account_id: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let base = if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        format!("ws://{}", base)
    };
    format!("{}/{}", base, account_id)
}

fn build_review_url(host: &str, port: u16) -> String {
    let display_host = match host.trim() {
        "" | "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    };
    format!("http://{}:{}/", display_host, port)
}

fn generate_access_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        let idx = (byte as usize) % CHARSET.len();
        out.push(CHARSET[idx] as char);
    }
    out
}

fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.trim().as_bytes());
    let digest = hasher.finalize();
    format!("sha256:{:x}", digest)
}

fn write_config_file(path: &str, root: &Value) -> Result<(), String> {
    let path_ref = Path::new(path);
    if let Some(parent) = path_ref.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create config directory {}: {}",
                    parent.display(),
                    err
                )
            })?;
        }
    }
    let mut output =
        serde_json::to_string_pretty(root).map_err(|err| format!("json error: {}", err))?;
    output.push('\n');
    fs::write(path_ref, output).map_err(|err| format!("failed to write {}: {}", path, err))
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

fn is_numeric(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn print_oobe_help() {
    println!("OQQWall_RUST oobe");
    println!();
    println!("Usage:");
    println!("  OQQWall_RUST oobe [--config <path>] [--force]");
    println!();
    println!("Options:");
    println!("  --config <path>  Config path (default: $OQQWALL_CONFIG or ./config.json)");
    println!("  --force          Allow overwriting an existing config without extra confirmation");
}

fn print_completion(completion: &OobeCompletion) {
    println!("Config saved to '{}'.", completion.config_path);
    println!("Review panel URL: {}", completion.review_url);
    if !completion.reverse_ws_urls.is_empty() {
        println!("NapCat reverse WS targets:");
        for entry in &completion.reverse_ws_urls {
            println!(
                "  account {} -> {} (token: {})",
                entry.account_id, entry.url, entry.token
            );
        }
    }
    println!();
}

fn oobe_error(status: StatusCode, code: &'static str, message: &str) -> axum::response::Response {
    (
        status,
        Json(OobeApiError {
            error: OobeApiErrorBody {
                code,
                message: message.to_string(),
            },
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{
        OobeConfigDraft, build_reverse_ws_url, generate_access_token, hash_password,
        is_valid_schedule, parse_schedule_entries, split_text_entries,
    };
    use crate::config::AppConfig;

    #[test]
    fn split_text_entries_supports_commas_and_newlines() {
        let values = split_text_entries("10001, 10002\n10003;10002\r\n10004");
        assert_eq!(values, vec!["10001", "10002", "10003", "10004"]);
    }

    #[test]
    fn parse_schedule_entries_rejects_invalid_time() {
        let err = parse_schedule_entries("08:30,25:00").expect_err("expected invalid schedule");
        assert!(err.contains("25:00"));
    }

    #[test]
    fn is_valid_schedule_accepts_hh_mm() {
        assert!(is_valid_schedule("00:00"));
        assert!(is_valid_schedule("23:59"));
        assert!(!is_valid_schedule("7:00"));
        assert!(!is_valid_schedule("24:00"));
    }

    #[test]
    fn draft_produces_valid_config_with_webview_port() {
        let draft = OobeConfigDraft {
            group_id: "bf".to_string(),
            audit_group_id: "1063478963".to_string(),
            account_ids: vec!["2741312979".to_string()],
            napcat_base_url: "0.0.0.0:3001/oqqwall/ws".to_string(),
            napcat_access_token: "token-123".to_string(),
            process_waittime_sec: 20,
            tz_offset_minutes: 480,
            max_cache_mb: 256,
            at_unprived_sender: false,
            max_post_stack: 4,
            max_image_number_one_post: 30,
            individual_image_in_posts: true,
            send_schedule: vec!["08:30".to_string()],
            webview_host: "0.0.0.0".to_string(),
            webview_port: 5999,
            admin_username: "admin".to_string(),
            admin_password_hash: hash_password("secret"),
        };
        let root = draft.to_config_value();
        let config = AppConfig::from_value(&root).expect("config should validate");
        assert!(config.webview_enabled);
        assert_eq!(config.webview_port, 5999);
        assert_eq!(config.webview_admins.len(), 1);
    }

    #[test]
    fn build_reverse_ws_url_adds_ws_scheme() {
        let url = build_reverse_ws_url("127.0.0.1:3001/oqqwall/ws", "12345");
        assert_eq!(url, "ws://127.0.0.1:3001/oqqwall/ws/12345");
    }

    #[test]
    fn generate_access_token_uses_expected_length() {
        let token = generate_access_token();
        assert_eq!(token.len(), 24);
    }
}
