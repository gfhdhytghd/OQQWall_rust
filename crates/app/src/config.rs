use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use oqqwall_rust_core::{CoreConfig, GroupConfig};
use oqqwall_rust_drivers::napcat::NapCatConfig;
use oqqwall_rust_drivers::renderer::{DEFAULT_CANVAS_WIDTH_PX, DEFAULT_MAX_HEIGHT_PX};
use oqqwall_rust_drivers::shortcut::{
    is_builtin_review_command_name, validate_global_shortcut_definition,
    validate_review_shortcut_definition, validate_shortcut_name,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

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

const BUILTIN_TELEMETRY_ENDPOINT_KEY: &[u8] = b"oqqwall-telemetry";
const BUILTIN_TELEMETRY_ENDPOINT_DATA: &[u8] = &[
    0x07, 0x05, 0x05, 0x07, 0x5b, 0x43, 0x43, 0x5a, 0x1d, 0x09, 0x0a, 0x09, 0x04, 0x0b, 0x5a, 0x11,
    0x16, 0x02, 0x4b, 0x40, 0x47, 0x58, 0x5e, 0x59, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x40, 0x07, 0x40, 0x58, 0x12, 0x19, 0x0e, 0x40, 0x1d, 0x16, 0x1f, 0x0c, 0x02, 0x0b,
    0x5b, 0x10, 0x18, 0x1b, 0x12, 0x19,
];
const BUILTIN_TELEMETRY_TOKEN_KEY: &[u8] = b"submission-uploader";
const BUILTIN_TELEMETRY_TOKEN_DATA: &[u8] = &[
    0x07, 0x2a, 0x56, 0x55, 0x0d, 0x11, 0x46, 0x0d, 0x0e, 0x57, 0x18, 0x45, 0x11, 0x5a, 0x0a, 0x50,
    0x56, 0x03, 0x45, 0x4a, 0x42, 0x51, 0x5f, 0x5c, 0x41, 0x11, 0x51, 0x58, 0x0b, 0x4e, 0x47, 0x16,
    0x5a, 0x5c,
];

#[derive(Debug, Clone)]
pub struct AppGroupConfig {
    pub group_id: String,
    pub audit_group_id: Option<String>,
    pub napcat: NapCatConfig,
    pub friend_request_window_sec: u32,
    pub friend_add_message: Option<String>,
    pub individual_image_in_posts: bool,
    pub watermark_text: Option<String>,
    pub quick_replies: HashMap<String, String>,
    pub review_shortcuts: HashMap<String, String>,
    pub global_shortcuts: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebviewRole {
    GlobalAdmin,
    GroupAdmin,
}

#[derive(Debug, Clone)]
pub struct WebviewAdminAccount {
    pub username: String,
    pub password_hash: String,
    pub role: WebviewRole,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub local_dir: String,
    pub upload_enabled: bool,
    pub upload_endpoint: Option<String>,
    pub upload_token: Option<String>,
    pub upload_interval_sec: u64,
    pub upload_batch_size: usize,
    pub max_append_messages: usize,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub groups: Vec<AppGroupConfig>,
    pub tz_offset_minutes: i32,
    pub fallback_napcat: Option<NapCatConfig>,
    pub max_cache_mb: u64,
    pub at_unprived_sender: bool,
    pub web_api_enabled: bool,
    pub web_api_port: u16,
    pub web_api_root_token: Option<String>,
    pub webview_enabled: bool,
    pub webview_host: String,
    pub webview_port: u16,
    pub webview_session_ttl_sec: i64,
    pub webview_admins: Vec<WebviewAdminAccount>,
    pub renderer_canvas_width_px: u32,
    pub renderer_max_height_px: u32,
    pub telemetry: TelemetryConfig,
    core_config: CoreConfig,
    #[cfg(debug_assertions)]
    pub dev_config: DevConfig,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Default)]
pub struct DevConfig {
    pub use_virt_qzone: bool,
}

impl AppConfig {
    pub fn load() -> Result<Self, String> {
        let path = resolve_config_path();
        debug_log!("loading config path={}", path);
        let data = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read config {}: {}", path, err))?;
        debug_log!("config bytes={}", data.len());
        let mut root: Value =
            serde_json::from_str(&data).map_err(|err| format!("invalid config json: {}", err))?;
        if normalize_config_in_place(&mut root)? {
            write_normalized_config(&path, &root)?;
        }
        Self::from_value(&root)
    }

    pub fn build_core_config(&self) -> CoreConfig {
        self.core_config.clone()
    }

    fn from_value(root: &Value) -> Result<Self, String> {
        let root_obj = root
            .as_object()
            .ok_or_else(|| "config must be a json object".to_string())?;
        let (common, groups) = split_config(root)?;
        let default_process_waittime_ms = env::var("OQQWALL_PROCESS_WAITTIME_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .or_else(|| {
                parse_duration_ms(common.get("process_waittime_sec"))
                    .map(|v| v.saturating_mul(1000))
            })
            .unwrap_or(20_000);

        let tz_offset_minutes = parse_i64(common.get("tz_offset_minutes")).unwrap_or(0) as i32;
        let max_cache_mb = env::var("OQQWALL_MAX_CACHE_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| parse_u64(common.get("max_cache_mb")))
            .unwrap_or(256);
        let at_unprived_sender = parse_bool(common.get("at_unprived_sender")).unwrap_or(false);
        let default_friend_request_window_sec =
            parse_u32(common.get("friend_request_window_sec")).unwrap_or(300);
        let default_friend_add_message = parse_string(common.get("friend_add_message"));
        let common_web_api = common.get("web_api").and_then(|value| value.as_object());
        let common_webview = common.get("webview").and_then(|value| value.as_object());
        let common_renderer = common.get("renderer").and_then(|value| value.as_object());
        let common_telemetry = common.get("telemetry").and_then(|value| value.as_object());
        let web_api_enabled = common_web_api
            .and_then(|obj| parse_bool(obj.get("enabled")))
            .unwrap_or(false);
        let web_api_port = common_web_api
            .and_then(|obj| parse_u32(obj.get("port")))
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(10923);
        let web_api_root_token = env_override(
            "OQQWALL_API_TOKEN",
            common_web_api
                .and_then(|obj| parse_string(obj.get("root_token")))
                .or_else(|| parse_string(common.get("root_token"))),
        )
        .and_then(|value| nonempty(Some(value)));
        let webview_enabled = common_webview
            .and_then(|obj| parse_bool(obj.get("enabled")))
            .unwrap_or(false);
        let webview_host = nonempty(common_webview.and_then(|obj| parse_string(obj.get("host"))))
            .unwrap_or_else(|| "0.0.0.0".to_string());
        let webview_port = common_webview
            .and_then(|obj| parse_u32(obj.get("port")))
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(10924);
        let webview_session_ttl_sec = common_webview
            .and_then(|obj| parse_i64(obj.get("session_ttl_sec")))
            .unwrap_or(12 * 60 * 60)
            .clamp(300, 7 * 24 * 60 * 60);
        let renderer_canvas_width_px = common_renderer
            .and_then(|obj| parse_u32(obj.get("canvas_width_px")))
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CANVAS_WIDTH_PX);
        let renderer_max_height_px = common_renderer
            .and_then(|obj| parse_u32(obj.get("max_height_px")))
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_HEIGHT_PX);
        let telemetry_enabled = common_telemetry
            .and_then(|obj| parse_bool(obj.get("enabled")))
            .unwrap_or(true);
        let telemetry_local_dir =
            nonempty(common_telemetry.and_then(|obj| parse_string(obj.get("local_dir"))))
                .unwrap_or_else(|| "telemetry".to_string());
        let telemetry_upload_enabled = common_telemetry
            .and_then(|obj| parse_bool(obj.get("upload_enabled")))
            .unwrap_or(true);
        let telemetry_upload_endpoint = if telemetry_upload_enabled {
            default_telemetry_upload_endpoint()
        } else {
            None
        };
        let telemetry_upload_token = if telemetry_upload_enabled {
            default_telemetry_upload_token()
        } else {
            None
        };
        let telemetry_upload_interval_sec = common_telemetry
            .and_then(|obj| parse_u64(obj.get("upload_interval_sec")))
            .unwrap_or(30)
            .clamp(1, 86_400);
        let telemetry_upload_batch_size = common_telemetry
            .and_then(|obj| parse_usize(obj.get("upload_batch_size")))
            .unwrap_or(20)
            .clamp(20, 20);
        let telemetry_max_append_messages = common_telemetry
            .and_then(|obj| parse_usize(obj.get("max_append_messages")))
            .unwrap_or(2)
            .clamp(1, 10);
        let telemetry = TelemetryConfig {
            enabled: telemetry_enabled,
            local_dir: telemetry_local_dir,
            upload_enabled: telemetry_upload_enabled,
            upload_endpoint: telemetry_upload_endpoint,
            upload_token: telemetry_upload_token,
            upload_interval_sec: telemetry_upload_interval_sec,
            upload_batch_size: telemetry_upload_batch_size,
            max_append_messages: telemetry_max_append_messages,
        };
        debug_log!(
            "config parsed: tz_offset_minutes={} default_process_waittime_ms={} max_cache_mb={} at_unprived_sender={} web_api_enabled={} web_api_port={} web_api_token_present={} webview_enabled={} webview_host={} webview_port={} webview_admins={} renderer_canvas_width_px={} renderer_max_height_px={} telemetry_enabled={} telemetry_upload_enabled={} telemetry_endpoint_present={}",
            tz_offset_minutes,
            default_process_waittime_ms,
            max_cache_mb,
            at_unprived_sender,
            web_api_enabled,
            web_api_port,
            web_api_root_token.is_some(),
            webview_enabled,
            webview_host,
            webview_port,
            parse_admin_entries(root_obj.get("webview_global_admins")).len(),
            renderer_canvas_width_px,
            renderer_max_height_px,
            telemetry.enabled,
            telemetry.upload_enabled,
            telemetry.upload_endpoint.is_some()
        );
        let core_config = build_core_config(&common, &groups, default_process_waittime_ms);
        let fallback_napcat = parse_napcat_config_optional(&common);

        let mut group_configs = Vec::new();
        let mut grouped_admins: HashMap<String, (String, Vec<String>)> = HashMap::new();
        for (group_id, group_value) in &groups {
            let audit_group_id = parse_string(group_value.get("mangroupid"))
                .ok_or_else(|| format!("group {}: missing required field mangroupid", group_id))?;
            let accounts = parse_accounts(group_value);
            if accounts.is_empty() {
                return Err(format!(
                    "group {}: missing required field accounts",
                    group_id
                ));
            }
            if let Some(invalid) = accounts.iter().find(|value| !is_numeric(value)) {
                return Err(format!(
                    "group {}: accounts contains non-numeric value {}",
                    group_id, invalid
                ));
            }
            let napcat = parse_napcat_config(&common, Some(group_value))
                .map_err(|err| format!("group {}: {}", group_id, err))?;
            let friend_request_window_sec = parse_u32(group_value.get("friend_request_window_sec"))
                .unwrap_or(default_friend_request_window_sec);
            let friend_add_message = parse_string(group_value.get("friend_add_message"))
                .or_else(|| default_friend_add_message.clone());
            let individual_image_in_posts =
                parse_bool(group_value.get("individual_image_in_posts")).unwrap_or(true);
            let watermark_text = nonempty(parse_string(group_value.get("watermark_text")));
            let quick_replies = parse_quick_replies(group_value.get("quick_replies"))
                .map_err(|err| format!("group {}: {}", group_id, err))?;
            let review_shortcuts =
                parse_review_shortcuts(group_value.get("review_shortcuts"), &quick_replies)
                    .map_err(|err| format!("group {}: {}", group_id, err))?;
            let global_shortcuts = parse_global_shortcuts(group_value.get("global_shortcuts"))
                .map_err(|err| format!("group {}: {}", group_id, err))?;
            let _napcat_ws_log = base_url_for_log(&napcat.base_url);
            debug_log!(
                "config group: group_id={} audit_group_id={:?} napcat_base_url={} napcat_token_present={}",
                group_id,
                audit_group_id,
                _napcat_ws_log,
                napcat.access_token.is_some()
            );
            group_configs.push(AppGroupConfig {
                group_id: group_id.clone(),
                audit_group_id: Some(audit_group_id),
                napcat,
                friend_request_window_sec,
                friend_add_message,
                individual_image_in_posts,
                watermark_text,
                quick_replies,
                review_shortcuts,
                global_shortcuts,
            });
            for admin in parse_admin_entries(group_value.get("webview_admins")) {
                let username = admin.username.trim().to_string();
                if username.is_empty() {
                    continue;
                }
                let password_hash = normalize_password_hash(&admin.password);
                grouped_admins
                    .entry(username)
                    .and_modify(|(existing_hash, group_ids)| {
                        if *existing_hash == password_hash && !group_ids.contains(group_id) {
                            group_ids.push(group_id.clone());
                        }
                    })
                    .or_insert_with(|| (password_hash, vec![group_id.clone()]));
            }
        }
        if group_configs.is_empty() {
            let Some(napcat) = fallback_napcat.clone() else {
                return Err("missing groups and napcat_base_url".to_string());
            };
            group_configs.push(AppGroupConfig {
                group_id: "default".to_string(),
                audit_group_id: None,
                napcat,
                friend_request_window_sec: default_friend_request_window_sec,
                friend_add_message: default_friend_add_message,
                individual_image_in_posts: true,
                watermark_text: None,
                quick_replies: HashMap::new(),
                review_shortcuts: HashMap::new(),
                global_shortcuts: HashMap::new(),
            });
        }
        #[cfg(debug_assertions)]
        let dev_config = load_dev_config()?;
        let mut webview_admins = Vec::new();
        for admin in parse_admin_entries(root_obj.get("webview_global_admins")) {
            let username = admin.username.trim().to_string();
            if username.is_empty() {
                continue;
            }
            webview_admins.push(WebviewAdminAccount {
                username,
                password_hash: normalize_password_hash(&admin.password),
                role: WebviewRole::GlobalAdmin,
                groups: Vec::new(),
            });
        }
        for (username, (password_hash, mut group_ids)) in grouped_admins {
            group_ids.sort();
            group_ids.dedup();
            webview_admins.push(WebviewAdminAccount {
                username,
                password_hash,
                role: WebviewRole::GroupAdmin,
                groups: group_ids,
            });
        }
        Ok(Self {
            groups: group_configs,
            tz_offset_minutes,
            fallback_napcat,
            max_cache_mb,
            at_unprived_sender,
            web_api_enabled,
            web_api_port,
            web_api_root_token,
            webview_enabled,
            webview_host,
            webview_port,
            webview_session_ttl_sec,
            webview_admins,
            renderer_canvas_width_px,
            renderer_max_height_px,
            telemetry,
            core_config,
            #[cfg(debug_assertions)]
            dev_config,
        })
    }
}

