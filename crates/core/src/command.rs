use crate::draft::IngressMessage;
use crate::event::Event;
use crate::ids::{AuditMsgId, ExternalCode, GroupId, ReviewCode, ReviewId, TimestampMs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ingress(IngressCommand),
    Tick(TickCommand),
    ReviewAction(ReviewActionCommand),
    ReviewActionBatch(ReviewActionBatchCommand),
    GlobalAction(GlobalActionCommand),
    GlobalActionBatch(GlobalActionBatchCommand),
    DriverEvent(Event),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressCommand {
    pub profile_id: String,
    pub chat_id: String,
    pub user_id: String,
    pub sender_name: Option<String>,
    pub group_id: GroupId,
    pub platform_msg_id: String,
    pub message: IngressMessage,
    pub received_at_ms: TimestampMs,
    pub close_immediately: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickCommand {
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewActionCommand {
    pub review_id: Option<ReviewId>,
    pub review_code: Option<ReviewCode>,
    pub audit_msg_id: Option<AuditMsgId>,
    pub action: ReviewAction,
    pub operator_id: String,
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewActionBatchCommand {
    pub review_id: Option<ReviewId>,
    pub review_code: Option<ReviewCode>,
    pub audit_msg_id: Option<AuditMsgId>,
    pub actions: Vec<ReviewAction>,
    pub operator_id: String,
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAction {
    Approve,
    Reject,
    Delete,
    Defer { delay_ms: TimestampMs },
    Skip,
    Immediate,
    Refresh,
    Rerender,
    SelectAllMessages,
    ToggleAnonymous,
    ExpandAudit,
    Show,
    Comment { text: String },
    Reply { text: String },
    Blacklist { reason: Option<String> },
    QuickReply { key: String },
    Merge { review_code: ReviewCode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutScope {
    Review,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalActionCommand {
    pub group_id: GroupId,
    pub action: GlobalAction,
    pub operator_id: String,
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalActionBatchCommand {
    pub group_id: GroupId,
    pub actions: Vec<GlobalAction>,
    pub operator_id: String,
    pub now_ms: TimestampMs,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalAction {
    Help,
    Recall {
        review_code: ReviewCode,
    },
    Withdraw {
        review_code: ReviewCode,
    },
    Info {
        review_code: ReviewCode,
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
        sender_id: String,
        reason: Option<String>,
    },
    BlacklistRemove {
        sender_id: String,
    },
    SetExternalNumber {
        value: ExternalCode,
    },
    QuickReplyList,
    QuickReplyAdd {
        key: String,
        text: String,
    },
    QuickReplyDelete {
        key: String,
    },
    ShortcutList,
    ShortcutAdd {
        scope: ShortcutScope,
        key: String,
        definition: String,
    },
    ShortcutDelete {
        scope: ShortcutScope,
        key: String,
    },
    SelfCheck,
    SystemRepair,
}
