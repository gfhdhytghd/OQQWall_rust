use crate::config::CoreConfig;
use crate::decide::builder::build_draft_from_messages;
use crate::event::{
    DraftEvent, Event, IngressEvent, ManualEvent, RenderEvent, ReviewDecision, ReviewEvent,
    ScheduleEvent, SendEvent, SendPriority,
};
use crate::ids::{PostId, ReviewCode, ReviewId, derive_review_id};
use crate::safety::detect_safe;
use crate::state::StateView;
use crate::{anonymous::detect_anonymous, state::PostMeta};

pub fn decide_driver_event(state: &StateView, event: &Event, config: &CoreConfig) -> Vec<Event> {
    // Driver events come from IO drivers. They must flow into the event stream
    // (reduce/broadcast), otherwise other drivers cannot observe key events like
    // BlobPersisted / RenderReady / MediaFetchSucceeded / SendSucceeded.
    // Pass through the original event and append any derived events as needed.
    let mut out = vec![event.clone()];

    let derived = match event {
        Event::Render(RenderEvent::PngReady { post_id, .. }) => {
            if !state.posts.contains_key(post_id) {
                return out;
            }
            let review_id = derive_review_id(&[&post_id.to_be_bytes()]);
            let review_meta = state.reviews.get(&review_id);
            let needs_republish = review_meta
                .map(|meta| meta.needs_republish)
                .unwrap_or(false);
            let mut events = Vec::new();
            let review_code = review_meta
                .map(|meta| meta.review_code)
                .unwrap_or(state.next_review_code);
            if review_meta.is_none() {
                events.push(Event::Review(ReviewEvent::ReviewItemCreated {
                    review_id,
                    post_id: *post_id,
                    review_code,
                }));
            }
            let already_published = review_meta
                .and_then(|meta| meta.audit_msg_id.as_ref())
                .is_some();
            if needs_republish || !already_published {
                events.push(Event::Review(ReviewEvent::ReviewInfoSynced {
                    review_id,
                    post_id: *post_id,
                    review_code,
                }));
                events.push(Event::Review(ReviewEvent::ReviewPublishRequested {
                    review_id,
                }));
            }
            events
        }
        Event::Send(SendEvent::SendFailed {
            post_id,
            retry_at_ms,
            attempt,
            error,
            ..
        }) => {
            let group_id = state
                .sending
                .get(post_id)
                .map(|sending| sending.group_id.clone())
                .or_else(|| state.posts.get(post_id).map(|meta| meta.group_id.clone()))
                .unwrap_or_default();
            let decided_at_ms = state.last_ts_ms.unwrap_or(0);
            if is_send_timeout_error(error) {
                return_to_pending(state, *post_id, decided_at_ms)
            } else {
                let max_attempts = config.send_max_attempts(&group_id);
                if max_attempts > 0 && *attempt >= max_attempts {
                    let reason = format!("send failed after {} attempts: {}", attempt, error);
                    let mut events = vec![
                        Event::Send(SendEvent::SendGaveUp {
                            post_id: *post_id,
                            reason: reason.clone(),
                        }),
                        Event::Manual(ManualEvent::ManualInterventionRequired {
                            post_id: *post_id,
                            reason,
                        }),
                    ];
                    events.extend(return_to_pending(state, *post_id, decided_at_ms));
                    events
                } else {
                    vec![Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                        post_id: *post_id,
                        group_id,
                        not_before_ms: *retry_at_ms,
                        priority: SendPriority::Normal,
                        seq: state.next_send_seq,
                    })]
                }
            }
        }
        Event::Send(SendEvent::SendGaveUp { post_id, .. }) => {
            let decided_at_ms = state.last_ts_ms.unwrap_or(0);
            return_to_pending(state, *post_id, decided_at_ms)
        }
        Event::Ingress(IngressEvent::MessageRecalled {
            ingress_id,
            recalled_at_ms,
        }) => derive_recall_events(state, *ingress_id, *recalled_at_ms),
        _ => Vec::new(),
    };

    out.extend(derived);
    out
}

fn is_send_timeout_error(error: &str) -> bool {
    error.starts_with("send timeout")
}

fn return_to_pending(state: &StateView, post_id: PostId, decided_at_ms: i64) -> Vec<Event> {
    let Some((review_id, review_code)) = resolve_review_meta(state, post_id) else {
        return Vec::new();
    };
    vec![
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision: ReviewDecision::Deferred,
            decided_by: "system".to_string(),
            decided_at_ms,
        }),
        Event::Review(ReviewEvent::ReviewInfoSynced {
            review_id,
            post_id,
            review_code,
        }),
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id }),
    ]
}

fn resolve_review_meta(state: &StateView, post_id: PostId) -> Option<(ReviewId, ReviewCode)> {
    let review_id = state
        .posts
        .get(&post_id)
        .and_then(|meta| meta.review_id)
        .or_else(|| {
            let derived = derive_review_id(&[&post_id.to_be_bytes()]);
            state.reviews.contains_key(&derived).then_some(derived)
        })?;
    let review = state.reviews.get(&review_id)?;
    Some((review_id, review.review_code))
}

fn derive_recall_events(
    state: &StateView,
    ingress_id: crate::ids::IngressId,
    now_ms: i64,
) -> Vec<Event> {
    let affected_posts = state
        .post_ingress
        .iter()
        .filter_map(|(post_id, ingress_ids)| ingress_ids.contains(&ingress_id).then_some(*post_id))
        .collect::<Vec<_>>();
    if affected_posts.is_empty() {
        return Vec::new();
    }

    let mut events = Vec::new();
    for post_id in affected_posts {
        let Some(post_meta) = state.posts.get(&post_id) else {
            continue;
        };
        let Some(source_ingress) = state.post_ingress.get(&post_id) else {
            continue;
        };
        let remaining_ingress = source_ingress
            .iter()
            .copied()
            .filter(|id| *id != ingress_id)
            .collect::<Vec<_>>();
        if remaining_ingress.is_empty() {
            if let Some(review_id) = post_meta.review_id {
                events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
                    review_id,
                    decision: ReviewDecision::Deleted,
                    decided_by: "system_recall".to_string(),
                    decided_at_ms: now_ms,
                }));
            }
            if state.send_plans.contains_key(&post_id) {
                events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
            }
            continue;
        }

        let draft_event =
            rebuild_draft_after_recall(state, post_meta, post_id, remaining_ingress, now_ms);
        events.push(Event::Draft(draft_event));
        if let Some(review_id) = post_meta.review_id {
            events.push(Event::Review(ReviewEvent::ReviewRefreshRequested {
                review_id,
            }));
        }
        events.push(Event::Render(RenderEvent::RenderRequested {
            post_id,
            attempt: 1,
            requested_at_ms: now_ms,
        }));
    }

    events
}

fn rebuild_draft_after_recall(
    state: &StateView,
    post_meta: &PostMeta,
    post_id: PostId,
    ingress_ids: Vec<crate::ids::IngressId>,
    now_ms: i64,
) -> DraftEvent {
    let mut messages = Vec::new();
    for ingress_id in &ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    let draft = build_draft_from_messages(&messages);
    let is_anonymous = detect_anonymous(&messages);
    let is_safe = detect_safe(&messages);

    DraftEvent::PostDraftCreated {
        post_id,
        session_id: post_meta.session_id,
        group_id: post_meta.group_id.clone(),
        ingress_ids,
        is_anonymous,
        is_safe,
        draft,
        created_at_ms: now_ms,
    }
}
