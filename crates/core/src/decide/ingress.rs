use crate::anonymous::detect_anonymous;
use crate::command::{IngressBatchCommand, IngressCommand};
use crate::config::CoreConfig;
use crate::decide::builder::build_draft_for_post;
use crate::draft::MediaReference;
use crate::event::{
    DraftEvent, Event, EventEnvelope, IngressEvent, IngressIgnoreReason, MediaEvent, RenderEvent,
    SessionEvent,
};
use crate::ids::{ActorId, Id128, derive_ingress_id};
use crate::ids::{derive_post_id, derive_session_id};
use crate::safety::detect_safe;
use crate::state::{SessionKey, StateView};
use std::collections::HashSet;

pub fn decide_ingress(state: &StateView, cmd: &IngressCommand, config: &CoreConfig) -> Vec<Event> {
    let ingress_id = derive_ingress_id(&[
        cmd.profile_id.as_bytes(),
        cmd.chat_id.as_bytes(),
        cmd.user_id.as_bytes(),
        cmd.platform_msg_id.as_bytes(),
    ]);
    if state.ingress_seen.contains(&ingress_id) {
        return vec![Event::Ingress(IngressEvent::MessageIgnored {
            ingress_id,
            reason: IngressIgnoreReason::Duplicate,
        })];
    }
    if is_blacklisted(state, &cmd.group_id, &cmd.user_id) {
        return vec![Event::Ingress(IngressEvent::MessageIgnored {
            ingress_id,
            reason: IngressIgnoreReason::Blacklisted,
        })];
    }

    let close_at_ms = initial_close_at(state, cmd, config);
    let key = SessionKey {
        chat_id: cmd.chat_id.clone(),
        user_id: cmd.user_id.clone(),
        group_id: cmd.group_id.clone(),
    };

    let mut events = Vec::new();
    events.push(Event::Ingress(IngressEvent::MessageAccepted {
        ingress_id,
        profile_id: cmd.profile_id.clone(),
        chat_id: cmd.chat_id.clone(),
        user_id: cmd.user_id.clone(),
        sender_name: cmd.sender_name.clone(),
        group_id: cmd.group_id.clone(),
        platform_msg_id: cmd.platform_msg_id.clone(),
        route_meta: cmd.route_meta.clone(),
        received_at_ms: cmd.received_at_ms,
        message: cmd.message.clone(),
    }));

    if let Some(session_id) = state.session_by_key.get(&key) {
        events.push(Event::Session(SessionEvent::Appended {
            session_id: *session_id,
            ingress_id,
            close_at_ms,
        }));
    } else {
        let ingress_bytes = ingress_id.to_be_bytes();
        let session_id = derive_session_id(&[
            cmd.chat_id.as_bytes(),
            cmd.user_id.as_bytes(),
            cmd.group_id.as_bytes(),
            &ingress_bytes,
        ]);
        events.push(Event::Session(SessionEvent::Opened {
            session_id,
            first_ingress_id: ingress_id,
            chat_id: cmd.chat_id.clone(),
            user_id: cmd.user_id.clone(),
            group_id: cmd.group_id.clone(),
            close_at_ms,
        }));
    }

    for (idx, attachment) in cmd.message.attachments.iter().enumerate() {
        if let MediaReference::RemoteUrl { url } = &attachment.reference {
            if !url.starts_with("data:") {
                events.push(Event::Media(MediaEvent::MediaFetchRequested {
                    ingress_id,
                    attachment_index: idx,
                    attempt: 1,
                }));
            }
        }
    }

    events
}

pub fn decide_ingress_batch(
    state: &StateView,
    cmd: &IngressBatchCommand,
    config: &CoreConfig,
) -> Vec<Event> {
    let mut scratch = state.clone();
    let mut out = Vec::new();
    let mut env_id = 1u128;
    let mut keys = Vec::new();
    let mut seen_keys = HashSet::new();

    for entry in &cmd.entries {
        let key = SessionKey {
            chat_id: entry.chat_id.clone(),
            user_id: entry.user_id.clone(),
            group_id: entry.group_id.clone(),
        };
        if seen_keys.insert(key.clone()) {
            keys.push(key);
        }
        let step_events = decide_ingress(&scratch, entry, config);
        for event in &step_events {
            scratch = reduce_scratch(&scratch, event.clone(), cmd.now_ms, &mut env_id);
        }
        out.extend(step_events);
    }

    for key in keys {
        let Some(session_id) = scratch.session_by_key.get(&key).copied() else {
            continue;
        };
        let Some(ingress_ids) = scratch.session_ingress.get(&session_id).cloned() else {
            continue;
        };
        if ingress_ids.is_empty() {
            continue;
        }
        let mut messages = Vec::new();
        for ingress_id in &ingress_ids {
            if let Some(message) = scratch.ingress_messages.get(ingress_id) {
                messages.push(message.clone());
            }
        }
        let is_anonymous = detect_anonymous(&messages);
        let is_safe = detect_safe(&messages);
        let session_bytes = session_id.to_be_bytes();
        let post_id = derive_post_id(&[&session_bytes]);
        let Some(draft) = build_draft_for_post(&scratch, post_id, &ingress_ids) else {
            continue;
        };

        let close_event = Event::Session(SessionEvent::Closed {
            session_id,
            closed_at_ms: cmd.now_ms,
        });
        scratch = reduce_scratch(&scratch, close_event.clone(), cmd.now_ms, &mut env_id);
        out.push(close_event);

        let draft_event = Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: key.group_id,
            ingress_ids,
            is_anonymous,
            is_safe,
            draft,
            created_at_ms: cmd.now_ms,
        });
        scratch = reduce_scratch(&scratch, draft_event.clone(), cmd.now_ms, &mut env_id);
        out.push(draft_event);

        out.push(Event::Render(RenderEvent::RenderRequested {
            post_id,
            attempt: 1,
            requested_at_ms: cmd.now_ms,
        }));
    }

    out
}

fn reduce_scratch(state: &StateView, event: Event, ts_ms: i64, env_id: &mut u128) -> StateView {
    let next = state.reduce(&EventEnvelope {
        id: Id128(*env_id),
        ts_ms,
        actor: ActorId::from_u128(0),
        correlation_id: None,
        event,
    });
    *env_id = (*env_id).saturating_add(1);
    next
}

fn initial_close_at(state: &StateView, cmd: &IngressCommand, config: &CoreConfig) -> i64 {
    if cmd.close_immediately {
        return cmd.received_at_ms;
    }
    let wait_ms = config.process_waittime_ms(&cmd.group_id);
    let key = SessionKey {
        chat_id: cmd.chat_id.clone(),
        user_id: cmd.user_id.clone(),
        group_id: cmd.group_id.clone(),
    };
    let (multiplier, last_status_ms) = match state.input_status.get(&key) {
        Some(meta) => (1, Some(meta.updated_at_ms)),
        None => (2, None),
    };
    let last_activity_ms = last_status_ms
        .map(|status_ms| status_ms.max(cmd.received_at_ms))
        .unwrap_or(cmd.received_at_ms);
    last_activity_ms.saturating_add(wait_ms.saturating_mul(multiplier))
}

fn is_blacklisted(state: &StateView, group_id: &str, user_id: &str) -> bool {
    state
        .blacklist
        .get(group_id)
        .map(|entries| entries.contains_key(user_id))
        .unwrap_or(false)
}
