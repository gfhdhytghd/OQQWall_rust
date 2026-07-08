use oqqwall_rust_core::event::{DraftEvent, Event, IngressEvent, RenderEvent};
use oqqwall_rust_core::{
    BlockKindFilter, BlockSelector, Command, CoreConfig, Draft, DraftBlock, DraftTransform,
    EventEnvelope, Id128, IngressAttachment, IngressMessage, MediaKind, MediaReference,
    PositionSpec, PostAction, PostActionCommand, StateView,
};

fn wrap(event: Event, id: u128) -> EventEnvelope {
    EventEnvelope {
        id: Id128(id),
        ts_ms: id as i64,
        actor: Id128(0),
        correlation_id: None,
        event,
    }
}

#[test]
fn set_draft_transforms_rebuilds_and_requests_render() {
    let post_id = Id128(10);
    let session_id = Id128(11);
    let image_ingress = Id128(12);
    let text_ingress = Id128(13);
    let mut state = seeded_state(post_id, session_id, image_ingress, text_ingress);
    let transform = move_paragraph_front();

    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::PostAction(PostActionCommand {
            post_id,
            action: PostAction::SetDraftTransforms {
                transforms: vec![transform.clone()],
            },
            operator_id: "agent".to_string(),
            now_ms: 100,
        }),
        &CoreConfig::default(),
    );

    assert!(matches!(
        events.first(),
        Some(Event::Draft(DraftEvent::DraftTransformsSet { transforms, .. }))
            if transforms == &vec![transform.clone()]
    ));
    assert!(matches!(
        events.get(1),
        Some(Event::Draft(DraftEvent::PostDraftCreated { draft, .. }))
            if draft_labels(draft) == vec!["p:caption", "img"]
    ));
    assert!(matches!(
        events.get(2),
        Some(Event::Render(RenderEvent::RenderRequested { post_id: id, .. })) if *id == post_id
    ));

    for (idx, event) in events.into_iter().enumerate() {
        state = state.reduce(&wrap(event, 100 + idx as u128));
    }
    assert_eq!(state.draft_transforms.get(&post_id), Some(&vec![transform]));
    assert_eq!(
        draft_labels(state.drafts.get(&post_id).unwrap()),
        vec!["p:caption", "img"]
    );
}

#[test]
fn set_draft_transforms_is_ignored_for_terminal_posts() {
    let post_id = Id128(20);
    let session_id = Id128(21);
    let image_ingress = Id128(22);
    let text_ingress = Id128(23);
    let mut state = seeded_state(post_id, session_id, image_ingress, text_ingress);
    state.update_post_stage(post_id, oqqwall_rust_core::state::PostStage::Sent);

    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::PostAction(PostActionCommand {
            post_id,
            action: PostAction::SetDraftTransforms {
                transforms: vec![move_paragraph_front()],
            },
            operator_id: "agent".to_string(),
            now_ms: 100,
        }),
        &CoreConfig::default(),
    );

    assert!(events.is_empty());
}

fn seeded_state(
    post_id: Id128,
    session_id: Id128,
    image_ingress: Id128,
    text_ingress: Id128,
) -> StateView {
    let mut state = StateView::default();
    state = state.reduce(&wrap(
        Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id: image_ingress,
            profile_id: "bot".to_string(),
            chat_id: "chat".to_string(),
            user_id: "user".to_string(),
            sender_name: None,
            group_id: "group-a".to_string(),
            platform_msg_id: "img".to_string(),
            route_meta: None,
            received_at_ms: 1,
            message: IngressMessage {
                text: String::new(),
                attachments: vec![IngressAttachment {
                    kind: MediaKind::Image,
                    name: None,
                    reference: MediaReference::Blob { blob_id: Id128(30) },
                    size_bytes: None,
                }],
            },
        }),
        1,
    ));
    state = state.reduce(&wrap(
        Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id: text_ingress,
            profile_id: "bot".to_string(),
            chat_id: "chat".to_string(),
            user_id: "user".to_string(),
            sender_name: None,
            group_id: "group-a".to_string(),
            platform_msg_id: "txt".to_string(),
            route_meta: None,
            received_at_ms: 2,
            message: IngressMessage {
                text: "caption".to_string(),
                attachments: Vec::new(),
            },
        }),
        2,
    ));
    state.reduce(&wrap(
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: "group-a".to_string(),
            ingress_ids: vec![image_ingress, text_ingress],
            is_anonymous: false,
            is_safe: true,
            draft: Draft {
                blocks: vec![
                    DraftBlock::Attachment {
                        kind: MediaKind::Image,
                        name: None,
                        reference: MediaReference::Blob { blob_id: Id128(30) },
                        size_bytes: None,
                    },
                    DraftBlock::Paragraph {
                        text: "caption".to_string(),
                    },
                ],
            },
            created_at_ms: 3,
        }),
        3,
    ))
}

fn move_paragraph_front() -> DraftTransform {
    DraftTransform::MoveBlocks {
        selector: BlockSelector {
            kinds: Some(vec![BlockKindFilter::Paragraph]),
            text: None,
            index: None,
        },
        position: PositionSpec::Front,
    }
}

fn draft_labels(draft: &Draft) -> Vec<String> {
    draft
        .blocks
        .iter()
        .map(|block| match block {
            DraftBlock::Paragraph { text } => format!("p:{}", text),
            DraftBlock::Attachment { kind, .. } if *kind == MediaKind::Image => "img".to_string(),
            DraftBlock::Attachment { .. } => "attachment".to_string(),
            DraftBlock::Reply { .. } => "reply".to_string(),
            DraftBlock::Poke => "poke".to_string(),
            DraftBlock::JsonCard { .. } => "json".to_string(),
            DraftBlock::Forward { .. } => "forward".to_string(),
        })
        .collect()
}
