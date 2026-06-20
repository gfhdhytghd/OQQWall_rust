use oqqwall_rust_drivers::blob_cache;
use oqqwall_rust_drivers::media_fetcher::{MediaFetcherRuntimeConfig, spawn_media_fetcher};
use oqqwall_rust_drivers::napcat::{NapCatRuntimeConfig, spawn_napcat_ws};
use oqqwall_rust_drivers::qzone::{QzoneRuntimeConfig, spawn_qzone_sender};
use oqqwall_rust_drivers::renderer::{RendererRuntimeConfig, spawn_renderer};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

pub fn spawn_napcat_drivers(handle: &EngineHandle, config: &AppConfig) -> Result<(), String> {
    blob_cache::configure_max_cache_mb(config.max_cache_mb);
    debug_log!(
        "blob cache configured: max_cache_mb={}",
        config.max_cache_mb
    );
    debug_log!(
        "spawning drivers: groups={} tz_offset_minutes={}",
        config.groups.len(),
        config.tz_offset_minutes
    );
    let core_config = config.build_core_config();
    let mut runtimes_by_base: HashMap<String, Vec<NapCatRuntimeConfig>> = HashMap::new();
    for group in &config.groups {
        let accounts = core_config
            .group_config(&group.group_id)
            .map(|cfg| cfg.accounts.clone())
            .unwrap_or_default();
        let runtime = NapCatRuntimeConfig {
            napcat: group.napcat.clone(),
            audit_group_id: group.audit_group_id.clone(),
            group_id: group.group_id.clone(),
            accounts,
            tz_offset_minutes: config.tz_offset_minutes,
            friend_request_window_sec: group.friend_request_window_sec,
            friend_add_message: group.friend_add_message.clone(),
            max_queue: core_config.max_queue(&group.group_id),
            max_images_per_post: core_config.max_images_per_post(&group.group_id),
            user_notifications: Arc::new(Mutex::new(group.user_notifications.clone())),
            quick_replies: Arc::new(Mutex::new(group.quick_replies.clone())),
            review_shortcuts: Arc::new(Mutex::new(group.review_shortcuts.clone())),
            global_shortcuts: Arc::new(Mutex::new(group.global_shortcuts.clone())),
            agent_commands: Arc::new(Mutex::new(group.agent_commands.clone())),
        };
        let _ws_log = base_url_for_log(&runtime.napcat.base_url);
        debug_log!(
            "spawn napcat ws: group_id={} audit_group_id={:?} base_url={} token_present={}",
            runtime.group_id,
            runtime.audit_group_id,
            _ws_log,
            runtime.napcat.access_token.is_some()
        );
        runtimes_by_base
            .entry(runtime.napcat.base_url.clone())
            .or_default()
            .push(runtime);
    }
    for (base_url, runtimes) in runtimes_by_base {
        let bus_rx = handle.subscribe();
        spawn_napcat_ws(handle.cmd_tx.clone(), bus_rx, base_url, runtimes);
    }
    debug_log!("spawn media fetcher");
    spawn_media_fetcher(
        handle.cmd_tx.clone(),
        handle.subscribe(),
        MediaFetcherRuntimeConfig::default(),
    );
    let mut napcat_by_group = HashMap::new();
    let mut accounts_by_group = HashMap::new();
    for group in &config.groups {
        napcat_by_group.insert(group.group_id.clone(), group.napcat.clone());
        accounts_by_group.insert(
            group.group_id.clone(),
            core_config
                .group_config(&group.group_id)
                .map(|cfg| cfg.accounts.clone())
                .unwrap_or_default(),
        );
    }
    let mut max_queue_by_group = HashMap::new();
    let mut max_images_per_post_by_group = HashMap::new();
    let mut individual_images_by_group = HashMap::new();
    let mut watermark_text_by_group = HashMap::new();
    for group in &config.groups {
        max_queue_by_group.insert(
            group.group_id.clone(),
            core_config.max_queue(&group.group_id),
        );
        max_images_per_post_by_group.insert(
            group.group_id.clone(),
            core_config.max_images_per_post(&group.group_id),
        );
        individual_images_by_group.insert(group.group_id.clone(), group.individual_image_in_posts);
        if let Some(text) = group.watermark_text.clone() {
            watermark_text_by_group.insert(group.group_id.clone(), text);
        }
    }
    debug_log!("spawn renderer");
    let renderer_config = RendererRuntimeConfig {
        napcat_by_group: napcat_by_group.clone(),
        default_napcat: config.fallback_napcat.clone(),
        watermark_text_by_group,
        ..RendererRuntimeConfig::default()
    };
    spawn_renderer(handle.cmd_tx.clone(), handle.subscribe(), renderer_config)?;
    let runtime = QzoneRuntimeConfig {
        napcat_by_group,
        default_napcat: config.fallback_napcat.clone(),
        accounts_by_group,
        at_unprived_sender: config.at_unprived_sender,
        max_queue_by_group,
        max_images_per_post_by_group,
        individual_images_by_group,
        default_max_queue: core_config.default_max_queue,
        default_max_images_per_post: core_config.default_max_images_per_post,
        default_individual_images: true,
        #[cfg(debug_assertions)]
        use_virt_qzone: config.dev_config.use_virt_qzone,
    };
    debug_log!(
        "spawn qzone sender: default_napcat={}",
        runtime.default_napcat.is_some()
    );
    spawn_qzone_sender(handle.cmd_tx.clone(), handle.subscribe(), runtime);
    Ok(())
}

#[cfg(debug_assertions)]
fn base_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

#[cfg(not(debug_assertions))]
fn base_url_for_log(_url: &str) -> &str {
    "<redacted>"
}