pub fn resolve_config_path() -> String {
    env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string())
}

pub fn config_exists(path: &str) -> Result<bool, String> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("failed to check config {}: {}", path, err)),
    }
}

#[cfg(debug_assertions)]
fn load_dev_config() -> Result<DevConfig, String> {
    let path = "devconfig.json";
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            debug_log!("dev config missing: {}", path);
            return Ok(DevConfig::default());
        }
        Err(err) => return Err(format!("failed to read dev config {}: {}", path, err)),
    };
    let root: Value =
        serde_json::from_str(&data).map_err(|err| format!("invalid dev config json: {}", err))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "dev config must be a json object".to_string())?;
    let use_virt_qzone = parse_bool(obj.get("use-virt-qzone")).unwrap_or(false);
    debug_log!("dev config parsed: use_virt_qzone={}", use_virt_qzone);
    Ok(DevConfig { use_virt_qzone })
}

fn split_config(root: &Value) -> Result<(Value, std::collections::HashMap<String, Value>), String> {
    let obj = root
        .as_object()
        .ok_or_else(|| "config must be a json object".to_string())?;
    let common = obj.get("common").cloned().unwrap_or(Value::Null);
    if let Some(groups) = obj.get("groups").and_then(|v| v.as_object()) {
        let map = groups.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        return Ok((common, map));
    }
    let mut map = std::collections::HashMap::new();
    for (key, value) in obj {
        if key == "common" || key == "schema_version" {
            continue;
        }
        map.insert(key.clone(), value.clone());
    }
    Ok((common, map))
}

