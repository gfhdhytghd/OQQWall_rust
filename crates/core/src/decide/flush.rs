use crate::event::{Event, ScheduleEvent};
use crate::ids::{GroupId, TimestampMs};
use crate::state::StateView;

pub fn build_group_flush_events(
    state: &StateView,
    group_id: &GroupId,
    now_ms: TimestampMs,
) -> Vec<Event> {
    let mut plans = state
        .send_plans
        .values()
        .filter(|plan| &plan.group_id == group_id)
        .collect::<Vec<_>>();
    plans.sort_by_key(|plan| (plan.priority, plan.seq, plan.post_id.0));
    plans
        .into_iter()
        .map(|plan| {
            Event::Schedule(ScheduleEvent::SendPlanRescheduled {
                post_id: plan.post_id,
                group_id: plan.group_id.clone(),
                not_before_ms: now_ms,
                priority: plan.priority,
                seq: plan.seq,
            })
        })
        .collect()
}
