use std::collections::HashMap;

use crate::event::{
    AccountEvent, BlobEvent, ConfigEvent, DraftEvent, Event, EventEnvelope, GroupFlushReason,
    IngressEvent, InputStatusKind, LifecycleEvent, ManualEvent, MediaEvent, RenderEvent,
    ReviewEvent, ScheduleEvent, SendEvent, SessionEvent,
};
use crate::ids::{BlobId, IngressId, PostId};
use crate::state::{
    AccountRuntime, BlobMeta, GroupRuntime, InputStatusMeta, MediaFetchKey, MediaFetchMeta,
    PostMeta, PostStage, QzonePublicationMeta, RenderMeta, ReviewMeta, SendDueKey, SendPlan,
    SendingMeta, SessionKey, SessionMeta, StateView,
};

pub fn reduce(state: &StateView, env: &EventEnvelope) -> StateView {
    let mut next = state.clone();
    reduce_in_place(&mut next, env);
    next
}

fn reduce_in_place(state: &mut StateView, env: &EventEnvelope) {
    state.last_event_id = Some(env.id);
    state.last_ts_ms = Some(env.ts_ms);

    match &env.event {
        Event::System(_) => {}
        Event::Config(ConfigEvent::Applied { version, .. }) => {
            state.config_version = Some(*version);
        }
        Event::Ingress(event) => reduce_ingress(state, event),
        Event::Session(event) => reduce_session(state, event),
        Event::Draft(event) => reduce_draft(state, event),
        Event::Media(event) => reduce_media(state, event),
        Event::Render(event) => reduce_render(state, event),
        Event::Review(event) => reduce_review(state, event),
        Event::Schedule(event) => reduce_schedule(state, event),
        Event::Send(event) => reduce_send(state, event),
        Event::Blob(event) => reduce_blob(state, event),
        Event::Account(event) => reduce_account(state, event),
        Event::Manual(event) => reduce_manual(state, event),
        Event::Lifecycle(event) => reduce_lifecycle(state, event),
    }
}

fn reduce_ingress(state: &mut StateView, event: &IngressEvent) {
    match event {
        IngressEvent::MessageAccepted {
            ingress_id,
            profile_id,
            chat_id,
            user_id,
            sender_name,
            group_id,
            platform_msg_id,
            route_meta,
            received_at_ms,
            message,
        }
        | IngressEvent::MessageSynced {
            ingress_id,
            profile_id,
            chat_id,
            user_id,
            sender_name,
            group_id,
            platform_msg_id,
            route_meta,
            received_at_ms,
            message,
        } => {
            state.ingress_seen.insert(*ingress_id);
            state.ingress_meta.insert(
                *ingress_id,
                crate::state::IngressMeta {
                    profile_id: profile_id.clone(),
                    chat_id: chat_id.clone(),
                    user_id: user_id.clone(),
                    sender_name: sender_name.clone(),
                    group_id: group_id.clone(),
                    platform_msg_id: platform_msg_id.clone(),
                    route_meta: route_meta.clone(),
                    received_at_ms: *received_at_ms,
                },
            );
            state.ingress_messages.insert(*ingress_id, message.clone());
        }
        IngressEvent::MessageIgnored { ingress_id, .. } => {
            state.ingress_seen.insert(*ingress_id);
        }
        IngressEvent::MessageRecalled { ingress_id, .. } => {
            state.ingress_messages.remove(ingress_id);
            state
                .media_fetch
                .retain(|key, _| key.ingress_id != *ingress_id);
            remove_ingress_from_sessions(state, *ingress_id);
            remove_ingress_from_posts(state, *ingress_id);
        }
        IngressEvent::InputStatusUpdated {
            chat_id,
            user_id,
            group_id,
            status,
            received_at_ms,
            ..
        } => {
            let key = SessionKey {
                chat_id: chat_id.clone(),
                user_id: user_id.clone(),
                group_id: group_id.clone(),
            };
            let is_active = input_status_active(*status);
            let active_since_ms = if is_active {
                match state.input_status.get(&key) {
                    Some(meta) if input_status_active(meta.status) => {
                        Some(meta.active_since_ms.unwrap_or(*received_at_ms))
                    }
                    _ => Some(*received_at_ms),
                }
            } else {
                None
            };
            state.input_status.insert(
                key,
                InputStatusMeta {
                    status: *status,
                    updated_at_ms: *received_at_ms,
                    active_since_ms,
                },
            );
        }
    }
}

