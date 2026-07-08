use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::event::{DraftEvent, Event, RenderEvent, ReviewDecision, ReviewEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, EventEnvelope, Id128, ReviewAction, ReviewActionBatchCommand,
    StateView, derive_review_id,
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

fn seed_review_state() -> (StateView, Id128, u32, Id128) {
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

    (state, post_id, review_code, review_id)
}

#[test]
fn review_batch_toggle_anonymous_then_approve_uses_updated_state() {
    let (state, post_id, review_code, review_id) = seed_review_state();
    let cmd = Command::ReviewActionBatch(ReviewActionBatchCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        actions: vec![ReviewAction::ToggleAnonymous, ReviewAction::Approve],
        operator_id: "admin".to_string(),
        now_ms: 1_010,
        tz_offset_minutes: 0,
    });

    let events = decide(&state, &cmd, &CoreConfig::default());
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewAnonToggled { review_id: rid }) if *rid == review_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id: rid,
            decision: ReviewDecision::Approved,
            ..
        }) if *rid == review_id
    )));

    let mut reduced = state;
    for (idx, event) in events.into_iter().enumerate() {
        reduced = reduced.reduce(&wrap(event, 10 + idx as u128, 2_000 + idx as i64));
    }

    let post = reduced.posts.get(&post_id).expect("missing post");
    assert!(post.is_anonymous);
    let review = reduced.reviews.get(&review_id).expect("missing review");
    assert_eq!(review.decision, Some(ReviewDecision::Approved));
}

#[test]
fn review_batch_reject_then_blacklist_keeps_followup_action() {
    let (state, _, review_code, review_id) = seed_review_state();
    let cmd = Command::ReviewActionBatch(ReviewActionBatchCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        actions: vec![
            ReviewAction::Reject,
            ReviewAction::Blacklist {
                reason: Some("spam".to_string()),
            },
        ],
        operator_id: "admin".to_string(),
        now_ms: 1_020,
        tz_offset_minutes: 0,
    });

    let events = decide(&state, &cmd, &CoreConfig::default());
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id: rid,
            decision: ReviewDecision::Rejected,
            ..
        }) if *rid == review_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewBlacklistRequested { review_id: rid, reason })
            if *rid == review_id && reason.as_deref() == Some("spam")
    )));
}

#[test]
fn review_batch_stops_after_first_invalid_followup() {
    let (state, _, review_code, review_id) = seed_review_state();
    let cmd = Command::ReviewActionBatch(ReviewActionBatchCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        actions: vec![
            ReviewAction::ToggleAnonymous,
            ReviewAction::Merge { review_code: 999 },
            ReviewAction::Approve,
        ],
        operator_id: "admin".to_string(),
        now_ms: 1_030,
        tz_offset_minutes: 0,
    });

    let events = decide(&state, &cmd, &CoreConfig::default());
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewAnonToggled { review_id: rid }) if *rid == review_id
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Render(RenderEvent::RenderRequested { .. })))
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            decision: ReviewDecision::Approved,
            ..
        })
    )));
}