fn parse_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(|v| if v.trim().is_empty() { None } else { Some(v) })
}

fn parse_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(n) => Some(n.as_i64().unwrap_or(0) != 0),
        _ => None,
    }
}

fn parse_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn parse_usize(value: Option<&Value>) -> Option<usize> {
    parse_i64(value).and_then(|v| if v >= 0 { Some(v as usize) } else { None })
}

fn parse_u32(value: Option<&Value>) -> Option<u32> {
    parse_i64(value).and_then(|v| if v >= 0 { u32::try_from(v).ok() } else { None })
}

fn parse_u64(value: Option<&Value>) -> Option<u64> {
    parse_i64(value).and_then(|v| if v >= 0 { u64::try_from(v).ok() } else { None })
}

fn parse_duration_ms(value: Option<&Value>) -> Option<i64> {
    parse_i64(value)
}

fn is_numeric(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn env_override(key: &str, fallback: Option<String>) -> Option<String> {
    env::var(key).ok().or(fallback)
}

fn default_telemetry_upload_endpoint() -> Option<String> {
    decode_obfuscated_ascii(
        BUILTIN_TELEMETRY_ENDPOINT_DATA,
        BUILTIN_TELEMETRY_ENDPOINT_KEY,
    )
}

fn default_telemetry_upload_token() -> Option<String> {
    decode_obfuscated_ascii(BUILTIN_TELEMETRY_TOKEN_DATA, BUILTIN_TELEMETRY_TOKEN_KEY)
}

fn decode_obfuscated_ascii(data: &[u8], key: &[u8]) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let decoded = data
        .iter()
        .enumerate()
        .map(|(idx, byte)| byte ^ key[idx % key.len()])
        .collect::<Vec<_>>();
    String::from_utf8(decoded)
        .ok()
        .and_then(|value| nonempty(Some(value)))
}

