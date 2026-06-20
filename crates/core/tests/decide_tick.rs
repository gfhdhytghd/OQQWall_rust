use oqqwall_rust_core::decide::decide;
use oqqwall_rust_core::event::{
    DraftEvent, GroupFlushReason, MediaEvent, RenderEvent, ReviewEvent, ScheduleEvent,
    SendPriority, SessionEvent,
};
use oqqwall_rust_core::state::{PostMeta, PostStage, RenderMeta, ReviewMeta};
use oqqwall_rust_core::{
    Command, CoreConfig, Event, EventEnvelope, GroupConfig, Id128, IngressAttachment,
    IngressCommand, IngressMessage, MediaFetchKey, MediaFetchMeta, MediaKind, MediaReference,
    StateView, TickCommand,
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
fn tick_closes_session_and_creates_draft() {
    let mut config = CoreConfig::default();
    config.default_process_waittime_ms = 1_000;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            ..Default::default()
        },
    );

    let ingress = IngressCommand {
        profile_id: "bot".to_string(),
        chat_id: "chat".to_string(),
        user_id: "user".to_string(),
        sender_name: None,
        group_id: "group-a".to_string(),
        platform_msg_id: "msg-1".to_string(),
        message: IngressMessage {
            text: "hello".to_string(),
            attachments: vec![IngressAttachment {
                kind: MediaKind::Image,
                name: None,
                reference: MediaReference::RemoteUrl {
                    url: "http://example.com/img.png".to_string(),
                },
                size_bytes: None,
            }],
        },
        received_at_ms: 1_000,
        route_meta: None,
        close_immediately: false,
    };

    let mut state = StateView::default();
    let events = decide(&state, &Command::Ingress(ingress), &config);
    for (idx, event) in events.into_iter().enumerate() {
        state = state.reduce(&wrap(event, idx as u128 + 1));
    }

    let tick = TickCommand {
        now_ms: 3_000,
        tz_offset_minutes: 0,
    };
    let tick_events = decide(&state, &Command::Tick(tick), &config);
    assert_eq!(tick_events.len(), 3);

    let mut saw_close = false;
    let mut saw_draft = false;
    let mut saw_render = false;

    for event in tick_events {
        match event {
            Event::Session(SessionEvent::Closed { .. }) => saw_close = true,
            Event::Draft(DraftEvent::PostDraftCreated {
                group_id,
                ingress_ids,
                ..
            }) => {
                assert_eq!(group_id, "group-a");
                assert_eq!(ingress_ids.len(), 1);
                saw_draft = true;
            }
            Event::Render(RenderEvent::RenderRequested { .. }) => {
                saw_render = true;
            }
            _ => {}
        }
    }

    assert!(saw_close);
    assert!(saw_draft);
    assert!(saw_render);
}

#[test]
fn tick_closes_explicit_immediate_session_without_waiting() {
    let mut config = CoreConfig::default();
    config.default_process_waittime_ms = 60_000;
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            ..Default::default()
        },
    );

    let ingress = IngressCommand {
        profile_id: "bot".to_string(),
        chat_id: "chat".to_string(),
        user_id: "user".to_string(),
        sender_name: None,
        group_id: "group-a".to_string(),
        platform_msg_id: "msg-1".to_string(),
        message: IngressMessage {
            text: "confirmed submission".to_string(),
            attachments: Vec::new(),
        },
        received_at_ms: 1_000,
        route_meta: None,
        close_immediately: true,
    };

    let mut state = StateView::default();
    let events = decide(&state, &Command::Ingress(ingress), &config);
    for (idx, event) in events.into_iter().enumerate() {
        state = state.reduce(&wrap(event, idx as u128 + 1));
    }

    let tick_events = decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: 1_000,
            tz_offset_minutes: 0,
        }),
        &config,
    );

    assert!(
        tick_events
            .iter()
            .any(|event| matches!(event, Event::Session(SessionEvent::Closed { .. })))
    );
    assert!(
        tick_events
            .iter()
            .any(|event| matches!(event, Event::Draft(DraftEvent::PostDraftCreated { .. })))
    );
}

#[test]
fn tick_group_flush_is_idempotent_for_same_minute() {
    let mut config = CoreConfig::default();
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            send_schedule_minutes: vec![0],
            ..Default::default()
        },
    );

    let state = StateView::default();
    let tick = TickCommand {
        now_ms: 0,
        tz_offset_minutes: 0,
    };

    let events = decide(&state, &Command::Tick(tick.clone()), &config);
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::Schedule(ScheduleEvent::GroupFlushRequested {
            group_id,
            minute_of_day,
            day_index,
            reason,
        }) => {
            assert_eq!(group_id, "group-a");
            assert_eq!(*minute_of_day, 0);
            assert_eq!(*day_index, 0);
            assert_eq!(*reason, GroupFlushReason::Scheduled);
        }
        _ => panic!("expected GroupFlushRequested"),
    }

    let mut reduced = state;
    for (idx, event) in events.into_iter().enumerate() {
        reduced = reduced.reduce(&wrap(event, idx as u128 + 1));
    }

    let events_again = decide(&reduced, &Command::Tick(tick), &config);
    assert!(events_again.is_empty());
}

