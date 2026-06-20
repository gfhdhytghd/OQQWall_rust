use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::event::{DraftEvent, Event, RenderEvent, ReviewEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, EventEnvelope, Id128, ReviewAction, ReviewActionCommand, StateView,
    derive_review_id,
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

#[test]
fn toggle_anonymous_rerenders_and_marks_republish() {
    let post_id = Id128(10);
    let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
    let review_code = 123;
    let t0 = 1_000;

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
            created_at_ms: t0,
        }),
        1,
        t0,
    ));
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        }),
        2,
        t0,
    ));

    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        action: ReviewAction::ToggleAnonymous,
        operator_id: "admin".to_string(),
        now_ms: t0 + 10,
        tz_offset_minutes: 0,
    });
    let config = CoreConfig::default();
    let events = decide(&state, &cmd, &config);

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewAnonToggled { review_id: rid }) if *rid == review_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Render(RenderEvent::RenderRequested { post_id: pid, .. }) if *pid == post_id
    )));

    let mut state = state;
    for (idx, event) in events.iter().cloned().enumerate() {
        state = state.reduce(&wrap(event, 10 + idx as u128, t0 + 20 + idx as i64));
    }

    let post = state.posts.get(&post_id).expect("missing post");
    assert!(post.is_anonymous);
    let review = state.reviews.get(&review_id).expect("missing review");
    assert!(review.needs_republish);
}
