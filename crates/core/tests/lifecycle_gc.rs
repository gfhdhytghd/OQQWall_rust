use std::collections::HashSet;

use oqqwall_rust_core::event::{BlobEvent, Event, LifecycleEvent, ReviewEvent, SendPriority};
use oqqwall_rust_core::state::{
    BlobMeta, IngressMeta, PostMeta, PostStage, RenderMeta, SendDueKey, SendPlan,
};
use oqqwall_rust_core::{
    Command, CoreConfig, Draft, DraftBlock, EventEnvelope, Id128, IngressMessage, MediaKind,
    MediaReference, StateView, TickCommand,
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
fn post_eviction_removes_post_state_but_keeps_shared_ingress() {
    let post_a = Id128(10);
    let post_b = Id128(11);
    let shared_ingress = Id128(20);
    let unique_ingress = Id128(21);
    let render_blob = Id128(30);

    let mut state = StateView::default();
    seed_ingress(&mut state, shared_ingress, "shared");
    seed_ingress(&mut state, unique_ingress, "unique");
    seed_post(&mut state, post_a, vec![shared_ingress, unique_ingress], 1);
    seed_post(&mut state, post_b, vec![shared_ingress], 2);
    state.render.insert(
        post_a,
        RenderMeta {
            png_blob: Some(render_blob),
            png_blobs: vec![render_blob],
            last_error: None,
            last_attempt: 1,
            retry_at_ms: None,
        },
    );
    state.external_code_by_post.insert(post_a, 42);
    state.send_plans.insert(
        post_a,
        SendPlan {
            post_id: post_a,
            group_id: "group-a".to_string(),
            not_before_ms: 50,
            priority: SendPriority::Normal,
            seq: 7,
        },
    );
    state.send_due.insert(SendDueKey {
        not_before_ms: 50,
        priority: SendPriority::Normal,
        seq: 7,
        post_id: post_a,
    });
    state = state.reduce(&wrap(
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id: Id128(40),
            post_id: post_a,
            review_code: 3,
        }),
        100,
    ));

    let next = state.reduce(&wrap(
        Event::Lifecycle(LifecycleEvent::PostEvicted {
            post_id: post_a,
            evicted_at_ms: 1_000,
            blob_ids: vec![render_blob],
            ingress_ids: vec![shared_ingress, unique_ingress],
        }),
        101,
    ));

    assert!(!next.posts.contains_key(&post_a));
    assert!(!next.drafts.contains_key(&post_a));
    assert!(!next.render.contains_key(&post_a));
    assert!(!next.external_code_by_post.contains_key(&post_a));
    assert!(!next.send_plans.contains_key(&post_a));
    assert!(!next.send_due.iter().any(|key| key.post_id == post_a));
    assert!(next.review_by_code.get(&3).is_none());
    assert!(next.ingress_messages.contains_key(&shared_ingress));
    assert!(!next.ingress_messages.contains_key(&unique_ingress));
    assert!(next.ingress_seen.contains(&shared_ingress));
    assert!(next.ingress_seen.contains(&unique_ingress));
    assert!(next.posts.contains_key(&post_b));
}

#[test]
fn tick_requests_gc_for_unreferenced_blobs_only() {
    let mut state = StateView::default();
    state.blobs.insert(Id128(1), blob_meta(Id128(1)));
    state.blobs.insert(Id128(2), blob_meta(Id128(2)));
    state.render.insert(
        Id128(10),
        RenderMeta {
            png_blob: Some(Id128(1)),
            png_blobs: vec![Id128(1)],
            last_error: None,
            last_attempt: 1,
            retry_at_ms: None,
        },
    );

    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: 1_000,
            tz_offset_minutes: 0,
        }),
        &CoreConfig::default(),
    );

    assert!(events.iter().any(|event| {
        matches!(
            event,
            Event::Blob(BlobEvent::BlobGcRequested { blob_id }) if *blob_id == Id128(2)
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            Event::Blob(BlobEvent::BlobGcRequested { blob_id }) if *blob_id == Id128(1)
        )
    }));
}

#[test]
fn tick_evicts_old_terminal_posts() {
    let post_id = Id128(10);
    let mut state = StateView::default();
    seed_post(&mut state, post_id, Vec::new(), 0);
    state.update_post_stage(post_id, PostStage::Sent);

    let config = CoreConfig {
        eviction_retention_ms: 100,
        ..CoreConfig::default()
    };
    let events = oqqwall_rust_core::decide::decide(
        &state,
        &Command::Tick(TickCommand {
            now_ms: 101,
            tz_offset_minutes: 0,
        }),
        &config,
    );

    assert!(matches!(
        events.first(),
        Some(Event::Lifecycle(LifecycleEvent::PostEvicted { post_id: id, .. })) if *id == post_id
    ));
}

fn seed_ingress(state: &mut StateView, ingress_id: Id128, text: &str) {
    state.ingress_seen.insert(ingress_id);
    state.ingress_meta.insert(
        ingress_id,
        IngressMeta {
            profile_id: "bot".to_string(),
            chat_id: "chat".to_string(),
            user_id: "user".to_string(),
            sender_name: None,
            group_id: "group-a".to_string(),
            platform_msg_id: format!("msg-{}", ingress_id.0),
            route_meta: None,
            received_at_ms: 1,
        },
    );
    state.ingress_messages.insert(
        ingress_id,
        IngressMessage {
            text: text.to_string(),
            attachments: Vec::new(),
        },
    );
}

fn seed_post(state: &mut StateView, post_id: Id128, ingress_ids: Vec<Id128>, created_at_ms: i64) {
    state.posts.insert(
        post_id,
        PostMeta {
            post_id,
            session_id: post_id,
            group_id: "group-a".to_string(),
            stage: PostStage::Drafted,
            review_id: None,
            created_at_ms,
            is_anonymous: false,
            is_safe: true,
            last_error: None,
        },
    );
    state
        .posts_by_stage
        .entry(PostStage::Drafted)
        .or_insert_with(HashSet::new)
        .insert(post_id);
    state.post_ingress.insert(post_id, ingress_ids);
    state.drafts.insert(
        post_id,
        Draft {
            blocks: vec![DraftBlock::Attachment {
                kind: MediaKind::Image,
                name: None,
                reference: MediaReference::Blob { blob_id: Id128(99) },
                size_bytes: None,
            }],
        },
    );
}

fn blob_meta(blob_id: Id128) -> BlobMeta {
    BlobMeta {
        blob_id,
        size_bytes: 1,
        persisted_path: Some(format!("/tmp/{}.bin", blob_id.0)),
        ref_count: 1,
    }
}