fn input_status_active(status: InputStatusKind) -> bool {
    matches!(status, InputStatusKind::Typing | InputStatusKind::Speaking)
}

fn remove_ingress_from_sessions(state: &mut StateView, ingress_id: crate::ids::IngressId) {
    let mut empty_sessions = Vec::new();
    for (session_id, ingress_ids) in &mut state.session_ingress {
        ingress_ids.retain(|id| *id != ingress_id);
        if ingress_ids.is_empty() {
            empty_sessions.push(*session_id);
        }
    }
    for session_id in empty_sessions {
        state.session_ingress.remove(&session_id);
        if let Some(meta) = state.sessions.remove(&session_id) {
            state.session_by_key.remove(&meta.key);
        }
    }
}

fn remove_ingress_from_posts(state: &mut StateView, ingress_id: crate::ids::IngressId) {
    for ingress_ids in state.post_ingress.values_mut() {
        ingress_ids.retain(|id| *id != ingress_id);
    }
}

fn reduce_session(state: &mut StateView, event: &SessionEvent) {
    match event {
        SessionEvent::Opened {
            session_id,
            first_ingress_id,
            chat_id,
            user_id,
            group_id,
            close_at_ms,
        } => {
            let key = SessionKey {
                chat_id: chat_id.clone(),
                user_id: user_id.clone(),
                group_id: group_id.clone(),
            };
            let meta = SessionMeta {
                session_id: *session_id,
                key: key.clone(),
                first_ingress_id: *first_ingress_id,
                last_ingress_id: *first_ingress_id,
                close_at_ms: *close_at_ms,
            };
            state.session_by_key.insert(key, *session_id);
            state.sessions.insert(*session_id, meta);
            state
                .session_ingress
                .insert(*session_id, vec![*first_ingress_id]);
        }
        SessionEvent::Appended {
            session_id,
            ingress_id,
            close_at_ms,
        } => {
            if let Some(meta) = state.sessions.get_mut(session_id) {
                meta.last_ingress_id = *ingress_id;
                meta.close_at_ms = *close_at_ms;
            }
            state
                .session_ingress
                .entry(*session_id)
                .or_default()
                .push(*ingress_id);
        }
        SessionEvent::Closed { session_id, .. } => {
            if let Some(meta) = state.sessions.remove(session_id) {
                state.session_by_key.remove(&meta.key);
            }
            state.session_ingress.remove(session_id);
        }
    }
}

fn reduce_draft(state: &mut StateView, event: &DraftEvent) {
    match event {
        DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id,
            ingress_ids,
            is_anonymous,
            is_safe,
            draft,
            created_at_ms,
        } => {
            state.drafts.insert(*post_id, draft.clone());
            state.post_ingress.insert(*post_id, ingress_ids.clone());
            let meta = state.posts.entry(*post_id).or_insert(PostMeta {
                post_id: *post_id,
                session_id: *session_id,
                group_id: group_id.clone(),
                stage: PostStage::Drafted,
                review_id: None,
                created_at_ms: *created_at_ms,
                is_anonymous: *is_anonymous,
                is_safe: *is_safe,
                last_error: None,
            });
            meta.session_id = *session_id;
            meta.group_id = group_id.clone();
            if meta.created_at_ms == 0 {
                meta.created_at_ms = *created_at_ms;
            }
            meta.is_anonymous = *is_anonymous;
            meta.is_safe = *is_safe;
            state.update_post_stage(*post_id, PostStage::Drafted);
        }
        DraftEvent::DraftTransformsSet {
            post_id,
            transforms,
            ..
        } => {
            if transforms.is_empty() {
                state.draft_transforms.remove(post_id);
            } else {
                state.draft_transforms.insert(*post_id, transforms.clone());
            }
        }
    }
}

