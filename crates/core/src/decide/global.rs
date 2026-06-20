use crate::command::{GlobalAction, GlobalActionBatchCommand, GlobalActionCommand};
use crate::config::CoreConfig;
use crate::decide::flush::build_group_flush_events;
use crate::decide::scheduler::{day_index, minute_of_day};
use crate::event::{
    Event, EventEnvelope, GroupFlushReason, RenderEvent, ReviewDecision, ReviewEvent,
    ScheduleEvent, SendEvent, SendPriority,
};
use crate::ids::{ActorId, ExternalCode, Id128};
use crate::state::StateView;

pub fn decide_global_action(
    state: &StateView,
    cmd: &GlobalActionCommand,
    _config: &CoreConfig,
) -> Vec<Event> {
    match &cmd.action {
        GlobalAction::SendQueueFlush => {
            let mut events = vec![Event::Schedule(ScheduleEvent::GroupFlushRequested {
                group_id: cmd.group_id.clone(),
                minute_of_day: minute_of_day(cmd.now_ms, cmd.tz_offset_minutes),
                day_index: day_index(cmd.now_ms, cmd.tz_offset_minutes),
                reason: GroupFlushReason::Manual,
            })];
            events.extend(build_group_flush_events(state, &cmd.group_id, cmd.now_ms));
            events
        }
        GlobalAction::SendQueueClear => {
            let post_ids = state
                .send_plans
                .values()
                .filter(|plan| plan.group_id == cmd.group_id)
                .map(|plan| plan.post_id)
                .collect::<Vec<_>>();
            post_ids
                .into_iter()
                .map(|post_id| Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }))
                .collect()
        }
        GlobalAction::SendInFlightClear => {
            let mut events = Vec::new();
            let mut seq = state.next_send_seq;
            for sending in state.sending.values() {
                if sending.group_id != cmd.group_id {
                    continue;
                }
                let retry_at_ms = cmd.now_ms;
                events.push(Event::Send(SendEvent::SendFailed {
                    post_id: sending.post_id,
                    account_id: sending.account_id.clone(),
                    attempt: 0,
                    retry_at_ms,
                    error: "manual clear sending".to_string(),
                }));
                events.push(Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                    post_id: sending.post_id,
                    group_id: sending.group_id.clone(),
                    not_before_ms: retry_at_ms,
                    priority: SendPriority::Normal,
                    seq,
                }));
                seq = seq.saturating_add(1);
            }
            events
        }
        GlobalAction::PendingClear => {
            let mut pending = state
                .reviews
                .iter()
                .filter_map(|(review_id, review)| {
                    let is_pending =
                        matches!(review.decision, None | Some(ReviewDecision::Deferred));
                    if !is_pending {
                        return None;
                    }
                    let post_meta = state.posts.get(&review.post_id)?;
                    if post_meta.group_id != cmd.group_id {
                        return None;
                    }
                    Some((
                        review.review_code,
                        *review_id,
                        review.post_id,
                        post_meta.group_id.clone(),
                    ))
                })
                .collect::<Vec<_>>();
            pending.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

            let mut next_by_group = state.next_external_code_by_group.clone();
            let mut events = Vec::new();
            for (_, review_id, post_id, group_id) in pending {
                events.push(Event::Review(ReviewEvent::ReviewDecisionRecorded {
                    review_id,
                    decision: ReviewDecision::Deleted,
                    decided_by: cmd.operator_id.clone(),
                    decided_at_ms: cmd.now_ms,
                }));
                if let Some(event) =
                    maybe_assign_external_code(state, &mut next_by_group, &group_id, post_id)
                {
                    events.push(event);
                }
                if state.send_plans.contains_key(&post_id) {
                    events.push(Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }));
                }
            }
            events
        }
        GlobalAction::Recall { review_code } => {
            let Some(review_id) = state.review_by_code.get(review_code).copied() else {
                return Vec::new();
            };
            let Some(meta) = state.reviews.get(&review_id) else {
                return Vec::new();
            };
            let Some(post_meta) = state.posts.get(&meta.post_id) else {
                return Vec::new();
            };
            if post_meta.group_id != cmd.group_id {
                return Vec::new();
            }
            vec![
                Event::Review(ReviewEvent::ReviewRerenderRequested { review_id }),
                Event::Render(RenderEvent::RenderRequested {
                    post_id: meta.post_id,
                    attempt: 1,
                    requested_at_ms: cmd.now_ms,
                }),
            ]
        }
        GlobalAction::Withdraw { review_code } => build_withdraw_events(state, cmd, *review_code),
        GlobalAction::SetExternalNumber { value } => {
            vec![Event::Review(ReviewEvent::ReviewExternalNumberSet {
                group_id: cmd.group_id.clone(),
                next_number: *value,
            })]
        }
        GlobalAction::BlacklistRemove { sender_id } => {
            vec![Event::Review(ReviewEvent::ReviewBlacklistRemoved {
                group_id: cmd.group_id.clone(),
                sender_id: sender_id.clone(),
            })]
        }
        GlobalAction::BlacklistAdd { sender_id, reason } => {
            vec![Event::Review(ReviewEvent::ReviewBlacklistAdded {
                group_id: cmd.group_id.clone(),
                sender_id: sender_id.clone(),
                reason: reason.clone(),
            })]
        }
        _ => Vec::new(),
    }
}

