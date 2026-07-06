use crate::draft::{Draft, DraftBlock, IngressMessage, parse_special_marker};
use crate::draft_transform::{DraftTransform, apply_transforms};
use crate::ids::{IngressId, PostId};
use crate::state::StateView;

pub fn build_draft_from_messages(messages: &[IngressMessage]) -> Draft {
    let mut blocks = Vec::new();

    for message in messages {
        append_blocks_from_text(&message.text, &mut blocks);
        for attachment in &message.attachments {
            blocks.push(DraftBlock::Attachment {
                kind: attachment.kind,
                name: attachment.name.clone(),
                reference: attachment.reference.clone(),
                size_bytes: attachment.size_bytes,
            });
        }
    }

    Draft { blocks }
}

pub fn build_draft_for_post(
    state: &StateView,
    post_id: PostId,
    ingress_ids: &[IngressId],
) -> Option<Draft> {
    build_draft_for_post_with_transforms(state, post_id, ingress_ids, None)
}

pub fn build_draft_for_post_with_transforms(
    state: &StateView,
    post_id: PostId,
    ingress_ids: &[IngressId],
    transforms: Option<&[DraftTransform]>,
) -> Option<Draft> {
    let mut messages = Vec::new();
    for ingress_id in ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    if messages.is_empty() {
        return None;
    }

    let draft = build_draft_from_messages(&messages);
    let transforms = transforms
        .or_else(|| state.draft_transforms.get(&post_id).map(Vec::as_slice))
        .unwrap_or(&[]);
    Some(apply_transforms(&draft, transforms))
}

fn append_blocks_from_text(text: &str, blocks: &mut Vec<DraftBlock>) {
    let mut literal = String::new();
    let mut remaining = text.trim();

    while !remaining.is_empty() {
        if remaining.starts_with("[[") {
            if let Some((block, consumed)) = parse_special_marker(remaining) {
                push_paragraph(&literal, blocks);
                literal.clear();
                blocks.push(block);
                remaining = &remaining[consumed..];
                continue;
            }
        }

        let Some(ch) = remaining.chars().next() else {
            break;
        };
        literal.push(ch);
        remaining = &remaining[ch.len_utf8()..];
    }

    push_paragraph(&literal, blocks);
}

fn push_paragraph(text: &str, blocks: &mut Vec<DraftBlock>) {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return;
    }
    blocks.push(DraftBlock::Paragraph {
        text: cleaned.to_string(),
    });
}
