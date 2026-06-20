use std::sync::OnceLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::draft::IngressMessage;

const WINDOW_MESSAGES: usize = 12;

const POSITIVE_PATTERNS: &[&str] = &[
    r"(求|请|要|需要|帮我|给我)?(打?马|打?码|马赛克)",
    r"(匿名|匿)(一下|处理|发)?",
    r"别(显示|露|暴露)(我的)?(名字|姓名|id|qq|q号|号)",
    r"(不要|别|不想)实名",
    r"不留名",
    r"(代发|帮朋友(匿名)?发|代po)",
    r"(走马|走码)",
    r"(匿下|腻|拟|逆|尼)",
    r"🙈|🐎|🐴|🆔|🔒",
    r"(打|加|上)马赛克",
    r"(隐藏|遮挡|屏蔽)(姓名|名字|id|账号)",
];

const NEGATIVE_PATTERNS: &[&str] = &[
    r"不(用|要)?(匿名|匿)",
    r"不(用|要)?打?马",
    r"不(用|要)?打?码",
    r"不(用|要)?(马赛克)",
    r"不(用|要)?(腻|拟|逆|尼)",
    r"可以?(挂|显示)(我|id|账号|名字)",
    r"(直接|就)发",
    r"(不用|无需)(匿名|打码|马赛克)",
];

const PUBLIC_KEYWORDS: &[&str] = &["实名", "公开", "可留名", "署名"];
const PUBLIC_NEGATION_PREFIXES: &[&str] = &["不要", "不想", "别"];

pub fn detect_anonymous(messages: &[IngressMessage]) -> bool {
    let start = messages.len().saturating_sub(WINDOW_MESSAGES);
    let mut texts = Vec::new();
    for message in &messages[start..] {
        let combined = combine_message_text(message);
        let normalized = normalize_text(&combined);
        if !normalized.is_empty() {
            texts.push(normalized);
        }
    }

    for text in texts.iter().rev() {
        if matches_negative(text) {
            return false;
        }
        if matches_positive(text) {
            return true;
        }
    }

    false
}

fn combine_message_text(message: &IngressMessage) -> String {
    let mut out = String::new();
    let text = message.text.trim();
    if !text.is_empty() {
        out.push_str(text);
    }
    for attachment in &message.attachments {
        let Some(name) = attachment
            .name
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(name);
    }
    out
}

fn normalize_text(input: &str) -> String {
    let nfkc = input.nfkc().collect::<String>();
    let lower = nfkc.to_lowercase();
    let without_ctrl = control_re().replace_all(&lower, "");
    let squashed = space_re().replace_all(&without_ctrl, " ");
    squashed.trim().to_string()
}

fn matches_positive(text: &str) -> bool {
    positive_patterns().iter().any(|re| re.is_match(text))
}

fn matches_negative(text: &str) -> bool {
    if matches_public_without_negation(text) {
        return true;
    }
    negative_patterns().iter().any(|re| re.is_match(text))
}

fn matches_public_without_negation(text: &str) -> bool {
    for keyword in PUBLIC_KEYWORDS {
        let mut start = 0usize;
        while let Some(idx) = text[start..].find(keyword) {
            let pos = start + idx;
            let prefix = text[..pos].trim_end();
            if !PUBLIC_NEGATION_PREFIXES
                .iter()
                .any(|neg| prefix.ends_with(neg))
            {
                return true;
            }
            start = pos + keyword.len();
        }
    }
    false
}

fn positive_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| compile_patterns(POSITIVE_PATTERNS))
}

fn negative_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| compile_patterns(NEGATIVE_PATTERNS))
}

fn control_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\p{C}]+").expect("control regex"))
}

fn space_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").expect("space regex"))
}

fn compile_patterns(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|pattern| Regex::new(pattern).expect("regex pattern"))
        .collect()
}
