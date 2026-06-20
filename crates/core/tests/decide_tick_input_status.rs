use oqqwall_rust_core::event::{Event, IngressEvent, InputStatusKind, SessionEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, EventEnvelope, Id128, IngressMessage, StateView, TickCommand,
};

fn wrap(event: Event, id: u128, ts_ms: i64) -> EventEnvelope {
    EventEnvelope {
        id: Id128(id),
        ts_ms,
        actor: Id128(0),
        correlation_id: None,
        event,
    }
}

fn build_state(status: Option<(InputStatusKind, i64)>, wait_ms: i64) -> (StateView, Id128) {
    let ingress_id = Id128(10);
    let session_id = Id128(11);
    let message = IngressMessage {
        text: "hi".to_string(),
        attachments: Vec::new(),
    };
    let t0 = 1_000;
    let mut state = StateView::default();
    let base_events = vec![
        (
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id,
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                sender_name: None,
                group_id: "group".to_string(),
                platform_msg_id: "msg-1".to_string(),
                received_at_ms: t0,
                message,
                route_meta: None,
            }),
            t0,
        ),
        (
            Event::Session(SessionEvent::Opened {
                session_id,
                first_ingress_id: ingress_id,
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                group_id: "group".to_string(),
                close_at_ms: t0 + wait_ms,
            }),
            t0,
        ),
    ];
    let mut next_id = 1;
    for (event, ts_ms) in base_events {
        state = state.reduce(&wrap(event, next_id, ts_ms));
        next_id += 1;
    }
    if let Some((status, ts_ms)) = status {
        state = state.reduce(&wrap(
            Event::Ingress(IngressEvent::InputStatusUpdated {
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                group_id: "group".to_string(),
                status,
                received_at_ms: ts_ms,
            }),
            next_id,
            ts_ms,
        ));
    }
    (state, session_id)
}

fn has_session_closed(events: &[Event], session_id: Id128) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            Event::Session(SessionEvent::Closed { session_id: id, .. })
                if *id == session_id
        )
    })
}

#[test]
fn typing_blocks_close_within_waittime() {
    let wait_ms = 10_000;
    let t0 = 1_000;
    let typing_ts = t0 + 2_000;
    let (state, session_id) = build_state(Some((InputStatusKind::Typing, typing_ts)), wait_ms);
    let config = CoreConfig {
        default_process_waittime_ms: wait_ms,
        ..CoreConfig::default()
    };
    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: t0 + wait_ms + 1_000,
            tz_offset_minutes: 0,
        }),
        &config,
    );
    assert!(!has_session_closed(&events, session_id));
}

#[test]
fn stopped_waits_before_close() {
    let wait_ms = 10_000;
    let t0 = 1_000;
    let stopped_ts = t0 + 2_000;
    let (state, session_id) = build_state(Some((InputStatusKind::Stopped, stopped_ts)), wait_ms);
    let config = CoreConfig {
        default_process_waittime_ms: wait_ms,
        ..CoreConfig::default()
    };
    let early_events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: stopped_ts + wait_ms - 500,
            tz_offset_minutes: 0,
        }),
        &config,
    );
    assert!(!has_session_closed(&early_events, session_id));
    let due_events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: stopped_ts + wait_ms,
            tz_offset_minutes: 0,
        }),
        &config,
    );
    assert!(has_session_closed(&due_events, session_id));
}

#[test]
fn no_input_status_uses_double_wait() {
    let wait_ms = 10_000;
    let t0 = 1_000;
    let (state, session_id) = build_state(None, wait_ms);
    let config = CoreConfig {
        default_process_waittime_ms: wait_ms,
        ..CoreConfig::default()
    };
    let early_events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: t0 + wait_ms + 1_000,
            tz_offset_minutes: 0,
        }),
        &config,
    );
    assert!(!has_session_closed(&early_events, session_id));
    let due_events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: t0 + wait_ms * 2 + 1,
            tz_offset_minutes: 0,
        }),
        &config,
    );
    assert!(has_session_closed(&due_events, session_id));
}

#[test]
fn typing_timeout_ignores_status() {
    let wait_ms = 10_000;
    let t0 = 1_000;
    let typing_ts = t0 + 2_000;
    let (state, session_id) = build_state(Some((InputStatusKind::Typing, typing_ts)), wait_ms);
    let config = CoreConfig {
        default_process_waittime_ms: wait_ms,
        ..CoreConfig::default()
    };
    let now_ms = typing_ts + 30 * 60 * 1000 + 1;
    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms,
            tz_offset_minutes: 0,
        }),
        &config,
    );
    assert!(has_session_closed(&events, session_id));
}
