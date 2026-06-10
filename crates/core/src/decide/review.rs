use crate::anonymous::detect_anonymous;
use crate::command::{ReviewAction, ReviewActionBatchCommand, ReviewActionCommand};
use crate::config::CoreConfig;
use crate::decide::builder::build_draft_from_messages;
use crate::decide::flush::build_group_flush_events;
use crate::decide::scheduler::{compute_not_before, day_index, minute_of_day};
use crate::draft::MediaKind;
use crate::event::{
    DraftEvent, Event, EventEnvelope, IngressEvent, RenderEvent, ReviewDecision, ReviewEvent,
    ScheduleEvent, SendPriority,
};
use crate::ids::{
    ActorId, ExternalCode, Id128, IngressId, PostId, ReviewCode, ReviewId, TimestampMs,
};
use crate::safety::detect_safe;
use crate::state::StateView;

const STACKING_HOLD_MS: i64 = 365 * 24 * 60 * 60 * 1000;

pub fn decide_review_action(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
) -> Vec<Event> {
    let Some(review_id) = resolve_review_id(state, cmd) else {
        return Vec::new();
    };
    let Some(review_meta) = state.reviews.get(&review_id) else {
        return Vec::new();
    };
    let post_id = review_meta.post_id;
    let group_id = state
        .posts
        .get(&post_id)
        .map(|meta| meta.group_id.clone())
        .unwrap_or_default();

    match &cmd.action {
        ReviewAction::Approve => {
            build_approve_events(state, cmd, config, review_id, post_id, group_id)
        }
        ReviewAction::Reject { reason } => {
            let mut events = build_terminal_decision_events(
                review_id,
                ReviewDecision::Rejected,
                reason,
                &cmd.operator_id,
                cmd.now_ms,
            );
            if state.send_plans.contains_key(&post_id) {
                events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
            }
            events
        }
        ReviewAction::Delete { reason } => {
            let mut events = build_terminal_decision_events(
                review_id,
                ReviewDecision::Deleted,
                reason,
                &cmd.operator_id,
                cmd.now_ms,
            );
            if let Some(event) = maybe_assign_external_code(state, &group_id, post_id) {
                events.push(event);
            }
            events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
            events
        }
        ReviewAction::Defer { delay_ms } => vec![
            Event::Review(ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision: ReviewDecision::Deferred,
                decided_by: cmd.operator_id.clone(),
                decided_at_ms: cmd.now_ms,
            }),
            Event::Review(ReviewEvent::ReviewDelayed {
                review_id,
                not_before_ms: cmd.now_ms.saturating_add(*delay_ms),
            }),
        ],
        ReviewAction::Skip => {
            let mut events = vec![Event::Review(ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision: ReviewDecision::Skipped,
                decided_by: cmd.operator_id.clone(),
                decided_at_ms: cmd.now_ms,
            })];
            if let Some(event) = maybe_assign_external_code(state, &group_id, post_id) {
                events.push(event);
            }
            if state.send_plans.contains_key(&post_id) {
                events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
            }
            events
        }
        ReviewAction::Immediate => {
            build_immediate_events(state, cmd, config, review_id, post_id, group_id)
        }
        ReviewAction::Refresh => {
            let mut events = Vec::new();
            events.extend(build_ingress_sync_events(state, post_id));
            if let Some(draft_event) = rebuild_draft_event(state, post_id, cmd.now_ms) {
                events.push(Event::Draft(draft_event));
            }
            events.push(Event::Review(ReviewEvent::ReviewRefreshRequested {
                review_id,
            }));
            events.push(Event::Render(RenderEvent::RenderRequested {
                post_id,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }));
            events
        }
        ReviewAction::Rerender => {
            let mut events = build_ingress_sync_events(state, post_id);
            events.push(Event::Review(ReviewEvent::ReviewRerenderRequested {
                review_id,
            }));
            events.push(Event::Render(RenderEvent::RenderRequested {
                post_id,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }));
            events
        }
        ReviewAction::SelectAllMessages => {
            let mut events = Vec::new();
            events.extend(build_ingress_sync_events(state, post_id));
            if let Some(draft_event) = rebuild_draft_event(state, post_id, cmd.now_ms) {
                events.push(Event::Draft(draft_event));
            }
            events.push(Event::Review(ReviewEvent::ReviewSelectAllRequested {
                review_id,
            }));
            events.push(Event::Render(RenderEvent::RenderRequested {
                post_id,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }));
            events
        }
        ReviewAction::ToggleAnonymous => {
            let mut events = build_ingress_sync_events(state, post_id);
            events.push(Event::Review(ReviewEvent::ReviewAnonToggled { review_id }));
            events.push(Event::Render(RenderEvent::RenderRequested {
                post_id,
                attempt: 1,
                requested_at_ms: cmd.now_ms,
            }));
            events
        }
        ReviewAction::ExpandAudit => vec![Event::Review(ReviewEvent::ReviewExpandRequested {
            review_id,
        })],
        ReviewAction::Show => vec![Event::Review(ReviewEvent::ReviewDisplayRequested {
            review_id,
        })],
        ReviewAction::Comment { text } => vec![Event::Review(ReviewEvent::ReviewCommentAdded {
            review_id,
            text: text.clone(),
        })],
        ReviewAction::Reply { text } => vec![Event::Review(ReviewEvent::ReviewReplyRequested {
            review_id,
            text: text.clone(),
        })],
        ReviewAction::Blacklist { reason } => {
            vec![Event::Review(ReviewEvent::ReviewBlacklistRequested {
                review_id,
                reason: reason.clone(),
            })]
        }
        ReviewAction::QuickReply { key } => {
            vec![Event::Review(ReviewEvent::ReviewQuickReplyRequested {
                review_id,
                key: key.clone(),
            })]
        }
        ReviewAction::Merge { review_code } => {
            build_merge_events(state, cmd, review_id, *review_code)
        }
    }
}

