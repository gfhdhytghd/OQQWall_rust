use crate::anonymous::detect_anonymous;
use crate::command::TickCommand;
use crate::config::CoreConfig;
use crate::decide::builder::build_draft_for_post;
use crate::decide::flush::build_group_flush_events;
use crate::decide::scheduler::{day_index, minute_of_day};
use crate::decide::sender::{AccountChoice, choose_account};
use crate::draft::{Draft, MediaReference};
use crate::event::{
    BlobEvent, DraftEvent, Event, GroupFlushReason, InputStatusKind, LifecycleEvent, MediaEvent,
    RenderEvent, ReviewEvent, ScheduleEvent, SendEvent, SendPriority, SessionEvent,
};
use crate::ids::{BlobId, IngressId, PostId, TimestampMs, derive_post_id};
use crate::safety::detect_safe;
use crate::state::{PostStage, SessionMeta, StateView};
use std::collections::BTreeSet;

const INPUT_STATUS_ACTIVE_MAX_MS: i64 = 30 * 60 * 1000;
const SEND_TIMEOUT_RETRY_DELAY_MS: i64 = 30 * 1000;

pub fn decide_tick(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    let mut events = Vec::new();

    events.extend(close_due_sessions(state, cmd, config));
    events.extend(trigger_review_delays(state, cmd));
    events.extend(retry_review_publish_failures(state, cmd));
    events.extend(retry_failed_renders(state, cmd));
    events.extend(retry_failed_media_fetches(state, cmd));
    events.extend(trigger_group_flush(state, cmd, config));
    events.extend(recover_stuck_sends(state, cmd, config));
    events.extend(maybe_start_send(state, cmd, config));
    events.extend(evict_expired_posts(state, cmd, config));
    events.extend(request_unreferenced_blob_gc(state));

    events
}

fn close_due_sessions(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    let mut due_sessions = state
        .sessions
        .values()
        .filter(|meta| {
            session_due_at_ms(state, meta, cmd.now_ms, config)
                .is_some_and(|due_at| cmd.now_ms >= due_at)
        })
        .map(|meta| meta.session_id)
        .collect::<Vec<_>>();
    due_sessions.sort();

    let mut events = Vec::new();
    for session_id in due_sessions {
        let Some(session_meta) = state.sessions.get(&session_id) else {
            continue;
        };
        let Some(ingress_ids) = state.session_ingress.get(&session_id) else {
            continue;
        };
        let mut messages = Vec::new();
        for ingress_id in ingress_ids {
            if let Some(message) = state.ingress_messages.get(ingress_id) {
                messages.push(message.clone());
            }
        }
        let is_anonymous = detect_anonymous(&messages);
        let is_safe = detect_safe(&messages);
        let session_bytes = session_id.to_be_bytes();
        let post_id = derive_post_id(&[&session_bytes]);
        let Some(draft) = build_draft_for_post(state, post_id, ingress_ids) else {
            continue;
        };

        events.push(Event::Session(SessionEvent::Closed {
            session_id,
            closed_at_ms: cmd.now_ms,
        }));
        events.push(Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: session_meta.key.group_id.clone(),
            ingress_ids: ingress_ids.clone(),
            is_anonymous,
            is_safe,
            draft,
            created_at_ms: cmd.now_ms,
        }));
        events.push(Event::Render(RenderEvent::RenderRequested {
            post_id,
            attempt: 1,
            requested_at_ms: cmd.now_ms,
        }));
    }

    events
}

fn session_due_at_ms(
    state: &StateView,
    meta: &SessionMeta,
    now_ms: TimestampMs,
    config: &CoreConfig,
) -> Option<TimestampMs> {
    let last_message_ms = state
        .ingress_meta
        .get(&meta.last_ingress_id)
        .map(|meta| meta.received_at_ms)?;
    let wait_ms = config.process_waittime_ms(&meta.key.group_id);
    if meta.close_at_ms <= last_message_ms {
        return Some(last_message_ms);
    }

    let Some(input_status) = state.input_status.get(&meta.key) else {
        return Some(last_message_ms.saturating_add(wait_ms.saturating_mul(2)));
    };

    let mut active = input_status_active(input_status.status);
    let mut ignore_status = false;
    if active {
        let active_since = input_status
            .active_since_ms
            .unwrap_or(input_status.updated_at_ms);
        if now_ms.saturating_sub(active_since) > INPUT_STATUS_ACTIVE_MAX_MS {
            active = false;
            ignore_status = true;
        }
    }
    if active {
        return None;
    }
    if ignore_status {
        return Some(last_message_ms.saturating_add(wait_ms));
    }

    let activity_ms = std::cmp::max(last_message_ms, input_status.updated_at_ms);
    Some(activity_ms.saturating_add(wait_ms))
}

fn input_status_active(status: InputStatusKind) -> bool {
    matches!(status, InputStatusKind::Typing | InputStatusKind::Speaking)
}

