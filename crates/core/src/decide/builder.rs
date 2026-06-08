use crate::draft::{Draft, DraftBlock, IngressMessage, parse_special_marker};

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