fn reduce_media(state: &mut StateView, event: &MediaEvent) {
    match event {
        MediaEvent::MediaFetchSucceeded {
            ingress_id,
            attachment_index,
            blob_id,
        } => {
            state.register_media_reference(*ingress_id, *attachment_index, *blob_id);
        }
        MediaEvent::MediaFetchRequested {
            ingress_id,
            attachment_index,
            attempt,
        } => {
            let key = MediaFetchKey {
                ingress_id: *ingress_id,
                attachment_index: *attachment_index,
            };
            let entry = state.media_fetch.entry(key).or_insert(MediaFetchMeta {
                attempt: *attempt,
                retry_at_ms: None,
                last_error: None,
            });
            entry.attempt = *attempt;
            entry.retry_at_ms = None;
            entry.last_error = None;
        }
        MediaEvent::MediaFetchFailed {
            ingress_id,
            attachment_index,
            attempt,
            retry_at_ms,
            error,
        } => {
            let key = MediaFetchKey {
                ingress_id: *ingress_id,
                attachment_index: *attachment_index,
            };
            let entry = state.media_fetch.entry(key).or_insert(MediaFetchMeta {
                attempt: *attempt,
                retry_at_ms: Some(*retry_at_ms),
                last_error: Some(error.clone()),
            });
            entry.attempt = *attempt;
            entry.retry_at_ms = Some(*retry_at_ms);
            entry.last_error = Some(error.clone());
        }
        MediaEvent::AvatarFetchRequested { .. } => {}
    }
}

fn reduce_render(state: &mut StateView, event: &RenderEvent) {
    match event {
        RenderEvent::RenderRequested {
            post_id, attempt, ..
        } => {
            let meta = state.render.entry(*post_id).or_insert(RenderMeta {
                png_blob: None,
                png_blobs: Vec::new(),
                last_error: None,
                last_attempt: 0,
                retry_at_ms: None,
            });
            meta.png_blob = None;
            meta.png_blobs.clear();
            meta.last_attempt = *attempt;
            meta.retry_at_ms = None;
            meta.last_error = None;
            state.update_post_stage(*post_id, PostStage::RenderRequested);
        }
        RenderEvent::PngReady { post_id, blob_id } => {
            reduce_png_ready(state, *post_id, &[*blob_id]);
        }
        RenderEvent::PngBatchReady { post_id, blob_ids } => {
            reduce_png_ready(state, *post_id, blob_ids);
        }
        RenderEvent::RenderFailed {
            post_id,
            attempt,
            retry_at_ms,
            error,
        } => {
            let meta = state.render.entry(*post_id).or_insert(RenderMeta {
                png_blob: None,
                png_blobs: Vec::new(),
                last_error: None,
                last_attempt: 0,
                retry_at_ms: None,
            });
            meta.last_error = Some(error.clone());
            meta.last_attempt = *attempt;
            meta.retry_at_ms = Some(*retry_at_ms);
            state.update_post_stage(*post_id, PostStage::Failed);
        }
    }
}

fn reduce_png_ready(state: &mut StateView, post_id: crate::ids::PostId, blob_ids: &[BlobId]) {
    if blob_ids.is_empty() {
        return;
    }
    let meta = state.render.entry(post_id).or_insert(RenderMeta {
        png_blob: None,
        png_blobs: Vec::new(),
        last_error: None,
        last_attempt: 0,
        retry_at_ms: None,
    });
    if meta.png_blob.is_none() {
        meta.png_blob = blob_ids.first().copied();
    }
    for blob_id in blob_ids {
        if !meta.png_blobs.contains(blob_id) {
            meta.png_blobs.push(*blob_id);
        }
    }
    meta.last_error = None;
    meta.retry_at_ms = None;
    state.update_post_stage(post_id, PostStage::Rendered);
}