fn build_terminal_decision_events(
    review_id: ReviewId,
    decision: ReviewDecision,
    reason: &Option<String>,
    decided_by: &str,
    decided_at_ms: TimestampMs,
) -> Vec<Event> {
    let reason = normalize_review_reason(reason);
    let mut events = vec![
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision,
            decided_by: decided_by.to_string(),
            decided_at_ms,
        }),
        Event::Review(ReviewEvent::ReviewDecisionReasonRecorded {
            review_id,
            decision,
            reason: reason.clone(),
        }),
    ];
    if matches!(decision, ReviewDecision::Rejected) {
        events.push(Event::Review(ReviewEvent::ReviewSubmitterNoticeRequested {
            review_id,
            kind: crate::event::ReviewSubmitterNoticeKind::Rejected,
            reason,
        }));
    }
    events
}

fn normalize_review_reason(reason: &Option<String>) -> Option<String> {
    reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(ToOwned::to_owned)
}

pub fn decide_review_action_batch(
    state: &StateView,
    cmd: &ReviewActionBatchCommand,
    config: &CoreConfig,
) -> Vec<Event> {
    let mut scratch = state.clone();
    let mut out = Vec::new();
    let mut env_id = 1u128;
    for action in &cmd.actions {
        let step_cmd = ReviewActionCommand {
            review_id: cmd.review_id,
            review_code: cmd.review_code,
            audit_msg_id: cmd.audit_msg_id.clone(),
            action: action.clone(),
            operator_id: cmd.operator_id.clone(),
            now_ms: cmd.now_ms,
            tz_offset_minutes: cmd.tz_offset_minutes,
        };
        let step_events = decide_review_action(&scratch, &step_cmd, config);
        if step_events.is_empty() {
            break;
        }
        for event in &step_events {
            scratch = scratch.reduce(&EventEnvelope {
                id: Id128(env_id),
                ts_ms: cmd.now_ms,
                actor: ActorId::from_u128(0),
                correlation_id: None,
                event: event.clone(),
            });
            env_id = env_id.saturating_add(1);
        }
        out.extend(step_events);
    }
    out
}