#[test]
fn tick_group_flush_reschedules_send_plans() {
    let mut config = CoreConfig::default();
    config.groups.insert(
        "group-a".to_string(),
        GroupConfig {
            group_id: "group-a".to_string(),
            send_schedule_minutes: vec![0],
            ..Default::default()
        },
    );

    let post_id = Id128(90);
    let mut state = StateView::default();
    state = state.reduce(&wrap(
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id,
            group_id: "group-a".to_string(),
            not_before_ms: 10_000,
            priority: SendPriority::Normal,
            seq: 3,
        }),
        1,
    ));

    let tick = TickCommand {
        now_ms: 0,
        tz_offset_minutes: 0,
    };
    let events = decide(&state, &Command::Tick(tick), &config);

    let mut saw_flush = false;
    let mut saw_reschedule = false;
    for event in events {
        match event {
            Event::Schedule(ScheduleEvent::GroupFlushRequested { group_id, .. }) => {
                assert_eq!(group_id, "group-a");
                saw_flush = true;
            }
            Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                post_id: event_post_id,
                group_id,
                not_before_ms,
                seq,
                ..
            }) => {
                assert_eq!(event_post_id, post_id);
                assert_eq!(group_id, "group-a");
                assert_eq!(not_before_ms, 0);
                assert_eq!(seq, 3);
                saw_reschedule = true;
            }
            _ => {}
        }
    }

    assert!(saw_flush);
    assert!(saw_reschedule);
}

#[test]
fn tick_retries_review_publish_after_failure() {
    let review_id = Id128(200);
    let post_id = Id128(201);
    let mut state = StateView::default();
    state.reviews.insert(
        review_id,
        ReviewMeta {
            review_id,
            post_id,
            review_code: 42,
            decision: None,
            audit_msg_id: None,
            delayed_until_ms: None,
            needs_republish: false,
            decided_by: None,
            decided_at_ms: None,
            publish_retry_at_ms: Some(900),
            publish_last_error: Some("send failed".to_string()),
            publish_attempt: 2,
        },
    );

    let tick = TickCommand {
        now_ms: 1_000,
        tz_offset_minutes: 0,
    };
    let events = decide(&state, &Command::Tick(tick), &CoreConfig::default());

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id: rid })
            if *rid == review_id
    )));
}

#[test]
fn tick_retries_render_after_failure() {
    let post_id = Id128(300);
    let mut state = StateView::default();
    state.posts.insert(
        post_id,
        PostMeta {
            post_id,
            session_id: Id128(1),
            group_id: "group-a".to_string(),
            stage: PostStage::Drafted,
            review_id: None,
            created_at_ms: 0,
            is_anonymous: false,
            is_safe: true,
            last_error: None,
        },
    );
    state.update_post_stage(post_id, PostStage::Failed);
    state.render.insert(
        post_id,
        RenderMeta {
            png_blob: None,
            last_error: Some("render failed".to_string()),
            last_attempt: 1,
            retry_at_ms: Some(900),
        },
    );

    let tick = TickCommand {
        now_ms: 1_000,
        tz_offset_minutes: 0,
    };
    let events = decide(&state, &Command::Tick(tick), &CoreConfig::default());

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Render(RenderEvent::RenderRequested {
            post_id: pid,
            attempt,
            requested_at_ms,
        }) if *pid == post_id && *attempt == 2 && *requested_at_ms == 1_000
    )));
}

#[test]
fn tick_retries_media_fetch_after_failure() {
    let ingress_id = Id128(400);
    let mut state = StateView::default();
    state.ingress_messages.insert(
        ingress_id,
        IngressMessage {
            text: "hi".to_string(),
            attachments: vec![IngressAttachment {
                kind: MediaKind::Image,
                name: None,
                reference: MediaReference::RemoteUrl {
                    url: "http://example.com/img.png".to_string(),
                },
                size_bytes: None,
            }],
        },
    );
    state.media_fetch.insert(
        MediaFetchKey {
            ingress_id,
            attachment_index: 0,
        },
        MediaFetchMeta {
            attempt: 2,
            retry_at_ms: Some(900),
            last_error: Some("download failed".to_string()),
        },
    );

    let tick = TickCommand {
        now_ms: 1_000,
        tz_offset_minutes: 0,
    };
    let events = decide(&state, &Command::Tick(tick), &CoreConfig::default());

    assert!(events.iter().any(|event| matches!(
        event,
        Event::Media(MediaEvent::MediaFetchRequested {
            ingress_id: id,
            attachment_index,
            attempt,
        }) if *id == ingress_id && *attachment_index == 0 && *attempt == 3
    )));
}