fn reduce_review(state: &mut StateView, event: &ReviewEvent) {
    match event {
        ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        } => {
            state.review_by_code.insert(*review_code, *review_id);
            state.reviews.insert(
                *review_id,
                ReviewMeta {
                    review_id: *review_id,
                    post_id: *post_id,
                    review_code: *review_code,
                    decision: None,
                    audit_msg_id: None,
                    delayed_until_ms: None,
                    needs_republish: false,
                    decided_by: None,
                    decided_at_ms: None,
                    decision_reason: None,
                    publish_retry_at_ms: None,
                    publish_last_error: None,
                    publish_attempt: 0,
                },
            );
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.review_id = Some(*review_id);
            }
            if state.next_review_code <= *review_code {
                state.next_review_code = review_code.saturating_add(1);
            }
            state.update_post_stage(*post_id, PostStage::ReviewPending);
        }
        ReviewEvent::ReviewPublishRequested { review_id } => {
            let post_id = state.reviews.get(review_id).map(|meta| meta.post_id);
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.delayed_until_ms = None;
                meta.needs_republish = false;
                meta.publish_retry_at_ms = None;
                meta.publish_last_error = None;
            }
            if let Some(post_id) = post_id {
                state.update_post_stage(post_id, PostStage::ReviewPending);
            }
        }
        ReviewEvent::ReviewPublished {
            review_id,
            audit_msg_id,
        } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.audit_msg_id = Some(audit_msg_id.clone());
                meta.publish_retry_at_ms = None;
                meta.publish_last_error = None;
                meta.publish_attempt = 0;
                state
                    .review_by_audit_msg
                    .insert(audit_msg_id.clone(), *review_id);
            }
        }
        ReviewEvent::ReviewPublishFailed {
            review_id,
            attempt,
            retry_at_ms,
            error,
        } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.publish_retry_at_ms = Some(*retry_at_ms);
                meta.publish_last_error = Some(error.clone());
                meta.publish_attempt = *attempt;
            }
        }
        ReviewEvent::ReviewDelayed {
            review_id,
            not_before_ms,
        } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.delayed_until_ms = Some(*not_before_ms);
            }
        }
        ReviewEvent::ReviewRefreshRequested { review_id }
        | ReviewEvent::ReviewRerenderRequested { review_id }
        | ReviewEvent::ReviewSelectAllRequested { review_id } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.needs_republish = true;
            }
        }
        ReviewEvent::ReviewAnonToggled { review_id } => {
            let post_id = state.reviews.get(review_id).map(|meta| meta.post_id);
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.needs_republish = true;
            }
            if let Some(post_id) = post_id {
                if let Some(meta) = state.posts.get_mut(&post_id) {
                    meta.is_anonymous = !meta.is_anonymous;
                }
            }
        }
        ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision,
            decided_by,
            decided_at_ms,
        } => {
            let post_id = state.reviews.get(review_id).map(|meta| meta.post_id);
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.decision = Some(*decision);
                meta.decided_by = Some(decided_by.clone());
                meta.decided_at_ms = Some(*decided_at_ms);
            }
            if let Some(post_id) = post_id {
                let stage = match decision {
                    crate::event::ReviewDecision::Approved => PostStage::Reviewed,
                    crate::event::ReviewDecision::Rejected => PostStage::Rejected,
                    crate::event::ReviewDecision::Deferred => PostStage::ReviewPending,
                    crate::event::ReviewDecision::Skipped => PostStage::Skipped,
                    crate::event::ReviewDecision::Deleted => PostStage::Deleted,
                };
                state.update_post_stage(post_id, stage);
            }
        }
        ReviewEvent::ReviewDecisionReasonRecorded {
            review_id, reason, ..
        } => {
            if let Some(meta) = state.reviews.get_mut(review_id) {
                meta.decision_reason = reason.clone();
            }
        }
        ReviewEvent::ReviewExternalNumberSet {
            group_id,
            next_number,
        } => {
            state
                .next_external_code_by_group
                .insert(group_id.clone(), *next_number);
        }
        ReviewEvent::ReviewExternalCodeCleared { post_id } => {
            state.external_code_by_post.remove(post_id);
        }
        ReviewEvent::ReviewExternalCodeAssigned {
            post_id,
            group_id,
            external_code,
        } => {
            state.external_code_by_post.insert(*post_id, *external_code);
            let next_value = external_code.saturating_add(1);
            let entry = state
                .next_external_code_by_group
                .entry(group_id.clone())
                .or_insert(next_value);
            if *entry < next_value {
                *entry = next_value;
            }
        }
        ReviewEvent::ReviewBlacklistRequested { review_id, reason } => {
            if let Some((group_id, sender_id)) = resolve_review_sender(state, *review_id) {
                let entry = state
                    .blacklist
                    .entry(group_id)
                    .or_default()
                    .entry(sender_id)
                    .or_insert(None);
                if reason.is_some() {
                    *entry = reason.clone();
                }
            }
        }
        ReviewEvent::ReviewBlacklistAdded {
            group_id,
            sender_id,
            reason,
        } => {
            let entry = state
                .blacklist
                .entry(group_id.clone())
                .or_default()
                .entry(sender_id.clone())
                .or_insert(None);
            if reason.is_some() {
                *entry = reason.clone();
            }
        }
        ReviewEvent::ReviewBlacklistRemoved {
            group_id,
            sender_id,
        } => {
            if let Some(group) = state.blacklist.get_mut(group_id) {
                group.remove(sender_id);
                if group.is_empty() {
                    state.blacklist.remove(group_id);
                }
            }
        }
        ReviewEvent::ReviewCommentAdded { .. }
        | ReviewEvent::ReviewSubmitterNoticeRequested { .. }
        | ReviewEvent::ReviewReplyRequested { .. }
        | ReviewEvent::ReviewExpandRequested { .. }
        | ReviewEvent::ReviewDisplayRequested { .. }
        | ReviewEvent::ReviewQuickReplyRequested { .. }
        | ReviewEvent::ReviewInfoSynced { .. } => {}
    }
}

