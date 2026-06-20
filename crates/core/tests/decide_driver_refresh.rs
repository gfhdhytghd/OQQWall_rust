use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::event::{DraftEvent, Event, RenderEvent, ReviewEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, EventEnvelope, Id128, StateView, derive_review_id,
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

#[test]
fn refresh_republishes_after_render_ready() {
    let post_id = Id128(10);
    let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
    let mut state = StateView::default();

    state = state.reduce(&wrap(
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id: Id128(11),
            group_id: "group-a".to_string(),
            ingress_ids: Vec::new(),
            is_anonymous: false,
            is_safe: true,
            draft: Draft { blocks: Vec::new() },
            created_at_ms: 1_000,
        }),
        1,
    ));
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code: 7,
        }),
        2,
    ));
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewPublished {
            review_id,
            audit_msg_id: "msg-1".to_string(),
        }),
        3,
    ));
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewRefreshRequested { review_id }),
        4,
    ));

    let config = CoreConfig::default();

    let events = decide(
        &state,
        &Command::DriverEvent(Event::Render(RenderEvent::PngReady {
            post_id,
            blob_id: Id128(99),
        })),
        &config,
    );

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id: rid }) if *rid == review_id
    )));
}
