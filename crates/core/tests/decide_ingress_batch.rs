use oqqwall_rust_core::event::{
    DraftEvent, Event, IngressEvent, MediaEvent, RenderEvent, ReviewEvent, SessionEvent,
};
use oqqwall_rust_core::state::{PostMeta, PostStage, ReviewMeta};
use oqqwall_rust_core::{
    Command, CoreConfig, DraftBlock, EventEnvelope, Id128, IngressAttachment, IngressBatchCommand,
    IngressCommand, IngressMessage, MediaKind, MediaReference, StateView, derive_ingress_id,
};

fn ingress(message_id: &str, text: &str) -> IngressCommand {
    IngressCommand {
        profile_id: "bot".to_string(),
        chat_id: "user_submission_1".to_string(),
        user_id: "user".to_string(),
        sender_name: Some("sender".to_string()),
        group_id: "group-a".to_string(),
        platform_msg_id: message_id.to_string(),
        message: IngressMessage {
            text: text.to_string(),
            attachments: Vec::new(),
        },
        route_meta: None,
        received_at_ms: 1000,
        close_immediately: true,
    }
}

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
fn ingress_batch_accepts_messages_and_closes_post_immediately() {
    let first = ingress("m1", "第一条");
    let second = ingress("m2", "第二条");
    let first_id = derive_ingress_id(&[b"bot", b"user_submission_1", b"user", b"m1"]);
    let second_id = derive_ingress_id(&[b"bot", b"user_submission_1", b"user", b"m2"]);

    let events = oqqwall_rust_core::decide::decide(
        &StateView::default(),
        &Command::IngressBatch(IngressBatchCommand {
            entries: vec![first, second],
            now_ms: 2000,
        }),
        &CoreConfig::default(),
    );

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Ingress(IngressEvent::MessageAccepted { .. })))
            .count(),
        2
    );
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Session(SessionEvent::Closed {
            closed_at_ms: 2000,
            ..
        })
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Draft(DraftEvent::PostDraftCreated { ingress_ids, draft, .. })
            if ingress_ids == &vec![first_id, second_id]
                && draft.blocks == vec![
                    DraftBlock::Paragraph { text: "第一条".to_string() },
                    DraftBlock::Paragraph { text: "第二条".to_string() },
                ]
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Render(RenderEvent::RenderRequested {
            requested_at_ms: 2000,
            ..
        })
    )));
}

#[test]
fn fetched_media_refreshes_existing_post_render() {
    let ingress_id = Id128(10);
    let post_id = Id128(11);
    let review_id = Id128(12);
    let blob_id = Id128(13);
    let mut state = StateView::default();
    state = state.reduce(&wrap(
        Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id,
            profile_id: "bot".to_string(),
            chat_id: "chat".to_string(),
            user_id: "user".to_string(),
            sender_name: None,
            group_id: "group-a".to_string(),
            platform_msg_id: "m1".to_string(),
            route_meta: None,
            received_at_ms: 1,
            message: IngressMessage {
                text: String::new(),
                attachments: vec![IngressAttachment {
                    kind: MediaKind::Image,
                    name: Some("a.jpg".to_string()),
                    reference: MediaReference::RemoteUrl {
                        url: "https://example.test/a.jpg".to_string(),
                    },
                    size_bytes: None,
                }],
            },
        }),
        1,
        1,
    ));
    state.post_ingress.insert(post_id, vec![ingress_id]);
    state.posts.insert(
        post_id,
        PostMeta {
            post_id,
            session_id: Id128(14),
            group_id: "group-a".to_string(),
            stage: PostStage::ReviewPending,
            review_id: Some(review_id),
            created_at_ms: 2,
            is_anonymous: false,
            is_safe: true,
            last_error: None,
        },
    );
    state.reviews.insert(
        review_id,
        ReviewMeta {
            review_id,
            post_id,
            review_code: 7,
            decision: None,
            audit_msg_id: Some("audit-1".to_string()),
            delayed_until_ms: None,
            needs_republish: false,
            decided_by: None,
            decided_at_ms: None,
            decision_reason: None,
            publish_retry_at_ms: None,
            publish_last_error: None,
            publish_attempt: 0,
        },
    );

    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::DriverEvent(Event::Media(MediaEvent::MediaFetchSucceeded {
            ingress_id,
            attachment_index: 0,
            blob_id,
        })),
        &CoreConfig::default(),
    );

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewRefreshRequested { review_id: id }) if *id == review_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Render(RenderEvent::RenderRequested { post_id: id, .. }) if *id == post_id
    )));
}