fn resolve_review_sender(
    state: &StateView,
    review_id: crate::ids::ReviewId,
) -> Option<(String, String)> {
    let review = state.reviews.get(&review_id)?;
    let ingress_ids = state.post_ingress.get(&review.post_id)?;
    let ingress_id = ingress_ids.first()?;
    let meta = state.ingress_meta.get(ingress_id)?;
    Some((meta.group_id.clone(), meta.user_id.clone()))
}

fn reduce_schedule(state: &mut StateView, event: &ScheduleEvent) {
    match event {
        ScheduleEvent::SendPlanCreated {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        }
        | ScheduleEvent::SendPlanRescheduled {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        } => {
            if let Some(prev) = state.send_plans.remove(post_id) {
                state.send_due.remove(&SendDueKey {
                    not_before_ms: prev.not_before_ms,
                    priority: prev.priority,
                    seq: prev.seq,
                    post_id: prev.post_id,
                });
            }
            let plan = SendPlan {
                post_id: *post_id,
                group_id: group_id.clone(),
                not_before_ms: *not_before_ms,
                priority: *priority,
                seq: *seq,
            };
            state.send_plans.insert(*post_id, plan.clone());
            state.send_due.insert(SendDueKey {
                not_before_ms: plan.not_before_ms,
                priority: plan.priority,
                seq: plan.seq,
                post_id: plan.post_id,
            });
            state.update_post_stage(*post_id, PostStage::Scheduled);
            if state.next_send_seq <= *seq {
                state.next_send_seq = seq.saturating_add(1);
            }
        }
        ScheduleEvent::SendPlanCanceled { post_id } => {
            if let Some(prev) = state.send_plans.remove(post_id) {
                state.send_due.remove(&SendDueKey {
                    not_before_ms: prev.not_before_ms,
                    priority: prev.priority,
                    seq: prev.seq,
                    post_id: prev.post_id,
                });
            }
        }
        ScheduleEvent::GroupFlushRequested {
            group_id,
            minute_of_day,
            day_index,
            reason: GroupFlushReason::Scheduled,
        }
        | ScheduleEvent::GroupFlushRequested {
            group_id,
            minute_of_day,
            day_index,
            reason: GroupFlushReason::Manual,
        } => {
            let runtime = state
                .group_runtime
                .entry(group_id.clone())
                .or_insert(GroupRuntime {
                    last_flush_mark: HashMap::new(),
                    last_send_ms: None,
                });
            runtime.last_flush_mark.insert(*minute_of_day, *day_index);
        }
    }
}

