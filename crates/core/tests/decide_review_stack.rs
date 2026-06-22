use oqqwall_rust_core::event::{
    DraftEvent, Event, IngressEvent, RenderEvent, ReviewEvent, ScheduleEvent, SendPriority,
};
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, DraftBlock, EventEnvelope, GroupConfig, Id128, IngressAttachment,
    IngressMessage, MediaKind, MediaReference, ReviewAction, ReviewActionCommand, StateView,
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
    ingress_id: Id128,
    group_id: &str,
    attachment_count: usize,
    with_render: bool,
    mut next_id: u128,
) -> u128 {
    let attachments = (0..attachment_count)
        .map(|idx| IngressAttachment {
            kind: MediaKind::Image,
            name: None,
            reference: MediaReference::RemoteUrl {
                url: format!("file://img-{}.png", idx),
            },
            size_bytes: None,
        })
        .collect::<Vec<_>>();
    let message = IngressMessage {
        text: "hello".to_string(),
        attachments,
    };
    apply_event(
        state,
        Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id,
            profile_id: "bot".to_string(),
            chat_id: "chat".to_string(),
            user_id: "user".to_string(),
            sender_name: None,
            group_id: group_id.to_string(),
            platform_msg_id: format!("msg-{}", ingress_id.0),
            received_at_ms: 0,
            message,
            route_meta: None,
        }),
        next_id,
    );
    next_id += 1;
    apply_event(
        state,
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id: Id128(post_id.0.saturating_add(1000)),
            group_id: group_id.to_string(),
            ingress_ids: vec![ingress_id],
            is_anonymous: false,
            is_safe: true,
            draft: Draft {
                blocks: vec![DraftBlock::Paragraph {
                    text: "hello".to_string(),
                }],
            },
            created_at_ms: 0,
        }),
        next_id,
    );
    next_id += 1;
    apply_event(
        state,
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        }),
        next_id,
    );
    next_id += 1;
    if with_render {
        apply_event(
            state,
            Event::Render(RenderEvent::PngReady {
                post_id,
                blob_id: Id128(post_id.0.saturating_add(9000)),
            }),
            next_id,
        );
        next_id += 1;
    }
    next_id
}

#[test]
fn approve_stacks_until_threshold() {
    let mut config = CoreConfig::default();
    config.default_max_queue = 3;
    config.default_max_images_per_post = 0;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            max_queue: Some(3),
            max_images_per_post: Some(0),
            ..Default::default()
        },
    );

    let mut state = StateView::default();
    let post_id = Id128(1);
    let review_id = Id128(101);
    let review_code = 100;
    let ingress_id = Id128(11);
    seed_post(
        &mut state,
        post_id,
        review_id,
        review_code,
        ingress_id,
        "group-a",
        0,
        false,
        1,
    );

    let now_ms = 1_000;
    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        action: ReviewAction::Approve,
        operator_id: "admin".to_string(),
        now_ms,
        tz_offset_minutes: 0,
    });
    let out = oqqwall_rust_core::decide::decide(&state, &cmd, &config);

    let hold_ms = 365 * 24 * 60 * 60 * 1000;
    let not_before = out.iter().find_map(|event| match event {
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id: id,
            not_before_ms,
            ..
        }) if *id == post_id => Some(*not_before_ms),
        _ => None,
    });
    assert_eq!(not_before, Some(now_ms + hold_ms));
    assert!(!out.iter().any(|event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::SendPlanRescheduled { .. })
        )
    }));
}

#[test]
fn approve_flushes_when_queue_full() {
    let mut config = CoreConfig::default();
    config.default_max_queue = 2;
    config.default_max_images_per_post = 0;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            max_queue: Some(2),
            max_images_per_post: Some(0),
            ..Default::default()
        },
    );

    let mut state = StateView::default();
    let post_id = Id128(1);
    let review_id = Id128(101);
    let review_code = 100;
    let ingress_id = Id128(11);
    let mut next_id = 1;
    next_id = seed_post(
        &mut state,
        post_id,
        review_id,
        review_code,
        ingress_id,
        "group-a",
        0,
        false,
        next_id,
    );

    let existing_post = Id128(2);
    apply_event(
        &mut state,
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id: existing_post,
            group_id: "group-a".to_string(),
            not_before_ms: 9_999,
            priority: SendPriority::Normal,
            seq: 1,
        }),
        next_id,
    );

    let now_ms = 1_000;
    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        action: ReviewAction::Approve,
        operator_id: "admin".to_string(),
        now_ms,
        tz_offset_minutes: 0,
    });
    let out = oqqwall_rust_core::decide::decide(&state, &cmd, &config);

    let created_now = out.iter().any(|event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::SendPlanCreated {
                post_id: id,
                not_before_ms,
                ..
            }) if *id == post_id && *not_before_ms == now_ms
        )
    });
    assert!(created_now);

    let rescheduled = out.iter().any(|event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                post_id: id,
                not_before_ms,
                ..
            }) if *id == existing_post && *not_before_ms == now_ms
        )
    });
    assert!(rescheduled);
}

#[test]
fn approve_flushes_when_image_limit_exceeded() {
    let mut config = CoreConfig::default();
    config.default_max_queue = 5;
    config.default_max_images_per_post = 3;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            max_queue: Some(5),
            max_images_per_post: Some(3),
            ..Default::default()
        },
    );

    let mut state = StateView::default();
    let existing_post = Id128(2);
    let existing_review = Id128(201);
    let existing_review_code = 200;
    let existing_ingress = Id128(21);
    let mut next_id = 1;
    next_id = seed_post(
        &mut state,
        existing_post,
        existing_review,
        existing_review_code,
        existing_ingress,
        "group-a",
        1,
        true,
        next_id,
    );
    apply_event(
        &mut state,
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id: existing_post,
            group_id: "group-a".to_string(),
            not_before_ms: 9_999,
            priority: SendPriority::Normal,
            seq: 1,
        }),
        next_id,
    );
    next_id += 1;

    let post_id = Id128(1);
    let review_id = Id128(101);
    let review_code = 100;
    let ingress_id = Id128(11);
    seed_post(
        &mut state,
        post_id,
        review_id,
        review_code,
        ingress_id,
        "group-a",
        1,
        true,
        next_id,
    );

    let now_ms = 1_000;
    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: None,
        review_code: Some(review_code),
        audit_msg_id: None,
        action: ReviewAction::Approve,
        operator_id: "admin".to_string(),
        now_ms,
        tz_offset_minutes: 0,
    });
    let out = oqqwall_rust_core::decide::decide(&state, &cmd, &config);

    let created_now = out.iter().any(|event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::SendPlanCreated {
                post_id: id,
                not_before_ms,
                ..
            }) if *id == post_id && *not_before_ms == now_ms
        )
    });
    assert!(created_now);

    let rescheduled = out.iter().any(|event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                post_id: id,
                not_before_ms,
                ..
            }) if *id == existing_post && *not_before_ms == now_ms
        )
    });
    assert!(rescheduled);
}
