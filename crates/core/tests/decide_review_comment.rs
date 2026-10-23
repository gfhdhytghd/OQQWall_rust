use oqqwall_rust_core::decide::{builder::build_draft_from_messages, decide};
use oqqwall_rust_core::event::{DraftEvent, Event, IngressEvent, RenderEvent, ReviewEvent};
use oqqwall_rust_core::{
    Command, CoreConfig, DraftBlock, EventEnvelope, Id128, IngressMessage, ReviewAction,
    ReviewActionCommand, StateView, derive_review_id,
};

fn apply(state: &mut StateView, events: &[Event]) {
    for event in events {
        *state = state.reduce(&EventEnvelope {
            id: Id128(state.last_event_id.map_or(1, |id| id.0 + 1)),
            ts_ms: 1_000,
            actor: Id128(0),
            correlation_id: None,
            event: event.clone(),
        });
    }
}

fn seed() -> StateView {
    let message = IngressMessage {
        text: "投稿正文".into(),
        attachments: vec![],
    };
    let mut state = StateView::default();
    apply(
        &mut state,
        &[
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id: Id128(1),
                profile_id: "bot".into(),
                chat_id: "chat".into(),
                user_id: "user".into(),
                sender_name: None,
                group_id: "group".into(),
                platform_msg_id: "msg".into(),
                received_at_ms: 1_000,
                message: message.clone(),
                route_meta: None,
            }),
            Event::Draft(DraftEvent::PostDraftCreated {
                post_id: Id128(10),
                session_id: Id128(11),
                group_id: "group".into(),
                ingress_ids: vec![Id128(1)],
                is_anonymous: true,
                is_safe: false,
                draft: build_draft_from_messages(&[message]),
                created_at_ms: 1_000,
            }),
            Event::Review(ReviewEvent::ReviewItemCreated {
                review_id: review_id(),
                post_id: Id128(10),
                review_code: 123,
            }),
            Event::Review(ReviewEvent::ReviewPublished {
                review_id: review_id(),
                audit_msg_id: "audit".into(),
            }),
        ],
    );
    state
}

fn review_id() -> Id128 {
    derive_review_id(&[&Id128(10).to_be_bytes()])
}

fn command(action: ReviewAction) -> Command {
    Command::ReviewAction(ReviewActionCommand {
        review_id: None,
        review_code: Some(123),
        audit_msg_id: None,
        action,
        operator_id: "admin".into(),
        now_ms: 2_000,
        tz_offset_minutes: 0,
    })
}

#[test]
fn comment_survives_refresh_replay_and_republishes_after_render() {
    let initial = seed();
    let config = CoreConfig::default();
    let mut state = initial.clone();
    let mut journal = vec![];
    for text in [" 已核实 多词\n第二行 ", "补充说明"] {
        let events = decide(
            &state,
            &command(ReviewAction::Comment { text: text.into() }),
            &config,
        );
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Render(RenderEvent::RenderRequested {
                post_id: Id128(10),
                ..
            })
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Draft(DraftEvent::PostDraftCreated {
                created_at_ms: 1_000,
                ..
            })
        )));
        apply(&mut state, &events);
        journal.extend(events);
    }
    assert!(state.reviews[&review_id()].needs_republish);
    assert_eq!(state.reviews[&review_id()].decision, None);
    assert!(state.posts[&Id128(10)].is_anonymous);
    assert!(!state.posts[&Id128(10)].is_safe);
    let expected = state.drafts[&Id128(10)].clone();
    assert_eq!(
        expected.blocks,
        vec![
            DraftBlock::Paragraph {
                text: "投稿正文".into()
            },
            DraftBlock::Comment {
                text: "已核实 多词\n第二行".into()
            },
            DraftBlock::Comment {
                text: "补充说明".into()
            },
        ]
    );
    for action in [
        ReviewAction::Refresh,
        ReviewAction::SelectAllMessages,
        ReviewAction::Rerender,
    ] {
        let events = decide(&state, &command(action), &config);
        apply(&mut state, &events);
        journal.extend(events);
        assert_eq!(state.drafts[&Id128(10)], expected);
    }
    let ready = Event::Render(RenderEvent::PngReady {
        post_id: Id128(10),
        blob_id: Id128(99),
    });
    let events = decide(&state, &Command::DriverEvent(ready), &config);
    assert!(
        events.contains(&Event::Review(ReviewEvent::ReviewPublishRequested {
            review_id: review_id()
        }))
    );
    apply(&mut state, &events);
    journal.extend(events);
    assert!(!state.reviews[&review_id()].needs_republish);
    let mut replayed = initial;
    apply(&mut replayed, &journal);
    assert_eq!(replayed, state);
    assert_eq!(state.ingress_messages[&Id128(1)].text, "投稿正文");
}

#[test]
fn empty_comment_does_not_change_or_render_post() {
    let state = seed();
    let events = decide(
        &state,
        &command(ReviewAction::Comment {
            text: " \n ".into(),
        }),
        &CoreConfig::default(),
    );
    assert!(events.is_empty());
}

#[test]
fn commented_post_can_still_be_approved_and_scheduled() {
    use oqqwall_rust_core::GroupConfig;
    use oqqwall_rust_core::event::{ReviewDecision, ScheduleEvent};
    let mut config = CoreConfig::default();
    config.groups.insert(
        "group".into(),
        GroupConfig {
            group_id: "group".into(),
            ..Default::default()
        },
    );
    let mut state = seed();
    let events = decide(
        &state,
        &command(ReviewAction::Comment {
            text: "已核实".into(),
        }),
        &config,
    );
    apply(&mut state, &events);
    let events = decide(
        &state,
        &Command::DriverEvent(Event::Render(RenderEvent::PngReady {
            post_id: Id128(10),
            blob_id: Id128(99),
        })),
        &config,
    );
    apply(&mut state, &events);
    let events = decide(&state, &command(ReviewAction::Approve), &config);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id: Id128(10),
            ..
        })
    )));
    apply(&mut state, &events);
    assert_eq!(
        state.reviews[&review_id()].decision,
        Some(ReviewDecision::Approved)
    );
    assert!(state.send_plans.contains_key(&Id128(10)));
    assert!(
        state.drafts[&Id128(10)]
            .blocks
            .iter()
            .any(|block| matches!(block, DraftBlock::Comment { text } if text == "已核实"))
    );
}
