use oqqwall_rust_core::event::{
    Event, ManualEvent, ReviewDecision, ReviewEvent, ScheduleEvent, SendEvent,
};
use oqqwall_rust_core::{
    Command, CoreConfig, EventEnvelope, GroupConfig, Id128, StateView, derive_review_id,
};

fn wrap(event: Event, id: u128) -> EventEnvelope {
    EventEnvelope {
        id: Id128(id),
        ts_ms: 0,
        actor: Id128(0),
        correlation_id: None,
        event,
    }
}

fn build_state_with_sending(post_id: Id128) -> StateView {
    let state = StateView::default();
    let started = Event::Send(SendEvent::SendStarted {
        post_id,
        group_id: "group-a".to_string(),
        account_id: "acc".to_string(),
        started_at_ms: 0,
    });
    state.reduce(&wrap(started, 1))
}

fn build_state_with_review_and_sending(post_id: Id128, review_code: u32) -> StateView {
    let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
    let created = Event::Review(ReviewEvent::ReviewItemCreated {
        review_id,
        post_id,
        review_code,
    });
    let mut state = StateView::default();
    state = state.reduce(&wrap(created, 1));
    let started = Event::Send(SendEvent::SendStarted {
        post_id,
        group_id: "group-a".to_string(),
        account_id: "acc".to_string(),
        started_at_ms: 0,
    });
    state.reduce(&wrap(started, 2))
}

#[test]
fn send_failed_below_max_reschedules() {
    let mut config = CoreConfig::default();
    config.default_send_max_attempts = 3;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            ..Default::default()
        },
    );

    let post_id = Id128(10);
    let state = build_state_with_sending(post_id);
    let failed = Event::Send(SendEvent::SendFailed {
        post_id,
        account_id: "acc".to_string(),
        attempt: 1,
        retry_at_ms: 1_000,
        error: "boom".to_string(),
    });

    let events = oqqwall_rust_core::decide::decide(&state, &Command::DriverEvent(failed), &config);

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            group_id,
            not_before_ms,
            ..
        }) if group_id == "group-a" && *not_before_ms == 1_000
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Send(SendEvent::SendGaveUp { .. })))
    );
}

#[test]
fn send_failed_at_max_gives_up() {
    let mut config = CoreConfig::default();
    config.default_send_max_attempts = 2;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            ..Default::default()
        },
    );

    let post_id = Id128(20);
    let state = build_state_with_sending(post_id);
    let failed = Event::Send(SendEvent::SendFailed {
        post_id,
        account_id: "acc".to_string(),
        attempt: 2,
        retry_at_ms: 2_000,
        error: "boom".to_string(),
    });

    let events = oqqwall_rust_core::decide::decide(&state, &Command::DriverEvent(failed), &config);

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Send(SendEvent::SendGaveUp { post_id: id, .. }) if *id == post_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Manual(ManualEvent::ManualInterventionRequired { post_id: id, .. }) if *id == post_id
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::Schedule(ScheduleEvent::SendPlanRescheduled { .. })
    )));
}

#[test]
fn send_timeout_returns_to_pending() {
    let mut config = CoreConfig::default();
    config.default_send_max_attempts = 3;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            ..Default::default()
        },
    );

    let post_id = Id128(30);
    let state = build_state_with_review_and_sending(post_id, 9);
    let failed = Event::Send(SendEvent::SendFailed {
        post_id,
        account_id: "acc".to_string(),
        attempt: 1,
        retry_at_ms: 1_000,
        error: "send timeout after 300000 ms".to_string(),
    });

    let events = oqqwall_rust_core::decide::decide(&state, &Command::DriverEvent(failed), &config);

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            decision: ReviewDecision::Deferred,
            ..
        })
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewPublishRequested { .. })
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::Schedule(ScheduleEvent::SendPlanRescheduled { .. })
    )));
}