fn trigger_review_delays(state: &StateView, cmd: &TickCommand) -> Vec<Event> {
    let mut events = Vec::new();
    for review in state.reviews.values() {
        if let Some(delay_until) = review.delayed_until_ms {
            if cmd.now_ms >= delay_until {
                events.push(Event::Review(crate::event::ReviewEvent::ReviewInfoSynced {
                    review_id: review.review_id,
                    post_id: review.post_id,
                    review_code: review.review_code,
                }));
                events.push(Event::Review(
                    crate::event::ReviewEvent::ReviewPublishRequested {
                        review_id: review.review_id,
                    },
                ));
            }
        }
    }
    events
}

fn retry_review_publish_failures(state: &StateView, cmd: &TickCommand) -> Vec<Event> {
    let mut events = Vec::new();
    for review in state.reviews.values() {
        let Some(retry_at_ms) = review.publish_retry_at_ms else {
            continue;
        };
        if cmd.now_ms < retry_at_ms {
            continue;
        }
        if review.audit_msg_id.is_some() {
            continue;
        }
        events.push(Event::Review(ReviewEvent::ReviewPublishRequested {
            review_id: review.review_id,
        }));
    }
    events
}

fn retry_failed_renders(state: &StateView, cmd: &TickCommand) -> Vec<Event> {
    let mut events = Vec::new();
    for (post_id, meta) in &state.render {
        let Some(retry_at_ms) = meta.retry_at_ms else {
            continue;
        };
        if cmd.now_ms < retry_at_ms {
            continue;
        }
        let Some(post) = state.posts.get(post_id) else {
            continue;
        };
        if post.stage != PostStage::Failed {
            continue;
        }
        let attempt = meta.last_attempt.saturating_add(1).max(1);
        events.push(Event::Render(RenderEvent::RenderRequested {
            post_id: *post_id,
            attempt,
            requested_at_ms: cmd.now_ms,
        }));
    }
    events
}

fn retry_failed_media_fetches(state: &StateView, cmd: &TickCommand) -> Vec<Event> {
    let mut events = Vec::new();
    for (key, meta) in &state.media_fetch {
        let Some(retry_at_ms) = meta.retry_at_ms else {
            continue;
        };
        if cmd.now_ms < retry_at_ms {
            continue;
        }
        let Some(message) = state.ingress_messages.get(&key.ingress_id) else {
            continue;
        };
        let Some(attachment) = message.attachments.get(key.attachment_index) else {
            continue;
        };
        if !matches!(attachment.reference, MediaReference::RemoteUrl { .. }) {
            continue;
        }
        let attempt = meta.attempt.saturating_add(1).max(1);
        events.push(Event::Media(MediaEvent::MediaFetchRequested {
            ingress_id: key.ingress_id,
            attachment_index: key.attachment_index,
            attempt,
        }));
    }
    events
}

fn trigger_group_flush(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    let mut events = Vec::new();
    let minute = minute_of_day(cmd.now_ms, cmd.tz_offset_minutes);
    let day = day_index(cmd.now_ms, cmd.tz_offset_minutes);

    for (group_id, group_cfg) in &config.groups {
        if !group_cfg.send_schedule_minutes.contains(&minute) {
            continue;
        }
        let already = state
            .group_runtime
            .get(group_id)
            .and_then(|runtime| runtime.last_flush_mark.get(&minute))
            .copied();
        if already == Some(day) {
            continue;
        }
        events.push(Event::Schedule(ScheduleEvent::GroupFlushRequested {
            group_id: group_id.clone(),
            minute_of_day: minute,
            day_index: day,
            reason: GroupFlushReason::Scheduled,
        }));
        events.extend(build_group_flush_events(state, group_id, cmd.now_ms));
    }

    events
}

fn recover_stuck_sends(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    if state.sending.is_empty() {
        return Vec::new();
    }

    let mut events = Vec::new();
    let mut seq = state.next_send_seq;
    for sending in state.sending.values() {
        let timeout_ms = config.send_timeout_ms(&sending.group_id);
        if timeout_ms <= 0 {
            continue;
        }
        let elapsed = cmd.now_ms.saturating_sub(sending.started_at_ms);
        if elapsed < timeout_ms {
            continue;
        }
        let retry_at_ms = cmd.now_ms.saturating_add(SEND_TIMEOUT_RETRY_DELAY_MS);
        events.push(Event::Send(SendEvent::SendFailed {
            post_id: sending.post_id,
            account_id: sending.account_id.clone(),
            attempt: 1,
            retry_at_ms,
            error: format!("send timeout after {} ms", timeout_ms),
        }));
        events.push(Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            post_id: sending.post_id,
            group_id: sending.group_id.clone(),
            not_before_ms: retry_at_ms,
            priority: SendPriority::Normal,
            seq,
        }));
        seq = seq.saturating_add(1);
    }
    events
}

