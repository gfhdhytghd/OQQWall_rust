use oqqwall_rust_core::event::{
    DraftEvent, Event, ReviewDecision, ReviewEvent, ScheduleEvent, SendPriority,
};
use oqqwall_rust_core::state::PostStage;
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, DraftBlock, EventEnvelope, GlobalAction, GlobalActionCommand,
    Id128, StateView,
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

fn apply_event(state: &mut StateView, event: Event, id: u128) {
    *state = state.reduce(&wrap(event, id));
}

fn seed_post(
    state: &mut StateView,
    post_id: Id128,
    review_id: Id128,
    review_code: u32,
    group_id: &str,
    external_code: u64,
    next_id: u128,
) -> u128 {
    apply_event(
        state,
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id: Id128(post_id.0.saturating_add(1000)),
            group_id: group_id.to_string(),
            ingress_ids: Vec::new(),
            is_anonymous: false,
            is_safe: true,
            draft: Draft {
                blocks: vec![DraftBlock::Paragraph {
                    text: format!("post-{}", post_id.0),
                }],
            },
            created_at_ms: 0,
        }),
        next_id,
    );
    apply_event(
        state,
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        }),
        next_id + 1,
    );
    apply_event(
        state,
        Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
            post_id,
            group_id: group_id.to_string(),
            external_code,
        }),
        next_id + 2,
    );
    next_id + 3
}

#[test]
fn withdraw_requeues_post_and_rebases_following_external_codes() {
    let mut state = StateView::default();
    let mut next_id = 1;

    let withdrawn_post = Id128(1);
    let withdrawn_review = Id128(101);
    next_id = seed_post(
        &mut state,
        withdrawn_post,
        withdrawn_review,
        100,
        "group-a",
        101,
        next_id,
    );

    let skipped_post = Id128(2);
    let skipped_review = Id128(102);
    next_id = seed_post(
        &mut state,
        skipped_post,
        skipped_review,
        101,
        "group-a",
        102,
        next_id,
    );
    apply_event(
        &mut state,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id: skipped_review,
            decision: ReviewDecision::Skipped,
            decided_by: "admin".to_string(),
            decided_at_ms: 1,
        }),
        next_id,
    );
    next_id += 1;

    let queued_after_post = Id128(3);
    let queued_after_review = Id128(103);
    next_id = seed_post(
        &mut state,
        queued_after_post,
        queued_after_review,
        102,
        "group-a",
        103,
        next_id,
    );

    apply_event(
        &mut state,
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id: withdrawn_post,
            group_id: "group-a".to_string(),
            not_before_ms: 10_000,
            priority: SendPriority::Normal,
            seq: 1,
        }),
        next_id,
    );
    apply_event(
        &mut state,
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id: queued_after_post,
            group_id: "group-a".to_string(),
            not_before_ms: 11_000,
            priority: SendPriority::Normal,
            seq: 2,
        }),
        next_id + 1,
    );

    let cmd = Command::GlobalAction(GlobalActionCommand {
        group_id: "group-a".to_string(),
        action: GlobalAction::Withdraw { review_code: 100 },
        operator_id: "admin".to_string(),
        now_ms: 5_000,
        tz_offset_minutes: 0,
    });
    let out = oqqwall_rust_core::decide::decide(&state, &cmd, &CoreConfig::default());

    assert!(out.iter().any(|event| matches!(
        event,
        Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }) if *post_id == withdrawn_post
    )));
    assert!(out.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision: ReviewDecision::Deferred,
            decided_by,
            decided_at_ms,
        }) if *review_id == withdrawn_review && decided_by == "admin" && *decided_at_ms == 5_000
    )));
    assert!(out.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewExternalCodeCleared { post_id }) if *post_id == withdrawn_post
    )));
    assert!(out.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
            post_id,
            group_id,
            external_code,
        }) if *post_id == queued_after_post && group_id == "group-a" && *external_code == 102
    )));
    assert!(out.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewExternalNumberSet {
            group_id,
            next_number,
        }) if group_id == "group-a" && *next_number == 103
    )));
    assert!(out.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id }) if *review_id == withdrawn_review
    )));

    let mut reduced = state.clone();
    for (idx, event) in out.into_iter().enumerate() {
        apply_event(&mut reduced, event, 100 + idx as u128);
    }

    assert!(!reduced.send_plans.contains_key(&withdrawn_post));
    assert_eq!(
        reduced.posts.get(&withdrawn_post).map(|meta| meta.stage),
        Some(PostStage::ReviewPending)
    );
    assert_eq!(reduced.external_code_by_post.get(&withdrawn_post), None);
    assert_eq!(reduced.external_code_by_post.get(&skipped_post), Some(&102));
    assert_eq!(
        reduced.external_code_by_post.get(&queued_after_post),
        Some(&102)
    );
    assert_eq!(
        reduced.next_external_code_by_group.get("group-a"),
        Some(&103)
    );
}

#[test]
fn withdraw_ignores_post_not_in_send_queue() {
    let mut state = StateView::default();
    seed_post(&mut state, Id128(1), Id128(101), 100, "group-a", 101, 1);

    let cmd = Command::GlobalAction(GlobalActionCommand {
        group_id: "group-a".to_string(),
        action: GlobalAction::Withdraw { review_code: 100 },
        operator_id: "admin".to_string(),
        now_ms: 5_000,
        tz_offset_minutes: 0,
    });

    let out = oqqwall_rust_core::decide::decide(&state, &cmd, &CoreConfig::default());
    assert!(out.is_empty());
}
