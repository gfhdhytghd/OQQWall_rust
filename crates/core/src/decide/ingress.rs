use crate::command::IngressCommand;
use crate::config::CoreConfig;
use crate::draft::MediaReference;
use crate::event::{Event, IngressEvent, IngressIgnoreReason, MediaEvent, SessionEvent};
use crate::ids::derive_ingress_id;
use crate::ids::derive_session_id;
use crate::state::{SessionKey, StateView};

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