fn maybe_start_send(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    if !state.sending.is_empty() {
        return Vec::new();
    }

    let due = state
        .send_due
        .iter()
        .find(|key| key.not_before_ms <= cmd.now_ms)
        .copied();
    let Some(due) = due else {
        return Vec::new();
    };
    let Some(plan) = state.send_plans.get(&due.post_id) else {
        return Vec::new();
    };

    match choose_account(state, config.group_config(&plan.group_id), cmd.now_ms) {
        AccountChoice::Available(account_id) => vec![Event::Send(SendEvent::SendStarted {
            post_id: plan.post_id,
            group_id: plan.group_id.clone(),
            account_id,
            started_at_ms: cmd.now_ms,
        })],
        AccountChoice::RetryAt(retry_at) => {
            vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                post_id: plan.post_id,
                group_id: plan.group_id.clone(),
                not_before_ms: retry_at,
                priority: plan.priority,
                seq: state.next_send_seq,
            })]
        }
        AccountChoice::Unavailable => vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            post_id: plan.post_id,
            group_id: plan.group_id.clone(),
            not_before_ms: cmd.now_ms.saturating_add(30_000),
            priority: SendPriority::Normal,
            seq: state.next_send_seq,
        })],
    }
}

fn evict_expired_posts(state: &StateView, cmd: &TickCommand, config: &CoreConfig) -> Vec<Event> {
    if config.eviction_retention_ms <= 0 {
        return Vec::new();
    }

    let mut candidates = state
        .posts
        .values()
        .filter(|post| is_evictable_stage(post.stage))
        .filter(|post| {
            cmd.now_ms.saturating_sub(post.created_at_ms) >= config.eviction_retention_ms
        })
        .map(|post| (post.created_at_ms, post.post_id))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.cmp(right));

    candidates
        .into_iter()
        .take(10)
        .map(|(_, post_id)| {
            let ingress_ids = state
                .post_ingress
                .get(&post_id)
                .cloned()
                .unwrap_or_default();
            let blob_ids = collect_post_blob_ids(state, post_id, &ingress_ids);
            Event::Lifecycle(LifecycleEvent::PostEvicted {
                post_id,
                evicted_at_ms: cmd.now_ms,
                blob_ids,
                ingress_ids,
            })
        })
        .collect()
}

fn request_unreferenced_blob_gc(state: &StateView) -> Vec<Event> {
    if state.blobs.is_empty() {
        return Vec::new();
    }

    let referenced = collect_referenced_blob_ids(state);
    let mut blob_ids = state
        .blobs
        .keys()
        .copied()
        .filter(|blob_id| !referenced.contains(blob_id))
        .collect::<Vec<_>>();
    blob_ids.sort();
    blob_ids
        .into_iter()
        .map(|blob_id| Event::Blob(BlobEvent::BlobGcRequested { blob_id }))
        .collect()
}

fn is_evictable_stage(stage: PostStage) -> bool {
    matches!(
        stage,
        PostStage::Sent
            | PostStage::Withdrawn
            | PostStage::Rejected
            | PostStage::Deleted
            | PostStage::Skipped
    )
}

fn collect_post_blob_ids(
    state: &StateView,
    post_id: PostId,
    ingress_ids: &[IngressId],
) -> Vec<BlobId> {
    let mut blob_ids = BTreeSet::new();
    if let Some(render) = state.render.get(&post_id) {
        if let Some(blob_id) = render.png_blob {
            blob_ids.insert(blob_id);
        }
        blob_ids.extend(render.png_blobs.iter().copied());
    }
    if let Some(draft) = state.drafts.get(&post_id) {
        collect_draft_blob_ids(draft, &mut blob_ids);
    }
    for ingress_id in ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            for attachment in &message.attachments {
                if let MediaReference::Blob { blob_id } = attachment.reference {
                    blob_ids.insert(blob_id);
                }
            }
        }
    }
    blob_ids.into_iter().collect()
}

fn collect_referenced_blob_ids(state: &StateView) -> BTreeSet<BlobId> {
    let mut blob_ids = BTreeSet::new();
    for render in state.render.values() {
        if let Some(blob_id) = render.png_blob {
            blob_ids.insert(blob_id);
        }
        blob_ids.extend(render.png_blobs.iter().copied());
    }
    for draft in state.drafts.values() {
        collect_draft_blob_ids(draft, &mut blob_ids);
    }
    for message in state.ingress_messages.values() {
        for attachment in &message.attachments {
            if let MediaReference::Blob { blob_id } = attachment.reference {
                blob_ids.insert(blob_id);
            }
        }
    }
    blob_ids
}

fn collect_draft_blob_ids(draft: &Draft, out: &mut BTreeSet<BlobId>) {
    for block in &draft.blocks {
        match block {
            crate::draft::DraftBlock::Attachment {
                reference: MediaReference::Blob { blob_id },
                ..
            } => {
                out.insert(*blob_id);
            }
            crate::draft::DraftBlock::Forward { items } => {
                for item in items {
                    let nested = Draft {
                        blocks: item.blocks.clone(),
                    };
                    collect_draft_blob_ids(&nested, out);
                }
            }
            _ => {}
        }
    }
}
