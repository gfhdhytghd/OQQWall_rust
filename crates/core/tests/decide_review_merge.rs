use oqqwall_rust_core::event::{DraftEvent, Event, IngressEvent, ReviewDecision, ReviewEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, DraftBlock, EventEnvelope, Id128, IngressMessage, ReviewAction,
    ReviewActionCommand, StateView,
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
fn merge_reviews_from_same_sender() {
    let ingress_a = Id128(1);
    let ingress_b = Id128(2);
    let session_a = Id128(10);
    let session_b = Id128(11);
    let post_a = Id128(100);
    let post_b = Id128(101);
    let review_a = Id128(200);
    let review_b = Id128(201);
    let review_code_a = 1000;
    let review_code_b = 1001;
    let t0 = 1_000;

    let message_a = IngressMessage {
        text: "a".to_string(),
        attachments: Vec::new(),
    };
    let message_b = IngressMessage {
        text: "b".to_string(),
        attachments: Vec::new(),
    };

    let mut state = StateView::default();
    let events = vec![
        wrap(
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id: ingress_a,
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                sender_name: None,
                group_id: "group".to_string(),
                platform_msg_id: "msg-a".to_string(),
                received_at_ms: t0,
                message: message_a,
                route_meta: None,
            }),
            1,
            t0,
        ),
        wrap(
            Event::Draft(DraftEvent::PostDraftCreated {
                post_id: post_a,
                session_id: session_a,
                group_id: "group".to_string(),
                ingress_ids: vec![ingress_a],
                is_anonymous: true,
                is_safe: true,
                draft: Draft {
                    blocks: vec![DraftBlock::Paragraph {
                        text: "a".to_string(),
                    }],
                },
                created_at_ms: t0,
            }),
            2,
            t0,
        ),
        wrap(
            Event::Review(ReviewEvent::ReviewItemCreated {
                review_id: review_a,
                post_id: post_a,
                review_code: review_code_a,
            }),
            3,
            t0,
        ),
        wrap(
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id: ingress_b,
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                sender_name: None,
                group_id: "group".to_string(),
                platform_msg_id: "msg-b".to_string(),
                received_at_ms: t0 + 10,
                message: message_b,
                route_meta: None,
            }),
            4,
            t0 + 10,
        ),
        wrap(
            Event::Draft(DraftEvent::PostDraftCreated {
                post_id: post_b,
                session_id: session_b,
                group_id: "group".to_string(),
                ingress_ids: vec![ingress_b],
                is_anonymous: false,
                is_safe: false,
                draft: Draft {
                    blocks: vec![DraftBlock::Paragraph {
                        text: "b".to_string(),
                    }],
                },
                created_at_ms: t0 + 10,
            }),
            5,
            t0 + 10,
        ),
        wrap(
            Event::Review(ReviewEvent::ReviewItemCreated {
                review_id: review_b,
                post_id: post_b,
                review_code: review_code_b,
            }),
            6,
            t0 + 10,
        ),
    ];
    for env in events {
        state = state.reduce(&env);
    }

    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: None,
        review_code: Some(review_code_a),
        audit_msg_id: None,
        action: ReviewAction::Merge {
            review_code: review_code_b,
        },
        operator_id: "admin".to_string(),
        now_ms: t0 + 100,
        tz_offset_minutes: 0,
    });
    let config = CoreConfig::default();
    let out = oqqwall_rust_core::decide::decide(&state, &cmd, &config);

    let merged = out.iter().find_map(|event| match event {
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            ingress_ids,
            is_anonymous,
            is_safe,
            ..
        }) if *post_id == post_a => Some((ingress_ids.clone(), *is_anonymous, *is_safe)),
        _ => None,
    });
    assert_eq!(merged, Some((vec![ingress_a, ingress_b], true, false)));

    let skipped = out.iter().any(|event| {
        matches!(
            event,
            Event::Review(ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision: ReviewDecision::Skipped,
                ..
            }) if *review_id == review_b
        )
    });
    assert!(skipped);
}
