use std::sync::OnceLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::draft::IngressMessage;

const UNSAFE_PATTERNS: &[&str] = &[
    r"(傻逼|草泥马|fuck|shit|妈的|操你|去死|滚)",
    r"(法轮功|六四|天安门|习近平|毛泽东|共产党)",
    r"(人身攻击|恶意中伤|网络暴力)",
];

pub fn detect_safe(messages: &[IngressMessage]) -> bool {
    for message in messages {
        let combined = combine_message_text(message);
        let normalized = normalize_text(&combined);
        if normalized.is_empty() {
            continue;
        }
        if unsafe_patterns().iter().any(|re| re.is_match(&normalized)) {
            return false;
        }
    }
    true
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

fn unsafe_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| compile_patterns(UNSAFE_PATTERNS))
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