fn build_core_config(
    common: &Value,
    groups: &HashMap<String, Value>,
    default_process_waittime_ms: i64,
) -> CoreConfig {
    let mut core = CoreConfig::default();
    core.default_process_waittime_ms = default_process_waittime_ms;
    core.default_min_interval_ms = parse_duration_ms(common.get("min_interval_ms")).unwrap_or(0);
    core.default_max_queue = 1;
    core.default_max_images_per_post =
        parse_usize(common.get("max_image_number_one_post")).unwrap_or(30);
    core.default_send_timeout_ms =
        parse_duration_ms(common.get("send_timeout_ms")).unwrap_or(300_000);
    core.default_send_max_attempts = parse_u32(common.get("send_max_attempts")).unwrap_or(3);

    for (group_id, value) in groups {
        let process_waittime_ms =
            parse_duration_ms(value.get("process_waittime_sec")).map(|v| v.saturating_mul(1000));

        let min_interval_ms = parse_duration_ms(value.get("min_interval_ms"));
        let max_queue = parse_usize(value.get("max_post_stack"));
        let max_images_per_post = parse_usize(value.get("max_image_number_one_post"));
        let send_timeout_ms = parse_duration_ms(value.get("send_timeout_ms"));
        let send_max_attempts = parse_u32(value.get("send_max_attempts"));
        let send_schedule_minutes = parse_schedule_minutes(value.get("send_schedule"));
        let accounts = parse_accounts(value);

        core.groups.insert(
            group_id.clone(),
            GroupConfig {
                group_id: group_id.clone(),
                process_waittime_ms,
                send_windows: Vec::new(),
                min_interval_ms,
                max_queue,
                max_images_per_post,
                send_schedule_minutes,
                accounts,
                send_timeout_ms,
                send_max_attempts,
            },
        );
    }

    if core.groups.is_empty() {
        let default_group_id = "default".to_string();
        core.groups.insert(
            default_group_id.clone(),
            GroupConfig {
                group_id: default_group_id,
                process_waittime_ms: Some(default_process_waittime_ms),
                send_timeout_ms: None,
                send_max_attempts: None,
                ..Default::default()
            },
        );
    }

    core
}

fn parse_accounts(value: &Value) -> Vec<String> {
    normalize_account_ids(parse_string_list(value.get("accounts")))
}

fn normalize_account_ids(ids: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_string();
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

fn normalize_config_in_place(root: &mut Value) -> Result<bool, String> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "config must be a json object".to_string())?;
    let mut changed = false;
    if let Some(common_obj) = obj
        .get_mut("common")
        .and_then(|value| value.as_object_mut())
    {
        if normalize_common_web(common_obj) {
            changed = true;
        }
        if normalize_common_renderer(common_obj) {
            changed = true;
        }
        if normalize_common_unsupported(common_obj) {
            changed = true;
        }
    }
    if normalize_global_admins(obj) {
        changed = true;
    }
    if let Some(groups) = obj
        .get_mut("groups")
        .and_then(|value| value.as_object_mut())
    {
        for group in groups.values_mut() {
            let Some(group_obj) = group.as_object_mut() else {
                continue;
            };
            if normalize_group_accounts(group_obj) {
                changed = true;
            }
            if normalize_group_webview_admins(group_obj) {
                changed = true;
            }
            if normalize_group_unsupported(group_obj) {
                changed = true;
            }
        }
        return Ok(changed);
    }

    for (key, value) in obj.iter_mut() {
        if key == "common" || key == "schema_version" {
            continue;
        }
        let Some(group_obj) = value.as_object_mut() else {
            continue;
        };
        if normalize_group_accounts(group_obj) {
            changed = true;
        }
        if normalize_group_webview_admins(group_obj) {
            changed = true;
        }
        if normalize_group_unsupported(group_obj) {
            changed = true;
        }
    }
    Ok(changed)
}

