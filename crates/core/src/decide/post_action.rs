use crate::command::{PostAction, PostActionCommand};
use crate::decide::builder::build_draft_for_post_with_transforms;
use crate::draft_transform::validate_transform;
use crate::event::{DraftEvent, Event, RenderEvent};
use crate::state::{PostStage, StateView};

pub fn decide_post_action(state: &StateView, cmd: &PostActionCommand) -> Vec<Event> {
    let Some(post) = state.posts.get(&cmd.post_id) else {
        return Vec::new();
    };
    if !can_transform_stage(post.stage) {
        return Vec::new();
    }

    match &cmd.action {
        PostAction::SetDraftTransforms { transforms } => {
            if transforms
                .iter()
                .any(|transform| validate_transform(transform).is_err())
            {
                return Vec::new();
            }
            let Some(ingress_ids) = state.post_ingress.get(&cmd.post_id) else {
                return Vec::new();
            };
            let Some(draft) = build_draft_for_post_with_transforms(
                state,
                cmd.post_id,
                ingress_ids,
                Some(transforms.as_slice()),
            ) else {
                return Vec::new();
            };

            vec![
                Event::Draft(DraftEvent::DraftTransformsSet {
                    post_id: cmd.post_id,
                    transforms: transforms.clone(),
                    set_at_ms: cmd.now_ms,
                }),
                Event::Draft(DraftEvent::PostDraftCreated {
                    post_id: cmd.post_id,
                    session_id: post.session_id,
                    group_id: post.group_id.clone(),
                    ingress_ids: ingress_ids.clone(),
                    is_anonymous: post.is_anonymous,
                    is_safe: post.is_safe,
                    draft,
                    created_at_ms: cmd.now_ms,
                }),
                Event::Render(RenderEvent::RenderRequested {
                    post_id: cmd.post_id,
                    attempt: 1,
                    requested_at_ms: cmd.now_ms,
                }),
            ]
        }
    }
}

fn can_transform_stage(stage: PostStage) -> bool {
    matches!(
        stage,
        PostStage::Drafted
            | PostStage::RenderRequested
            | PostStage::Rendered
            | PostStage::ReviewPending
            | PostStage::Reviewed
            | PostStage::Scheduled
            | PostStage::Failed
    )
}