fn build_approve_events(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
    review_id: ReviewId,
    post_id: crate::ids::PostId,
    group_id: String,
) -> Vec<Event> {
    let mut events = Vec::new();
    events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
        review_id,
        decision: ReviewDecision::Approved,
        decided_by: cmd.operator_id.clone(),
        decided_at_ms: cmd.now_ms,
    }));
    if let Some(event) = maybe_assign_external_code(state, &group_id, post_id) {
        events.push(event);
    }

    events.extend(build_send_plan_events(
        state,
        cmd,
        config,
        post_id,
        group_id,
        SendPriority::Normal,
    ));

    events
}

fn build_immediate_events(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
    review_id: ReviewId,
    post_id: crate::ids::PostId,
    group_id: String,
) -> Vec<Event> {
    let mut events = Vec::new();
    events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
        review_id,
        decision: ReviewDecision::Approved,
        decided_by: cmd.operator_id.clone(),
        decided_at_ms: cmd.now_ms,
    }));
    if let Some(event) = maybe_assign_external_code(state, &group_id, post_id) {
        events.push(event);
    }

    if config.group_config(&group_id).is_some() {
        let stacking_enabled = config.max_queue(&group_id) > 1;
        let not_before_ms = if stacking_enabled {
            cmd.now_ms.saturating_add(1)
        } else {
            cmd.now_ms
        };
        events.push(Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id,
            group_id: group_id.clone(),
            not_before_ms,
            priority: SendPriority::High,
            seq: state.next_send_seq,
        }));
    }
    let minute = minute_of_day(cmd.now_ms, cmd.tz_offset_minutes);
    let day = day_index(cmd.now_ms, cmd.tz_offset_minutes);
    events.push(Event::Schedule(ScheduleEvent::GroupFlushRequested {
        group_id: group_id.clone(),
        minute_of_day: minute,
        day_index: day,
        reason: crate::event::GroupFlushReason::Manual,
    }));
    events.extend(build_group_flush_events(state, &group_id, cmd.now_ms));

    events
}

fn build_send_plan_events(
    state: &StateView,
    cmd: &ReviewActionCommand,
    config: &CoreConfig,
    post_id: crate::ids::PostId,
    group_id: String,
    priority: SendPriority,
) -> Vec<Event> {
    if config.group_config(&group_id).is_none() {
        return Vec::new();
    }
    let max_queue = config.max_queue(&group_id);
    let max_images = config.max_images_per_post(&group_id);
    let queue_depth = state
        .send_plans
        .values()
        .filter(|plan| plan.group_id == group_id)
        .count();
    let queue_depth_after = queue_depth.saturating_add(1);
    let image_count_after = count_group_queue_images(state, &group_id, Some(post_id));
    let stacking_enabled = max_queue > 1;
    let should_flush = stacking_enabled
        && ((max_queue > 0 && queue_depth_after >= max_queue)
            || (max_images > 0 && image_count_after > max_images));
    let not_before_ms = if stacking_enabled && !should_flush {
        cmd.now_ms.saturating_add(STACKING_HOLD_MS)
    } else if should_flush {
        cmd.now_ms
    } else {
        let last_send_ms = state
            .group_runtime
            .get(&group_id)
            .and_then(|runtime| runtime.last_send_ms);
        compute_not_before(
            cmd.now_ms,
            None,
            config.send_windows(&group_id),
            config.min_interval_ms(&group_id),
            last_send_ms,
            queue_depth,
            max_queue,
            cmd.tz_offset_minutes,
        )
    };
    let mut events = vec![Event::Schedule(ScheduleEvent::SendPlanCreated {
        post_id,
        group_id: group_id.clone(),
        not_before_ms,
        priority,
        seq: state.next_send_seq,
    })];
    if should_flush {
        events.extend(build_group_flush_events(state, &group_id, cmd.now_ms));
    }
    events
}

fn maybe_assign_external_code(state: &StateView, group_id: &str, post_id: PostId) -> Option<Event> {
    if state.external_code_by_post.contains_key(&post_id) {
        return None;
    }
    let next_code: ExternalCode = state
        .next_external_code_by_group
        .get(group_id)
        .copied()
        .unwrap_or(1);
    Some(Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
        post_id,
        group_id: group_id.to_string(),
        external_code: next_code,
    }))
}

