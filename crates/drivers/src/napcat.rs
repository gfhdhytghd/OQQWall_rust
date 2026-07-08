use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::command::{
    GlobalAction, GlobalActionBatchCommand, GlobalActionCommand, PostAction, PostActionCommand,
    ReviewAction, ReviewActionBatchCommand, ReviewActionCommand, ShortcutScope,
};
use oqqwall_rust_core::draft::{
    Draft, IngressAttachment, IngressMessage, IngressRouteMeta, MediaKind, MediaReference,
    ReplyPreview, json_card_marker, poke_marker, reply_marker,
};
use oqqwall_rust_core::draft_transform::{
    DraftTransform, RuleCondition, evaluate_condition, validate_condition, validate_transform,
};
use oqqwall_rust_core::event::{
    BlobEvent, DraftEvent, Event, IngressEvent, InputStatusKind, LifecycleEvent, MediaEvent,
    ReviewDecision, ReviewEvent, ScheduleEvent, SendEvent, SendPriority,
};
use oqqwall_rust_core::ids::{BlobId, ExternalCode, IngressId, PostId, ReviewCode, ReviewId};
use oqqwall_rust_core::state::PostStage;
use oqqwall_rust_core::{Command, IngressCommand, StateView, derive_blob_id, derive_ingress_id};
use oqqwall_rust_infra::{LocalJournal, SnapshotStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
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
use crate::thankyou_filter::{self, ThankYouFeedbackKind, ThankYouFilterRuntimeConfig};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendSuccessReplyConfig {
    pub enabled: bool,
    pub text_template: String,
    pub images: Vec<String>,
}

impl Default for SendSuccessReplyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            text_template: "#<code>已发送".to_string(),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserNotificationStage {
    QueueEntered,
    ReviewQueued,
    SendSucceeded,
    Rejected,
}

impl UserNotificationStage {
    pub fn as_str(self) -> &'static str {
        match self {
            UserNotificationStage::QueueEntered => "queue_entered",
            UserNotificationStage::ReviewQueued => "review_queued",
            UserNotificationStage::SendSucceeded => "send_succeeded",
            UserNotificationStage::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNotificationTemplate {
    pub enabled: bool,
    pub include_post_tags: bool,
    pub text_template: String,
    pub tags: Vec<String>,
    pub images: Vec<String>,
}

impl UserNotificationTemplate {
    pub fn queue_entered_default() -> Self {
        Self {
            enabled: false,
            include_post_tags: false,
            text_template: "#<code>已进入发送队列".to_string(),
            tags: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn send_succeeded_default() -> Self {
        Self {
            enabled: true,
            include_post_tags: false,
            text_template: "#<code>已发送".to_string(),
            tags: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn rejected_default() -> Self {
        Self {
            enabled: true,
            include_post_tags: false,
            text_template: "你的投稿已被拒，请修改后再发送".to_string(),
            tags: Vec::new(),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagValueMappingGroup {
    pub tag: String,
    #[serde(default)]
    pub mappings: Vec<TagValueMappingEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagValueMappingEntry {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNotificationSettings {
    pub queue_entered: UserNotificationTemplate,
    pub review_queued: UserNotificationTemplate,
    pub send_succeeded: UserNotificationTemplate,
    pub rejected: UserNotificationTemplate,
    #[serde(default)]
    pub webhook_tag_map: HashMap<String, String>,
    #[serde(default)]
    pub tag_value_maps: Vec<TagValueMappingGroup>,
}

impl Default for UserNotificationSettings {
    fn default() -> Self {
        Self {
            queue_entered: UserNotificationTemplate::queue_entered_default(),
            review_queued: UserNotificationTemplate::queue_entered_default(),
            send_succeeded: UserNotificationTemplate::send_succeeded_default(),
            rejected: UserNotificationTemplate::rejected_default(),
            webhook_tag_map: HashMap::new(),
            tag_value_maps: Vec::new(),
        }
    }
}

impl UserNotificationSettings {
    pub fn stage(&self, stage: UserNotificationStage) -> &UserNotificationTemplate {
        match stage {
            UserNotificationStage::QueueEntered => &self.queue_entered,
            UserNotificationStage::ReviewQueued => &self.review_queued,
            UserNotificationStage::SendSucceeded => &self.send_succeeded,
            UserNotificationStage::Rejected => &self.rejected,
        }
    }
}

fn agent_command_enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentCommandTrigger {
    #[default]
    PrivateCommand,
    SubmissionReceived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCommandConfig {
    #[serde(default = "agent_command_enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub admin_only: bool,
    #[serde(default)]
    pub trigger: AgentCommandTrigger,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub blocks: Vec<AgentCommandBlock>,
}

impl Default for AgentCommandConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            admin_only: false,
            trigger: AgentCommandTrigger::PrivateCommand,
            description: String::new(),
            blocks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCommandQueueInsertPosition {
    Before,
    After,
}

impl Default for AgentCommandQueueInsertPosition {
    fn default() -> Self {
        Self::Before
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCommandShortcutScope {
    Review,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentCommandReviewAction {
    Approve,
    Reject,
    Delete,
    Defer {
        #[serde(default)]
        delay_ms: String,
    },
    Skip,
    Immediate,
    Refresh,
    Rerender,
    SelectAllMessages,
    ToggleAnonymous,
    ExpandAudit,
    Show,
    Comment {
        #[serde(default)]
        text_template: String,
    },
    Reply {
        #[serde(default)]
        text_template: String,
    },
    Blacklist {
        #[serde(default)]
        reason_template: String,
    },
    QuickReply {
        #[serde(default)]
        key_template: String,
    },
    Merge {
        #[serde(default)]
        target_review_code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentCommandGlobalAction {
    Help,
    Recall {
        #[serde(default)]
        review_code: String,
    },
    Withdraw {
        #[serde(default)]
        review_code: String,
    },
    Info {
        #[serde(default)]
        review_code: String,
    },
    ManualRelogin,
    AutoRelogin,
    PendingList,
    PendingClear,
    SendQueueClear,
    SendQueueFlush,
    SendInFlightClear,
    BlacklistList,
    BlacklistAdd {
        #[serde(default)]
        sender_id: String,
        #[serde(default)]
        reason_template: String,
    },
    BlacklistRemove {
        #[serde(default)]
        sender_id: String,
    },
    SetExternalNumber {
        #[serde(default)]
        value_template: String,
    },
    QuickReplyList,
    QuickReplyAdd {
        #[serde(default)]
        key_template: String,
        #[serde(default)]
        text_template: String,
    },
    QuickReplyDelete {
        #[serde(default)]
        key_template: String,
    },
    ShortcutList,
    ShortcutAdd {
        scope: AgentCommandShortcutScope,
        #[serde(default)]
        key_template: String,
        #[serde(default)]
        definition_template: String,
    },
    ShortcutDelete {
        scope: AgentCommandShortcutScope,
        #[serde(default)]
        key_template: String,
    },
    SelfCheck,
    SystemRepair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum AgentCommandPostTarget {
    TriggeringPost,
    ReviewCode {
        #[serde(default)]
        template: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentCommandBlock {
    ReplyPrivateMessage {
        #[serde(default)]
        text_template: String,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        images: Vec<String>,
    },
    StartSubmissionSession,
    FinishSubmissionSession,
    ResumeSubmissionSession,
    SubmitSubmissionSession,
    CancelSubmissionSession,
    InsertQueuedPost {
        #[serde(default)]
        moving_post_code: String,
        #[serde(default)]
        anchor_post_code: String,
        #[serde(default)]
        position: AgentCommandQueueInsertPosition,
    },
    ExecuteReviewAction {
        #[serde(default)]
        review_code: String,
        action: AgentCommandReviewAction,
    },
    ExecuteGlobalAction {
        action: AgentCommandGlobalAction,
    },
    If {
        condition: RuleCondition,
        #[serde(default)]
        then_blocks: Vec<AgentCommandBlock>,
        #[serde(default)]
        else_blocks: Vec<AgentCommandBlock>,
    },
    SetDraftTransforms {
        target: AgentCommandPostTarget,
        #[serde(default)]
        transforms: Vec<DraftTransform>,
    },
    SendWebhook {
        #[serde(default)]
        url: String,
        #[serde(default)]
        source_webhook: String,
        #[serde(default)]
        text_template: String,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        images: Vec<String>,
    },
}

pub fn validate_agent_command_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim().trim_start_matches('#').trim();
    if trimmed.is_empty() {
        return Err("agent 指令名不能为空".to_string());
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err("agent 指令名不能包含空白字符".to_string());
    }
    if is_builtin_private_submission_command_name(trimmed) {
        return Err(format!("agent 指令名与内置私聊指令冲突：{}", trimmed));
    }
    Ok(trimmed.to_string())
}

pub fn normalize_agent_command_config(config: &AgentCommandConfig) -> AgentCommandConfig {
    AgentCommandConfig {
        enabled: config.enabled,
        admin_only: config.admin_only,
        trigger: config.trigger,
        description: config.description.trim().to_string(),
        blocks: config
            .blocks
            .iter()
            .map(normalize_agent_command_block)
            .collect(),
    }
}

pub fn validate_agent_command_config(
    name: &str,
    config: &AgentCommandConfig,
) -> Result<(), String> {
    let normalized_name = validate_agent_command_name(name)?;
    if config.blocks.is_empty() {
        return Err(format!(
            "agent_commands['{}'] 至少需要一个积木块",
            normalized_name
        ));
    }
    for (index, block) in config.blocks.iter().enumerate() {
        validate_agent_command_block(&normalized_name, config.trigger, index, block, 0)?;
    }
    Ok(())
}

fn normalize_agent_command_block(block: &AgentCommandBlock) -> AgentCommandBlock {
    match block {
        AgentCommandBlock::ReplyPrivateMessage {
            text_template,
            tags,
            images,
        } => AgentCommandBlock::ReplyPrivateMessage {
            text_template: text_template.replace("\r\n", "\n"),
            tags: normalize_agent_command_values(tags),
            images: normalize_agent_command_values(images),
        },
        AgentCommandBlock::StartSubmissionSession => AgentCommandBlock::StartSubmissionSession,
        AgentCommandBlock::FinishSubmissionSession => AgentCommandBlock::FinishSubmissionSession,
        AgentCommandBlock::ResumeSubmissionSession => AgentCommandBlock::ResumeSubmissionSession,
        AgentCommandBlock::SubmitSubmissionSession => AgentCommandBlock::SubmitSubmissionSession,
        AgentCommandBlock::CancelSubmissionSession => AgentCommandBlock::CancelSubmissionSession,
        AgentCommandBlock::InsertQueuedPost {
            moving_post_code,
            anchor_post_code,
            position,
        } => AgentCommandBlock::InsertQueuedPost {
            moving_post_code: moving_post_code.trim().to_string(),
            anchor_post_code: anchor_post_code.trim().to_string(),
            position: position.clone(),
        },
        AgentCommandBlock::ExecuteReviewAction {
            review_code,
            action,
        } => AgentCommandBlock::ExecuteReviewAction {
            review_code: review_code.trim().to_string(),
            action: normalize_agent_command_review_action(action),
        },
        AgentCommandBlock::ExecuteGlobalAction { action } => {
            AgentCommandBlock::ExecuteGlobalAction {
                action: normalize_agent_command_global_action(action),
            }
        }
        AgentCommandBlock::If {
            condition,
            then_blocks,
            else_blocks,
        } => AgentCommandBlock::If {
            condition: condition.clone(),
            then_blocks: then_blocks
                .iter()
                .map(normalize_agent_command_block)
                .collect(),
            else_blocks: else_blocks
                .iter()
                .map(normalize_agent_command_block)
                .collect(),
        },
        AgentCommandBlock::SetDraftTransforms { target, transforms } => {
            AgentCommandBlock::SetDraftTransforms {
                target: normalize_agent_command_post_target(target),
                transforms: transforms.clone(),
            }
        }
        AgentCommandBlock::SendWebhook {
            url,
            source_webhook,
            text_template,
            tags,
            images,
        } => AgentCommandBlock::SendWebhook {
            url: url.trim().to_string(),
            source_webhook: source_webhook.trim().to_string(),
            text_template: text_template.replace("\r\n", "\n"),
            tags: normalize_agent_command_values(tags),
            images: normalize_agent_command_values(images),
        },
    }
}

fn normalize_agent_command_post_target(target: &AgentCommandPostTarget) -> AgentCommandPostTarget {
    match target {
        AgentCommandPostTarget::TriggeringPost => AgentCommandPostTarget::TriggeringPost,
        AgentCommandPostTarget::ReviewCode { template } => AgentCommandPostTarget::ReviewCode {
            template: template.trim().to_string(),
        },
    }
}

fn normalize_agent_command_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn normalize_agent_command_review_action(
    action: &AgentCommandReviewAction,
) -> AgentCommandReviewAction {
    match action {
        AgentCommandReviewAction::Approve => AgentCommandReviewAction::Approve,
        AgentCommandReviewAction::Reject => AgentCommandReviewAction::Reject,
        AgentCommandReviewAction::Delete => AgentCommandReviewAction::Delete,
        AgentCommandReviewAction::Defer { delay_ms } => AgentCommandReviewAction::Defer {
            delay_ms: delay_ms.trim().to_string(),
        },
        AgentCommandReviewAction::Skip => AgentCommandReviewAction::Skip,
        AgentCommandReviewAction::Immediate => AgentCommandReviewAction::Immediate,
        AgentCommandReviewAction::Refresh => AgentCommandReviewAction::Refresh,
        AgentCommandReviewAction::Rerender => AgentCommandReviewAction::Rerender,
        AgentCommandReviewAction::SelectAllMessages => AgentCommandReviewAction::SelectAllMessages,
        AgentCommandReviewAction::ToggleAnonymous => AgentCommandReviewAction::ToggleAnonymous,
        AgentCommandReviewAction::ExpandAudit => AgentCommandReviewAction::ExpandAudit,
        AgentCommandReviewAction::Show => AgentCommandReviewAction::Show,
        AgentCommandReviewAction::Comment { text_template } => AgentCommandReviewAction::Comment {
            text_template: text_template.replace("\r\n", "\n"),
        },
        AgentCommandReviewAction::Reply { text_template } => AgentCommandReviewAction::Reply {
            text_template: text_template.replace("\r\n", "\n"),
        },
        AgentCommandReviewAction::Blacklist { reason_template } => {
            AgentCommandReviewAction::Blacklist {
                reason_template: reason_template.replace("\r\n", "\n"),
            }
        }
        AgentCommandReviewAction::QuickReply { key_template } => {
            AgentCommandReviewAction::QuickReply {
                key_template: key_template.trim().to_string(),
            }
        }
        AgentCommandReviewAction::Merge { target_review_code } => AgentCommandReviewAction::Merge {
            target_review_code: target_review_code.trim().to_string(),
        },
    }
}

fn normalize_agent_command_global_action(
    action: &AgentCommandGlobalAction,
) -> AgentCommandGlobalAction {
    match action {
        AgentCommandGlobalAction::Help => AgentCommandGlobalAction::Help,
        AgentCommandGlobalAction::Recall { review_code } => AgentCommandGlobalAction::Recall {
            review_code: review_code.trim().to_string(),
        },
        AgentCommandGlobalAction::Withdraw { review_code } => AgentCommandGlobalAction::Withdraw {
            review_code: review_code.trim().to_string(),
        },
        AgentCommandGlobalAction::Info { review_code } => AgentCommandGlobalAction::Info {
            review_code: review_code.trim().to_string(),
        },
        AgentCommandGlobalAction::ManualRelogin => AgentCommandGlobalAction::ManualRelogin,
        AgentCommandGlobalAction::AutoRelogin => AgentCommandGlobalAction::AutoRelogin,
        AgentCommandGlobalAction::PendingList => AgentCommandGlobalAction::PendingList,
        AgentCommandGlobalAction::PendingClear => AgentCommandGlobalAction::PendingClear,
        AgentCommandGlobalAction::SendQueueClear => AgentCommandGlobalAction::SendQueueClear,
        AgentCommandGlobalAction::SendQueueFlush => AgentCommandGlobalAction::SendQueueFlush,
        AgentCommandGlobalAction::SendInFlightClear => AgentCommandGlobalAction::SendInFlightClear,
        AgentCommandGlobalAction::BlacklistList => AgentCommandGlobalAction::BlacklistList,
        AgentCommandGlobalAction::BlacklistAdd {
            sender_id,
            reason_template,
        } => AgentCommandGlobalAction::BlacklistAdd {
            sender_id: sender_id.trim().to_string(),
            reason_template: reason_template.replace("\r\n", "\n"),
        },
        AgentCommandGlobalAction::BlacklistRemove { sender_id } => {
            AgentCommandGlobalAction::BlacklistRemove {
                sender_id: sender_id.trim().to_string(),
            }
        }
        AgentCommandGlobalAction::SetExternalNumber { value_template } => {
            AgentCommandGlobalAction::SetExternalNumber {
                value_template: value_template.trim().to_string(),
            }
        }
        AgentCommandGlobalAction::QuickReplyList => AgentCommandGlobalAction::QuickReplyList,
        AgentCommandGlobalAction::QuickReplyAdd {
            key_template,
            text_template,
        } => AgentCommandGlobalAction::QuickReplyAdd {
            key_template: key_template.trim().to_string(),
            text_template: text_template.replace("\r\n", "\n"),
        },
        AgentCommandGlobalAction::QuickReplyDelete { key_template } => {
            AgentCommandGlobalAction::QuickReplyDelete {
                key_template: key_template.trim().to_string(),
            }
        }
        AgentCommandGlobalAction::ShortcutList => AgentCommandGlobalAction::ShortcutList,
        AgentCommandGlobalAction::ShortcutAdd {
            scope,
            key_template,
            definition_template,
        } => AgentCommandGlobalAction::ShortcutAdd {
            scope: scope.clone(),
            key_template: key_template.trim().to_string(),
            definition_template: definition_template.replace("\r\n", "\n"),
        },
        AgentCommandGlobalAction::ShortcutDelete {
            scope,
            key_template,
        } => AgentCommandGlobalAction::ShortcutDelete {
            scope: scope.clone(),
            key_template: key_template.trim().to_string(),
        },
        AgentCommandGlobalAction::SelfCheck => AgentCommandGlobalAction::SelfCheck,
        AgentCommandGlobalAction::SystemRepair => AgentCommandGlobalAction::SystemRepair,
    }
}

fn validate_agent_command_block(
    command_name: &str,
    trigger: AgentCommandTrigger,
    index: usize,
    block: &AgentCommandBlock,
    depth: usize,
) -> Result<(), String> {
    if depth > 8 {
        return Err(format!(
            "agent_commands['{}'] 的第 {} 个积木嵌套过深",
            command_name,
            index + 1
        ));
    }
    match block {
        AgentCommandBlock::ReplyPrivateMessage { .. }
        | AgentCommandBlock::StartSubmissionSession
        | AgentCommandBlock::FinishSubmissionSession
        | AgentCommandBlock::ResumeSubmissionSession
        | AgentCommandBlock::SubmitSubmissionSession
        | AgentCommandBlock::CancelSubmissionSession => {
            if trigger == AgentCommandTrigger::SubmissionReceived
                && matches!(
                    block,
                    AgentCommandBlock::StartSubmissionSession
                        | AgentCommandBlock::FinishSubmissionSession
                        | AgentCommandBlock::ResumeSubmissionSession
                        | AgentCommandBlock::SubmitSubmissionSession
                        | AgentCommandBlock::CancelSubmissionSession
                )
            {
                Err(format!(
                    "agent_commands['{}'] 的第 {} 个积木仅可用于私聊触发的指令",
                    command_name,
                    index + 1
                ))
            } else {
                Ok(())
            }
        }
        AgentCommandBlock::InsertQueuedPost {
            moving_post_code,
            anchor_post_code,
            ..
        } => {
            validate_agent_command_required_field(
                command_name,
                index,
                "要移动的投稿编号",
                moving_post_code,
            )?;
            validate_agent_command_required_field(
                command_name,
                index,
                "目标投稿编号",
                anchor_post_code,
            )
        }
        AgentCommandBlock::ExecuteReviewAction {
            review_code,
            action,
        } => {
            if trigger == AgentCommandTrigger::SubmissionReceived {
                return Err(format!(
                    "agent_commands['{}'] 的第 {} 个积木仅可用于私聊触发的指令",
                    command_name,
                    index + 1
                ));
            }
            validate_agent_command_required_field(
                command_name,
                index,
                "审核目标编号",
                review_code,
            )?;
            validate_agent_command_review_action(command_name, index, action)
        }
        AgentCommandBlock::ExecuteGlobalAction { action } => {
            validate_agent_command_global_action(command_name, index, action)
        }
        AgentCommandBlock::If {
            condition,
            then_blocks,
            else_blocks,
        } => {
            if trigger != AgentCommandTrigger::SubmissionReceived {
                return Err(format!(
                    "agent_commands['{}'] 的第 {} 个积木仅可用于收到新投稿触发的指令",
                    command_name,
                    index + 1
                ));
            }
            validate_condition(condition).map_err(|err| {
                format!(
                    "agent_commands['{}'] 的第 {} 个积木条件无效: {}",
                    command_name,
                    index + 1,
                    err
                )
            })?;
            for (child_index, block) in then_blocks.iter().enumerate() {
                validate_agent_command_block(command_name, trigger, child_index, block, depth + 1)?;
            }
            for (child_index, block) in else_blocks.iter().enumerate() {
                validate_agent_command_block(command_name, trigger, child_index, block, depth + 1)?;
            }
            Ok(())
        }
        AgentCommandBlock::SetDraftTransforms { target, transforms } => {
            if transforms.is_empty() {
                return Err(format!(
                    "agent_commands['{}'] 的第 {} 个积木缺少稿件变换规则",
                    command_name,
                    index + 1
                ));
            }
            match target {
                AgentCommandPostTarget::TriggeringPost => {
                    if trigger != AgentCommandTrigger::SubmissionReceived {
                        return Err(format!(
                            "agent_commands['{}'] 的第 {} 个积木的当前触发稿件目标仅可用于收到新投稿触发的指令",
                            command_name,
                            index + 1
                        ));
                    }
                }
                AgentCommandPostTarget::ReviewCode { template } => {
                    validate_agent_command_required_field(
                        command_name,
                        index,
                        "稿件编号",
                        template,
                    )?;
                }
            }
            for transform in transforms {
                validate_transform(transform).map_err(|err| {
                    format!(
                        "agent_commands['{}'] 的第 {} 个积木变换规则无效: {}",
                        command_name,
                        index + 1,
                        err
                    )
                })?;
            }
            Ok(())
        }
        AgentCommandBlock::SendWebhook { url, .. } => {
            if url.trim().is_empty() {
                Err(format!(
                    "agent_commands['{}'] 的第 {} 个积木缺少 webhook 地址",
                    command_name,
                    index + 1
                ))
            } else {
                validate_agent_command_webhook_url_template(command_name, index, url)
            }
        }
    }
}

fn validate_agent_command_webhook_url_template(
    command_name: &str,
    index: usize,
    url: &str,
) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.contains('<') {
        return Ok(());
    }
    validate_agent_command_webhook_url(trimmed).map_err(|err| {
        format!(
            "agent_commands['{}'] 的第 {} 个积木的 {}",
            command_name,
            index + 1,
            err
        )
    })
}

fn validate_agent_command_webhook_url(url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|_| "webhook 地址无效".to_string())?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err("webhook 地址无效".to_string());
    }
    if parsed.host_str().map(str::trim).unwrap_or("").is_empty() {
        return Err("webhook 地址无效".to_string());
    }
    Ok(())
}

fn validate_agent_command_required_field(
    command_name: &str,
    index: usize,
    field_label: &str,
    value: &str,
) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!(
            "agent_commands['{}'] 的第 {} 个积木缺少{}",
            command_name,
            index + 1,
            field_label
        ))
    } else {
        Ok(())
    }
}

fn validate_agent_command_review_action(
    command_name: &str,
    index: usize,
    action: &AgentCommandReviewAction,
) -> Result<(), String> {
    match action {
        AgentCommandReviewAction::Approve
        | AgentCommandReviewAction::Reject
        | AgentCommandReviewAction::Delete
        | AgentCommandReviewAction::Skip
        | AgentCommandReviewAction::Immediate
        | AgentCommandReviewAction::Refresh
        | AgentCommandReviewAction::Rerender
        | AgentCommandReviewAction::SelectAllMessages
        | AgentCommandReviewAction::ToggleAnonymous
        | AgentCommandReviewAction::ExpandAudit
        | AgentCommandReviewAction::Show => Ok(()),
        AgentCommandReviewAction::Defer { delay_ms } => {
            validate_agent_command_required_field(command_name, index, "延期毫秒数", delay_ms)
        }
        AgentCommandReviewAction::Comment { text_template }
        | AgentCommandReviewAction::Reply { text_template } => {
            validate_agent_command_required_field(command_name, index, "文本内容", text_template)
        }
        AgentCommandReviewAction::Blacklist { .. } => Ok(()),
        AgentCommandReviewAction::QuickReply { key_template } => {
            validate_agent_command_required_field(command_name, index, "快捷回复键", key_template)
        }
        AgentCommandReviewAction::Merge { target_review_code } => {
            validate_agent_command_required_field(
                command_name,
                index,
                "合并目标编号",
                target_review_code,
            )
        }
    }
}

fn validate_agent_command_global_action(
    command_name: &str,
    index: usize,
    action: &AgentCommandGlobalAction,
) -> Result<(), String> {
    match action {
        AgentCommandGlobalAction::Help
        | AgentCommandGlobalAction::ManualRelogin
        | AgentCommandGlobalAction::AutoRelogin
        | AgentCommandGlobalAction::PendingList
        | AgentCommandGlobalAction::PendingClear
        | AgentCommandGlobalAction::SendQueueClear
        | AgentCommandGlobalAction::SendQueueFlush
        | AgentCommandGlobalAction::SendInFlightClear
        | AgentCommandGlobalAction::BlacklistList
        | AgentCommandGlobalAction::QuickReplyList
        | AgentCommandGlobalAction::ShortcutList
        | AgentCommandGlobalAction::SelfCheck
        | AgentCommandGlobalAction::SystemRepair => Ok(()),
        AgentCommandGlobalAction::Recall { review_code }
        | AgentCommandGlobalAction::Withdraw { review_code }
        | AgentCommandGlobalAction::Info { review_code } => {
            validate_agent_command_required_field(command_name, index, "目标编号", review_code)
        }
        AgentCommandGlobalAction::BlacklistAdd { sender_id, .. }
        | AgentCommandGlobalAction::BlacklistRemove { sender_id } => {
            validate_agent_command_required_field(command_name, index, "用户 QQ", sender_id)
        }
        AgentCommandGlobalAction::SetExternalNumber { value_template } => {
            validate_agent_command_required_field(
                command_name,
                index,
                "外部编号起始值",
                value_template,
            )
        }
        AgentCommandGlobalAction::QuickReplyAdd {
            key_template,
            text_template,
        } => {
            validate_agent_command_required_field(command_name, index, "快捷回复键", key_template)?;
            validate_agent_command_required_field(
                command_name,
                index,
                "快捷回复内容",
                text_template,
            )
        }
        AgentCommandGlobalAction::QuickReplyDelete { key_template } => {
            validate_agent_command_required_field(command_name, index, "快捷回复键", key_template)
        }
        AgentCommandGlobalAction::ShortcutAdd {
            key_template,
            definition_template,
            ..
        } => {
            validate_agent_command_required_field(command_name, index, "快捷指令名", key_template)?;
            validate_agent_command_required_field(
                command_name,
                index,
                "快捷指令定义",
                definition_template,
            )
        }
        AgentCommandGlobalAction::ShortcutDelete { key_template, .. } => {
            validate_agent_command_required_field(command_name, index, "快捷指令名", key_template)
        }
    }
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
    pub max_images_per_post: usize,
    pub thank_you_filter: ThankYouFilterRuntimeConfig,
    pub user_notifications: Arc<std::sync::Mutex<UserNotificationSettings>>,
    pub quick_replies: Arc<std::sync::Mutex<HashMap<String, String>>>,
    pub review_shortcuts: Arc<std::sync::Mutex<HashMap<String, String>>>,
    pub global_shortcuts: Arc<std::sync::Mutex<HashMap<String, String>>>,
    pub agent_commands: Arc<std::sync::Mutex<HashMap<String, AgentCommandConfig>>>,
    pub agent_command_admins: Arc<std::sync::Mutex<Vec<String>>>,
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
static GROUP_USER_NOTIFICATION_SETTINGS: OnceLock<
    std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<UserNotificationSettings>>>>,
> = OnceLock::new();
static GROUP_AGENT_COMMANDS: OnceLock<
    std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<HashMap<String, AgentCommandConfig>>>>>,
> = OnceLock::new();
static GROUP_AGENT_COMMAND_ADMINS: OnceLock<
    std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<Vec<String>>>>>,
> = OnceLock::new();
static AGENT_WEBHOOK_CLIENT: OnceLock<Client> = OnceLock::new();
static THANK_YOU_HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

#[derive(Debug, Clone)]
struct ReviewInfo {
    review_code: ReviewCode,
    post_id: PostId,
    group_id: String,
    decision: Option<ReviewDecision>,
    decided_by: Option<String>,
    decided_at_ms: Option<i64>,
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
    route_meta: Option<IngressRouteMeta>,
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

#[derive(Debug, Clone)]
struct ThankYouFeedbackRecord {
    sent_at_ms: i64,
    kind: ThankYouFeedbackKind,
    silenced_count: u8,
}

#[derive(Default)]
struct NapCatState {
    review_info: HashMap<ReviewId, ReviewInfo>,
    review_by_code: HashMap<ReviewCode, ReviewId>,
    review_publish_attempts: HashMap<ReviewId, u32>,
    ingress_summary: HashMap<IngressId, IngressSummary>,
    pending_summary: HashMap<IngressId, String>,
    post_ingress: HashMap<PostId, Vec<IngressId>>,
    post_draft: HashMap<PostId, Draft>,
    post_group: HashMap<PostId, String>,
    post_stage: HashMap<PostId, PostStage>,
    post_created_at_ms: HashMap<PostId, i64>,
    post_safe: HashMap<PostId, bool>,
    post_review_id: HashMap<PostId, ReviewId>,
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
    thank_you_feedback: HashMap<String, ThankYouFeedbackRecord>,
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
                route_meta: meta.route_meta.clone(),
            },
        );
    }
    for (post_id, ingress_ids) in &view.post_ingress {
        state.post_ingress.insert(*post_id, ingress_ids.clone());
    }
    for (post_id, draft) in &view.drafts {
        state.post_draft.insert(*post_id, draft.clone());
    }
    for (post_id, post) in &view.posts {
        state.post_group.insert(*post_id, post.group_id.clone());
        state.post_stage.insert(*post_id, post.stage);
        state
            .post_created_at_ms
            .insert(*post_id, post.created_at_ms);
        state.post_safe.insert(*post_id, post.is_safe);
        if let Some(review_id) = post.review_id {
            state.post_review_id.insert(*post_id, review_id);
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
                decision: review.decision,
                decided_by: review.decided_by.clone(),
                decided_at_ms: review.decided_at_ms,
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

fn evict_napcat_post_cache(state: &mut NapCatState, post_id: PostId, ingress_ids: &[IngressId]) {
    state.post_draft.remove(&post_id);
    state.post_group.remove(&post_id);
    state.post_stage.remove(&post_id);
    state.post_created_at_ms.remove(&post_id);
    state.post_safe.remove(&post_id);
    state.post_external_code.remove(&post_id);
    state.send_plans.remove(&post_id);
    state.sending.remove(&post_id);

    if let Some(review_id) = state.post_review_id.remove(&post_id) {
        if let Some(info) = state.review_info.remove(&review_id) {
            state.review_by_code.remove(&info.review_code);
        }
        state.review_publish_attempts.remove(&review_id);
        state.review_submitter.remove(&review_id);
        state.processed_reviews.remove(&review_id);
        state
            .audit_msg_to_review
            .retain(|_, value| *value != review_id);
    }
    if let Some(review_code) = state.post_review_code.remove(&post_id) {
        state.review_by_code.remove(&review_code);
    }

    let removed_ingress = state
        .post_ingress
        .remove(&post_id)
        .unwrap_or_else(|| ingress_ids.to_vec());
    for ingress_id in removed_ingress {
        let still_referenced = state
            .post_ingress
            .values()
            .any(|ids| ids.contains(&ingress_id));
        if !still_referenced {
            state.ingress_summary.remove(&ingress_id);
            state.pending_summary.remove(&ingress_id);
        }
    }
}

fn delete_persisted_blob_file(path: &str) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            debug_log!("blob gc remove failed: path={} error={}", path, err);
        }
    }
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
            set_group_user_notification_settings(
                &runtime.group_id,
                runtime.user_notifications.clone(),
            );
            set_group_agent_commands(&runtime.group_id, runtime.agent_commands.clone());
            set_group_agent_command_admins(&runtime.group_id, runtime.agent_command_admins.clone());
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

    let mut writer = tokio::spawn(async move {
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
    let mut reader = tokio::spawn(async move {
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
                &cmd_tx_read,
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
    let cmd_tx_bus = cmd_tx.clone();
    let account_id_bus = account_id.clone();
    let mut bus_task = tokio::spawn(async move {
        loop {
            let env = match bus_task_rx.recv().await {
                Ok(env) => env,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            let action = build_action_from_event(
                &runtime_bus,
                &state_bus,
                &cmd_tx_bus,
                &out_tx_bus,
                &account_id_bus,
                env.ts_ms,
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

    // A closed WebSocket reader must tear down the whole session; otherwise
    // reconnects are treated as duplicate accounts until the app restarts.
    tokio::select! {
        _ = &mut writer => {}
        _ = &mut reader => {}
        _ = &mut bus_task => {}
    }
    writer.abort();
    reader.abort();
    bus_task.abort();
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

fn group_user_notification_settings()
-> &'static std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<UserNotificationSettings>>>> {
    GROUP_USER_NOTIFICATION_SETTINGS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn group_agent_commands() -> &'static std::sync::Mutex<
    HashMap<String, Arc<std::sync::Mutex<HashMap<String, AgentCommandConfig>>>>,
> {
    GROUP_AGENT_COMMANDS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn group_agent_command_admins()
-> &'static std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<Vec<String>>>>> {
    GROUP_AGENT_COMMAND_ADMINS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
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

fn set_group_user_notification_settings(
    group_id: &str,
    config: Arc<std::sync::Mutex<UserNotificationSettings>>,
) {
    let mut guard = match group_user_notification_settings().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(group_id.to_string(), config);
}

fn set_group_agent_commands(
    group_id: &str,
    config: Arc<std::sync::Mutex<HashMap<String, AgentCommandConfig>>>,
) {
    let mut guard = match group_agent_commands().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(group_id.to_string(), config);
}

fn set_group_agent_command_admins(group_id: &str, admins: Arc<std::sync::Mutex<Vec<String>>>) {
    let mut guard = match group_agent_command_admins().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(group_id.to_string(), admins);
}

pub fn update_group_user_notification_settings(
    group_id: &str,
    config: UserNotificationSettings,
) -> Result<(), String> {
    let guard = match group_user_notification_settings().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    let Some(shared) = guard.get(group_id).cloned() else {
        return Err(format!(
            "group {} user_notifications runtime not found",
            group_id
        ));
    };
    let mut shared_guard = shared
        .lock()
        .map_err(|_| format!("group {} user_notifications lock poisoned", group_id))?;
    *shared_guard = config;
    Ok(())
}

pub fn update_group_agent_commands(
    group_id: &str,
    commands: HashMap<String, AgentCommandConfig>,
) -> Result<(), String> {
    let guard = match group_agent_commands().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    let Some(shared) = guard.get(group_id).cloned() else {
        return Err(format!(
            "group {} agent_commands runtime not found",
            group_id
        ));
    };
    let mut shared_guard = shared
        .lock()
        .map_err(|_| format!("group {} agent_commands lock poisoned", group_id))?;
    *shared_guard = commands;
    Ok(())
}

pub fn update_group_agent_command_admins(
    group_id: &str,
    admins: Vec<String>,
) -> Result<(), String> {
    let guard = match group_agent_command_admins().lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    let Some(shared) = guard.get(group_id).cloned() else {
        return Err(format!(
            "group {} agent_command_admins runtime not found",
            group_id
        ));
    };
    let mut shared_guard = shared
        .lock()
        .map_err(|_| format!("group {} agent_command_admins lock poisoned", group_id))?;
    *shared_guard = admins;
    Ok(())
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
    cmd_tx: &mpsc::Sender<Command>,
    out_tx: &mpsc::Sender<String>,
    account_id: &str,
    event_timestamp_ms: i64,
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
            route_meta,
            ..
        })
        | Event::Ingress(IngressEvent::MessageSynced {
            ingress_id,
            user_id,
            sender_name,
            message,
            route_meta,
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
                    route_meta,
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
            created_at_ms,
            draft,
            ..
        }) => {
            let is_new_post = {
                let mut guard = state.lock().await;
                let is_new_post = !guard.post_ingress.contains_key(&post_id);
                guard.post_ingress.insert(post_id, ingress_ids);
                guard.post_draft.insert(post_id, draft);
                guard.post_group.insert(post_id, group_id);
                guard.post_stage.insert(post_id, PostStage::Drafted);
                guard.post_created_at_ms.insert(post_id, created_at_ms);
                guard.post_safe.insert(post_id, is_safe);
                is_new_post
            };
            if is_new_post {
                spawn_submission_agent_commands(
                    runtime,
                    state,
                    cmd_tx,
                    out_tx,
                    account_id,
                    post_id,
                    event_timestamp_ms,
                )
                .await;
            }
            None
        }
        Event::Draft(DraftEvent::DraftTransformsSet { .. }) => None,
        Event::Lifecycle(LifecycleEvent::PostEvicted {
            post_id,
            blob_ids,
            ingress_ids,
            ..
        }) => {
            let mut guard = state.lock().await;
            evict_napcat_post_cache(&mut guard, post_id, &ingress_ids);
            blob_cache::release_many(blob_ids);
            None
        }
        Event::Review(ReviewEvent::ReviewInfoSynced {
            review_id,
            post_id,
            review_code,
        }) => {
            let mut guard = state.lock().await;
            let group_id = guard.post_group.get(&post_id).cloned().unwrap_or_default();
            let previous = guard.review_info.get(&review_id).cloned();
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                    group_id,
                    decision: previous.as_ref().and_then(|info| info.decision),
                    decided_by: previous.as_ref().and_then(|info| info.decided_by.clone()),
                    decided_at_ms: previous.and_then(|info| info.decided_at_ms),
                },
            );
            guard.post_review_id.insert(post_id, review_id);
            guard.review_by_code.insert(review_code, review_id);
            guard.post_review_code.insert(post_id, review_code);
            guard.post_stage.insert(post_id, PostStage::ReviewPending);
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
        Event::Blob(BlobEvent::BlobReleased { blob_id }) => {
            let mut guard = state.lock().await;
            guard.blob_paths.remove(&blob_id);
            blob_cache::release(blob_id);
            None
        }
        Event::Blob(BlobEvent::BlobGcRequested { blob_id }) => {
            let path = {
                let mut guard = state.lock().await;
                guard.blob_paths.remove(&blob_id)
            };
            blob_cache::release(blob_id);
            if let Some(path) = path {
                delete_persisted_blob_file(&path);
            }
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
            let previous = guard.review_info.get(&review_id).cloned();
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                    group_id,
                    decision: previous.as_ref().and_then(|info| info.decision),
                    decided_by: previous.as_ref().and_then(|info| info.decided_by.clone()),
                    decided_at_ms: previous.and_then(|info| info.decided_at_ms),
                },
            );
            guard.post_review_id.insert(post_id, review_id);
            guard.review_by_code.insert(review_code, review_id);
            guard.post_review_code.insert(post_id, review_code);
            guard.post_stage.insert(post_id, PostStage::ReviewPending);
            if let Some(user_id) = resolve_post_submitter(&guard, post_id) {
                guard.review_submitter.insert(review_id, user_id);
            }
            let group_id = guard.post_group.get(&post_id).cloned().unwrap_or_default();
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            let Some(user_id) = resolve_post_submitter(&guard, post_id) else {
                debug_log!("napcat review queued notify skipped: missing submitter info");
                return None;
            };
            let settings = runtime
                .user_notifications
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            let context = build_user_notification_context(
                &guard,
                runtime,
                &settings,
                post_id,
                UserNotificationStage::ReviewQueued,
                "",
                event_timestamp_ms,
                None,
            );
            let message = build_user_notification_message(
                &settings,
                UserNotificationStage::ReviewQueued,
                &context,
            );
            if message.is_empty() {
                return None;
            }
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message
                }
            });
            let _ = out_tx.send(payload.to_string()).await;
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
            decided_at_ms,
            ..
        }) => {
            let settings = runtime
                .user_notifications
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            let should_notify_reject = matches!(decision, ReviewDecision::Rejected);
            let should_notify_recall_deleted =
                matches!(decision, ReviewDecision::Deleted) && decided_by == "system_recall";
            let (submitter, reject_post_id, recall_group_msg) = {
                let mut guard = state.lock().await;
                if let Some(info) = guard.review_info.get_mut(&review_id) {
                    info.decision = Some(decision);
                    info.decided_by = Some(decided_by.clone());
                    info.decided_at_ms = Some(decided_at_ms);
                }
                if let Some(post_id) = guard.review_info.get(&review_id).map(|info| info.post_id) {
                    let stage = match decision {
                        ReviewDecision::Approved => PostStage::Reviewed,
                        ReviewDecision::Rejected => PostStage::Rejected,
                        ReviewDecision::Deferred => PostStage::ReviewPending,
                        ReviewDecision::Skipped => PostStage::Skipped,
                        ReviewDecision::Deleted => PostStage::Deleted,
                    };
                    guard.post_stage.insert(post_id, stage);
                }
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
                let submitter = if should_notify_reject {
                    resolve_review_submitter(&guard, review_id)
                } else {
                    None
                };
                let reject_post_id = if should_notify_reject {
                    guard.review_info.get(&review_id).map(|info| info.post_id)
                } else {
                    None
                };
                let recall_group_msg = if should_notify_recall_deleted {
                    guard.review_info.get(&review_id).map(|info| {
                        format!("发件者撤回了#{}的全部内容,已自动删除稿件", info.review_code)
                    })
                } else {
                    None
                };
                (submitter, reject_post_id, recall_group_msg)
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
            /*
            if stacking_enabled {
                let text = format!("{}宸插瓨鍏ユ殏瀛樺尯", label_plain);
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(target_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                return Some(payload.to_string());
            }
            */
            if !should_notify_reject {
                return None;
            }
            let Some((group_id, user_id)) = submitter else {
                debug_log!("napcat reject notify skipped: missing submitter info");
                return None;
            };
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            /*
            let text = "你的投稿已被拒，请修改后再发送";
            */
            let Some(post_id) = reject_post_id else {
                debug_log!("napcat reject notify skipped: missing post info");
                return None;
            };
            let context = {
                let guard = state.lock().await;
                build_user_notification_context(
                    &guard,
                    runtime,
                    &settings,
                    post_id,
                    UserNotificationStage::Rejected,
                    "",
                    decided_at_ms,
                    None,
                )
            };
            let message = build_user_notification_message(
                &settings,
                UserNotificationStage::Rejected,
                &context,
            );
            if message.is_empty() {
                return None;
            }
            /*
            if stacking_enabled {
                let text = format!("{}宸插瓨鍏ユ殏瀛樺尯", label_plain);
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(&audit_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                return Some(payload.to_string());
            }
            let text = format!("{}姝ｅ湪鍙戦€?..", label);
            */
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message
                }
            });
            {
                let mut guard = state.lock().await;
                record_thank_you_feedback(
                    &mut guard,
                    &user_id,
                    ThankYouFeedbackKind::Rejected,
                    decided_at_ms,
                );
            }
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
            {
                let mut guard = state.lock().await;
                record_thank_you_feedback(
                    &mut guard,
                    &user_id,
                    ThankYouFeedbackKind::ManualReply,
                    now_ms(),
                );
            }
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
            {
                let mut guard = state.lock().await;
                record_thank_you_feedback(
                    &mut guard,
                    &user_id,
                    ThankYouFeedbackKind::ManualReply,
                    now_ms(),
                );
            }
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
            let settings = runtime
                .user_notifications
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            let (label, label_plain, submitter_id, should_notify, audit_group_id, user_message) = {
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
                guard.post_stage.insert(post_id, PostStage::Scheduled);
                let queue_event_ms = guard
                    .post_review_id
                    .get(&post_id)
                    .and_then(|review_id| guard.review_info.get(review_id))
                    .and_then(|info| info.decided_at_ms)
                    .unwrap_or(not_before_ms);
                let context = build_user_notification_context(
                    &guard,
                    runtime,
                    &settings,
                    post_id,
                    UserNotificationStage::QueueEntered,
                    "",
                    queue_event_ms,
                    Some(not_before_ms),
                );
                let user_message = build_user_notification_message(
                    &settings,
                    UserNotificationStage::QueueEntered,
                    &context,
                );
                (
                    post_label(&guard, post_id),
                    post_label_plain(&guard, post_id),
                    resolve_post_submitter(&guard, post_id),
                    group_id == runtime.group_id,
                    runtime.audit_group_id.clone(),
                    user_message,
                )
            };
            if !should_notify {
                return None;
            }
            let Some(audit_group_id) = audit_group_id else {
                return None;
            };
            if let Some(user_id) = submitter_id.as_ref() {
                if !user_message.is_empty() {
                    let payload = serde_json::json!({
                        "action": "send_private_msg",
                        "params": {
                            "user_id": json_id(user_id),
                            "message": user_message
                        }
                    });
                    let _ = out_tx.send(payload.to_string()).await;
                }
            }
            /*
                let code_text: Option<String> = None;
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
                * /
                if stacking_enabled {
                    let text = format!("{}宸插瓨鍏ユ殏瀛樺尯", label_plain);
                    let payload = serde_json::json!({
                        "action": "send_group_msg",
                        "params": {
                            "group_id": json_id(&audit_group_id),
                            "message": message_segments_from_text(&text)
                        }
                    });
                    return Some(payload.to_string());
                }
                let text = format!("{}姝ｅ湪鍙戦€?..", label);
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(&audit_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                Some(payload.to_string())
            }
                */
            if stacking_enabled {
                /*
                        let text = format!("{}宸插瓨鍏ユ殏瀛樺尯", label_plain);
                        let payload = serde_json::json!({
                            "action": "send_group_msg",
                            "params": {
                                "group_id": json_id(&audit_group_id),
                                "message": message_segments_from_text(&text)
                            }
                        });
                        return Some(payload.to_string());
                    }
                    let text = format!("{}姝ｅ湪鍙戦€?..", label);
                    let payload = serde_json::json!({
                        "action": "send_group_msg",
                        "params": {
                            "group_id": json_id(&audit_group_id),
                            "message": message_segments_from_text(&text)
                        }
                    });
                    Some(payload.to_string())
                }
                        */
                let text = format!("{} queued", label_plain);
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(&audit_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                return Some(payload.to_string());
            }
            let text = format!("{} sending", label);
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
            guard.post_stage.insert(post_id, PostStage::Scheduled);
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
                    started_at_ms,
                    runtime.max_queue,
                    runtime.max_images_per_post,
                );
                let batch_label = post_batch_label(&guard, &batch_posts);
                for batch_post_id in batch_posts {
                    guard.post_stage.insert(batch_post_id, PostStage::Sending);
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
        Event::Send(SendEvent::SendSucceeded {
            post_id,
            account_id,
            finished_at_ms,
            ..
        }) => {
            let settings = runtime
                .user_notifications
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            let (group_id, submitter_id, context) = {
                let mut guard = state.lock().await;
                let sending_info = guard.sending.remove(&post_id);
                guard.post_stage.insert(post_id, PostStage::Sent);
                let group_id = sending_info
                    .as_ref()
                    .map(|info| info.group_id.clone())
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_else(|| runtime.group_id.clone());
                let submitter_id = resolve_post_submitter(&guard, post_id);
                let context = build_user_notification_context(
                    &guard,
                    runtime,
                    &settings,
                    post_id,
                    UserNotificationStage::SendSucceeded,
                    &account_id,
                    finished_at_ms,
                    None,
                );
                (group_id, submitter_id, context)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            if let Some(user_id) = submitter_id {
                let message = build_user_notification_message(
                    &settings,
                    UserNotificationStage::SendSucceeded,
                    &context,
                );
                if message.is_empty() {
                    return None;
                }
                let payload = serde_json::json!({
                    "action": "send_private_msg",
                    "params": {
                        "user_id": json_id(&user_id),
                        "message": message
                    }
                });
                {
                    let mut guard = state.lock().await;
                    record_thank_you_feedback(
                        &mut guard,
                        &user_id,
                        ThankYouFeedbackKind::SendSucceeded,
                        finished_at_ms,
                    );
                }
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
                guard.post_stage.insert(post_id, PostStage::Failed);
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
                guard.post_stage.insert(post_id, PostStage::Failed);
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
            let preview = rendered_png_preview(info.post_id);
            let is_safe = guard.post_safe.get(&info.post_id).copied().unwrap_or(true);
            let summary = build_audit_message(
                info.review_code,
                info.post_id,
                &ingress_ids,
                &guard.ingress_summary,
                preview,
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
    cmd_tx: &mpsc::Sender<Command>,
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
                        route_meta: None,
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
                if let Some((command_name, command_args)) =
                    parse_private_agent_command_line(raw_trimmed)
                {
                    match private_agent_command_match_with_state(
                        runtime,
                        &guard,
                        &command_name,
                        &user_id,
                    ) {
                        PrivateAgentCommandMatch::Execute => {
                            drop(guard);
                            spawn_private_agent_command(
                                runtime.clone(),
                                Arc::clone(state),
                                cmd_tx.clone(),
                                out_tx.clone(),
                                user_id.clone(),
                                sender_name.clone(),
                                self_id.clone(),
                                raw_trimmed.to_string(),
                                raw_trimmed.to_string(),
                                command_name,
                                command_args,
                                timestamp_ms,
                            );
                            return None;
                        }
                        PrivateAgentCommandMatch::IgnoredBlacklisted => {
                            drop(guard);
                            return None;
                        }
                        PrivateAgentCommandMatch::NoMatch => {}
                    }
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
        if let Some((command_name, command_args)) = parse_private_agent_command_line(raw_trimmed) {
            match private_agent_command_match(runtime, state, &command_name, &user_id).await {
                PrivateAgentCommandMatch::Execute => {
                    spawn_private_agent_command(
                        runtime.clone(),
                        Arc::clone(state),
                        cmd_tx.clone(),
                        out_tx.clone(),
                        user_id.clone(),
                        sender_name.clone(),
                        self_id.clone(),
                        raw_trimmed.to_string(),
                        text.trim().to_string(),
                        command_name,
                        command_args,
                        timestamp_ms,
                    );
                    return None;
                }
                PrivateAgentCommandMatch::IgnoredBlacklisted => return None,
                PrivateAgentCommandMatch::NoMatch => {}
            }
        }
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
        let thank_you_feedback = {
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
            current_thank_you_feedback(&guard, runtime, &user_id, timestamp_ms)
        };
        if let Some(feedback) = thank_you_feedback {
            if let Some(matched) = thankyou_filter::evaluate_message(
                &runtime.thank_you_filter,
                feedback.kind,
                value.get("message"),
                raw_message,
                thank_you_http_client(),
            )
            .await
            {
                let mut guard = state.lock().await;
                if current_thank_you_feedback(&guard, runtime, &user_id, timestamp_ms).is_some() {
                    mark_thank_you_silenced(&mut guard, &user_id);
                    debug_log!(
                        "napcat inbound private thank-you silenced: user_id={} rule={}",
                        user_id,
                        matched.rule
                    );
                    return None;
                }
            }
        }
        {
            let mut guard = state.lock().await;
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
            route_meta: None,
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
    let agent_commands = runtime
        .agent_commands
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
        "系统自检报告\n组: {}\n审核群: {}\nNapCat: {} (token {})\n账号: 配置 {} 个, 在线 {} 个\n账号列表: {}\n待审核: {}\n待发送: {}\n发送中: {}\n黑名单: {}\n快捷回复: {}\n审核快捷指令: {}\n全局快捷指令: {}\nAgent 指令: {}\n队列策略: max_post_stack={}",
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
        agent_commands,
        runtime.max_queue
    )
}

fn quick_reply_key_conflicts(key: &str) -> bool {
    is_builtin_review_command_name(key)
}

fn is_builtin_private_submission_command_name(name: &str) -> bool {
    matches!(
        name.trim(),
        "开始投稿" | "结束投稿" | "确认" | "取消" | "追加"
    )
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
    started_at_ms: i64,
    max_queue: usize,
    max_images_per_post: usize,
) -> Vec<PostId> {
    let mut queued = state
        .send_plans
        .iter()
        .filter(|(_, plan)| {
            plan.group_id == group_id
                && plan.priority == leader_priority
                && plan.not_before_ms <= started_at_ms
        })
        .map(|(post_id, plan)| (plan.seq, *post_id))
        .collect::<Vec<_>>();
    queued.sort_by_key(|(seq, post_id)| (*seq, post_id.0));
    let max_batch_posts = if max_queue == 0 {
        usize::MAX
    } else {
        max_queue.max(1)
    };
    let mut out = Vec::with_capacity(max_batch_posts.min(queued.len().saturating_add(1)));
    let mut total_images = count_post_notify_images(state, leader);
    out.push(leader);
    if leader_priority != SendPriority::Normal || max_batch_posts <= 1 {
        return out;
    }
    for (_, post_id) in queued {
        if post_id != leader {
            if out.len() >= max_batch_posts {
                break;
            }
            let image_count = count_post_notify_images(state, post_id);
            if max_images_per_post > 0
                && total_images.saturating_add(image_count) > max_images_per_post
            {
                break;
            }
            out.push(post_id);
            total_images = total_images.saturating_add(image_count);
        }
    }
    out
}

fn count_post_notify_images(state: &NapCatState, post_id: PostId) -> usize {
    let mut total = 0usize;
    if rendered_png_preview(post_id).is_some() {
        total = total.saturating_add(1);
    }
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            let Some(summary) = state.ingress_summary.get(ingress_id) else {
                continue;
            };
            total = total.saturating_add(
                summary
                    .attachments
                    .iter()
                    .filter(|attachment| attachment.kind == MediaKind::Image)
                    .count(),
            );
        }
    }
    total
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

#[derive(Debug, Clone, Default)]
struct PostRouteMeta {
    source_webhook: String,
    source_webhook_tag: String,
    raw_tags: Vec<String>,
    mapped_tags: Vec<String>,
}

#[derive(Debug, Clone)]
struct UserNotificationTemplateContext {
    stage: String,
    code: String,
    external_code: String,
    internal_code: String,
    post_id: String,
    review_id: String,
    group_id: String,
    sender_id: String,
    account_id: String,
    send_time: String,
    send_timestamp_ms: String,
    reviewer: String,
    reviewer_display: String,
    reviewed_at: String,
    queue_time: String,
    queue_timestamp_ms: String,
    scheduled_for: String,
    scheduled_timestamp_ms: String,
    source_webhook: String,
    source_webhook_tag: String,
    raw_tag_list: String,
    tag_list: String,
    tag_count: String,
    mapped_tags: Vec<String>,
}

fn build_user_notification_context(
    state: &NapCatState,
    runtime: &NapCatRuntimeConfig,
    settings: &UserNotificationSettings,
    post_id: PostId,
    stage: UserNotificationStage,
    account_id: &str,
    event_timestamp_ms: i64,
    scheduled_timestamp_ms: Option<i64>,
) -> UserNotificationTemplateContext {
    let review_id = state.post_review_id.get(&post_id).copied().or_else(|| {
        state
            .review_info
            .iter()
            .find_map(|(review_id, info)| (info.post_id == post_id).then_some(*review_id))
    });
    let review_info = review_id.and_then(|id| state.review_info.get(&id));
    let route_meta = collect_post_route_meta(state, settings, post_id);
    let scheduled_for = scheduled_timestamp_ms
        .map(|ts| format_local_datetime(ts, runtime.tz_offset_minutes))
        .unwrap_or_default();
    UserNotificationTemplateContext {
        stage: stage.as_str().to_string(),
        code: post_code_text(state, post_id).unwrap_or_default(),
        external_code: state
            .post_external_code
            .get(&post_id)
            .map(|code| code.to_string())
            .unwrap_or_default(),
        internal_code: state
            .post_review_code
            .get(&post_id)
            .map(|code| code.to_string())
            .unwrap_or_default(),
        post_id: post_id.0.to_string(),
        review_id: review_id.map(|id| id.0.to_string()).unwrap_or_default(),
        group_id: state
            .post_group
            .get(&post_id)
            .cloned()
            .unwrap_or_else(|| runtime.group_id.clone()),
        sender_id: resolve_post_submitter(state, post_id).unwrap_or_default(),
        account_id: account_id.trim().to_string(),
        send_time: if matches!(stage, UserNotificationStage::SendSucceeded) {
            format_local_datetime(event_timestamp_ms, runtime.tz_offset_minutes)
        } else {
            String::new()
        },
        send_timestamp_ms: if matches!(stage, UserNotificationStage::SendSucceeded) {
            event_timestamp_ms.to_string()
        } else {
            String::new()
        },
        reviewer: review_info
            .and_then(|info| info.decided_by.as_deref())
            .unwrap_or_default()
            .to_string(),
        reviewer_display: review_info
            .and_then(|info| info.decided_by.as_deref())
            .map(display_operator_name)
            .unwrap_or_default()
            .to_string(),
        reviewed_at: review_info
            .and_then(|info| info.decided_at_ms)
            .map(|ts| format_local_datetime(ts, runtime.tz_offset_minutes))
            .unwrap_or_default(),
        queue_time: if matches!(
            stage,
            UserNotificationStage::QueueEntered | UserNotificationStage::ReviewQueued
        ) {
            format_local_datetime(event_timestamp_ms, runtime.tz_offset_minutes)
        } else {
            String::new()
        },
        queue_timestamp_ms: if matches!(
            stage,
            UserNotificationStage::QueueEntered | UserNotificationStage::ReviewQueued
        ) {
            event_timestamp_ms.to_string()
        } else {
            String::new()
        },
        scheduled_for,
        scheduled_timestamp_ms: scheduled_timestamp_ms
            .map(|ts| ts.to_string())
            .unwrap_or_default(),
        source_webhook: route_meta.source_webhook,
        source_webhook_tag: route_meta.source_webhook_tag,
        raw_tag_list: route_meta.raw_tags.join(", "),
        tag_list: route_meta.mapped_tags.join(", "),
        tag_count: route_meta.mapped_tags.len().to_string(),
        mapped_tags: route_meta.mapped_tags,
    }
}

fn collect_post_route_meta(
    state: &NapCatState,
    settings: &UserNotificationSettings,
    post_id: PostId,
) -> PostRouteMeta {
    let mut source_webhook = String::new();
    let mut raw_tags = Vec::new();
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            let Some(summary) = state.ingress_summary.get(ingress_id) else {
                continue;
            };
            let Some(route_meta) = summary.route_meta.as_ref() else {
                continue;
            };
            if source_webhook.is_empty() {
                if let Some(webhook) = route_meta
                    .source_webhook
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    source_webhook = webhook.to_string();
                }
            }
            for tag in &route_meta.tags {
                push_unique_string(&mut raw_tags, tag.trim());
            }
        }
    }
    let source_webhook_tag = settings
        .webhook_tag_map
        .get(source_webhook.as_str())
        .map(|value| map_tag_value(settings, value))
        .unwrap_or_default();
    if !source_webhook_tag.is_empty() {
        push_unique_string(&mut raw_tags, source_webhook_tag.as_str());
    }
    let mapped_tags = raw_tags
        .iter()
        .map(|tag| map_tag_value(settings, tag))
        .filter(|tag| !tag.is_empty())
        .fold(Vec::new(), |mut acc, tag| {
            push_unique_string(&mut acc, tag.as_str());
            acc
        });
    PostRouteMeta {
        source_webhook,
        source_webhook_tag,
        raw_tags,
        mapped_tags,
    }
}

fn map_tag_value(settings: &UserNotificationSettings, raw: &str) -> String {
    let source = raw.trim().to_string();
    if source.is_empty() {
        return String::new();
    }
    let normalized = source
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string();
    let try_group = |group_name: &str| {
        settings
            .tag_value_maps
            .iter()
            .find(|group| group.tag.trim() == group_name)
            .and_then(|group| {
                group
                    .mappings
                    .iter()
                    .find(|item| item.source.trim() == source || item.source.trim() == normalized)
                    .map(|item| {
                        let target = item.target.trim().to_string();
                        if target.is_empty() {
                            source.clone()
                        } else {
                            target
                        }
                    })
            })
    };

    try_group(&normalized)
        .or_else(|| try_group(&source))
        .or_else(|| {
            settings.tag_value_maps.iter().find_map(|group| {
                group
                    .mappings
                    .iter()
                    .find(|item| item.source.trim() == source)
                    .map(|item| {
                        let target = item.target.trim().to_string();
                        if target.is_empty() {
                            source.clone()
                        } else {
                            target
                        }
                    })
            })
        })
        .unwrap_or(source)
}

fn push_unique_string(values: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() || values.iter().any(|value| value == trimmed) {
        return;
    }
    values.push(trimmed.to_string());
}

fn render_user_notification_template(
    template: &str,
    context: &UserNotificationTemplateContext,
    settings: &UserNotificationSettings,
) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'<' {
            if let Some(close_rel) = template[idx + 1..].find('>') {
                let close = idx + 1 + close_rel;
                let key = template[idx + 1..close].trim();
                if let Some(value) = user_notification_variable_value(context, key) {
                    out.push_str(&map_tag_value_for_group(settings, key, value));
                    idx = close + 1;
                    continue;
                }
            }
        }
        let ch = template[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

fn map_tag_value_for_group(
    settings: &UserNotificationSettings,
    group_name: &str,
    raw: &str,
) -> String {
    let source = raw.trim().to_string();
    if source.is_empty() {
        return String::new();
    }
    let normalized_group = group_name
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    let normalized_source = source
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string();
    settings
        .tag_value_maps
        .iter()
        .find(|group| group.tag.trim() == normalized_group)
        .and_then(|group| {
            group
                .mappings
                .iter()
                .find(|item| {
                    item.source.trim() == source
                        || item.source.trim() == normalized_source
                        || item.source.trim() == normalized_group
                })
                .map(|item| {
                    let target = item.target.trim().to_string();
                    if target.is_empty() {
                        source.clone()
                    } else {
                        target
                    }
                })
        })
        .unwrap_or(source)
}

fn user_notification_variable_value<'a>(
    context: &'a UserNotificationTemplateContext,
    key: &str,
) -> Option<&'a str> {
    match key {
        "stage" => Some(context.stage.as_str()),
        "code" => Some(context.code.as_str()),
        "external_code" => Some(context.external_code.as_str()),
        "internal_code" => Some(context.internal_code.as_str()),
        "post_id" => Some(context.post_id.as_str()),
        "review_id" => Some(context.review_id.as_str()),
        "group_id" => Some(context.group_id.as_str()),
        "sender_id" => Some(context.sender_id.as_str()),
        "account_id" => Some(context.account_id.as_str()),
        "send_time" => Some(context.send_time.as_str()),
        "send_timestamp_ms" => Some(context.send_timestamp_ms.as_str()),
        "reviewer" => Some(context.reviewer.as_str()),
        "reviewer_display" => Some(context.reviewer_display.as_str()),
        "reviewed_at" => Some(context.reviewed_at.as_str()),
        "queue_time" => Some(context.queue_time.as_str()),
        "queue_timestamp_ms" => Some(context.queue_timestamp_ms.as_str()),
        "scheduled_for" => Some(context.scheduled_for.as_str()),
        "scheduled_timestamp_ms" => Some(context.scheduled_timestamp_ms.as_str()),
        "source_webhook" => Some(context.source_webhook.as_str()),
        "source_webhook_tag" => Some(context.source_webhook_tag.as_str()),
        "raw_tag_list" => Some(context.raw_tag_list.as_str()),
        "tag_list" => Some(context.tag_list.as_str()),
        "tag_count" => Some(context.tag_count.as_str()),
        _ => None,
    }
}

fn split_rendered_tag_values(rendered: &str) -> Vec<String> {
    rendered
        .split(|ch| matches!(ch, ',' | '，' | ';' | '；' | '|' | '\n' | '\r'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn build_user_notification_message(
    settings: &UserNotificationSettings,
    stage: UserNotificationStage,
    context: &UserNotificationTemplateContext,
) -> Vec<Value> {
    let template = settings.stage(stage);
    if !template.enabled {
        return Vec::new();
    }

    let mut tags = Vec::new();
    if template.include_post_tags {
        for tag in &context.mapped_tags {
            push_unique_string(&mut tags, tag);
        }
    }
    for configured_tag in &template.tags {
        let rendered = render_user_notification_template(configured_tag, context, settings);
        for tag in split_rendered_tag_values(&rendered) {
            let mapped = map_tag_value(settings, &tag);
            push_unique_string(&mut tags, &mapped);
        }
    }

    let tag_prefix = tags
        .iter()
        .map(|tag| format!("[{}]", tag))
        .collect::<Vec<_>>()
        .join(" ");
    let rendered_text =
        render_user_notification_template(&template.text_template, context, settings);
    let trimmed_text = rendered_text.trim();

    let mut message = Vec::new();
    let text = match (tag_prefix.is_empty(), trimmed_text.is_empty()) {
        (false, false) => format!("{}\n{}", tag_prefix, trimmed_text),
        (false, true) => tag_prefix,
        (true, false) => trimmed_text.to_string(),
        (true, true) => String::new(),
    };
    if !text.is_empty() {
        message.extend(message_segments_from_text(&text));
    }
    for image in &template.images {
        let rendered = render_user_notification_template(image, context, settings);
        let trimmed = rendered.trim();
        if trimmed.is_empty() {
            continue;
        }
        message.push(serde_json::json!({
            "type": "image",
            "data": { "file": trimmed }
        }));
    }
    message
}

#[derive(Debug, Clone)]
struct AgentCommandTemplateContext {
    command_name: String,
    command_args: String,
    command_text: String,
    raw_message: String,
    message_text: String,
    sender_id: String,
    sender_name: String,
    group_id: String,
    account_id: String,
    received_at: String,
    received_timestamp_ms: String,
    submission_session_active: bool,
    submission_session_message_count: usize,
    previous_post_id: String,
    previous_post_code: String,
    previous_post_external_code: String,
    previous_post_internal_code: String,
    previous_post_info: String,
    previous_post_created_at: String,
    previous_post_created_timestamp_ms: String,
    submission_post_id: String,
    submission_sender_id: String,
    submission_sender_name: String,
    submission_message_count: String,
    submission_image_count: String,
    submission_text_message_count: String,
    submission_is_multi_image_single_text: String,
}

#[derive(Debug, Clone)]
struct AgentCommandExecutionMeta {
    trigger: AgentCommandTrigger,
    submission_post_id: Option<PostId>,
    user_id: String,
    sender_name: Option<String>,
    account_id: String,
    raw_message: String,
    message_text: String,
    command_args: String,
    timestamp_ms: i64,
}

#[derive(Debug, Clone, Default)]
struct SubmissionTemplateData {
    post_id: String,
    sender_id: String,
    sender_name: String,
    message_count: String,
    image_count: String,
    text_message_count: String,
    is_multi_image_single_text: String,
}

fn find_latest_sender_post(state: &NapCatState, group_id: &str, sender_id: &str) -> Option<PostId> {
    state
        .post_created_at_ms
        .iter()
        .filter_map(|(post_id, created_at_ms)| {
            let post_group = state.post_group.get(post_id)?;
            if post_group != group_id {
                return None;
            }
            let submitter = resolve_post_submitter(state, *post_id)?;
            if submitter != sender_id {
                return None;
            }
            Some((*created_at_ms, post_id.0, *post_id))
        })
        .max_by_key(|(created_at_ms, post_key, _)| (*created_at_ms, *post_key))
        .map(|(_, _, post_id)| post_id)
}

fn build_agent_post_summary(state: &NapCatState, post_id: PostId) -> String {
    let mut lines = Vec::new();
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            let Some(summary) = state.ingress_summary.get(ingress_id) else {
                continue;
            };
            if let Some(line) = sanitize_summary_line(&summary.text) {
                lines.push(line);
            }
            for attachment in &summary.attachments {
                if attachment.kind != MediaKind::Image {
                    lines.push(attachment_placeholder(attachment.kind).to_string());
                }
            }
        }
    }
    if lines.is_empty() {
        post_label(state, post_id)
    } else {
        lines.join(" | ")
    }
}

async fn build_agent_command_context(
    state: &Arc<Mutex<NapCatState>>,
    runtime: &NapCatRuntimeConfig,
    command_name: &str,
    command_args: &str,
    raw_message: &str,
    message_text: &str,
    user_id: &str,
    sender_name: Option<&str>,
    account_id: &str,
    timestamp_ms: i64,
    submission_post_id: Option<PostId>,
) -> AgentCommandTemplateContext {
    let (
        submission_session_active,
        submission_session_message_count,
        previous_post_id,
        previous_post_code,
        previous_post_external_code,
        previous_post_internal_code,
        previous_post_info,
        previous_post_created_at,
        previous_post_created_timestamp_ms,
        submission_data,
    ) = {
        let guard = state.lock().await;
        let (submission_session_active, submission_session_message_count) =
            match guard.submission_sessions.get(user_id) {
                Some(session) => (true, session.messages.len()),
                None => (false, 0usize),
            };
        let previous_post = find_latest_sender_post(&guard, &runtime.group_id, user_id);
        let previous_post_id = previous_post
            .map(|post_id| post_id.0.to_string())
            .unwrap_or_default();
        let previous_post_code = previous_post
            .and_then(|post_id| post_code_text(&guard, post_id))
            .unwrap_or_default();
        let previous_post_external_code = previous_post
            .and_then(|post_id| guard.post_external_code.get(&post_id).copied())
            .map(|code| code.to_string())
            .unwrap_or_default();
        let previous_post_internal_code = previous_post
            .and_then(|post_id| guard.post_review_code.get(&post_id).copied())
            .map(|code| code.to_string())
            .unwrap_or_default();
        let previous_post_info = previous_post
            .map(|post_id| build_agent_post_summary(&guard, post_id))
            .unwrap_or_default();
        let previous_post_created_timestamp_ms = previous_post
            .and_then(|post_id| guard.post_created_at_ms.get(&post_id).copied())
            .unwrap_or_default();
        let previous_post_created_at = if previous_post_created_timestamp_ms > 0 {
            format_local_datetime(
                previous_post_created_timestamp_ms,
                runtime.tz_offset_minutes,
            )
        } else {
            String::new()
        };
        let submission_data = submission_post_id
            .map(|post_id| build_submission_template_data(&guard, post_id))
            .unwrap_or_default();
        (
            submission_session_active,
            submission_session_message_count,
            previous_post_id,
            previous_post_code,
            previous_post_external_code,
            previous_post_internal_code,
            previous_post_info,
            previous_post_created_at,
            if previous_post_created_timestamp_ms > 0 {
                previous_post_created_timestamp_ms.to_string()
            } else {
                String::new()
            },
            submission_data,
        )
    };
    AgentCommandTemplateContext {
        command_name: command_name.to_string(),
        command_args: command_args.trim().to_string(),
        command_text: raw_message.trim().to_string(),
        raw_message: raw_message.to_string(),
        message_text: message_text.to_string(),
        sender_id: user_id.to_string(),
        sender_name: sender_name.unwrap_or("").trim().to_string(),
        group_id: runtime.group_id.clone(),
        account_id: account_id.trim().to_string(),
        received_at: format_local_datetime(timestamp_ms, runtime.tz_offset_minutes),
        received_timestamp_ms: timestamp_ms.to_string(),
        submission_session_active,
        submission_session_message_count,
        previous_post_id,
        previous_post_code,
        previous_post_external_code,
        previous_post_internal_code,
        previous_post_info,
        previous_post_created_at,
        previous_post_created_timestamp_ms,
        submission_post_id: submission_data.post_id,
        submission_sender_id: submission_data.sender_id,
        submission_sender_name: submission_data.sender_name,
        submission_message_count: submission_data.message_count,
        submission_image_count: submission_data.image_count,
        submission_text_message_count: submission_data.text_message_count,
        submission_is_multi_image_single_text: submission_data.is_multi_image_single_text,
    }
}

fn build_submission_template_data(state: &NapCatState, post_id: PostId) -> SubmissionTemplateData {
    let ingress_ids = state
        .post_ingress
        .get(&post_id)
        .cloned()
        .unwrap_or_default();
    let (sender_id, sender_name) = ingress_ids
        .first()
        .and_then(|ingress_id| state.ingress_summary.get(ingress_id))
        .map(|summary| {
            (
                summary.user_id.clone(),
                summary.sender_name.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    let mut image_count = 0usize;
    let mut text_message_count = 0usize;
    for ingress_id in &ingress_ids {
        let Some(summary) = state.ingress_summary.get(ingress_id) else {
            continue;
        };
        if !summary.text.trim().is_empty() && summary.attachments.is_empty() {
            text_message_count = text_message_count.saturating_add(1);
        }
        image_count = image_count.saturating_add(
            summary
                .attachments
                .iter()
                .filter(|attachment| attachment.kind == MediaKind::Image)
                .count(),
        );
    }
    SubmissionTemplateData {
        post_id: post_id.0.to_string(),
        sender_id,
        sender_name,
        message_count: ingress_ids.len().to_string(),
        image_count: image_count.to_string(),
        text_message_count: text_message_count.to_string(),
        is_multi_image_single_text: (image_count >= 2 && text_message_count == 1).to_string(),
    }
}

fn render_agent_command_template(template: &str, context: &AgentCommandTemplateContext) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'<' {
            if let Some(close_rel) = template[idx + 1..].find('>') {
                let close = idx + 1 + close_rel;
                let key = template[idx + 1..close].trim();
                if let Some(value) = agent_command_variable_value(context, key) {
                    out.push_str(&value);
                    idx = close + 1;
                    continue;
                }
            }
        }
        let ch = template[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

fn agent_command_variable_value(
    context: &AgentCommandTemplateContext,
    key: &str,
) -> Option<String> {
    match key {
        "command_name" => Some(context.command_name.clone()),
        "command_args" => Some(context.command_args.clone()),
        "command_text" => Some(context.command_text.clone()),
        "raw_message" => Some(context.raw_message.clone()),
        "message_text" => Some(context.message_text.clone()),
        "sender_id" => Some(context.sender_id.clone()),
        "sender_name" => Some(context.sender_name.clone()),
        "group_id" => Some(context.group_id.clone()),
        "account_id" => Some(context.account_id.clone()),
        "received_at" => Some(context.received_at.clone()),
        "received_timestamp_ms" => Some(context.received_timestamp_ms.clone()),
        "submission_session_active" => Some(context.submission_session_active.to_string()),
        "submission_session_message_count" => {
            Some(context.submission_session_message_count.to_string())
        }
        "previous_post_id" => Some(context.previous_post_id.clone()),
        "previous_post_code" => Some(context.previous_post_code.clone()),
        "previous_post_external_code" => Some(context.previous_post_external_code.clone()),
        "previous_post_internal_code" => Some(context.previous_post_internal_code.clone()),
        "previous_post_info" => Some(context.previous_post_info.clone()),
        "previous_post_created_at" => Some(context.previous_post_created_at.clone()),
        "previous_post_created_timestamp_ms" => {
            Some(context.previous_post_created_timestamp_ms.clone())
        }
        "submission_post_id" => Some(context.submission_post_id.clone()),
        "submission_sender_id" => Some(context.submission_sender_id.clone()),
        "submission_sender_name" => Some(context.submission_sender_name.clone()),
        "submission_message_count" => Some(context.submission_message_count.clone()),
        "submission_image_count" => Some(context.submission_image_count.clone()),
        "submission_text_message_count" => Some(context.submission_text_message_count.clone()),
        "submission_is_multi_image_single_text" => {
            Some(context.submission_is_multi_image_single_text.clone())
        }
        _ => None,
    }
}

fn build_agent_command_message(
    settings: &UserNotificationSettings,
    text_template: &str,
    tags: &[String],
    images: &[String],
    context: &AgentCommandTemplateContext,
) -> Vec<Value> {
    let rendered_tags = render_agent_command_tags(settings, tags, context);
    let tag_prefix = rendered_tags
        .iter()
        .map(|tag| format!("[{}]", tag))
        .collect::<Vec<_>>()
        .join(" ");
    let rendered_text = render_agent_command_template(text_template, context);
    let trimmed_text = rendered_text.trim();
    let mut message = Vec::new();
    let text = match (tag_prefix.is_empty(), trimmed_text.is_empty()) {
        (false, false) => format!("{}\n{}", tag_prefix, trimmed_text),
        (false, true) => tag_prefix,
        (true, false) => trimmed_text.to_string(),
        (true, true) => String::new(),
    };
    if !text.is_empty() {
        message.extend(message_segments_from_text(&text));
    }
    for image in images {
        let rendered = render_agent_command_template(image, context);
        let trimmed = rendered.trim();
        if trimmed.is_empty() {
            continue;
        }
        message.push(serde_json::json!({
            "type": "image",
            "data": { "file": trimmed }
        }));
    }
    message
}

fn render_agent_command_tags(
    settings: &UserNotificationSettings,
    tags: &[String],
    context: &AgentCommandTemplateContext,
) -> Vec<String> {
    let mut rendered_tags = Vec::new();
    for tag in tags {
        let rendered = render_agent_command_template(tag, context);
        for raw_tag in split_rendered_tag_values(&rendered) {
            let mapped = map_tag_value(settings, &raw_tag);
            push_unique_string(&mut rendered_tags, &mapped);
        }
    }
    rendered_tags
}

fn render_agent_command_images(
    images: &[String],
    context: &AgentCommandTemplateContext,
) -> Vec<String> {
    images
        .iter()
        .map(|image| render_agent_command_template(image, context))
        .map(|image| image.trim().to_string())
        .filter(|image| !image.is_empty())
        .collect()
}

fn parse_agent_command_review_code(value: &str) -> Result<ReviewCode, String> {
    let trimmed = value.trim().trim_start_matches('#').trim();
    if trimmed.is_empty() {
        return Err("审核编号不能为空".to_string());
    }
    trimmed
        .parse::<ReviewCode>()
        .map_err(|_| format!("无效的审核编号: {}", trimmed))
}

fn parse_agent_command_external_code(value: &str) -> Result<ExternalCode, String> {
    let trimmed = value.trim().trim_start_matches('#').trim();
    if trimmed.is_empty() {
        return Err("外部编号不能为空".to_string());
    }
    trimmed
        .parse::<ExternalCode>()
        .map_err(|_| format!("无效的外部编号: {}", trimmed))
}

fn resolve_agent_review_id_by_code(
    state: &NapCatState,
    group_id: &str,
    review_code: ReviewCode,
) -> Result<ReviewId, String> {
    let Some(review_id) = state.review_by_code.get(&review_code).copied() else {
        return Err(format!("找不到审核编号 #{}", review_code));
    };
    let Some(info) = state.review_info.get(&review_id) else {
        return Err(format!("找不到审核编号 #{}", review_code));
    };
    if !info.group_id.is_empty() && info.group_id != group_id {
        return Err(format!("审核编号 #{} 不属于当前分组", review_code));
    }
    Ok(review_id)
}

fn resolve_agent_post_id_by_code(
    state: &NapCatState,
    group_id: &str,
    value: &str,
) -> Result<PostId, String> {
    let trimmed = value.trim().trim_start_matches('#').trim();
    if trimmed.is_empty() {
        return Err("投稿编号不能为空".to_string());
    }
    if let Ok(review_code) = trimmed.parse::<ReviewCode>() {
        if let Some(review_id) = state.review_by_code.get(&review_code).copied() {
            let Some(info) = state.review_info.get(&review_id) else {
                return Err(format!("找不到编号 #{} 对应的稿件", review_code));
            };
            if !info.group_id.is_empty() && info.group_id != group_id {
                return Err(format!("编号 #{} 不属于当前分组", review_code));
            }
            return Ok(info.post_id);
        }
    }
    let external_code = trimmed
        .parse::<ExternalCode>()
        .map_err(|_| format!("无效的投稿编号: {}", trimmed))?;
    let Some((post_id, post_group)) =
        state.post_external_code.iter().find_map(|(post_id, code)| {
            (*code == external_code).then(|| (*post_id, state.post_group.get(post_id).cloned()))
        })
    else {
        return Err(format!("找不到编号 #{} 对应的稿件", external_code));
    };
    if let Some(post_group) = post_group {
        if !post_group.is_empty() && post_group != group_id {
            return Err(format!("编号 #{} 不属于当前分组", external_code));
        }
    }
    Ok(post_id)
}

fn build_agent_review_info_text(
    state: &NapCatState,
    group_id: &str,
    review_code: ReviewCode,
    tz_offset_minutes: i32,
) -> Result<String, String> {
    let review_id = resolve_agent_review_id_by_code(state, group_id, review_code)?;
    let Some(info) = state.review_info.get(&review_id) else {
        return Err(format!("找不到审核编号 #{}", review_code));
    };
    let post_id = info.post_id;
    let display_code = post_code_text(state, post_id).unwrap_or_else(|| review_code.to_string());
    let external_code = state
        .post_external_code
        .get(&post_id)
        .map(|code| code.to_string())
        .unwrap_or_else(|| "未分配".to_string());
    let sender_id = resolve_post_submitter(state, post_id).unwrap_or_else(|| "未知".to_string());
    let group = state
        .post_group
        .get(&post_id)
        .cloned()
        .unwrap_or_else(|| group_id.to_string());
    let created_at = state
        .post_created_at_ms
        .get(&post_id)
        .copied()
        .map(|ts| format_local_datetime(ts, tz_offset_minutes))
        .unwrap_or_else(|| "未知".to_string());
    let decision = match info.decision {
        Some(ReviewDecision::Approved) => "已通过",
        Some(ReviewDecision::Rejected) => "已拒稿",
        Some(ReviewDecision::Deferred) => "已延后",
        Some(ReviewDecision::Skipped) => "已跳过",
        Some(ReviewDecision::Deleted) => "已删除",
        None => "待审核",
    };
    let reviewer = info
        .decided_by
        .as_deref()
        .map(display_operator_name)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("-");
    let reviewed_at = info
        .decided_at_ms
        .map(|ts| format_local_datetime(ts, tz_offset_minutes))
        .unwrap_or_else(|| "-".to_string());
    let summary = build_agent_post_summary(state, post_id);
    Ok([
        format!("稿件信息 #{}", display_code),
        format!("内部编号: #{}", review_code),
        format!("外部编号: {}", external_code),
        format!("post_id: {}", post_id.0),
        format!("review_id: {}", review_id.0),
        format!("分组: {}", group),
        format!("投稿人: {}", sender_id),
        format!("创建时间: {}", created_at),
        format!("审核状态: {}", decision),
        format!("审核人: {}", reviewer),
        format!("审核时间: {}", reviewed_at),
        format!("摘要: {}", summary),
    ]
    .join("\n"))
}

fn build_submission_session_ingress(
    runtime: &NapCatRuntimeConfig,
    state: &mut NapCatState,
    account_id: &str,
    user_id: &str,
    timestamp_ms: i64,
) -> Result<Command, String> {
    let Some(session) = state.submission_sessions.remove(user_id) else {
        return Err("当前用户没有进行中的投稿会话".to_string());
    };
    if !session.confirming {
        state
            .submission_sessions
            .insert(user_id.to_string(), session);
        return Err("请先结束投稿，再执行提交。".to_string());
    }
    if session.messages.is_empty() {
        return Err("当前投稿会话没有可提交的内容".to_string());
    }
    let first_msg = session
        .messages
        .first()
        .ok_or_else(|| "当前投稿会话没有可提交的内容".to_string())?;
    let first_msg_id = value_opt_to_string(first_msg.message.get("message_id"))
        .unwrap_or_else(|| format!("submission-{}", session.started_at_ms));
    let sender_name = extract_sender_name(&first_msg.message).or_else(|| Some(user_id.to_string()));
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
        account_id.as_bytes(),
        chat_id.as_bytes(),
        user_id.as_bytes(),
        first_msg_id.as_bytes(),
    ]);
    state.pending_summary.insert(ingress_id, combined_summary);
    let received_at_ms = if session.started_at_ms > 0 {
        session.started_at_ms
    } else {
        timestamp_ms
    };
    Ok(Command::Ingress(IngressCommand {
        profile_id: account_id.to_string(),
        chat_id,
        user_id: user_id.to_string(),
        sender_name,
        group_id: if session.group_id.trim().is_empty() {
            runtime.group_id.clone()
        } else {
            session.group_id
        },
        platform_msg_id: first_msg_id,
        message: IngressMessage {
            text: combined_text,
            attachments: combined_attachments,
        },
        route_meta: None,
        received_at_ms,
        close_immediately: true,
    }))
}

fn build_agent_review_action(
    action: &AgentCommandReviewAction,
    context: &AgentCommandTemplateContext,
) -> Result<ReviewAction, String> {
    Ok(match action {
        AgentCommandReviewAction::Approve => ReviewAction::Approve,
        AgentCommandReviewAction::Reject => ReviewAction::Reject,
        AgentCommandReviewAction::Delete => ReviewAction::Delete,
        AgentCommandReviewAction::Defer { delay_ms } => ReviewAction::Defer {
            delay_ms: render_agent_command_template(delay_ms, context)
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("无效的延后毫秒数: {}", delay_ms.trim()))?,
        },
        AgentCommandReviewAction::Skip => ReviewAction::Skip,
        AgentCommandReviewAction::Immediate => ReviewAction::Immediate,
        AgentCommandReviewAction::Refresh => ReviewAction::Refresh,
        AgentCommandReviewAction::Rerender => ReviewAction::Rerender,
        AgentCommandReviewAction::SelectAllMessages => ReviewAction::SelectAllMessages,
        AgentCommandReviewAction::ToggleAnonymous => ReviewAction::ToggleAnonymous,
        AgentCommandReviewAction::ExpandAudit => ReviewAction::ExpandAudit,
        AgentCommandReviewAction::Show => ReviewAction::Show,
        AgentCommandReviewAction::Comment { text_template } => ReviewAction::Comment {
            text: render_agent_command_template(text_template, context)
                .trim()
                .to_string(),
        },
        AgentCommandReviewAction::Reply { text_template } => ReviewAction::Reply {
            text: render_agent_command_template(text_template, context)
                .trim()
                .to_string(),
        },
        AgentCommandReviewAction::Blacklist { reason_template } => {
            let rendered = render_agent_command_template(reason_template, context);
            ReviewAction::Blacklist {
                reason: (!rendered.trim().is_empty()).then(|| rendered.trim().to_string()),
            }
        }
        AgentCommandReviewAction::QuickReply { key_template } => ReviewAction::QuickReply {
            key: render_agent_command_template(key_template, context)
                .trim()
                .to_string(),
        },
        AgentCommandReviewAction::Merge { target_review_code } => ReviewAction::Merge {
            review_code: parse_agent_command_review_code(
                render_agent_command_template(target_review_code, context).trim(),
            )?,
        },
    })
}

fn build_agent_global_action(
    action: &AgentCommandGlobalAction,
    context: &AgentCommandTemplateContext,
) -> Result<GlobalAction, String> {
    Ok(match action {
        AgentCommandGlobalAction::Help => GlobalAction::Help,
        AgentCommandGlobalAction::Recall { review_code } => GlobalAction::Recall {
            review_code: parse_agent_command_review_code(
                render_agent_command_template(review_code, context).trim(),
            )?,
        },
        AgentCommandGlobalAction::Withdraw { review_code } => GlobalAction::Withdraw {
            review_code: parse_agent_command_review_code(
                render_agent_command_template(review_code, context).trim(),
            )?,
        },
        AgentCommandGlobalAction::Info { review_code } => GlobalAction::Info {
            review_code: parse_agent_command_review_code(
                render_agent_command_template(review_code, context).trim(),
            )?,
        },
        AgentCommandGlobalAction::ManualRelogin => GlobalAction::ManualRelogin,
        AgentCommandGlobalAction::AutoRelogin => GlobalAction::AutoRelogin,
        AgentCommandGlobalAction::PendingList => GlobalAction::PendingList,
        AgentCommandGlobalAction::PendingClear => GlobalAction::PendingClear,
        AgentCommandGlobalAction::SendQueueClear => GlobalAction::SendQueueClear,
        AgentCommandGlobalAction::SendQueueFlush => GlobalAction::SendQueueFlush,
        AgentCommandGlobalAction::SendInFlightClear => GlobalAction::SendInFlightClear,
        AgentCommandGlobalAction::BlacklistList => GlobalAction::BlacklistList,
        AgentCommandGlobalAction::BlacklistAdd {
            sender_id,
            reason_template,
        } => {
            let rendered_reason = render_agent_command_template(reason_template, context);
            GlobalAction::BlacklistAdd {
                sender_id: render_agent_command_template(sender_id, context)
                    .trim()
                    .to_string(),
                reason: (!rendered_reason.trim().is_empty())
                    .then(|| rendered_reason.trim().to_string()),
            }
        }
        AgentCommandGlobalAction::BlacklistRemove { sender_id } => GlobalAction::BlacklistRemove {
            sender_id: render_agent_command_template(sender_id, context)
                .trim()
                .to_string(),
        },
        AgentCommandGlobalAction::SetExternalNumber { value_template } => {
            GlobalAction::SetExternalNumber {
                value: parse_agent_command_external_code(
                    render_agent_command_template(value_template, context).trim(),
                )?,
            }
        }
        AgentCommandGlobalAction::QuickReplyList => GlobalAction::QuickReplyList,
        AgentCommandGlobalAction::QuickReplyAdd {
            key_template,
            text_template,
        } => GlobalAction::QuickReplyAdd {
            key: render_agent_command_template(key_template, context)
                .trim()
                .to_string(),
            text: render_agent_command_template(text_template, context)
                .trim()
                .to_string(),
        },
        AgentCommandGlobalAction::QuickReplyDelete { key_template } => {
            GlobalAction::QuickReplyDelete {
                key: render_agent_command_template(key_template, context)
                    .trim()
                    .to_string(),
            }
        }
        AgentCommandGlobalAction::ShortcutList => GlobalAction::ShortcutList,
        AgentCommandGlobalAction::ShortcutAdd {
            scope,
            key_template,
            definition_template,
        } => GlobalAction::ShortcutAdd {
            scope: match scope {
                AgentCommandShortcutScope::Review => ShortcutScope::Review,
                AgentCommandShortcutScope::Global => ShortcutScope::Global,
            },
            key: render_agent_command_template(key_template, context)
                .trim()
                .to_string(),
            definition: render_agent_command_template(definition_template, context)
                .trim()
                .to_string(),
        },
        AgentCommandGlobalAction::ShortcutDelete {
            scope,
            key_template,
        } => GlobalAction::ShortcutDelete {
            scope: match scope {
                AgentCommandShortcutScope::Review => ShortcutScope::Review,
                AgentCommandShortcutScope::Global => ShortcutScope::Global,
            },
            key: render_agent_command_template(key_template, context)
                .trim()
                .to_string(),
        },
        AgentCommandGlobalAction::SelfCheck => GlobalAction::SelfCheck,
        AgentCommandGlobalAction::SystemRepair => GlobalAction::SystemRepair,
    })
}

async fn execute_agent_insert_queued_post(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    moving_post_code: &str,
    anchor_post_code: &str,
    position: AgentCommandQueueInsertPosition,
) -> Result<(), String> {
    let events = {
        let guard = state.lock().await;
        let moving_post_id =
            resolve_agent_post_id_by_code(&guard, &runtime.group_id, moving_post_code)?;
        let anchor_post_id =
            resolve_agent_post_id_by_code(&guard, &runtime.group_id, anchor_post_code)?;
        if moving_post_id == anchor_post_id {
            return Err("不能把稿件插入到它自己前后".to_string());
        }
        let Some(moving_plan) = guard.send_plans.get(&moving_post_id).cloned() else {
            return Err(format!(
                "稿件 #{} 当前不在发送队列中",
                moving_post_code.trim()
            ));
        };
        let Some(anchor_plan) = guard.send_plans.get(&anchor_post_id).cloned() else {
            return Err(format!(
                "稿件 #{} 当前不在发送队列中",
                anchor_post_code.trim()
            ));
        };
        if moving_plan.group_id != runtime.group_id || anchor_plan.group_id != runtime.group_id {
            return Err("只能调整当前分组的发送队列".to_string());
        }
        let mut queue = guard
            .send_plans
            .iter()
            .filter_map(|(post_id, plan)| {
                (plan.group_id == runtime.group_id).then_some((*post_id, plan.clone()))
            })
            .collect::<Vec<_>>();
        queue.sort_by(|a, b| {
            (a.1.not_before_ms, a.1.priority, a.1.seq, a.0.0).cmp(&(
                b.1.not_before_ms,
                b.1.priority,
                b.1.seq,
                b.0.0,
            ))
        });
        let moving_entry = queue
            .iter()
            .find(|(post_id, _)| *post_id == moving_post_id)
            .cloned()
            .ok_or_else(|| format!("稿件 #{} 当前不在发送队列中", moving_post_code.trim()))?;
        queue.retain(|(post_id, _)| *post_id != moving_post_id);
        let Some(anchor_index) = queue
            .iter()
            .position(|(post_id, _)| *post_id == anchor_post_id)
        else {
            return Err(format!(
                "稿件 #{} 当前不在发送队列中",
                anchor_post_code.trim()
            ));
        };
        let insert_index = match position {
            AgentCommandQueueInsertPosition::Before => anchor_index,
            AgentCommandQueueInsertPosition::After => anchor_index.saturating_add(1),
        };
        let mut updated_plan = moving_entry.1.clone();
        updated_plan.group_id = anchor_plan.group_id.clone();
        updated_plan.not_before_ms = anchor_plan.not_before_ms;
        updated_plan.priority = anchor_plan.priority;
        queue.insert(
            insert_index.min(queue.len()),
            (moving_entry.0, updated_plan),
        );
        let mut next_seq = queue.iter().map(|(_, plan)| plan.seq).min().unwrap_or(1);
        queue
            .into_iter()
            .map(|(post_id, plan)| {
                let event = Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                    post_id,
                    group_id: plan.group_id,
                    not_before_ms: plan.not_before_ms,
                    priority: plan.priority,
                    seq: next_seq,
                });
                next_seq = next_seq.saturating_add(1);
                event
            })
            .collect::<Vec<_>>()
    };
    for event in events {
        cmd_tx
            .send(Command::DriverEvent(event))
            .await
            .map_err(|err| format!("发送队列调整事件失败: {}", err))?;
    }
    Ok(())
}

async fn execute_agent_review_action(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    review_code_text: &str,
    action: &AgentCommandReviewAction,
    context: &AgentCommandTemplateContext,
    operator_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    let review_code = parse_agent_command_review_code(review_code_text)?;
    let review_action = build_agent_review_action(action, context)?;
    let command = {
        let guard = state.lock().await;
        let review_id = resolve_agent_review_id_by_code(&guard, &runtime.group_id, review_code)?;
        if guard.processed_reviews.contains(&review_id) {
            return Err(format!("审核编号 #{} 已处理，不能重复执行", review_code));
        }
        Command::ReviewAction(ReviewActionCommand {
            review_id: Some(review_id),
            review_code: None,
            audit_msg_id: None,
            action: review_action,
            operator_id: operator_id.to_string(),
            now_ms,
            tz_offset_minutes: runtime.tz_offset_minutes,
        })
    };
    cmd_tx
        .send(command)
        .await
        .map_err(|err| format!("发送审核指令失败: {}", err))
}

async fn execute_agent_global_action(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    out_tx: &mpsc::Sender<String>,
    user_id: &str,
    action: &AgentCommandGlobalAction,
    context: &AgentCommandTemplateContext,
    operator_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    let global_action = build_agent_global_action(action, context)?;
    match &global_action {
        GlobalAction::Help => {
            send_private_text(out_tx, user_id, HELP_TEXT).await;
            return Ok(());
        }
        GlobalAction::PendingList => {
            let text = {
                let guard = state.lock().await;
                build_pending_list_text(&guard, &runtime.group_id)
            };
            send_private_text(out_tx, user_id, &text).await;
            return Ok(());
        }
        GlobalAction::BlacklistList => {
            let text = {
                let guard = state.lock().await;
                build_blacklist_list_text(&guard, &runtime.group_id)
            };
            send_private_text(out_tx, user_id, &text).await;
            return Ok(());
        }
        GlobalAction::QuickReplyList => {
            let text = build_quick_reply_list_text(runtime);
            send_private_text(out_tx, user_id, &text).await;
            return Ok(());
        }
        GlobalAction::ShortcutList => {
            let text = build_shortcut_list_text(runtime);
            send_private_text(out_tx, user_id, &text).await;
            return Ok(());
        }
        GlobalAction::SelfCheck => {
            let text = {
                let guard = state.lock().await;
                build_selfcheck_report(runtime, &guard)
            };
            send_private_text(out_tx, user_id, &text).await;
            return Ok(());
        }
        GlobalAction::Info { review_code } => {
            let text = {
                let guard = state.lock().await;
                build_agent_review_info_text(
                    &guard,
                    &runtime.group_id,
                    *review_code,
                    runtime.tz_offset_minutes,
                )?
            };
            send_private_text(out_tx, user_id, &text).await;
            return Ok(());
        }
        GlobalAction::ManualRelogin => {
            send_private_text(
                out_tx,
                user_id,
                "当前版本暂未把“手动重新登录”接入 agent 积木执行链，请在原审核面板或运维流程中执行。",
            )
            .await;
            return Ok(());
        }
        GlobalAction::AutoRelogin => {
            send_private_text(
                out_tx,
                user_id,
                "当前版本暂未把“自动重新登录”接入 agent 积木执行链，请在原审核面板或运维流程中执行。",
            )
            .await;
            return Ok(());
        }
        GlobalAction::SystemRepair => {
            send_private_text(
                out_tx,
                user_id,
                "当前版本暂未把“系统修复”接入 agent 积木执行链，请在原审核面板或运维流程中执行。",
            )
            .await;
            return Ok(());
        }
        _ => {}
    }
    {
        let mut guard = state.lock().await;
        if let Err(msg) = validate_global_action(&guard, &runtime.group_id, &global_action) {
            return Err(msg.to_string());
        }
        if let GlobalAction::Recall { review_code } = &global_action {
            if let Some(review_id) = guard.review_by_code.get(review_code).copied() {
                guard.processed_reviews.remove(&review_id);
            }
        }
    }
    cmd_tx
        .send(Command::GlobalAction(GlobalActionCommand {
            group_id: runtime.group_id.clone(),
            action: global_action,
            operator_id: operator_id.to_string(),
            now_ms,
            tz_offset_minutes: runtime.tz_offset_minutes,
        }))
        .await
        .map_err(|err| format!("发送全局指令失败: {}", err))
}

fn parse_private_agent_command_line(raw_trimmed: &str) -> Option<(String, String)> {
    let body = raw_trimmed.trim().strip_prefix('#')?;
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let mut iter = body.splitn(2, char::is_whitespace);
    let name = iter.next()?.trim();
    if name.is_empty() {
        return None;
    }
    let args = iter.next().unwrap_or("").trim().to_string();
    Some((name.to_string(), args))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateAgentCommandMatch {
    Execute,
    IgnoredBlacklisted,
    NoMatch,
}

fn is_blacklisted_agent_command_sender(
    state: &NapCatState,
    runtime: &NapCatRuntimeConfig,
    user_id: &str,
) -> bool {
    state
        .blacklist
        .get(&runtime.group_id)
        .map(|entries| entries.contains_key(user_id))
        .unwrap_or(false)
}

fn is_agent_command_admin(runtime: &NapCatRuntimeConfig, user_id: &str) -> bool {
    let guard = runtime
        .agent_command_admins
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    guard.iter().any(|admin| admin.trim() == user_id)
}

fn private_agent_command_match_with_state(
    runtime: &NapCatRuntimeConfig,
    state: &NapCatState,
    command_name: &str,
    user_id: &str,
) -> PrivateAgentCommandMatch {
    let command = {
        let guard = runtime
            .agent_commands
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        guard.get(command_name).cloned()
    };
    let Some(command) = command else {
        return PrivateAgentCommandMatch::NoMatch;
    };
    if !command.enabled {
        return PrivateAgentCommandMatch::NoMatch;
    }
    if command.trigger != AgentCommandTrigger::PrivateCommand {
        return PrivateAgentCommandMatch::NoMatch;
    }
    if is_blacklisted_agent_command_sender(state, runtime, user_id) {
        return PrivateAgentCommandMatch::IgnoredBlacklisted;
    }
    if command.admin_only && !is_agent_command_admin(runtime, user_id) {
        return PrivateAgentCommandMatch::NoMatch;
    }
    PrivateAgentCommandMatch::Execute
}

async fn private_agent_command_match(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    command_name: &str,
    user_id: &str,
) -> PrivateAgentCommandMatch {
    let guard = state.lock().await;
    private_agent_command_match_with_state(runtime, &guard, command_name, user_id)
}

fn spawn_private_agent_command(
    runtime: NapCatRuntimeConfig,
    state: Arc<Mutex<NapCatState>>,
    cmd_tx: mpsc::Sender<Command>,
    out_tx: mpsc::Sender<String>,
    user_id: String,
    sender_name: Option<String>,
    account_id: String,
    raw_message: String,
    message_text: String,
    command_name: String,
    command_args: String,
    timestamp_ms: i64,
) {
    tokio::spawn(async move {
        let result = execute_private_agent_command(
            &runtime,
            &state,
            &cmd_tx,
            &out_tx,
            &user_id,
            sender_name.as_deref(),
            &account_id,
            &raw_message,
            &message_text,
            &command_name,
            &command_args,
            timestamp_ms,
        )
        .await;
        if let Err(err) = result {
            debug_log!(
                "agent command execution failed group_id={} user_id={} command={} err={}",
                runtime.group_id,
                user_id,
                command_name,
                err
            );
            send_private_text(
                &out_tx,
                &user_id,
                &format!("指令 #{} 执行失败：{}", command_name, err),
            )
            .await;
        }
    });
}

async fn spawn_submission_agent_commands(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    out_tx: &mpsc::Sender<String>,
    account_id: &str,
    post_id: PostId,
    timestamp_ms: i64,
) {
    let mut commands = {
        let guard = runtime
            .agent_commands
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        guard
            .iter()
            .filter(|(_, command)| {
                command.enabled && command.trigger == AgentCommandTrigger::SubmissionReceived
            })
            .map(|(name, command)| (name.clone(), command.clone()))
            .collect::<Vec<_>>()
    };
    commands.sort_by(|left, right| left.0.cmp(&right.0));
    for (command_name, command) in commands {
        spawn_submission_agent_command(
            runtime.clone(),
            Arc::clone(state),
            cmd_tx.clone(),
            out_tx.clone(),
            account_id.to_string(),
            post_id,
            command_name,
            command,
            timestamp_ms,
        );
    }
}

fn spawn_submission_agent_command(
    runtime: NapCatRuntimeConfig,
    state: Arc<Mutex<NapCatState>>,
    cmd_tx: mpsc::Sender<Command>,
    out_tx: mpsc::Sender<String>,
    account_id: String,
    post_id: PostId,
    command_name: String,
    command: AgentCommandConfig,
    timestamp_ms: i64,
) {
    tokio::spawn(async move {
        let result = execute_submission_agent_command(
            &runtime,
            &state,
            &cmd_tx,
            &out_tx,
            &account_id,
            post_id,
            &command_name,
            command,
            timestamp_ms,
        )
        .await;
        if let Err(err) = result {
            debug_log!(
                "submission agent command failed group_id={} post_id={} command={} err={}",
                runtime.group_id,
                post_id.0,
                command_name,
                err
            );
        }
    });
}

fn agent_command_client() -> &'static Client {
    AGENT_WEBHOOK_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| Client::new())
    })
}

async fn execute_agent_command_blocks(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    out_tx: &mpsc::Sender<String>,
    settings: UserNotificationSettings,
    command_name: String,
    blocks: Vec<AgentCommandBlock>,
    meta: AgentCommandExecutionMeta,
    depth: usize,
) -> Result<(), String> {
    if depth > 8 {
        return Err("agent 指令积木嵌套过深".to_string());
    }
    for block in blocks {
        let context = build_agent_command_context(
            state,
            runtime,
            &command_name,
            &meta.command_args,
            &meta.raw_message,
            &meta.message_text,
            &meta.user_id,
            meta.sender_name.as_deref(),
            &meta.account_id,
            meta.timestamp_ms,
            meta.submission_post_id,
        )
        .await;
        match block {
            AgentCommandBlock::ReplyPrivateMessage {
                text_template,
                tags,
                images,
            } => {
                let message = build_agent_command_message(
                    &settings,
                    &text_template,
                    &tags,
                    &images,
                    &context,
                );
                if !message.is_empty() {
                    send_private_segments(out_tx, &meta.user_id, message).await;
                }
            }
            AgentCommandBlock::StartSubmissionSession => {
                ensure_private_agent_block(meta.trigger)?;
                let mut guard = state.lock().await;
                guard.submission_sessions.insert(
                    meta.user_id.clone(),
                    SubmissionSession {
                        messages: Vec::new(),
                        started_at_ms: meta.timestamp_ms,
                        group_id: runtime.group_id.clone(),
                        confirming: false,
                    },
                );
            }
            AgentCommandBlock::FinishSubmissionSession => {
                ensure_private_agent_block(meta.trigger)?;
                let mut guard = state.lock().await;
                if let Some(session) = guard.submission_sessions.get_mut(&meta.user_id) {
                    session.confirming = true;
                }
            }
            AgentCommandBlock::ResumeSubmissionSession => {
                ensure_private_agent_block(meta.trigger)?;
                let mut guard = state.lock().await;
                if let Some(session) = guard.submission_sessions.get_mut(&meta.user_id) {
                    session.confirming = false;
                }
            }
            AgentCommandBlock::SubmitSubmissionSession => {
                ensure_private_agent_block(meta.trigger)?;
                let mut guard = state.lock().await;
                let command = build_submission_session_ingress(
                    runtime,
                    &mut guard,
                    &meta.account_id,
                    &meta.user_id,
                    meta.timestamp_ms,
                )?;
                drop(guard);
                cmd_tx
                    .send(command)
                    .await
                    .map_err(|err| format!("提交投稿会话失败: {}", err))?;
                send_private_text(out_tx, &meta.user_id, "投稿会话已提交，系统正在继续处理。")
                    .await;
            }
            AgentCommandBlock::CancelSubmissionSession => {
                ensure_private_agent_block(meta.trigger)?;
                let mut guard = state.lock().await;
                guard.submission_sessions.remove(&meta.user_id);
            }
            AgentCommandBlock::InsertQueuedPost {
                moving_post_code,
                anchor_post_code,
                position,
            } => {
                let moving_post_code = render_agent_command_template(&moving_post_code, &context)
                    .trim()
                    .to_string();
                let anchor_post_code = render_agent_command_template(&anchor_post_code, &context)
                    .trim()
                    .to_string();
                execute_agent_insert_queued_post(
                    runtime,
                    state,
                    cmd_tx,
                    &moving_post_code,
                    &anchor_post_code,
                    position,
                )
                .await?;
            }
            AgentCommandBlock::ExecuteReviewAction {
                review_code,
                action,
            } => {
                ensure_private_agent_block(meta.trigger)?;
                let review_code = render_agent_command_template(&review_code, &context)
                    .trim()
                    .to_string();
                execute_agent_review_action(
                    runtime,
                    state,
                    cmd_tx,
                    &review_code,
                    &action,
                    &context,
                    &meta.user_id,
                    meta.timestamp_ms,
                )
                .await?;
            }
            AgentCommandBlock::ExecuteGlobalAction { action } => {
                let operator_id =
                    agent_command_operator_id(meta.trigger, &command_name, &meta.user_id);
                execute_agent_global_action(
                    runtime,
                    state,
                    cmd_tx,
                    out_tx,
                    &meta.user_id,
                    &action,
                    &context,
                    &operator_id,
                    meta.timestamp_ms,
                )
                .await?;
            }
            AgentCommandBlock::If {
                condition,
                then_blocks,
                else_blocks,
            } => {
                let post_id = meta
                    .submission_post_id
                    .ok_or_else(|| "if 条件只能用于收到新投稿触发的指令".to_string())?;
                let draft = {
                    let guard = state.lock().await;
                    guard.post_draft.get(&post_id).cloned()
                }
                .ok_or_else(|| "找不到当前触发稿件的草稿".to_string())?;
                let selected_blocks = if evaluate_condition(&draft, &condition) {
                    then_blocks
                } else {
                    else_blocks
                };
                Box::pin(execute_agent_command_blocks(
                    runtime,
                    state,
                    cmd_tx,
                    out_tx,
                    settings.clone(),
                    command_name.clone(),
                    selected_blocks,
                    meta.clone(),
                    depth + 1,
                ))
                .await?;
            }
            AgentCommandBlock::SetDraftTransforms { target, transforms } => {
                execute_agent_set_draft_transforms(
                    runtime,
                    state,
                    cmd_tx,
                    &command_name,
                    &context,
                    &meta,
                    &target,
                    transforms,
                )
                .await?;
            }
            AgentCommandBlock::SendWebhook {
                url,
                source_webhook,
                text_template,
                tags,
                images,
            } => {
                let rendered_url = render_agent_command_template(&url, &context);
                let target_url = rendered_url.trim();
                if target_url.is_empty() {
                    return Err("webhook 地址为空".to_string());
                }
                validate_agent_command_webhook_url(target_url)?;
                let rendered_source_webhook =
                    render_agent_command_template(&source_webhook, &context);
                let rendered_tags = render_agent_command_tags(&settings, &tags, &context);
                let rendered_images = render_agent_command_images(&images, &context);
                let rendered_text = render_agent_command_template(&text_template, &context);
                let payload = serde_json::json!({
                    "command_name": context.command_name,
                    "command_args": context.command_args,
                    "command_text": context.command_text,
                    "raw_message": context.raw_message,
                    "message_text": context.message_text,
                    "sender_id": context.sender_id,
                    "sender_name": context.sender_name,
                    "group_id": context.group_id,
                    "account_id": context.account_id,
                    "received_at": context.received_at,
                    "received_timestamp_ms": context.received_timestamp_ms,
                    "submission_session_active": context.submission_session_active,
                    "submission_session_message_count": context.submission_session_message_count,
                    "submission_post_id": context.submission_post_id,
                    "submission_sender_id": context.submission_sender_id,
                    "submission_sender_name": context.submission_sender_name,
                    "submission_message_count": context.submission_message_count,
                    "submission_image_count": context.submission_image_count,
                    "submission_text_message_count": context.submission_text_message_count,
                    "submission_is_multi_image_single_text": context.submission_is_multi_image_single_text,
                    "source_webhook": rendered_source_webhook.trim(),
                    "tags": rendered_tags,
                    "text": rendered_text.trim(),
                    "images": rendered_images,
                });
                let response = agent_command_client()
                    .post(target_url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|err| format!("webhook 请求失败: {}", err))?;
                response
                    .error_for_status()
                    .map_err(|err| format!("webhook 响应失败: {}", err))?;
            }
        }
    }
    Ok(())
}

fn ensure_private_agent_block(trigger: AgentCommandTrigger) -> Result<(), String> {
    if trigger == AgentCommandTrigger::PrivateCommand {
        Ok(())
    } else {
        Err("该积木只能用于私聊触发的 agent 指令".to_string())
    }
}

fn agent_command_operator_id(
    trigger: AgentCommandTrigger,
    command_name: &str,
    user_id: &str,
) -> String {
    match trigger {
        AgentCommandTrigger::PrivateCommand => user_id.to_string(),
        AgentCommandTrigger::SubmissionReceived => format!("agent:{}", command_name),
    }
}

async fn execute_agent_set_draft_transforms(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    command_name: &str,
    context: &AgentCommandTemplateContext,
    meta: &AgentCommandExecutionMeta,
    target: &AgentCommandPostTarget,
    transforms: Vec<DraftTransform>,
) -> Result<(), String> {
    for transform in &transforms {
        validate_transform(transform)?;
    }
    let post_id = match target {
        AgentCommandPostTarget::TriggeringPost => meta
            .submission_post_id
            .ok_or_else(|| "当前触发稿件目标只能用于收到新投稿触发的指令".to_string())?,
        AgentCommandPostTarget::ReviewCode { template } => {
            let rendered = render_agent_command_template(template, context);
            let guard = state.lock().await;
            resolve_agent_post_id_by_code(&guard, &runtime.group_id, rendered.trim())?
        }
    };
    {
        let guard = state.lock().await;
        if let Some(stage) = guard.post_stage.get(&post_id).copied() {
            if !agent_can_set_transforms_at_stage(stage) {
                return Err("当前稿件阶段不允许修改内容块规则".to_string());
            }
        }
    }
    cmd_tx
        .send(Command::PostAction(PostActionCommand {
            post_id,
            action: PostAction::SetDraftTransforms { transforms },
            operator_id: format!("agent:{}", command_name),
            now_ms: meta.timestamp_ms,
        }))
        .await
        .map_err(|err| format!("设置稿件内容块规则失败: {}", err))
}

fn agent_can_set_transforms_at_stage(stage: PostStage) -> bool {
    matches!(
        stage,
        PostStage::Drafted
            | PostStage::RenderRequested
            | PostStage::Rendered
            | PostStage::ReviewPending
            | PostStage::Reviewed
            | PostStage::Scheduled
            | PostStage::Failed
    )
}

async fn execute_submission_agent_command(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    out_tx: &mpsc::Sender<String>,
    account_id: &str,
    post_id: PostId,
    command_name: &str,
    command: AgentCommandConfig,
    timestamp_ms: i64,
) -> Result<(), String> {
    if !command.enabled || command.trigger != AgentCommandTrigger::SubmissionReceived {
        return Ok(());
    }
    let (user_id, sender_name) = {
        let guard = state.lock().await;
        resolve_post_submitter_with_name(&guard, post_id)
            .ok_or_else(|| "找不到投稿者信息".to_string())?
    };
    let settings = runtime
        .user_notifications
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    execute_agent_command_blocks(
        runtime,
        state,
        cmd_tx,
        out_tx,
        settings,
        command_name.to_string(),
        command.blocks,
        AgentCommandExecutionMeta {
            trigger: AgentCommandTrigger::SubmissionReceived,
            submission_post_id: Some(post_id),
            user_id,
            sender_name,
            account_id: account_id.to_string(),
            raw_message: String::new(),
            message_text: String::new(),
            command_args: String::new(),
            timestamp_ms,
        },
        0,
    )
    .await
}

async fn execute_private_agent_command(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    cmd_tx: &mpsc::Sender<Command>,
    out_tx: &mpsc::Sender<String>,
    user_id: &str,
    sender_name: Option<&str>,
    account_id: &str,
    raw_message: &str,
    message_text: &str,
    command_name: &str,
    command_args: &str,
    timestamp_ms: i64,
) -> Result<(), String> {
    let command = {
        let guard = runtime
            .agent_commands
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        guard.get(command_name).cloned()
    }
    .ok_or_else(|| format!("agent command not found: {}", command_name))?;
    if !command.enabled {
        return Ok(());
    }
    {
        let guard = state.lock().await;
        if is_blacklisted_agent_command_sender(&guard, runtime, user_id) {
            return Ok(());
        }
    }
    if command.admin_only && !is_agent_command_admin(runtime, user_id) {
        return Ok(());
    }
    if command.trigger != AgentCommandTrigger::PrivateCommand {
        return Ok(());
    }
    let settings = runtime
        .user_notifications
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    execute_agent_command_blocks(
        runtime,
        state,
        cmd_tx,
        out_tx,
        settings,
        command_name.to_string(),
        command.blocks,
        AgentCommandExecutionMeta {
            trigger: AgentCommandTrigger::PrivateCommand,
            submission_post_id: None,
            user_id: user_id.to_string(),
            sender_name: sender_name.map(str::to_string),
            account_id: account_id.to_string(),
            raw_message: raw_message.to_string(),
            message_text: message_text.to_string(),
            command_args: command_args.to_string(),
            timestamp_ms,
        },
        0,
    )
    .await
}

fn display_operator_name(raw: &str) -> &str {
    raw.strip_prefix("webview:")
        .or_else(|| raw.strip_prefix("api:"))
        .or_else(|| raw.strip_prefix("tui:"))
        .unwrap_or(raw)
}

fn format_local_datetime(ms: i64, tz_offset_minutes: i32) -> String {
    let offset_ms = i64::from(tz_offset_minutes).saturating_mul(60_000);
    let local = ms.saturating_add(offset_ms);
    let day = local.div_euclid(86_400_000);
    let time_ms = local.rem_euclid(86_400_000);
    let hour = time_ms.div_euclid(3_600_000);
    let minute = time_ms.rem_euclid(3_600_000).div_euclid(60_000);
    let second = time_ms.rem_euclid(60_000).div_euclid(1_000);
    format!(
        "{} {:02}:{:02}:{:02}",
        civil_from_days(day),
        hour,
        minute,
        second
    )
}

fn civil_from_days(days_since_epoch: i64) -> String {
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

fn resolve_post_submitter(state: &NapCatState, post_id: PostId) -> Option<String> {
    let ingress_ids = state.post_ingress.get(&post_id)?;
    resolve_post_submitter_with_ingress(state, ingress_ids)
}

fn resolve_post_submitter_with_name(
    state: &NapCatState,
    post_id: PostId,
) -> Option<(String, Option<String>)> {
    let ingress_ids = state.post_ingress.get(&post_id)?;
    ingress_ids.iter().find_map(|ingress_id| {
        let summary = state.ingress_summary.get(ingress_id)?;
        Some((summary.user_id.clone(), summary.sender_name.clone()))
    })
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
    preview_image: Option<String>,
    blob_paths: &HashMap<BlobId, String>,
    is_safe: bool,
) -> AuditMessage {
    let mut images = Vec::new();
    if let Some(preview) = preview_image {
        images.push(preview);
    }
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

fn rendered_png_preview(post_id: PostId) -> Option<String> {
    let blob_id = rendered_png_blob_id(post_id);
    if let Some(bytes) = blob_cache::get_bytes(blob_id) {
        return Some(format!("base64://{}", STANDARD.encode(bytes.as_ref())));
    }
    let path = rendered_png_path(post_id);
    let meta = fs::metadata(&path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    Some(file_uri_from_path(&path))
}

fn rendered_png_blob_id(post_id: PostId) -> BlobId {
    derive_blob_id(&[&post_id.to_be_bytes(), b"png"])
}

fn rendered_png_path(post_id: PostId) -> PathBuf {
    let blob_id = rendered_png_blob_id(post_id);
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
    send_private_segments(
        out_tx,
        user_id,
        vec![serde_json::json!({
            "type": "text",
            "data": { "text": text }
        })],
    )
    .await;
}

async fn send_private_segments(out_tx: &mpsc::Sender<String>, user_id: &str, message: Vec<Value>) {
    let payload = serde_json::json!({
        "action": "send_private_msg",
        "params": {
            "user_id": json_id(user_id),
            "message": message
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

fn thank_you_http_client() -> &'static Client {
    THANK_YOU_HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| Client::new())
    })
}

fn record_thank_you_feedback(
    state: &mut NapCatState,
    user_id: &str,
    kind: ThankYouFeedbackKind,
    sent_at_ms: i64,
) {
    state.thank_you_feedback.insert(
        user_id.to_string(),
        ThankYouFeedbackRecord {
            sent_at_ms,
            kind,
            silenced_count: 0,
        },
    );
}

fn current_thank_you_feedback(
    state: &NapCatState,
    runtime: &NapCatRuntimeConfig,
    user_id: &str,
    now_ms: i64,
) -> Option<ThankYouFeedbackRecord> {
    if !runtime.thank_you_filter.enabled {
        return None;
    }
    let record = state.thank_you_feedback.get(user_id)?;
    if record.silenced_count > 0 || now_ms < record.sent_at_ms {
        return None;
    }
    let age_ms = now_ms - record.sent_at_ms;
    let window_ms =
        i64::try_from(runtime.thank_you_filter.window_sec.saturating_mul(1000)).unwrap_or(i64::MAX);
    if age_ms > window_ms {
        return None;
    }
    Some(record.clone())
}

fn mark_thank_you_silenced(state: &mut NapCatState, user_id: &str) {
    if let Some(record) = state.thank_you_feedback.get_mut(user_id) {
        record.silenced_count = record.silenced_count.saturating_add(1);
    }
}

fn next_echo(state: &mut NapCatState) -> String {
    state.next_echo = state.next_echo.saturating_add(1);
    format!("echo-{}", state.next_echo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oqqwall_rust_core::Id128;
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
            max_images_per_post: 0,
            thank_you_filter: ThankYouFilterRuntimeConfig::disabled(),
            user_notifications: Arc::new(
                std::sync::Mutex::new(UserNotificationSettings::default()),
            ),
            quick_replies: Arc::new(std::sync::Mutex::new(HashMap::new())),
            review_shortcuts: Arc::new(std::sync::Mutex::new(HashMap::new())),
            global_shortcuts: Arc::new(std::sync::Mutex::new(HashMap::new())),
            agent_commands: Arc::new(std::sync::Mutex::new(HashMap::new())),
            agent_command_admins: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn parse_cmd(text: &str, has_reply: bool) -> Option<AuditCommand> {
        parse_audit_command(text, has_reply, &test_runtime())
    }

    fn test_agent_context() -> AgentCommandTemplateContext {
        AgentCommandTemplateContext {
            command_name: "test".to_string(),
            command_args: "args".to_string(),
            command_text: "#test args".to_string(),
            raw_message: "#test args".to_string(),
            message_text: "#test args".to_string(),
            sender_id: "20002".to_string(),
            sender_name: "sender".to_string(),
            group_id: "group-a".to_string(),
            account_id: "10001".to_string(),
            received_at: "1970-01-01 00:00:00".to_string(),
            received_timestamp_ms: "0".to_string(),
            submission_session_active: true,
            submission_session_message_count: 1,
            previous_post_id: "post-1".to_string(),
            previous_post_code: "42".to_string(),
            previous_post_external_code: "10042".to_string(),
            previous_post_internal_code: "42".to_string(),
            previous_post_info: "summary".to_string(),
            previous_post_created_at: "1970-01-01 00:00:00".to_string(),
            previous_post_created_timestamp_ms: "0".to_string(),
            submission_post_id: "13".to_string(),
            submission_sender_id: "20002".to_string(),
            submission_sender_name: "sender".to_string(),
            submission_message_count: "2".to_string(),
            submission_image_count: "2".to_string(),
            submission_text_message_count: "1".to_string(),
            submission_is_multi_image_single_text: "true".to_string(),
        }
    }

    fn clear_group_accounts_for_test(group_id: &str) {
        let mut guard = match group_accounts().lock() {
            Ok(guard) => guard,
            Err(err) => err.into_inner(),
        };
        guard.remove(group_id);
    }

    #[test]
    fn agent_command_code_templates_render_before_parse() {
        let context = test_agent_context();
        for template in [
            "<previous_post_internal_code>",
            " #<previous_post_internal_code> ",
        ] {
            let rendered = render_agent_command_template(template, &context);
            assert_eq!(
                parse_agent_command_review_code(rendered.trim()).expect("review code"),
                42
            );
        }
    }

    #[tokio::test]
    async fn private_agent_command_runs_during_confirming_submission_session() {
        let mut runtime = test_runtime();
        runtime.agent_commands = Arc::new(std::sync::Mutex::new(HashMap::from([(
            "取消投稿".to_string(),
            AgentCommandConfig {
                enabled: true,
                admin_only: false,
                trigger: AgentCommandTrigger::PrivateCommand,
                description: String::new(),
                blocks: vec![AgentCommandBlock::CancelSubmissionSession],
            },
        )])));
        let state = Arc::new(Mutex::new(NapCatState::default()));
        {
            let mut guard = state.lock().await;
            guard.submission_sessions.insert(
                "20002".to_string(),
                SubmissionSession {
                    messages: Vec::new(),
                    started_at_ms: 1_000,
                    group_id: runtime.group_id.clone(),
                    confirming: true,
                },
            );
        }
        let (cmd_tx, _cmd_rx) = mpsc::channel(4);
        let (out_tx, _out_rx) = mpsc::channel(4);
        let value = serde_json::json!({
            "post_type": "message",
            "message_type": "private",
            "self_id": "10001",
            "user_id": "20002",
            "message_id": "m1",
            "time": 1001,
            "raw_message": "#取消投稿",
            "message": [
                {"type": "text", "data": {"text": "#取消投稿"}}
            ]
        });

        let command =
            parse_inbound_event(&runtime, &state, &cmd_tx, &out_tx, "10001", &value).await;
        assert!(command.is_none());

        let mut removed = false;
        for _ in 0..10 {
            {
                let guard = state.lock().await;
                if !guard.submission_sessions.contains_key("20002") {
                    removed = true;
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
        assert!(removed, "agent command should cancel the active session");
    }

    #[tokio::test]
    async fn submission_agent_triggers_only_for_new_post() {
        let mut runtime = test_runtime();
        runtime.accounts = vec!["sub-agent-100".to_string()];
        register_ws_session("sub-agent-100", mock_session());
        runtime.agent_commands = Arc::new(std::sync::Mutex::new(HashMap::from([(
            "规则".to_string(),
            AgentCommandConfig {
                enabled: true,
                admin_only: false,
                trigger: AgentCommandTrigger::SubmissionReceived,
                description: String::new(),
                blocks: vec![AgentCommandBlock::SetDraftTransforms {
                    target: AgentCommandPostTarget::TriggeringPost,
                    transforms: vec![DraftTransform::MoveBlocks {
                        selector: oqqwall_rust_core::BlockSelector {
                            kinds: Some(vec![oqqwall_rust_core::BlockKindFilter::Paragraph]),
                            text: None,
                            index: None,
                        },
                        position: oqqwall_rust_core::PositionSpec::Front,
                    }],
                }],
            },
        )])));
        let state = Arc::new(Mutex::new(NapCatState::default()));
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let (out_tx, _out_rx) = mpsc::channel(4);
        let ingress_id = Id128(100);
        let post_id = Id128(101);
        let session_id = Id128(102);
        let draft = Draft {
            blocks: vec![oqqwall_rust_core::DraftBlock::Paragraph {
                text: "caption".to_string(),
            }],
        };

        let _ = build_action_from_event(
            &runtime,
            &state,
            &cmd_tx,
            &out_tx,
            "sub-agent-100",
            1,
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id,
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "20002".to_string(),
                sender_name: Some("sender".to_string()),
                group_id: runtime.group_id.clone(),
                platform_msg_id: "msg-1".to_string(),
                route_meta: None,
                received_at_ms: 1,
                message: IngressMessage {
                    text: "caption".to_string(),
                    attachments: Vec::new(),
                },
            }),
        )
        .await;
        let draft_event = Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: runtime.group_id.clone(),
            ingress_ids: vec![ingress_id],
            is_anonymous: false,
            is_safe: true,
            draft,
            created_at_ms: 2,
        });

        let _ = build_action_from_event(
            &runtime,
            &state,
            &cmd_tx,
            &out_tx,
            "sub-agent-100",
            2,
            draft_event.clone(),
        )
        .await;
        let command = tokio::time::timeout(Duration::from_secs(1), cmd_rx.recv())
            .await
            .expect("submission command")
            .expect("command value");
        assert!(matches!(
            command,
            Command::PostAction(PostActionCommand { post_id: id, .. }) if id == post_id
        ));

        let _ = build_action_from_event(
            &runtime,
            &state,
            &cmd_tx,
            &out_tx,
            "sub-agent-100",
            3,
            draft_event,
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), cmd_rx.recv())
                .await
                .is_err(),
            "rebuilt post should not retrigger submission agent"
        );
        unregister_ws_session("sub-agent-100");
    }

    #[test]
    fn private_match_ignores_submission_trigger_commands() {
        let mut runtime = test_runtime();
        runtime.agent_commands = Arc::new(std::sync::Mutex::new(HashMap::from([(
            "规则".to_string(),
            AgentCommandConfig {
                enabled: true,
                admin_only: false,
                trigger: AgentCommandTrigger::SubmissionReceived,
                description: String::new(),
                blocks: vec![AgentCommandBlock::ReplyPrivateMessage {
                    text_template: "ok".to_string(),
                    tags: Vec::new(),
                    images: Vec::new(),
                }],
            },
        )])));
        let state = NapCatState::default();

        assert_eq!(
            private_agent_command_match_with_state(&runtime, &state, "规则", "20002"),
            PrivateAgentCommandMatch::NoMatch
        );
    }

    #[test]
    fn validate_rejects_triggering_post_target_for_private_command() {
        let config = AgentCommandConfig {
            enabled: true,
            admin_only: false,
            trigger: AgentCommandTrigger::PrivateCommand,
            description: String::new(),
            blocks: vec![AgentCommandBlock::SetDraftTransforms {
                target: AgentCommandPostTarget::TriggeringPost,
                transforms: vec![DraftTransform::MoveBlocks {
                    selector: oqqwall_rust_core::BlockSelector {
                        kinds: Some(vec![oqqwall_rust_core::BlockKindFilter::Paragraph]),
                        text: None,
                        index: None,
                    },
                    position: oqqwall_rust_core::PositionSpec::Front,
                }],
            }],
        };

        assert!(validate_agent_command_config("规则", &config).is_err());
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
                action: ParsedReviewAction::Builtin(ReviewAction::Delete),
            })
        );
        assert_eq!(
            parse_cmd("123 拒", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ParsedReviewAction::Builtin(ReviewAction::Reject),
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

    #[tokio::test]
    async fn private_thank_you_reply_is_silenced_once_per_feedback_window() {
        let mut runtime = test_runtime();
        runtime.thank_you_filter = ThankYouFilterRuntimeConfig::with_registry_json(
            true,
            1800,
            16,
            6,
            r#"{"face_ids":[],"mfaces":[],"file_uniques":[],"images":[]}"#,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(NapCatState::default()));
        {
            let mut guard = state.lock().await;
            record_thank_you_feedback(
                &mut guard,
                "20002",
                ThankYouFeedbackKind::SendSucceeded,
                1_000_000,
            );
        }
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        let (out_tx, _out_rx) = mpsc::channel(1);
        let first = serde_json::json!({
            "post_type": "message",
            "message_type": "private",
            "self_id": "10001",
            "user_id": "20002",
            "message_id": "m1",
            "time": 1001,
            "raw_message": "谢谢",
            "message": [
                {"type": "text", "data": {"text": "谢谢"}}
            ]
        });
        let command =
            parse_inbound_event(&runtime, &state, &cmd_tx, &out_tx, "10001", &first).await;
        assert!(command.is_none());
        {
            let guard = state.lock().await;
            assert_eq!(guard.pending_summary.len(), 0);
            assert_eq!(
                guard
                    .thank_you_feedback
                    .get("20002")
                    .map(|record| record.silenced_count),
                Some(1)
            );
        }

        let second = serde_json::json!({
            "post_type": "message",
            "message_type": "private",
            "self_id": "10001",
            "user_id": "20002",
            "message_id": "m2",
            "time": 1002,
            "raw_message": "谢谢",
            "message": [
                {"type": "text", "data": {"text": "谢谢"}}
            ]
        });
        let command =
            parse_inbound_event(&runtime, &state, &cmd_tx, &out_tx, "10001", &second).await;
        assert!(matches!(command, Some(Command::Ingress(_))));
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
        let batch =
            collect_batch_post_ids_for_notify(&state, "g", leader, SendPriority::Normal, 0, 3, 0);
        assert_eq!(batch, vec![leader, second, third]);
    }

    #[test]
    fn collect_batch_post_ids_for_notify_respects_image_limit() {
        let leader = PostId::from_u128(21);
        let second = PostId::from_u128(22);
        let third = PostId::from_u128(23);
        let leader_ingress = IngressId::from_u128(121);
        let second_ingress = IngressId::from_u128(122);
        let third_ingress = IngressId::from_u128(123);
        let mut state = NapCatState::default();
        state.send_plans.insert(
            leader,
            SendPlanInfo {
                group_id: "g".to_string(),
                not_before_ms: 0,
                priority: SendPriority::Normal,
                seq: 1,
            },
        );
        state.send_plans.insert(
            second,
            SendPlanInfo {
                group_id: "g".to_string(),
                not_before_ms: 0,
                priority: SendPriority::Normal,
                seq: 2,
            },
        );
        state.send_plans.insert(
            third,
            SendPlanInfo {
                group_id: "g".to_string(),
                not_before_ms: 0,
                priority: SendPriority::Normal,
                seq: 3,
            },
        );
        state.post_ingress.insert(leader, vec![leader_ingress]);
        state.post_ingress.insert(second, vec![second_ingress]);
        state.post_ingress.insert(third, vec![third_ingress]);
        state
            .ingress_summary
            .insert(leader_ingress, make_ingress_summary_with_images("u1", 3));
        state
            .ingress_summary
            .insert(second_ingress, make_ingress_summary_with_images("u2", 3));
        state
            .ingress_summary
            .insert(third_ingress, make_ingress_summary_with_images("u3", 5));

        let batch =
            collect_batch_post_ids_for_notify(&state, "g", leader, SendPriority::Normal, 0, 3, 6);
        assert_eq!(batch, vec![leader, second]);
    }

    fn make_ingress_summary_with_images(user_id: &str, image_count: usize) -> IngressSummary {
        IngressSummary {
            user_id: user_id.to_string(),
            sender_name: Some(user_id.to_string()),
            text: String::new(),
            attachments: (0..image_count)
                .map(|idx| IngressAttachment {
                    kind: MediaKind::Image,
                    name: None,
                    reference: MediaReference::RemoteUrl {
                        url: format!("file:///tmp/{}_{}.png", user_id, idx),
                    },
                    size_bytes: None,
                })
                .collect(),
            route_meta: None,
        }
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
                decision: None,
                decided_by: None,
                decided_at_ms: None,
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