pub fn decide_global_action_batch(
    state: &StateView,
    cmd: &GlobalActionBatchCommand,
    config: &CoreConfig,
) -> Vec<Event> {
    let mut scratch = state.clone();
    let mut out = Vec::new();
    let mut env_id = 1u128;
    for action in &cmd.actions {
        let step_cmd = GlobalActionCommand {
            group_id: cmd.group_id.clone(),
            action: action.clone(),
            operator_id: cmd.operator_id.clone(),
            now_ms: cmd.now_ms,
            tz_offset_minutes: cmd.tz_offset_minutes,
        };
        let step_events = decide_global_action(&scratch, &step_cmd, config);
        if step_events.is_empty() {
            break;
        }
        for event in &step_events {
            scratch = scratch.reduce(&EventEnvelope {
                id: Id128(env_id),
                ts_ms: cmd.now_ms,
                actor: ActorId::from_u128(0),
                correlation_id: None,
                event: event.clone(),
            });
            env_id = env_id.saturating_add(1);
        }
        out.extend(step_events);
    }
    out
}

fn maybe_assign_external_code(
    state: &StateView,
    next_by_group: &mut std::collections::HashMap<String, ExternalCode>,
    group_id: &str,
    post_id: crate::ids::PostId,
) -> Option<Event> {
    if state.external_code_by_post.contains_key(&post_id) {
        return None;
    }
    let next_code = next_by_group.get(group_id).copied().unwrap_or(1);
    next_by_group.insert(group_id.to_string(), next_code.saturating_add(1));
    Some(Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
        post_id,
        group_id: group_id.to_string(),
        external_code: next_code,
    }))
}

fn build_withdraw_events(
    state: &StateView,
    cmd: &GlobalActionCommand,
    review_code: crate::ids::ReviewCode,
) -> Vec<Event> {
    let Some(review_id) = state.review_by_code.get(&review_code).copied() else {
        return Vec::new();
    };
    let Some(review) = state.reviews.get(&review_id) else {
        return Vec::new();
    };
    let post_id = review.post_id;
    let Some(post_meta) = state.posts.get(&post_id) else {
        return Vec::new();
    };
    if post_meta.group_id != cmd.group_id {
        return Vec::new();
    }
    let Some(plan) = state.send_plans.get(&post_id) else {
        return Vec::new();
    };
    if plan.group_id != cmd.group_id {
        return Vec::new();
    }
    let Some(recalled_code) = state.external_code_by_post.get(&post_id).copied() else {
        return Vec::new();
    };

    let mut events = vec![
        Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }),
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision: ReviewDecision::Deferred,
            decided_by: cmd.operator_id.clone(),
            decided_at_ms: cmd.now_ms,
        }),
        Event::Review(ReviewEvent::ReviewExternalCodeCleared { post_id }),
    ];

    let mut affected = state
        .send_plans
        .iter()
        .filter_map(|(queued_post_id, queued_plan)| {
            if *queued_post_id == post_id || queued_plan.group_id != cmd.group_id {
                return None;
            }
            let external_code = state.external_code_by_post.get(queued_post_id).copied()?;
            (external_code > recalled_code).then_some((external_code, *queued_post_id))
        })
        .collect::<Vec<_>>();
    affected.sort_by(|a, b| (a.0, a.1.0).cmp(&(b.0, b.1.0)));
    for (external_code, queued_post_id) in affected {
        events.push(Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
            post_id: queued_post_id,
            group_id: cmd.group_id.clone(),
            external_code: external_code.saturating_sub(1),
        }));
    }

    let current_next = state
        .next_external_code_by_group
        .get(&cmd.group_id)
        .copied()
        .unwrap_or_else(|| recalled_code.saturating_add(1));
    events.push(Event::Review(ReviewEvent::ReviewExternalNumberSet {
        group_id: cmd.group_id.clone(),
        next_number: current_next.saturating_sub(1).max(1),
    }));
    events.push(Event::Review(ReviewEvent::ReviewInfoSynced {
        review_id,
        post_id,
        review_code,
    }));
    events.push(Event::Review(ReviewEvent::ReviewPublishRequested {
        review_id,
    }));

    events
}