fn reduce_send(state: &mut StateView, event: &SendEvent) {
    match event {
        SendEvent::SendStarted {
            post_id,
            group_id,
            account_id,
            started_at_ms,
        } => {
            if let Some(prev) = state.send_plans.remove(post_id) {
                state.send_due.remove(&SendDueKey {
                    not_before_ms: prev.not_before_ms,
                    priority: prev.priority,
                    seq: prev.seq,
                    post_id: prev.post_id,
                });
            }
            state.sending.insert(
                *post_id,
                SendingMeta {
                    post_id: *post_id,
                    group_id: group_id.clone(),
                    account_id: account_id.clone(),
                    started_at_ms: *started_at_ms,
                },
            );
            state.update_post_stage(*post_id, PostStage::Sending);
            let runtime = state
                .group_runtime
                .entry(group_id.clone())
                .or_insert(GroupRuntime {
                    last_flush_mark: HashMap::new(),
                    last_send_ms: None,
                });
            runtime.last_send_ms = Some(*started_at_ms);
        }
        SendEvent::SendAccountSucceeded {
            account_id,
            finished_at_ms,
            ..
        } => {
            let runtime = state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
            runtime.last_send_ms = Some(*finished_at_ms);
        }
        SendEvent::SendAccountFailed { account_id, .. } => {
            state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
        }
        SendEvent::SendSucceeded {
            post_id,
            account_id,
            finished_at_ms,
            ..
        } => {
            let sending = state.sending.remove(post_id);
            state.update_post_stage(*post_id, PostStage::Sent);
            let runtime = state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
            runtime.last_send_ms = Some(*finished_at_ms);
            if let Some(sending) = sending {
                let runtime = state
                    .group_runtime
                    .entry(sending.group_id)
                    .or_insert(GroupRuntime {
                        last_flush_mark: HashMap::new(),
                        last_send_ms: None,
                    });
                runtime.last_send_ms = Some(*finished_at_ms);
            }
        }
        SendEvent::SendFailed {
            post_id,
            account_id,
            error,
            ..
        } => {
            state.sending.remove(post_id);
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.last_error = Some(error.clone());
            }
            state.update_post_stage(*post_id, PostStage::Failed);
            state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
        }
        SendEvent::SendGaveUp { post_id, reason } => {
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.last_error = Some(reason.clone());
            }
            state.update_post_stage(*post_id, PostStage::Manual);
        }
        SendEvent::QzonePostPublished {
            group_id,
            account_id,
            remote_id,
            text,
            items,
        } => {
            let key = crate::event::QzonePublicationKey {
                account_id: account_id.clone(),
                remote_id: remote_id.clone(),
            };
            state.qzone_publications.insert(
                key.clone(),
                QzonePublicationMeta {
                    group_id: group_id.clone(),
                    account_id: account_id.clone(),
                    remote_id: remote_id.clone(),
                    text: text.clone(),
                    items: items.clone(),
                    withdrawn_posts: Default::default(),
                    pending_withdrawn_posts: Default::default(),
                },
            );
            for item in items {
                state
                    .qzone_publications_by_post
                    .entry(item.post_id)
                    .or_default()
                    .insert(key.clone());
            }
        }
        SendEvent::QzonePostWithdrawRequested {
            account_id,
            remote_id,
            withdrawn_post_ids,
            ..
        } => {
            let key = crate::event::QzonePublicationKey {
                account_id: account_id.clone(),
                remote_id: remote_id.clone(),
            };
            if let Some(publication) = state.qzone_publications.get_mut(&key) {
                publication
                    .pending_withdrawn_posts
                    .extend(withdrawn_post_ids.iter().copied());
            }
        }
        SendEvent::QzonePostWithdrawSucceeded {
            post_id,
            account_id,
            remote_id,
            text,
            withdrawn_post_ids,
            ..
        } => {
            let effective_withdrawn_post_ids =
                effective_withdrawn_post_ids(*post_id, withdrawn_post_ids);
            let key = crate::event::QzonePublicationKey {
                account_id: account_id.clone(),
                remote_id: remote_id.clone(),
            };
            if let Some(publication) = state.qzone_publications.get_mut(&key) {
                publication.text = text.clone();
                publication
                    .withdrawn_posts
                    .extend(effective_withdrawn_post_ids.iter().copied());
                for withdrawn_post_id in &effective_withdrawn_post_ids {
                    publication
                        .pending_withdrawn_posts
                        .remove(withdrawn_post_id);
                }
            }
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.last_error = None;
            }
            for withdrawn_post_id in &effective_withdrawn_post_ids {
                if let Some(meta) = state.posts.get_mut(withdrawn_post_id) {
                    meta.last_error = None;
                }
                if qzone_post_fully_withdrawn(state, *withdrawn_post_id) {
                    state.update_post_stage(*withdrawn_post_id, PostStage::Withdrawn);
                }
            }
        }
        SendEvent::QzonePostWithdrawFailed {
            post_id,
            account_id,
            remote_id,
            withdrawn_post_ids,
            error,
        } => {
            let effective_withdrawn_post_ids =
                effective_withdrawn_post_ids(*post_id, withdrawn_post_ids);
            let key = crate::event::QzonePublicationKey {
                account_id: account_id.clone(),
                remote_id: remote_id.clone(),
            };
            if let Some(publication) = state.qzone_publications.get_mut(&key) {
                for withdrawn_post_id in &effective_withdrawn_post_ids {
                    publication
                        .pending_withdrawn_posts
                        .remove(withdrawn_post_id);
                }
            }
            if let Some(meta) = state.posts.get_mut(post_id) {
                meta.last_error = Some(error.clone());
            }
        }
    }
}

