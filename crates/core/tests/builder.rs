use oqqwall_rust_core::{
    DraftBlock, IngressAttachment, IngressMessage, MediaKind, MediaReference,
    build_draft_from_messages, poke_marker,
};

#[test]
fn build_draft_splits_paragraphs_and_keeps_attachments() {
    let message = IngressMessage {
        text: "alpha\n\n beta".to_string(),
        attachments: vec![IngressAttachment {
            kind: MediaKind::Image,
            name: None,
            reference: MediaReference::RemoteUrl {
                url: "http://example.com/img.png".to_string(),
            },
            size_bytes: None,
        }],
    };

    let draft = build_draft_from_messages(&[message]);
    assert_eq!(draft.blocks.len(), 2);
    assert!(matches!(draft.blocks[0], DraftBlock::Paragraph { .. }));
    assert!(matches!(draft.blocks[1], DraftBlock::Attachment { .. }));
}

#[test]
fn build_draft_extracts_known_special_markers() {
    let message = IngressMessage {
        text: format!("alpha{}beta", poke_marker()),
        attachments: Vec::new(),
    };

    let draft = build_draft_from_messages(&[message]);
    assert_eq!(draft.blocks.len(), 3);
    assert_eq!(
        draft.blocks[0],
        DraftBlock::Paragraph {
            text: "alpha".to_string()
        }
    );
    assert_eq!(draft.blocks[1], DraftBlock::Poke);
    assert_eq!(
        draft.blocks[2],
        DraftBlock::Paragraph {
            text: "beta".to_string()
        }
    );
}

#[test]
fn build_draft_keeps_unknown_double_bracket_markers_literal() {
    let message = IngressMessage {
        text: "hello [[face:5]] world".to_string(),
        attachments: Vec::new(),
    };

    let draft = build_draft_from_messages(&[message]);
    assert_eq!(
        draft.blocks,
        vec![DraftBlock::Paragraph {
            text: "hello [[face:5]] world".to_string()
        }]
    );
}