fn resolve_review_id(state: &StateView, cmd: &ReviewActionCommand) -> Option<ReviewId> {
    if let Some(review_id) = cmd.review_id {
        return Some(review_id);
    }
    if let Some(audit_msg_id) = cmd.audit_msg_id.as_ref() {
        if let Some(mapped) = state.review_by_audit_msg.get(audit_msg_id) {
            return Some(*mapped);
        }
    }
    let code = cmd.review_code?;
    state.review_by_code.get(&code).copied()
}

fn rebuild_draft_event(
    state: &StateView,
    post_id: crate::ids::PostId,
    now_ms: crate::ids::TimestampMs,
) -> Option<DraftEvent> {
    let post = state.posts.get(&post_id)?;
    let ingress_ids = state.post_ingress.get(&post_id)?.clone();
    let mut messages = Vec::new();
    for ingress_id in &ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    let draft = build_draft_from_messages(&messages);
    let is_anonymous = state
        .posts
        .get(&post_id)
        .map(|meta| meta.is_anonymous)
        .unwrap_or_else(|| detect_anonymous(&messages));
    let is_safe = state
        .posts
        .get(&post_id)
        .map(|meta| meta.is_safe)
        .unwrap_or_else(|| detect_safe(&messages));
    Some(DraftEvent::PostDraftCreated {
        post_id,
        session_id: post.session_id,
        group_id: post.group_id.clone(),
        ingress_ids,
        is_anonymous,
        is_safe,
        draft,
        created_at_ms: now_ms,
    })
}

fn build_ingress_sync_events(state: &StateView, post_id: crate::ids::PostId) -> Vec<Event> {
    let Some(ingress_ids) = state.post_ingress.get(&post_id) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    for ingress_id in ingress_ids {
        let Some(meta) = state.ingress_meta.get(ingress_id) else {
            continue;
        };
        let Some(message) = state.ingress_messages.get(ingress_id) else {
            continue;
        };
        events.push(Event::Ingress(IngressEvent::MessageSynced {
            ingress_id: *ingress_id,
            profile_id: meta.profile_id.clone(),
            chat_id: meta.chat_id.clone(),
            user_id: meta.user_id.clone(),
            sender_name: meta.sender_name.clone(),
            group_id: meta.group_id.clone(),
            platform_msg_id: meta.platform_msg_id.clone(),
            received_at_ms: meta.received_at_ms,
            message: message.clone(),
        }));
    }
    events
}

fn build_merge_events(
    state: &StateView,
    cmd: &ReviewActionCommand,
    review_id: ReviewId,
    target_review_code: ReviewCode,
) -> Vec<Event> {
    let Some(target_review_id) = state.review_by_code.get(&target_review_code).copied() else {
        return Vec::new();
    };
    if target_review_id == review_id {
        return Vec::new();
    }
    let Some(review_meta) = state.reviews.get(&review_id) else {
        return Vec::new();
    };
    let Some(target_review_meta) = state.reviews.get(&target_review_id) else {
        return Vec::new();
    };
    let post_id = review_meta.post_id;
    let target_post_id = target_review_meta.post_id;
    let Some(post_meta) = state.posts.get(&post_id) else {
        return Vec::new();
    };
    let Some(target_post_meta) = state.posts.get(&target_post_id) else {
        return Vec::new();
    };
    if post_meta.group_id != target_post_meta.group_id {
        return Vec::new();
    }

    let Some(sender_key) = post_sender_key(state, post_id) else {
        return Vec::new();
    };
    let Some(target_sender_key) = post_sender_key(state, target_post_id) else {
        return Vec::new();
    };
    if sender_key != target_sender_key {
        return Vec::new();
    }

    let Some(ingress_ids) = merge_ingress_ids(state, post_id, target_post_id) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    for ingress_id in &ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    let draft = build_draft_from_messages(&messages);
    let is_anonymous =
        post_is_anonymous(state, post_id) || post_is_anonymous(state, target_post_id);
    let is_safe = post_is_safe(state, post_id) && post_is_safe(state, target_post_id);

    let mut events = Vec::new();
    events.extend(build_ingress_sync_events(state, post_id));
    events.extend(build_ingress_sync_events(state, target_post_id));
    events.push(Event::Draft(DraftEvent::PostDraftCreated {
        post_id,
        session_id: post_meta.session_id,
        group_id: post_meta.group_id.clone(),
        ingress_ids,
        is_anonymous,
        is_safe,
        draft,
        created_at_ms: cmd.now_ms,
    }));
    events.push(Event::Review(ReviewEvent::ReviewRefreshRequested {
        review_id,
    }));
    events.push(Event::Render(RenderEvent::RenderRequested {
        post_id,
        attempt: 1,
        requested_at_ms: cmd.now_ms,
    }));
    events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
        review_id: target_review_id,
        decision: ReviewDecision::Skipped,
        decided_by: cmd.operator_id.clone(),
        decided_at_ms: cmd.now_ms,
    }));
    if state.send_plans.contains_key(&target_post_id) {
        events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled {
            post_id: target_post_id,
        }));
    }

    events
}

