use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::draft::IngressMessage;
use oqqwall_rust_core::event::{
    DraftEvent, Event, IngressEvent, RenderEvent, ReviewDecision, ReviewEvent, ScheduleEvent,
    SendPriority, SessionEvent,
};
use oqqwall_rust_core::state::PostStage;
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, EventEnvelope, Id128, IngressAttachment, StateView,
    derive_review_id,
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

fn ingress_event(ingress_id: Id128, msg_id: &str) -> Event {
    Event::Ingress(IngressEvent::MessageAccepted {
        ingress_id,
        profile_id: "bot".to_string(),
        chat_id: "u1".to_string(),
        user_id: "u1".to_string(),
        sender_name: Some("sender".to_string()),
        group_id: "group-a".to_string(),
        platform_msg_id: msg_id.to_string(),
        received_at_ms: 1,
        message: IngressMessage {
            text: format!("text-{}", msg_id),
            attachments: Vec::<IngressAttachment>::new(),
        },
    })
}

#[test]
fn recall_before_draft_creation_only_passes_through() {
    let ingress_id = Id128(10);
    let mut state = StateView::default();
    state = state.reduce(&wrap(ingress_event(ingress_id, "m1"), 1));
    state = state.reduce(&wrap(
        Event::Session(SessionEvent::Opened {
            session_id: Id128(11),
            first_ingress_id: ingress_id,
            chat_id: "u1".to_string(),
            user_id: "u1".to_string(),
            group_id: "group-a".to_string(),
            close_at_ms: 999,
        }),
        2,
    ));

    let events = decide(
        &state,
        &Command::DriverEvent(Event::Ingress(IngressEvent::MessageRecalled {
            ingress_id,
            recalled_at_ms: 2,
        })),
        &CoreConfig::default(),
    );

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::Ingress(IngressEvent::MessageRecalled {
            ingress_id: id,
            recalled_at_ms: 2,
        }) if id == ingress_id
    ));
}

#[test]
fn recall_on_drafted_post_rebuilds_and_refreshes() {
    let ingress_a = Id128(20);
    let ingress_b = Id128(21);
    let post_id = Id128(30);
    let review_id = derive_review_id(&[&post_id.to_be_bytes()]);

    let mut state = StateView::default();
    state = state.reduce(&wrap(ingress_event(ingress_a, "a"), 1));
    state = state.reduce(&wrap(ingress_event(ingress_b, "b"), 2));
    state = state.reduce(&wrap(
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id: Id128(40),
            group_id: "group-a".to_string(),
            ingress_ids: vec![ingress_a, ingress_b],
            is_anonymous: false,
            is_safe: true,
            draft: Draft { blocks: Vec::new() },
            created_at_ms: 3,
        }),
        3,
    ));
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code: 7,
        }),
        4,
    ));

    let events = decide(
        &state,
        &Command::DriverEvent(Event::Ingress(IngressEvent::MessageRecalled {
            ingress_id: ingress_a,
            recalled_at_ms: 9,
        })),
        &CoreConfig::default(),
    );

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id: id,
            ingress_ids,
            ..
        }) if *id == post_id && ingress_ids == &vec![ingress_b]
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewRefreshRequested { review_id: id }) if *id == review_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Render(RenderEvent::RenderRequested { post_id: id, .. }) if *id == post_id
    )));
}

#[test]
fn recall_removing_last_message_marks_deleted_and_cancels_plan() {
    let ingress_id = Id128(50);
    let post_id = Id128(51);
    let review_id = derive_review_id(&[&post_id.to_be_bytes()]);

    let mut state = StateView::default();
    state = state.reduce(&wrap(ingress_event(ingress_id, "x"), 1));
    state = state.reduce(&wrap(
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id: Id128(52),
            group_id: "group-a".to_string(),
            ingress_ids: vec![ingress_id],
            is_anonymous: false,
            is_safe: true,
            draft: Draft { blocks: Vec::new() },
            created_at_ms: 2,
        }),
        2,
    ));
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code: 9,
        }),
        3,
    ));
    state = state.reduce(&wrap(
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id,
            group_id: "group-a".to_string(),
            not_before_ms: 10_000,
            priority: SendPriority::Normal,
            seq: 1,
        }),
        4,
    ));

    let events = decide(
        &state,
        &Command::DriverEvent(Event::Ingress(IngressEvent::MessageRecalled {
            ingress_id,
            recalled_at_ms: 99,
        })),
        &CoreConfig::default(),
    );

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id: id,
            decision: ReviewDecision::Deleted,
            decided_by,
            ..
        }) if *id == review_id && decided_by == "system_recall"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id: id }) if *id == post_id
    )));
    let mut reduced = state;
    for (idx, event) in events.into_iter().enumerate() {
        reduced = reduced.reduce(&wrap(event, 10 + idx as u128));
    }
    assert_eq!(
        reduced.posts.get(&post_id).map(|post| post.stage),
        Some(PostStage::Deleted)
    );
}