fn effective_withdrawn_post_ids(
    post_id: crate::ids::PostId,
    withdrawn_post_ids: &[crate::ids::PostId],
) -> Vec<crate::ids::PostId> {
    if withdrawn_post_ids.is_empty() {
        vec![post_id]
    } else {
        withdrawn_post_ids.to_vec()
    }
}

fn qzone_post_fully_withdrawn(state: &StateView, post_id: crate::ids::PostId) -> bool {
    let Some(publication_keys) = state.qzone_publications_by_post.get(&post_id) else {
        return false;
    };
    !publication_keys.is_empty()
        && publication_keys.iter().all(|key| {
            state
                .qzone_publications
                .get(key)
                .map(|publication| publication.withdrawn_posts.contains(&post_id))
                .unwrap_or(false)
        })
}

fn reduce_blob(state: &mut StateView, event: &BlobEvent) {
    match event {
        BlobEvent::BlobRegistered {
            blob_id,
            size_bytes,
        } => {
            state.blobs.insert(
                *blob_id,
                BlobMeta {
                    blob_id: *blob_id,
                    size_bytes: *size_bytes,
                    persisted_path: None,
                    ref_count: 1,
                },
            );
        }
        BlobEvent::BlobPersisted { blob_id, path } => {
            if let Some(meta) = state.blobs.get_mut(blob_id) {
                meta.persisted_path = Some(path.clone());
            }
        }
        BlobEvent::BlobReleased { blob_id } => {
            if let Some(meta) = state.blobs.get_mut(blob_id) {
                if meta.ref_count > 0 {
                    meta.ref_count -= 1;
                }
            }
        }
        BlobEvent::BlobGcRequested { blob_id } => {
            state.blobs.remove(blob_id);
        }
    }
}

fn reduce_lifecycle(state: &mut StateView, event: &LifecycleEvent) {
    match event {
        LifecycleEvent::PostEvicted {
            post_id,
            ingress_ids,
            ..
        } => {
            evict_post(state, *post_id, ingress_ids);
        }
    }
}

