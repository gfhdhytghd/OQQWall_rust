use oqqwall_rust_core::event::{
    BlobEvent, ConfigEvent, DraftEvent, Event, IngressEvent, ManualEvent, RenderEvent, ReviewEvent,
    ScheduleEvent, SendEvent, SendPriority, SessionEvent,
};
use oqqwall_rust_core::{
    Draft, DraftBlock, EventEnvelope, Id128, IngressAttachment, IngressMessage, MediaKind,
    MediaReference, StateView,
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
fn reducer_replay_matches_full_apply() {
    let ingress_id_a = Id128(10);
    let ingress_id_b = Id128(11);
    let session_id = Id128(12);
    let post_id = Id128(13);
    let review_id = Id128(14);
    let blob_id = Id128(15);

    let message = IngressMessage {
        text: "hello".to_string(),
        attachments: vec![IngressAttachment {
            kind: MediaKind::Image,
            name: Some("img.png".to_string()),
            reference: MediaReference::RemoteUrl {
                url: "http://example.com/img.png".to_string(),
            },
            size_bytes: None,
        }],
    };

    let events = vec![
        wrap(
            Event::Config(ConfigEvent::Applied {
                version: 1,
                config_blob: None,
            }),
            1,
            1_000,
        ),
        wrap(
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id: ingress_id_a,
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                sender_name: Some("sender".to_string()),
                group_id: "group-a".to_string(),
                platform_msg_id: "msg-1".to_string(),
                received_at_ms: 1_100,
                message: message.clone(),
                route_meta: None,
            }),
            2,
            1_100,
        ),
        wrap(
            Event::Session(SessionEvent::Opened {
                session_id,
                first_ingress_id: ingress_id_a,
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                group_id: "group-a".to_string(),
                close_at_ms: 2_000,
            }),
            3,
            1_200,
        ),
        wrap(
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id: ingress_id_b,
                profile_id: "bot".to_string(),
                chat_id: "chat".to_string(),
                user_id: "user".to_string(),
                sender_name: None,
                group_id: "group-a".to_string(),
                platform_msg_id: "msg-2".to_string(),
                received_at_ms: 1_300,
                message,
                route_meta: None,
            }),
            4,
            1_300,
        ),
        wrap(
            Event::Session(SessionEvent::Appended {
                session_id,
                ingress_id: ingress_id_b,
                close_at_ms: 2_500,
            }),
            5,
            1_400,
        ),
        wrap(
            Event::Draft(DraftEvent::PostDraftCreated {
                post_id,
                session_id,
                group_id: "group-a".to_string(),
                ingress_ids: vec![ingress_id_a, ingress_id_b],
                is_anonymous: false,
                is_safe: true,
                draft: Draft {
                    blocks: vec![DraftBlock::Paragraph {
                        text: "hello".to_string(),
                    }],
                },
                created_at_ms: 1_500,
            }),
            6,
            1_500,
        ),
        wrap(
            Event::Render(RenderEvent::RenderRequested {
                post_id,
                attempt: 1,
                requested_at_ms: 1_600,
            }),
            7,
            1_600,
        ),
        wrap(
            Event::Review(ReviewEvent::ReviewItemCreated {
                review_id,
                post_id,
                review_code: 42,
            }),
            8,
            1_700,
        ),
        wrap(
            Event::Schedule(ScheduleEvent::SendPlanCreated {
                post_id,
                group_id: "group-a".to_string(),
                not_before_ms: 2_000,
                priority: SendPriority::Normal,
                seq: 7,
            }),
            9,
            1_800,
        ),
        wrap(
            Event::Send(SendEvent::SendStarted {
                post_id,
                group_id: "group-a".to_string(),
                account_id: "acc-1".to_string(),
                started_at_ms: 2_100,
            }),
            10,
            1_900,
        ),
        wrap(
            Event::Blob(BlobEvent::BlobRegistered {
                blob_id,
                size_bytes: 99,
            }),
            11,
            2_000,
        ),
        wrap(
            Event::Manual(ManualEvent::ManualInterventionRequired {
                post_id,
                reason: "needs review".to_string(),
            }),
            12,
            2_100,
        ),
    ];

    let mut full_state = StateView::default();
    for env in &events {
        full_state = full_state.reduce(env);
    }

    let split = 6;
    let mut replay_state = StateView::default();
    for env in &events[..split] {
        replay_state = replay_state.reduce(env);
    }
    for env in &events[split..] {
        replay_state = replay_state.reduce(env);
    }

    assert_eq!(full_state, replay_state);
}
