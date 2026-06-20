use std::collections::HashMap;

use crate::ids::{AccountId, GroupId, TimestampMs};

#[derive(Debug, Clone, Default)]
pub struct CoreConfig {
    pub default_process_waittime_ms: TimestampMs,
    pub default_send_windows: Vec<TimeWindow>,
    pub default_min_interval_ms: TimestampMs,
    pub default_max_queue: usize,
    pub default_max_images_per_post: usize,
    pub default_send_timeout_ms: TimestampMs,
    pub default_send_max_attempts: u32,
    pub groups: HashMap<GroupId, GroupConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct GroupConfig {
    pub group_id: GroupId,
    pub process_waittime_ms: Option<TimestampMs>,
    pub send_windows: Vec<TimeWindow>,
    pub min_interval_ms: Option<TimestampMs>,
    pub max_queue: Option<usize>,
    pub max_images_per_post: Option<usize>,
    pub send_schedule_minutes: Vec<u16>,
    pub accounts: Vec<AccountId>,
    pub send_timeout_ms: Option<TimestampMs>,
    pub send_max_attempts: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    pub start_minute: u16,
    pub end_minute: u16,
}

impl CoreConfig {
    pub fn group_config(&self, group_id: &GroupId) -> Option<&GroupConfig> {
        self.groups.get(group_id)
    }

    pub fn process_waittime_ms(&self, group_id: &GroupId) -> TimestampMs {
        self.group_config(group_id)
            .and_then(|cfg| cfg.process_waittime_ms)
            .unwrap_or(self.default_process_waittime_ms)
    }

    pub fn send_windows(&self, group_id: &GroupId) -> &[TimeWindow] {
        if let Some(cfg) = self.group_config(group_id) {
            if !cfg.send_windows.is_empty() {
                return &cfg.send_windows;
            }
        }
        &self.default_send_windows
    }

    pub fn min_interval_ms(&self, group_id: &GroupId) -> TimestampMs {
        self.group_config(group_id)
            .and_then(|cfg| cfg.min_interval_ms)
            .unwrap_or(self.default_min_interval_ms)
    }

    pub fn max_queue(&self, group_id: &GroupId) -> usize {
        self.group_config(group_id)
            .and_then(|cfg| cfg.max_queue)
            .unwrap_or(self.default_max_queue)
    }

    pub fn max_images_per_post(&self, group_id: &GroupId) -> usize {
        self.group_config(group_id)
            .and_then(|cfg| cfg.max_images_per_post)
            .unwrap_or(self.default_max_images_per_post)
    }

    pub fn send_timeout_ms(&self, group_id: &GroupId) -> TimestampMs {
        self.group_config(group_id)
            .and_then(|cfg| cfg.send_timeout_ms)
            .unwrap_or(self.default_send_timeout_ms)
    }

    pub fn send_max_attempts(&self, group_id: &GroupId) -> u32 {
        self.group_config(group_id)
            .and_then(|cfg| cfg.send_max_attempts)
            .unwrap_or(self.default_send_max_attempts)
    }
}
