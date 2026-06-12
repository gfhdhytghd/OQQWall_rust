use oqqwall_rust_core::event::{DraftEvent, Event, IngressEvent, RenderEvent, SessionEvent};
use oqqwall_rust_core::{
    Draft, EventEnvelope, Id128, IngressAttachment, IngressMessage, StateView,
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

fn ingress_event(ingress_id: Id128, msg_id: &str) -> Event {
    Event::Ingress(IngressEvent::MessageAccepted {
        ingress_id,
        profile_id: "bot".to_string(),
        chat_id: "u1".to_string(),
        user_id: "u1".to_string(),
        sender_name: Some("sender".to_string()),
        group_id: "group-a".to_string(),
        platform_msg_id: msg_id.to_string(),
        received_at_ms: 1,
        message: IngressMessage {
            text: format!("text-{}", msg_id),
            attachments: Vec::<IngressAttachment>::new(),
        },
    })
}

#[test]
fn message_recalled_prunes_session_and_post_links() {
    let ingress_id = Id128(10);
    let session_id = Id128(11);
    let post_id = Id128(12);

    let mut state = StateView::default();
    state = state.reduce(&wrap(ingress_event(ingress_id, "m1"), 1, 1));
    state = state.reduce(&wrap(
        Event::Session(SessionEvent::Opened {
            session_id,
            first_ingress_id: ingress_id,
            chat_id: "u1".to_string(),
            user_id: "u1".to_string(),
            group_id: "group-a".to_string(),
            close_at_ms: 99,
        }),
        2,
        2,
    ));
    state = state.reduce(&wrap(
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            session_id,
            group_id: "group-a".to_string(),
            ingress_ids: vec![ingress_id],
            is_anonymous: false,
            is_safe: true,
            draft: Draft { blocks: Vec::new() },
            created_at_ms: 3,
        }),
        3,
        3,
    ));

    state = state.reduce(&wrap(
        Event::Ingress(IngressEvent::MessageRecalled {
            ingress_id,
            recalled_at_ms: 10,
        }),
        4,
        4,
    ));

    assert!(!state.ingress_messages.contains_key(&ingress_id));
    assert!(match state.session_ingress.get(&session_id) {
        Some(ingress) => ingress.is_empty(),
        None => true,
    });
    assert!(!state.sessions.contains_key(&session_id));
    assert!(
        state
            .post_ingress
            .get(&post_id)
            .is_some_and(|ingress| ingress.is_empty())
    );
}

#[test]
fn render_requested_clears_existing_png_blob() {
    let post_id = Id128(20);
    let blob_id = Id128(21);
    let second_blob_id = Id128(22);

    let mut state = StateView::default();
    state = state.reduce(&wrap(
        Event::Render(RenderEvent::PngReady { post_id, blob_id }),
        1,
        1,
    ));
    state = state.reduce(&wrap(
        Event::Render(RenderEvent::PngReady {
            post_id,
            blob_id: second_blob_id,
        }),
        2,
        2,
    ));
    assert_eq!(
        state.render.get(&post_id).and_then(|meta| meta.png_blob),
        Some(blob_id)
    );
    assert_eq!(
        state
            .render
            .get(&post_id)
            .map(|meta| meta.png_blobs.as_slice()),
        Some([blob_id, second_blob_id].as_slice())
    );

    state = state.reduce(&wrap(
        Event::Render(RenderEvent::RenderRequested {
            post_id,
            attempt: 2,
            requested_at_ms: 2,
        }),
        3,
        3,
    ));

    assert_eq!(
        state.render.get(&post_id).and_then(|meta| meta.png_blob),
        None
    );
    assert_eq!(
        state
            .render
            .get(&post_id)
            .map(|meta| meta.png_blobs.as_slice()),
        Some([].as_slice())
    );
}

#[test]
fn png_batch_ready_records_all_page_blobs() {
    let post_id = Id128(30);
    let blob_ids = vec![Id128(31), Id128(32), Id128(33)];

    let state = StateView::default().reduce(&wrap(
        Event::Render(RenderEvent::PngBatchReady {
            post_id,
            blob_ids: blob_ids.clone(),
        }),
        1,
        1,
    ));

    assert_eq!(
        state.render.get(&post_id).and_then(|meta| meta.png_blob),
        Some(blob_ids[0])
    );
    assert_eq!(
        state
            .render
            .get(&post_id)
            .map(|meta| meta.png_blobs.as_slice()),
        Some(blob_ids.as_slice())
    );
}