fn evict_post(state: &mut StateView, post_id: PostId, event_ingress_ids: &[IngressId]) {
    if let Some(post) = state.posts.remove(&post_id) {
        if let Some(set) = state.posts_by_stage.get_mut(&post.stage) {
            set.remove(&post_id);
            if set.is_empty() {
                state.posts_by_stage.remove(&post.stage);
            }
        }
    } else {
        for set in state.posts_by_stage.values_mut() {
            set.remove(&post_id);
        }
    }

    state.drafts.remove(&post_id);
    state.render.remove(&post_id);
    state.draft_transforms.remove(&post_id);
    state.external_code_by_post.remove(&post_id);
    state.manual_interventions.remove(&post_id);
    state.sending.remove(&post_id);
    remove_send_plan(state, post_id);
    remove_post_reviews(state, post_id);
    remove_post_qzone_publications(state, post_id);

    let ingress_ids = state
        .post_ingress
        .remove(&post_id)
        .unwrap_or_else(|| event_ingress_ids.to_vec());
    for ingress_id in ingress_ids {
        remove_ingress_if_unreferenced(state, ingress_id);
    }
}

fn remove_send_plan(state: &mut StateView, post_id: PostId) {
    if let Some(prev) = state.send_plans.remove(&post_id) {
        state.send_due.remove(&SendDueKey {
            not_before_ms: prev.not_before_ms,
            priority: prev.priority,
            seq: prev.seq,
            post_id: prev.post_id,
        });
    }
}

fn remove_post_reviews(state: &mut StateView, post_id: PostId) {
    let review_ids = state
        .reviews
        .iter()
        .filter_map(|(review_id, review)| (review.post_id == post_id).then_some(*review_id))
        .collect::<Vec<_>>();
    for review_id in review_ids {
        if let Some(review) = state.reviews.remove(&review_id) {
            state.review_by_code.remove(&review.review_code);
            if let Some(audit_msg_id) = review.audit_msg_id {
                state.review_by_audit_msg.remove(&audit_msg_id);
            }
        }
    }
}

fn remove_post_qzone_publications(state: &mut StateView, post_id: PostId) {
    state.qzone_publications_by_post.remove(&post_id);
    let mut empty_publications = Vec::new();
    for (key, publication) in &mut state.qzone_publications {
        publication.items.retain(|item| item.post_id != post_id);
        publication.withdrawn_posts.remove(&post_id);
        publication.pending_withdrawn_posts.remove(&post_id);
        if publication.items.is_empty() {
            empty_publications.push(key.clone());
        }
    }
    for key in &empty_publications {
        state.qzone_publications.remove(key);
    }
    if !empty_publications.is_empty() {
        for keys in state.qzone_publications_by_post.values_mut() {
            for key in &empty_publications {
                keys.remove(key);
            }
        }
    }
}

fn remove_ingress_if_unreferenced(state: &mut StateView, ingress_id: IngressId) {
    let still_referenced = state
        .post_ingress
        .values()
        .any(|ingress_ids| ingress_ids.contains(&ingress_id));
    if still_referenced {
        return;
    }
    state.ingress_meta.remove(&ingress_id);
    state.ingress_messages.remove(&ingress_id);
    state
        .media_fetch
        .retain(|key, _| key.ingress_id != ingress_id);
}

fn reduce_account(state: &mut StateView, event: &AccountEvent) {
    match event {
        AccountEvent::AccountEnabled { account_id } => {
            let runtime = state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
            runtime.enabled = true;
        }
        AccountEvent::AccountDisabled { account_id } => {
            let runtime = state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: false,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
            runtime.enabled = false;
        }
        AccountEvent::AccountCooldownSet {
            account_id,
            cooldown_until_ms,
        } => {
            let runtime = state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
            runtime.cooldown_until_ms = Some(*cooldown_until_ms);
        }
        AccountEvent::AccountLastSendUpdated {
            account_id,
            last_send_ms,
        } => {
            let runtime = state
                .accounts
                .entry(account_id.clone())
                .or_insert(AccountRuntime {
                    enabled: true,
                    cooldown_until_ms: None,
                    last_send_ms: None,
                });
            runtime.last_send_ms = Some(*last_send_ms);
        }
    }
}

fn reduce_manual(state: &mut StateView, event: &ManualEvent) {
    match event {
        ManualEvent::ManualInterventionRequired { post_id, .. } => {
            state.manual_interventions.insert(*post_id);
            state.update_post_stage(*post_id, PostStage::Manual);
        }
        ManualEvent::ManualInterventionResolved { post_id } => {
            state.manual_interventions.remove(post_id);
        }
    }
}