fn normalize_common_web(common_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    let mut web_api_obj = common_obj
        .get("web_api")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(value) = common_obj.remove("use_web_review") {
        if !web_api_obj.contains_key("enabled") {
            web_api_obj.insert("enabled".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("web_review_port") {
        if !web_api_obj.contains_key("port") {
            web_api_obj.insert("port".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("api_token") {
        if !web_api_obj.contains_key("root_token") {
            web_api_obj.insert("root_token".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("token") {
        if !web_api_obj.contains_key("root_token") {
            web_api_obj.insert("root_token".to_string(), value);
        }
        changed = true;
    }
    if !web_api_obj.is_empty() {
        if common_obj
            .get("web_api")
            .and_then(|value| value.as_object())
            != Some(&web_api_obj)
        {
            common_obj.insert("web_api".to_string(), Value::Object(web_api_obj));
            changed = true;
        }
    }
    changed
}

fn normalize_common_renderer(common_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    let mut renderer_obj = common_obj
        .get("renderer")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();

    if let Some(value) = common_obj.remove("canvas_width_px") {
        if !renderer_obj.contains_key("canvas_width_px") {
            renderer_obj.insert("canvas_width_px".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("max_height_px") {
        if !renderer_obj.contains_key("max_height_px") {
            renderer_obj.insert("max_height_px".to_string(), value);
        }
        changed = true;
    }
    if !renderer_obj.is_empty()
        && common_obj
            .get("renderer")
            .and_then(|value| value.as_object())
            != Some(&renderer_obj)
    {
        common_obj.insert("renderer".to_string(), Value::Object(renderer_obj));
        changed = true;
    }

    changed
}

fn normalize_common_unsupported(common_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    if normalize_common_telemetry(common_obj) {
        changed = true;
    }
    for key in [
        "manage_napcat_internal",
        "renewcookies_use_napcat",
        "max_attempts_qzone_autologin",
        "force_chromium_no_sandbox",
        "http-serv-port",
        "max_queue",
    ] {
        if common_obj.remove(key).is_some() {
            changed = true;
        }
    }
    changed
}

fn normalize_common_telemetry(common_obj: &mut Map<String, Value>) -> bool {
    let Some(telemetry_obj) = common_obj
        .get_mut("telemetry")
        .and_then(|value| value.as_object_mut())
    else {
        return false;
    };
    let mut changed = false;
    for key in ["upload_endpoint", "upload_token"] {
        if telemetry_obj.remove(key).is_some() {
            changed = true;
        }
    }
    if telemetry_obj.is_empty() {
        common_obj.remove("telemetry");
        changed = true;
    }
    changed
}

fn normalize_group_webview_admins(group_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    if !group_obj.contains_key("webview_admins") {
        if let Some(admins) = group_obj.remove("admins") {
            group_obj.insert("webview_admins".to_string(), admins);
            changed = true;
        }
    } else if group_obj.contains_key("admins") {
        group_obj.remove("admins");
        changed = true;
    }
    let Some(raw_admins) = group_obj.get("webview_admins").cloned() else {
        return changed;
    };
    let normalized = normalize_admin_value(&raw_admins, false);
    if normalized != raw_admins {
        group_obj.insert("webview_admins".to_string(), normalized);
        changed = true;
    }
    changed
}

fn normalize_global_admins(root_obj: &mut Map<String, Value>) -> bool {
    let Some(raw_admins) = root_obj.get("webview_global_admins").cloned() else {
        return false;
    };
    let normalized = normalize_admin_value(&raw_admins, true);
    if normalized == raw_admins {
        return false;
    }
    root_obj.insert("webview_global_admins".to_string(), normalized);
    true
}

fn normalize_admin_value(value: &Value, default_global_role: bool) -> Value {
    let entries = parse_admin_entries(Some(value))
        .into_iter()
        .map(|entry| {
            let role = if entry.role.trim().is_empty() {
                if default_global_role {
                    "global_admin".to_string()
                } else {
                    "group_admin".to_string()
                }
            } else {
                entry.role
            };
            serde_json::json!({
                "username": entry.username,
                "password": normalize_password_hash(&entry.password),
                "role": role
            })
        })
        .collect::<Vec<_>>();
    Value::Array(entries)
}

fn normalize_group_accounts(group_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    if !group_obj.contains_key("accounts") {
        if let Some(alias) = group_obj.remove("acount") {
            group_obj.insert("accounts".to_string(), alias);
            changed = true;
        }
    } else if group_obj.contains_key("acount") {
        group_obj.remove("acount");
        changed = true;
    }

    let mut accounts = parse_string_list(group_obj.get("accounts"));
    if accounts.is_empty() {
        let mut legacy = Vec::new();
        if let Some(main) = parse_string(group_obj.get("mainqqid")) {
            legacy.push(main);
        }
        legacy.extend(parse_string_list(group_obj.get("minorqqid")));
        if !legacy.is_empty() {
            accounts = legacy;
            changed = true;
        }
    }
    let accounts = normalize_account_ids(accounts);
    if !accounts.is_empty() {
        let current = parse_string_list(group_obj.get("accounts"));
        if normalize_account_ids(current) != accounts {
            changed = true;
        }
        group_obj.insert(
            "accounts".to_string(),
            Value::Array(accounts.into_iter().map(Value::String).collect()),
        );
    }

    for key in [
        "mainqqid",
        "minorqqid",
        "mainqq_http_port",
        "minorqq_http_port",
    ] {
        if group_obj.remove(key).is_some() {
            changed = true;
        }
    }

    changed
}

fn normalize_group_unsupported(group_obj: &mut Map<String, Value>) -> bool {
    let _ = group_obj;
    false
}

fn write_normalized_config(path: &str, root: &Value) -> Result<(), String> {
    let mut output = serde_json::to_string_pretty(root)
        .map_err(|err| format!("failed to encode normalized config {}: {}", path, err))?;
    output.push('\n');
    let mut temp = PathBuf::from(path);
    temp.set_extension("tmp");
    {
        let mut file = fs::File::create(&temp).map_err(|err| {
            format!(
                "failed to write normalized config temp {}: {}",
                temp.display(),
                err
            )
        })?;
        file.write_all(output.as_bytes()).map_err(|err| {
            format!(
                "failed to write normalized config temp {}: {}",
                temp.display(),
                err
            )
        })?;
        file.sync_all().map_err(|err| {
            format!(
                "failed to sync normalized config temp {}: {}",
                temp.display(),
                err
            )
        })?;
    }
    fs::rename(&temp, path).map_err(|err| {
        format!(
            "failed to replace config with normalized version {} -> {}: {}",
            temp.display(),
            path,
            err
        )
    })?;
    Ok(())
}

fn parse_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items.iter().filter_map(|v| parse_string(Some(v))).collect(),
        Value::String(s) => s
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect(),
        Value::Number(n) => vec![n.to_string()],
        _ => Vec::new(),
    }
}

fn parse_quick_replies(value: Option<&Value>) -> Result<HashMap<String, String>, String> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let Value::Object(map) = value else {
        return Err("quick_replies must be an object".to_string());
    };
    let mut out = HashMap::new();
    for (raw_key, raw_value) in map {
        let key = raw_key.trim();
        if key.is_empty() {
            return Err("quick_replies contains empty key".to_string());
        }
        if is_builtin_review_command_name(key) {
            return Err(format!(
                "quick_replies key '{}' conflicts with review command",
                key
            ));
        }
        let Some(value_text) = parse_string(Some(raw_value)) else {
            return Err(format!(
                "quick_replies['{}'] must be string",
                raw_key.replace('\'', "\\'")
            ));
        };
        let value_text = value_text.trim();
        if value_text.is_empty() {
            return Err(format!(
                "quick_replies['{}'] must not be empty",
                raw_key.replace('\'', "\\'")
            ));
        }
        out.insert(key.to_string(), value_text.to_string());
    }
    Ok(out)
}

fn parse_review_shortcuts(
    value: Option<&Value>,
    quick_replies: &HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    parse_shortcut_map(
        value,
        validate_review_shortcut_definition,
        Some(quick_replies),
    )
}

fn parse_global_shortcuts(value: Option<&Value>) -> Result<HashMap<String, String>, String> {
    parse_shortcut_map(value, validate_global_shortcut_definition, None)
}

fn parse_shortcut_map(
    value: Option<&Value>,
    validate_definition: fn(&str) -> Result<(), String>,
    quick_replies: Option<&HashMap<String, String>>,
) -> Result<HashMap<String, String>, String> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let Value::Object(map) = value else {
        return Err("shortcut map must be an object".to_string());
    };
    let mut out = HashMap::new();
    for (raw_key, raw_value) in map {
        let key = validate_shortcut_name(raw_key)
            .map_err(|err| format!("shortcut '{}': {}", raw_key, err))?;
        if quick_replies
            .map(|entries| entries.contains_key(&key))
            .unwrap_or(false)
        {
            return Err(format!(
                "review_shortcuts key '{}' conflicts with quick_replies",
                key
            ));
        }
        let Some(value_text) = parse_string(Some(raw_value)) else {
            return Err(format!(
                "shortcut['{}'] must be string",
                raw_key.replace('\'', "\\'")
            ));
        };
        let value_text = value_text.trim();
        if value_text.is_empty() {
            return Err(format!(
                "shortcut['{}'] must not be empty",
                raw_key.replace('\'', "\\'")
            ));
        }
        validate_definition(value_text)
            .map_err(|err| format!("shortcut '{}': {}", raw_key, err))?;
        out.insert(key, value_text.to_string());
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct RawAdminEntry {
    username: String,
    password: String,
    role: String,
}

fn parse_admin_entries(value: Option<&Value>) -> Vec<RawAdminEntry> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let username = parse_string(obj.get("username"))?;
            let password = parse_string(obj.get("password"))?;
            let role = parse_string(obj.get("role")).unwrap_or_default();
            Some(RawAdminEntry {
                username,
                password,
                role,
            })
        })
        .collect()
}

fn normalize_password_hash(password: &str) -> String {
    let trimmed = password.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(existing) = trimmed.strip_prefix("sha256:") {
        let normalized = existing.trim().to_ascii_lowercase();
        if normalized.len() == 64 && normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return format!("sha256:{}", normalized);
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    let digest = hasher.finalize();
    format!("sha256:{:x}", digest)
}

fn parse_schedule_minutes(value: Option<&Value>) -> Vec<u16> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items.iter().filter_map(parse_schedule_item).collect(),
        Value::String(s) => s
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .filter_map(parse_schedule_str)
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_schedule_item(value: &Value) -> Option<u16> {
    match value {
        Value::String(s) => parse_schedule_str(s),
        Value::Number(n) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        _ => None,
    }
}

fn parse_schedule_str(value: &str) -> Option<u16> {
    let mut parts = value.split(':');
    let hour = parts.next()?.trim().parse::<u16>().ok()?;
    let minute = parts.next()?.trim().parse::<u16>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour.saturating_mul(60).saturating_add(minute))
}

fn parse_napcat_config(common: &Value, group: Option<&Value>) -> Result<NapCatConfig, String> {
    let base_url = nonempty(resolve_napcat_base_url(common, group))
        .ok_or_else(|| "missing napcat_base_url".to_string())?;
    let base_url = normalize_napcat_base_url(&base_url);
    let access_token = resolve_napcat_token(common, group);
    let _base_log = base_url_for_log(&base_url);
    debug_log!(
        "napcat config resolved: base_url={} token_present={}",
        _base_log,
        access_token.is_some()
    );
    Ok(NapCatConfig {
        base_url,
        access_token,
    })
}

fn parse_napcat_config_optional(common: &Value) -> Option<NapCatConfig> {
    let base_url = nonempty(resolve_napcat_base_url(common, None))?;
    let base_url = normalize_napcat_base_url(&base_url);
    let access_token = resolve_napcat_token(common, None);
    let _base_log = base_url_for_log(&base_url);
    debug_log!(
        "napcat config resolved: base_url={} token_present={}",
        _base_log,
        access_token.is_some()
    );
    Some(NapCatConfig {
        base_url,
        access_token,
    })
}

fn resolve_napcat_base_url(common: &Value, group: Option<&Value>) -> Option<String> {
    let group_base = group.and_then(|v| parse_string(v.get("napcat_base_url")));
    let common_base = parse_string(common.get("napcat_base_url"));
    env_override("OQQWALL_NAPCAT_BASE_URL", group_base.or(common_base))
}

fn resolve_napcat_token(common: &Value, group: Option<&Value>) -> Option<String> {
    let group_token = group.and_then(|v| parse_string(v.get("napcat_access_token")));
    let common_token = parse_string(common.get("napcat_access_token"));
    env_override("OQQWALL_NAPCAT_TOKEN", group_token.or(common_token))
}

fn normalize_napcat_base_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_prefix("ws://")
        .or_else(|| trimmed.strip_prefix("wss://"))
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    trimmed.trim_end_matches('/').to_string()
}

#[cfg(debug_assertions)]
fn base_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

#[cfg(not(debug_assertions))]
fn base_url_for_log(_url: &str) -> &str {
    "<redacted>"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs::File;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("oqqwall_{name}_{nanos}.json"))
    }

    fn sha256_hex(text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn normalize_group_accounts_prefers_accounts_and_removes_legacy_keys() {
        let mut group = json!({
            "accounts": ["10001", "10002"],
            "mainqqid": "20001",
            "minorqqid": ["20002"],
            "mainqq_http_port": "3000",
            "minorqq_http_port": ["3001"]
        })
        .as_object()
        .cloned()
        .expect("group object");
        assert!(normalize_group_accounts(&mut group));
        assert_eq!(group.get("accounts"), Some(&json!(["10001", "10002"])));
        assert!(!group.contains_key("mainqqid"));
        assert!(!group.contains_key("minorqqid"));
        assert!(!group.contains_key("mainqq_http_port"));
        assert!(!group.contains_key("minorqq_http_port"));
    }

    #[test]
    fn normalize_group_accounts_migrates_from_acount_alias() {
        let mut group = json!({
            "acount": ["12345", "12346"]
        })
        .as_object()
        .cloned()
        .expect("group object");
        assert!(normalize_group_accounts(&mut group));
        assert_eq!(group.get("accounts"), Some(&json!(["12345", "12346"])));
        assert!(!group.contains_key("acount"));
    }

    #[test]
    fn normalize_group_accounts_migrates_from_legacy_main_minor() {
        let mut group = json!({
            "mainqqid": "12345",
            "minorqqid": ["12346", "12347"]
        })
        .as_object()
        .cloned()
        .expect("group object");
        assert!(normalize_group_accounts(&mut group));
        assert_eq!(
            group.get("accounts"),
            Some(&json!(["12345", "12346", "12347"]))
        );
    }

    #[test]
    fn parse_quick_replies_accepts_valid_object() {
        let value = json!({
            "格式错误": "请按模板重新发送",
            "补充信息": "请补充时间地点"
        });
        let parsed = parse_quick_replies(Some(&value)).expect("quick replies");
        assert_eq!(
            parsed.get("格式错误"),
            Some(&"请按模板重新发送".to_string())
        );
        assert_eq!(parsed.get("补充信息"), Some(&"请补充时间地点".to_string()));
    }

    #[test]
    fn parse_quick_replies_rejects_conflicting_command() {
        let value = json!({
            "是": "冲突指令"
        });
        let err = parse_quick_replies(Some(&value)).expect_err("should fail");
        assert!(err.contains("conflicts"));
    }

    #[test]
    fn parse_review_shortcuts_accepts_valid_definition() {
        let value = json!({
            "匿": "匿 | 是",
            "滚": "拒 | 拉黑 {group_id}"
        });
        let parsed = parse_review_shortcuts(Some(&value), &HashMap::new()).expect("shortcuts");
        assert_eq!(parsed.get("匿"), Some(&"匿 | 是".to_string()));
        assert_eq!(parsed.get("滚"), Some(&"拒 | 拉黑 {group_id}".to_string()));
    }

    #[test]
    fn parse_review_shortcuts_rejects_quick_reply_conflict() {
        let value = json!({
            "模板": "匿 | 是"
        });
        let mut quick_replies = HashMap::new();
        quick_replies.insert("模板".to_string(), "hello".to_string());
        let err = parse_review_shortcuts(Some(&value), &quick_replies).expect_err("should fail");
        assert!(err.contains("quick_replies"));
    }

    #[test]
    fn parse_global_shortcuts_rejects_review_placeholders() {
        let value = json!({
            "撤": "撤回 {review_code}"
        });
        let err = parse_global_shortcuts(Some(&value)).expect_err("should fail");
        assert!(err.contains("{review_code}"));
    }

    #[test]
    fn resolve_config_path_defaults_to_config_json() {
        let key = "OQQWALL_CONFIG";
        unsafe {
            std::env::remove_var(key);
        }
        assert_eq!(resolve_config_path(), "config.json");
    }

    #[test]
    fn resolve_config_path_respects_env_override() {
        let key = "OQQWALL_CONFIG";
        unsafe {
            std::env::set_var(key, "/tmp/custom-config.json");
        }
        assert_eq!(resolve_config_path(), "/tmp/custom-config.json");
        unsafe {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn config_exists_returns_false_for_missing_file() {
        let path = unique_test_path("missing");
        assert_eq!(
            config_exists(path.to_string_lossy().as_ref()).expect("exists check"),
            false
        );
    }

    #[test]
    fn config_exists_returns_true_for_existing_file() {
        let path = unique_test_path("exists");
        let _file = File::create(&path).expect("create temp config");
        assert_eq!(
            config_exists(path.to_string_lossy().as_ref()).expect("exists check"),
            true
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn builtin_telemetry_defaults_decode_to_expected_values() {
        assert_eq!(
            default_telemetry_upload_endpoint().as_deref(),
            Some("http://wilflin.com:10925/telemetry/v1/submission/batch")
        );
        let token = default_telemetry_upload_token().expect("builtin token");
        assert!(token.starts_with("t_"));
        assert_eq!(token.len(), 34);
        assert_eq!(
            sha256_hex(&token),
            "cf01510a1c3e68dc648a7fabb0363c5061f0652aeb0568cc8510b0756cbe390f"
        );
    }

    #[test]
    fn telemetry_uses_builtin_defaults_when_upload_enabled_and_unset() {
        let root = json!({
            "common": {
                "napcat_base_url": "127.0.0.1:3001/oqqwall/ws",
                "telemetry": {
                    "upload_enabled": true
                }
            }
        });
        let config = AppConfig::from_value(&root).expect("config");
        assert_eq!(
            config.telemetry.upload_endpoint.as_deref(),
            Some("http://wilflin.com:10925/telemetry/v1/submission/batch")
        );
        let token = config
            .telemetry
            .upload_token
            .as_deref()
            .expect("builtin token");
        assert!(token.starts_with("t_"));
        assert_eq!(token.len(), 34);
        assert_eq!(
            sha256_hex(token),
            "cf01510a1c3e68dc648a7fabb0363c5061f0652aeb0568cc8510b0756cbe390f"
        );
    }

    #[test]
    fn telemetry_defaults_to_enabled_upload_and_builtin_target() {
        let root = json!({
            "common": {
                "napcat_base_url": "127.0.0.1:3001/oqqwall/ws"
            }
        });
        let config = AppConfig::from_value(&root).expect("config");
        assert!(config.telemetry.enabled);
        assert!(config.telemetry.upload_enabled);
        assert_eq!(
            config.telemetry.upload_endpoint.as_deref(),
            Some("http://wilflin.com:10925/telemetry/v1/submission/batch")
        );
        let token = config
            .telemetry
            .upload_token
            .as_deref()
            .expect("builtin token");
        assert!(token.starts_with("t_"));
        assert_eq!(token.len(), 34);
    }

    #[test]
    fn renderer_config_reads_common_renderer_dimensions() {
        let root = json!({
            "common": {
                "napcat_base_url": "127.0.0.1:3001/oqqwall/ws",
                "renderer": {
                    "canvas_width_px": 512,
                    "max_height_px": 4096
                }
            }
        });
        let config = AppConfig::from_value(&root).expect("config");
        assert_eq!(config.renderer_canvas_width_px, 512);
        assert_eq!(config.renderer_max_height_px, 4096);
    }

    #[test]
    fn normalize_config_migrates_renderer_flat_keys() {
        let mut root = json!({
            "common": {
                "canvas_width_px": 512,
                "max_height_px": 4096
            }
        });
        assert!(normalize_config_in_place(&mut root).expect("normalize"));
        assert_eq!(root["common"].get("canvas_width_px"), None);
        assert_eq!(root["common"].get("max_height_px"), None);
        assert_eq!(root["common"]["renderer"]["canvas_width_px"], json!(512));
        assert_eq!(root["common"]["renderer"]["max_height_px"], json!(4096));
    }

    #[test]
    fn telemetry_ignores_endpoint_and_token_overrides() {
        let root = json!({
            "common": {
                "napcat_base_url": "127.0.0.1:3001/oqqwall/ws",
                "telemetry": {
                    "upload_enabled": true,
                    "upload_endpoint": "http://127.0.0.1:10925/telemetry/v1/submission/batch",
                    "upload_token": "custom-token"
                }
            }
        });
        let config = AppConfig::from_value(&root).expect("config");
        assert_eq!(
            config.telemetry.upload_endpoint.as_deref(),
            Some("http://wilflin.com:10925/telemetry/v1/submission/batch")
        );
        assert_eq!(
            config.telemetry.upload_token.as_deref(),
            default_telemetry_upload_token().as_deref()
        );
    }

    #[test]
    fn normalize_config_removes_telemetry_endpoint_and_token() {
        let mut root = json!({
            "common": {
                "telemetry": {
                    "upload_endpoint": "http://127.0.0.1:10925/telemetry/v1/submission/batch",
                    "upload_token": "custom-token",
                    "upload_enabled": false
                }
            }
        });
        assert!(normalize_config_in_place(&mut root).expect("normalize"));
        assert_eq!(
            root.get("common")
                .and_then(|value| value.get("telemetry"))
                .and_then(|value| value.get("upload_endpoint")),
            None
        );
        assert_eq!(
            root.get("common")
                .and_then(|value| value.get("telemetry"))
                .and_then(|value| value.get("upload_token")),
            None
        );
        assert_eq!(
            root.get("common")
                .and_then(|value| value.get("telemetry"))
                .and_then(|value| value.get("upload_enabled"))
                .and_then(|value| value.as_bool()),
            Some(false)
        );
    }
}
