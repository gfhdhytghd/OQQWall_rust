use oqqwall_rust_core::{
    BlockKindFilter, BlockSelector, Draft, DraftBlock, DraftTransform, Id128, IndexFilter,
    MediaKind, MediaReference, PositionSpec, RuleCondition, TextMatcher, apply_transforms,
    evaluate_condition, validate_transform,
};

#[test]
fn move_paragraph_to_front_keeps_selected_order() {
    let draft = draft(vec![image(), paragraph("caption"), image()]);
    let transformed = apply_transforms(
        &draft,
        &[DraftTransform::MoveBlocks {
            selector: selector_kind(BlockKindFilter::Paragraph),
            position: PositionSpec::Front,
        }],
    );

    assert_eq!(labels(&transformed), vec!["p:caption", "img", "img"]);
}

#[test]
fn selector_without_matches_is_noop() {
    let draft = draft(vec![image(), paragraph("caption")]);
    let transformed = apply_transforms(
        &draft,
        &[DraftTransform::MoveBlocks {
            selector: BlockSelector {
                kinds: Some(vec![BlockKindFilter::Paragraph]),
                text: Some(TextMatcher::Contains {
                    needle: "missing".to_string(),
                }),
                index: None,
            },
            position: PositionSpec::Front,
        }],
    );

    assert_eq!(transformed, draft);
}

#[test]
fn negative_nth_selects_from_filtered_tail() {
    let draft = draft(vec![
        paragraph("a"),
        image(),
        paragraph("b"),
        paragraph("c"),
    ]);
    let transformed = apply_transforms(
        &draft,
        &[DraftTransform::MoveBlocks {
            selector: BlockSelector {
                kinds: Some(vec![BlockKindFilter::Paragraph]),
                text: None,
                index: Some(IndexFilter::Nth { n: -1 }),
            },
            position: PositionSpec::Front,
        }],
    );

    assert_eq!(labels(&transformed), vec!["p:c", "p:a", "img", "p:b"]);
}

#[test]
fn before_and_after_fallbacks_are_deterministic() {
    let draft = draft(vec![paragraph("a"), image()]);
    let no_match = BlockSelector {
        kinds: Some(vec![BlockKindFilter::Paragraph]),
        text: Some(TextMatcher::Contains {
            needle: "missing".to_string(),
        }),
        index: None,
    };
    let paragraph_to_back = apply_transforms(
        &draft,
        &[DraftTransform::MoveBlocks {
            selector: selector_kind(BlockKindFilter::Paragraph),
            position: PositionSpec::Before {
                selector: no_match.clone(),
            },
        }],
    );
    let image_to_front = apply_transforms(
        &draft,
        &[DraftTransform::MoveBlocks {
            selector: selector_kind(BlockKindFilter::Attachment {
                media_kind: Some(MediaKind::Image),
            }),
            position: PositionSpec::After { selector: no_match },
        }],
    );

    assert_eq!(labels(&paragraph_to_back), vec!["img", "p:a"]);
    assert_eq!(labels(&image_to_front), vec!["img", "p:a"]);
}

#[test]
fn transform_order_is_sensitive() {
    let draft = draft(vec![paragraph("a"), image(), paragraph("b")]);
    let image_selector = selector_kind(BlockKindFilter::Attachment {
        media_kind: Some(MediaKind::Image),
    });
    let move_image_back = DraftTransform::MoveBlocks {
        selector: image_selector.clone(),
        position: PositionSpec::Back,
    };
    let move_first_paragraph_after_image = DraftTransform::MoveBlocks {
        selector: BlockSelector {
            kinds: Some(vec![BlockKindFilter::Paragraph]),
            text: None,
            index: Some(IndexFilter::First),
        },
        position: PositionSpec::After {
            selector: image_selector,
        },
    };

    let forward = apply_transforms(
        &draft,
        &[
            move_image_back.clone(),
            move_first_paragraph_after_image.clone(),
        ],
    );
    let reverse = apply_transforms(&draft, &[move_first_paragraph_after_image, move_image_back]);

    assert_ne!(labels(&forward), labels(&reverse));
}

#[test]
fn conditions_count_and_compose_selectors() {
    let draft = draft(vec![paragraph("caption"), image(), image()]);
    let image_selector = selector_kind(BlockKindFilter::Attachment {
        media_kind: Some(MediaKind::Image),
    });
    let caption_selector = BlockSelector {
        kinds: Some(vec![BlockKindFilter::Paragraph]),
        text: Some(TextMatcher::Regex {
            pattern: "^cap".to_string(),
        }),
        index: None,
    };

    assert!(evaluate_condition(
        &draft,
        &RuleCondition::All {
            conditions: vec![
                RuleCondition::BlockCountEquals {
                    selector: caption_selector.clone(),
                    n: 1,
                },
                RuleCondition::BlockCountAtLeast {
                    selector: image_selector,
                    n: 2,
                },
                RuleCondition::Not {
                    condition: Box::new(RuleCondition::HasBlock {
                        selector: BlockSelector {
                            kinds: Some(vec![BlockKindFilter::Paragraph]),
                            text: Some(TextMatcher::Contains {
                                needle: "missing".to_string(),
                            }),
                            index: None,
                        },
                    }),
                },
            ],
        }
    ));
    assert!(
        validate_transform(&DraftTransform::MoveBlocks {
            selector: caption_selector,
            position: PositionSpec::Front,
        })
        .is_ok()
    );
}

fn draft(blocks: Vec<DraftBlock>) -> Draft {
    Draft { blocks }
}

fn paragraph(text: &str) -> DraftBlock {
    DraftBlock::Paragraph {
        text: text.to_string(),
    }
}

fn image() -> DraftBlock {
    DraftBlock::Attachment {
        kind: MediaKind::Image,
        name: None,
        reference: MediaReference::Blob { blob_id: Id128(1) },
        size_bytes: None,
    }
}

fn selector_kind(kind: BlockKindFilter) -> BlockSelector {
    BlockSelector {
        kinds: Some(vec![kind]),
        text: None,
        index: None,
    }
}

fn labels(draft: &Draft) -> Vec<String> {
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
