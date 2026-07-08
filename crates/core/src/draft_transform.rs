use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::draft::{Draft, DraftBlock, MediaKind};

const MAX_REGEX_PATTERN_LEN: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKindFilter {
    Paragraph,
    Attachment { media_kind: Option<MediaKind> },
    Reply,
    Poke,
    JsonCard,
    Forward,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TextMatcher {
    Contains { needle: String },
    StartsWith { prefix: String },
    Regex { pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum IndexFilter {
    Nth { n: i32 },
    Range { start: i32, end: i32 },
    First,
    Last,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSelector {
    #[serde(default)]
    pub kinds: Option<Vec<BlockKindFilter>>,
    #[serde(default)]
    pub text: Option<TextMatcher>,
    #[serde(default)]
    pub index: Option<IndexFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "pos", rename_all = "snake_case")]
pub enum PositionSpec {
    Front,
    Back,
    Index { n: i32 },
    Before { selector: BlockSelector },
    After { selector: BlockSelector },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DraftTransform {
    MoveBlocks {
        selector: BlockSelector,
        position: PositionSpec,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleCondition {
    All { conditions: Vec<RuleCondition> },
    Any { conditions: Vec<RuleCondition> },
    Not { condition: Box<RuleCondition> },
    HasBlock { selector: BlockSelector },
    BlockCountAtLeast { selector: BlockSelector, n: usize },
    BlockCountEquals { selector: BlockSelector, n: usize },
}

pub fn apply_transforms(draft: &Draft, transforms: &[DraftTransform]) -> Draft {
    transforms.iter().fold(draft.clone(), |current, transform| {
        apply_transform(&current, transform)
    })
}

pub fn evaluate_condition(draft: &Draft, condition: &RuleCondition) -> bool {
    match condition {
        RuleCondition::All { conditions } => conditions
            .iter()
            .all(|condition| evaluate_condition(draft, condition)),
        RuleCondition::Any { conditions } => conditions
            .iter()
            .any(|condition| evaluate_condition(draft, condition)),
        RuleCondition::Not { condition } => !evaluate_condition(draft, condition),
        RuleCondition::HasBlock { selector } => !select_indices(draft, selector).is_empty(),
        RuleCondition::BlockCountAtLeast { selector, n } => {
            select_indices(draft, selector).len() >= *n
        }
        RuleCondition::BlockCountEquals { selector, n } => {
            select_indices(draft, selector).len() == *n
        }
    }
}

pub fn validate_transform(transform: &DraftTransform) -> Result<(), String> {
    match transform {
        DraftTransform::MoveBlocks { selector, position } => {
            validate_selector(selector)?;
            validate_position(position)
        }
    }
}

pub fn validate_condition(condition: &RuleCondition) -> Result<(), String> {
    match condition {
        RuleCondition::All { conditions } | RuleCondition::Any { conditions } => {
            for condition in conditions {
                validate_condition(condition)?;
            }
            Ok(())
        }
        RuleCondition::Not { condition } => validate_condition(condition),
        RuleCondition::HasBlock { selector }
        | RuleCondition::BlockCountAtLeast { selector, .. }
        | RuleCondition::BlockCountEquals { selector, .. } => validate_selector(selector),
    }
}

fn apply_transform(draft: &Draft, transform: &DraftTransform) -> Draft {
    match transform {
        DraftTransform::MoveBlocks { selector, position } => move_blocks(draft, selector, position),
    }
}

fn move_blocks(draft: &Draft, selector: &BlockSelector, position: &PositionSpec) -> Draft {
    let selected_indices = select_indices(draft, selector);
    if selected_indices.is_empty() {
        return draft.clone();
    }

    let selected_set = selected_indices
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut selected = Vec::new();
    let mut remaining = Vec::new();
    for (idx, block) in draft.blocks.iter().cloned().enumerate() {
        if selected_set.contains(&idx) {
            selected.push(block);
        } else {
            remaining.push(block);
        }
    }

    let insert_at = resolve_position(&remaining, position);
    remaining.splice(insert_at..insert_at, selected);
    Draft { blocks: remaining }
}

pub fn select_indices(draft: &Draft, selector: &BlockSelector) -> Vec<usize> {
    let mut candidates = draft
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            selector
                .kinds
                .as_ref()
                .is_none_or(|kinds| kinds.iter().any(|kind| kind_matches(block, kind)))
        })
        .filter(|(_, block)| {
            selector
                .text
                .as_ref()
                .is_none_or(|matcher| text_matches(block, matcher))
        })
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();

    if let Some(index) = &selector.index {
        candidates = apply_index_filter(candidates, index);
    }

    candidates
}

fn kind_matches(block: &DraftBlock, filter: &BlockKindFilter) -> bool {
    match (block, filter) {
        (DraftBlock::Paragraph { .. }, BlockKindFilter::Paragraph) => true,
        (DraftBlock::Attachment { kind, .. }, BlockKindFilter::Attachment { media_kind }) => {
            media_kind.is_none_or(|media_kind| *kind == media_kind)
        }
        (DraftBlock::Reply { .. }, BlockKindFilter::Reply) => true,
        (DraftBlock::Poke, BlockKindFilter::Poke) => true,
        (DraftBlock::JsonCard { .. }, BlockKindFilter::JsonCard) => true,
        (DraftBlock::Forward { .. }, BlockKindFilter::Forward) => true,
        _ => false,
    }
}

fn text_matches(block: &DraftBlock, matcher: &TextMatcher) -> bool {
    let DraftBlock::Paragraph { text } = block else {
        return false;
    };
    match matcher {
        TextMatcher::Contains { needle } => text.contains(needle),
        TextMatcher::StartsWith { prefix } => text.starts_with(prefix),
        TextMatcher::Regex { pattern } => Regex::new(pattern)
            .map(|regex| regex.is_match(text))
            .unwrap_or(false),
    }
}

fn apply_index_filter(candidates: Vec<usize>, filter: &IndexFilter) -> Vec<usize> {
    let len = candidates.len();
    if len == 0 {
        return Vec::new();
    }

    match filter {
        IndexFilter::Nth { n } => normalize_existing_index(*n, len)
            .and_then(|idx| candidates.get(idx).copied())
            .into_iter()
            .collect(),
        IndexFilter::Range { start, end } => {
            let Some(start) = normalize_existing_index(*start, len) else {
                return Vec::new();
            };
            let Some(end) = normalize_existing_index(*end, len) else {
                return Vec::new();
            };
            let (lo, hi) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            candidates[lo..=hi].to_vec()
        }
        IndexFilter::First => candidates.first().copied().into_iter().collect(),
        IndexFilter::Last => candidates.last().copied().into_iter().collect(),
    }
}

fn resolve_position(remaining: &[DraftBlock], position: &PositionSpec) -> usize {
    match position {
        PositionSpec::Front => 0,
        PositionSpec::Back => remaining.len(),
        PositionSpec::Index { n } => normalize_insert_index(*n, remaining.len()),
        PositionSpec::Before { selector } => {
            let draft = Draft {
                blocks: remaining.to_vec(),
            };
            select_indices(&draft, selector)
                .into_iter()
                .min()
                .unwrap_or(remaining.len())
        }
        PositionSpec::After { selector } => {
            let draft = Draft {
                blocks: remaining.to_vec(),
            };
            select_indices(&draft, selector)
                .into_iter()
                .max()
                .map(|idx| idx.saturating_add(1))
                .unwrap_or(0)
        }
    }
}

fn normalize_existing_index(index: i32, len: usize) -> Option<usize> {
    let idx = if index >= 0 {
        index as isize
    } else {
        len as isize + index as isize
    };
    if idx < 0 || idx >= len as isize {
        return None;
    }
    Some(idx as usize)
}

fn normalize_insert_index(index: i32, len: usize) -> usize {
    let idx = if index >= 0 {
        index as isize
    } else {
        len as isize + index as isize
    };
    idx.clamp(0, len as isize) as usize
}

fn validate_selector(selector: &BlockSelector) -> Result<(), String> {
    if let Some(kinds) = &selector.kinds {
        if kinds.is_empty() {
            return Err("selector kinds must not be empty".to_string());
        }
    }
    if let Some(text) = &selector.text {
        validate_text_matcher(text)?;
    }
    Ok(())
}

fn validate_position(position: &PositionSpec) -> Result<(), String> {
    match position {
        PositionSpec::Before { selector } | PositionSpec::After { selector } => {
            validate_selector(selector)
        }
        PositionSpec::Front | PositionSpec::Back | PositionSpec::Index { .. } => Ok(()),
    }
}

fn validate_text_matcher(matcher: &TextMatcher) -> Result<(), String> {
    if let TextMatcher::Regex { pattern } = matcher {
        if pattern.len() > MAX_REGEX_PATTERN_LEN {
            return Err(format!(
                "regex pattern too long: {} > {}",
                pattern.len(),
                MAX_REGEX_PATTERN_LEN
            ));
        }
        Regex::new(pattern).map_err(|err| format!("invalid regex: {}", err))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{MediaReference, ReplyPreview};
    use crate::ids::Id128;

    #[test]
    fn move_paragraph_to_front_before_images() {
        let draft = sample_draft();
        let moved = apply_transforms(
            &draft,
            &[DraftTransform::MoveBlocks {
                selector: paragraph_selector(),
                position: PositionSpec::Front,
            }],
        );

        assert_eq!(
            block_labels(&moved),
            vec!["p:intro", "p:tail", "img", "img"]
        );
    }

    #[test]
    fn no_match_is_noop() {
        let draft = sample_draft();
        let moved = apply_transforms(
            &draft,
            &[DraftTransform::MoveBlocks {
                selector: BlockSelector {
                    kinds: Some(vec![BlockKindFilter::Reply]),
                    text: None,
                    index: None,
                },
                position: PositionSpec::Front,
            }],
        );

        assert_eq!(moved, draft);
    }

    #[test]
    fn negative_index_selects_from_tail() {
        let draft = sample_draft();
        let moved = apply_transforms(
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

        assert_eq!(
            block_labels(&moved),
            vec!["p:tail", "img", "p:intro", "img"]
        );
    }

    #[test]
    fn before_after_fallbacks_are_deterministic() {
        let draft = sample_draft();
        let missing_reply = BlockSelector {
            kinds: Some(vec![BlockKindFilter::Reply]),
            text: None,
            index: None,
        };

        let before_missing = apply_transforms(
            &draft,
            &[DraftTransform::MoveBlocks {
                selector: paragraph_selector(),
                position: PositionSpec::Before {
                    selector: missing_reply.clone(),
                },
            }],
        );
        assert_eq!(
            block_labels(&before_missing),
            vec!["img", "img", "p:intro", "p:tail"]
        );

        let after_missing = apply_transforms(
            &draft,
            &[DraftTransform::MoveBlocks {
                selector: paragraph_selector(),
                position: PositionSpec::After {
                    selector: missing_reply,
                },
            }],
        );
        assert_eq!(
            block_labels(&after_missing),
            vec!["p:intro", "p:tail", "img", "img"]
        );
    }

    #[test]
    fn transform_order_is_sensitive_but_repeatable() {
        let draft = sample_draft();
        let first = DraftTransform::MoveBlocks {
            selector: BlockSelector {
                kinds: Some(vec![BlockKindFilter::Paragraph]),
                text: None,
                index: Some(IndexFilter::First),
            },
            position: PositionSpec::Back,
        };
        let second = DraftTransform::MoveBlocks {
            selector: BlockSelector {
                kinds: Some(vec![BlockKindFilter::Paragraph]),
                text: None,
                index: Some(IndexFilter::Last),
            },
            position: PositionSpec::Front,
        };

        let a = apply_transforms(&draft, &[first.clone(), second.clone()]);
        let b = apply_transforms(&draft, &[second.clone(), first.clone()]);
        assert_eq!(block_labels(&a), vec!["p:intro", "img", "img", "p:tail"]);
        assert_eq!(block_labels(&b), vec!["img", "p:intro", "img", "p:tail"]);
        assert_ne!(a, b);
        assert_eq!(a, apply_transforms(&draft, &[first, second]));
    }

    #[test]
    fn conditions_and_regex_validation_work() {
        let draft = sample_draft();
        let condition = RuleCondition::All {
            conditions: vec![
                RuleCondition::BlockCountEquals {
                    selector: paragraph_selector(),
                    n: 2,
                },
                RuleCondition::HasBlock {
                    selector: BlockSelector {
                        kinds: Some(vec![BlockKindFilter::Paragraph]),
                        text: Some(TextMatcher::Regex {
                            pattern: "^int".to_string(),
                        }),
                        index: None,
                    },
                },
            ],
        };

        assert!(validate_condition(&condition).is_ok());
        assert!(evaluate_condition(&draft, &condition));
        assert!(
            validate_condition(&RuleCondition::HasBlock {
                selector: BlockSelector {
                    kinds: Some(vec![BlockKindFilter::Paragraph]),
                    text: Some(TextMatcher::Regex {
                        pattern: "(".to_string(),
                    }),
                    index: None,
                },
            })
            .is_err()
        );
    }

    fn sample_draft() -> Draft {
        Draft {
            blocks: vec![
                DraftBlock::Attachment {
                    kind: MediaKind::Image,
                    name: None,
                    reference: MediaReference::Blob { blob_id: Id128(1) },
                    size_bytes: None,
                },
                DraftBlock::Paragraph {
                    text: "intro".to_string(),
                },
                DraftBlock::Attachment {
                    kind: MediaKind::Image,
                    name: None,
                    reference: MediaReference::Blob { blob_id: Id128(2) },
                    size_bytes: None,
                },
                DraftBlock::Paragraph {
                    text: "tail".to_string(),
                },
            ],
        }
    }

    fn paragraph_selector() -> BlockSelector {
        BlockSelector {
            kinds: Some(vec![BlockKindFilter::Paragraph]),
            text: None,
            index: None,
        }
    }

    fn block_labels(draft: &Draft) -> Vec<String> {
        draft
            .blocks
            .iter()
            .map(|block| match block {
                DraftBlock::Paragraph { text } => format!("p:{}", text),
                DraftBlock::Attachment { kind, .. } if *kind == MediaKind::Image => {
                    "img".to_string()
                }
                DraftBlock::Attachment { .. } => "file".to_string(),
                DraftBlock::Reply {
                    preview: ReplyPreview { .. },
                } => "reply".to_string(),
                DraftBlock::Poke => "poke".to_string(),
                DraftBlock::JsonCard { .. } => "json".to_string(),
                DraftBlock::Forward { .. } => "forward".to_string(),
            })
            .collect()
    }
}