fn post_is_anonymous(state: &StateView, post_id: PostId) -> bool {
    if let Some(meta) = state.posts.get(&post_id) {
        return meta.is_anonymous;
    }
    let Some(ingress_ids) = state.post_ingress.get(&post_id) else {
        return false;
    };
    let mut messages = Vec::new();
    for ingress_id in ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    detect_anonymous(&messages)
}

fn post_is_safe(state: &StateView, post_id: PostId) -> bool {
    if let Some(meta) = state.posts.get(&post_id) {
        return meta.is_safe;
    }
    let Some(ingress_ids) = state.post_ingress.get(&post_id) else {
        return true;
    };
    let mut messages = Vec::new();
    for ingress_id in ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    detect_safe(&messages)
}

fn merge_ingress_ids(
    state: &StateView,
    post_id: PostId,
    target_post_id: PostId,
) -> Option<Vec<IngressId>> {
    let mut ingress_ids = state.post_ingress.get(&post_id)?.clone();
    ingress_ids.extend_from_slice(state.post_ingress.get(&target_post_id)?);
    ingress_ids.sort_by(|left, right| {
        let left_ms = state
            .ingress_meta
            .get(left)
            .map(|meta| meta.received_at_ms)
            .unwrap_or(0);
        let right_ms = state
            .ingress_meta
            .get(right)
            .map(|meta| meta.received_at_ms)
            .unwrap_or(0);
        (left_ms, left.0).cmp(&(right_ms, right.0))
    });
    ingress_ids.dedup();
    Some(ingress_ids)
}

fn post_sender_key(state: &StateView, post_id: PostId) -> Option<(String, String, String)> {
    let ingress_ids = state.post_ingress.get(&post_id)?;
    let first_ingress_id = ingress_ids.first()?;
    let meta = state.ingress_meta.get(first_ingress_id)?;
    Some((
        meta.chat_id.clone(),
        meta.user_id.clone(),
        meta.group_id.clone(),
    ))
}

fn count_group_queue_images(
    state: &StateView,
    group_id: &str,
    extra_post_id: Option<PostId>,
) -> usize {
    let mut total: usize = 0;
    for plan in state
        .send_plans
        .values()
        .filter(|plan| plan.group_id == group_id)
    {
        total = total.saturating_add(count_post_images(state, plan.post_id));
    }
    if let Some(post_id) = extra_post_id {
        if !state.send_plans.contains_key(&post_id) {
            total = total.saturating_add(count_post_images(state, post_id));
        }
    }
    total
}

fn count_post_images(state: &StateView, post_id: PostId) -> usize {
    let mut total: usize = 0;
    if let Some(render) = state.render.get(&post_id) {
        if render.png_blob.is_some() {
            total = total.saturating_add(1);
        }
    }
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            let Some(message) = state.ingress_messages.get(ingress_id) else {
                continue;
            };
            let images = message
                .attachments
                .iter()
                .filter(|attachment| attachment.kind == MediaKind::Image)
                .count();
            total = total.saturating_add(images);
        }
    }
    total
}
