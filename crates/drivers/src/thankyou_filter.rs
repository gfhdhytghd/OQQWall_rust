use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::imageops::FilterType;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const BUILTIN_REGISTRY_JSON: &str = include_str!("../../../res/thankyou/sticker_hashes.json");
const TEXT_REMOVE_CHARS: &str =
    r#" 　\t\r\n“”‘’《》〈〉【】。，、：；？！!?,.（）()「」『』[]{}<>~～·'"`*_-=+|/\^$#@%&"#;

#[derive(Debug, Clone)]
pub struct ThankYouFilterRuntimeConfig {
    pub enabled: bool,
    pub window_sec: u64,
    pub max_text_chars: usize,
    pub phash_distance: u32,
    pub registry: Arc<ThankYouRegistry>,
}

impl ThankYouFilterRuntimeConfig {
    pub fn builtin_enabled() -> Result<Self, String> {
        Self::with_builtin_registry(true, 30 * 60, 16, 6)
    }

    pub fn with_builtin_registry(
        enabled: bool,
        window_sec: u64,
        max_text_chars: usize,
        phash_distance: u32,
    ) -> Result<Self, String> {
        Ok(Self {
            enabled,
            window_sec,
            max_text_chars,
            phash_distance,
            registry: Arc::new(ThankYouRegistry::from_json(BUILTIN_REGISTRY_JSON)?),
        })
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            window_sec: 30 * 60,
            max_text_chars: 16,
            phash_distance: 6,
            registry: Arc::new(ThankYouRegistry::empty()),
        }
    }

    pub fn with_registry_json(
        enabled: bool,
        window_sec: u64,
        max_text_chars: usize,
        phash_distance: u32,
        json: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            enabled,
            window_sec,
            max_text_chars,
            phash_distance,
            registry: Arc::new(ThankYouRegistry::from_json(json)?),
        })
    }
}

impl Default for ThankYouFilterRuntimeConfig {
    fn default() -> Self {
        Self::builtin_enabled().unwrap_or_else(|_| Self::disabled())
    }
}

#[derive(Debug, Clone)]
pub struct ThankYouRegistry {
    face_ids: HashSet<String>,
    mfaces: HashSet<MfaceKey>,
    file_uniques: HashSet<String>,
    content_sha256: HashSet<String>,
    phashes: Vec<u64>,
}

impl ThankYouRegistry {
    fn empty() -> Self {
        Self {
            face_ids: HashSet::new(),
            mfaces: HashSet::new(),
            file_uniques: HashSet::new(),
            content_sha256: HashSet::new(),
            phashes: Vec::new(),
        }
    }

    pub fn from_json(input: &str) -> Result<Self, String> {
        let raw: RegistryFile = serde_json::from_str(input)
            .map_err(|err| format!("invalid thank_you_filter registry json: {}", err))?;
        let mut registry = Self::empty();
        for entry in raw.face_ids {
            if let Some(id) = nonempty(entry.id) {
                registry.face_ids.insert(id);
            }
            if let Some(hash) = normalize_hex(entry.sha256) {
                registry.content_sha256.insert(hash);
            }
            if let Some(hash) = entry.phash.and_then(|value| parse_u64_hex(&value)) {
                registry.phashes.push(hash);
            }
        }
        for entry in raw.mfaces {
            let key = MfaceKey {
                emoji_package_id: entry
                    .emoji_package_id
                    .and_then(nonempty)
                    .unwrap_or_default(),
                emoji_id: entry.emoji_id.and_then(nonempty).unwrap_or_default(),
                key: entry.key.and_then(nonempty).unwrap_or_default(),
            };
            if !key.emoji_package_id.is_empty() || !key.emoji_id.is_empty() || !key.key.is_empty() {
                registry.mfaces.insert(key);
            }
        }
        for value in raw.file_uniques {
            if let Some(value) = nonempty(value) {
                registry.file_uniques.insert(value);
            }
        }
        for entry in raw.images {
            if let Some(value) = entry.file_unique.and_then(nonempty) {
                registry.file_uniques.insert(value);
            }
            if let Some(hash) = normalize_hex(entry.sha256) {
                registry.content_sha256.insert(hash);
            }
            if let Some(hash) = entry.phash.and_then(|value| parse_u64_hex(&value)) {
                registry.phashes.push(hash);
            }
        }
        Ok(registry)
    }

    fn has_image_hashes(&self) -> bool {
        !self.content_sha256.is_empty() || !self.phashes.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThankYouFeedbackKind {
    SendSucceeded,
    Rejected,
    ManualReply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThankYouMatch {
    pub rule: &'static str,
}

pub async fn evaluate_message(
    config: &ThankYouFilterRuntimeConfig,
    feedback_kind: ThankYouFeedbackKind,
    message: Option<&Value>,
    raw_message: Option<&str>,
    client: &Client,
) -> Option<ThankYouMatch> {
    if !config.enabled {
        return None;
    }
    let candidate = segment_candidate(message, raw_message)?;
    match candidate {
        SegmentCandidate::Text(text) => match_text(config, feedback_kind, &text),
        SegmentCandidate::Face { id } => config
            .registry
            .face_ids
            .contains(&id)
            .then_some(ThankYouMatch { rule: "face_id" }),
        SegmentCandidate::Mface {
            emoji_package_id,
            emoji_id,
            key,
            summary,
        } => {
            let tuple = MfaceKey {
                emoji_package_id,
                emoji_id,
                key,
            };
            if config.registry.mfaces.contains(&tuple) {
                Some(ThankYouMatch {
                    rule: "mface_tuple",
                })
            } else {
                summary.and_then(|text| match_text(config, feedback_kind, &text))
            }
        }
        SegmentCandidate::Image {
            file_unique,
            source,
            summary,
        } => {
            if file_unique
                .as_deref()
                .is_some_and(|value| config.registry.file_uniques.contains(value))
            {
                return Some(ThankYouMatch {
                    rule: "image_file_unique",
                });
            }
            if let Some(text_match) =
                summary.and_then(|text| match_text(config, feedback_kind, &text))
            {
                return Some(text_match);
            }
            let source = source?;
            if !config.registry.has_image_hashes() {
                return None;
            }
            let bytes = fetch_image_bytes(&source, client).await.ok()?;
            let sha256 = sha256_hex(&bytes);
            if config.registry.content_sha256.contains(&sha256) {
                return Some(ThankYouMatch {
                    rule: "image_sha256",
                });
            }
            let phash = average_hash64(&bytes).ok()?;
            config
                .registry
                .phashes
                .iter()
                .any(|known| (*known ^ phash).count_ones() <= config.phash_distance)
                .then_some(ThankYouMatch {
                    rule: "image_phash",
                })
        }
    }
}

fn match_text(
    config: &ThankYouFilterRuntimeConfig,
    feedback_kind: ThankYouFeedbackKind,
    input: &str,
) -> Option<ThankYouMatch> {
    if has_continuation_signal(input) {
        return None;
    }
    let normalized = normalize_text(input);
    if normalized.is_empty() {
        return None;
    }
    if normalized.chars().count() > config.max_text_chars {
        return None;
    }
    if has_continuation_signal(&normalized) {
        return None;
    }
    if has_blocking_digit(&normalized) {
        return None;
    }
    if is_gratitude_text(&normalized) {
        return Some(ThankYouMatch { rule: "text_exact" });
    }
    if matches!(
        feedback_kind,
        ThankYouFeedbackKind::SendSucceeded | ThankYouFeedbackKind::Rejected
    ) && is_confirmation_text(&normalized)
    {
        return Some(ThankYouMatch {
            rule: "text_confirm",
        });
    }
    if is_thanks_emoji_only(&normalized) {
        return Some(ThankYouMatch { rule: "text_emoji" });
    }
    None
}

fn normalize_text(input: &str) -> String {
    input
        .chars()
        .filter_map(|ch| {
            let mapped = match ch {
                '謝' => '谢',
                '妳' => '你',
                '嗎' => '吗',
                '啦' => '啦',
                '　' => ' ',
                '\u{fe0f}' | '\u{200d}' => return None,
                '\u{ff01}'..='\u{ff5e}' => char::from_u32(ch as u32 - 0xfee0).unwrap_or(ch),
                _ => ch,
            };
            if mapped.is_whitespace() || TEXT_REMOVE_CHARS.contains(mapped) {
                None
            } else {
                Some(mapped.to_ascii_lowercase())
            }
        })
        .collect()
}

fn has_continuation_signal(value: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "但",
        "但是",
        "不过",
        "另外",
        "还有",
        "补充",
        "改一下",
        "修改",
        "帮我",
        "能不能",
        "为什么",
        "怎么",
        "吗",
        "？",
        "?",
        "http",
        "www",
    ];
    NEEDLES.iter().any(|needle| value.contains(needle))
}

fn has_blocking_digit(value: &str) -> bool {
    value.chars().any(|ch| ch.is_ascii_digit()) && value != "3q"
}

fn is_gratitude_text(value: &str) -> bool {
    const EXACT: &[&str] = &[
        "谢谢",
        "谢谢你",
        "谢谢您",
        "谢谢啦",
        "谢谢了",
        "谢谢哈",
        "谢啦",
        "谢了",
        "多谢",
        "感谢",
        "感谢你",
        "感谢您",
        "感謝",
        "拜谢",
        "蟹蟹",
        "栓q",
        "thx",
        "thanks",
        "thankyou",
        "3q",
        "辛苦了",
        "麻烦了",
        "好的谢谢",
        "好谢谢",
        "收到谢谢",
        "ok谢谢",
        "嗯嗯谢谢",
    ];
    EXACT.contains(&value)
}

fn is_confirmation_text(value: &str) -> bool {
    const EXACT: &[&str] = &[
        "好",
        "好的",
        "好滴",
        "好嘞",
        "收到",
        "已收到",
        "明白",
        "明白了",
        "了解",
        "了解了",
        "ok",
        "okay",
        "嗯",
        "嗯嗯",
        "行",
        "可以",
    ];
    EXACT.contains(&value)
}

fn is_thanks_emoji_only(value: &str) -> bool {
    let mut count = 0usize;
    for ch in value.chars() {
        if matches!(ch, '🙏' | '🙇' | '🤝' | '👍' | '❤' | '💗' | '💖') {
            count += 1;
        } else {
            return false;
        }
    }
    count > 0
}

fn segment_candidate(
    message: Option<&Value>,
    raw_message: Option<&str>,
) -> Option<SegmentCandidate> {
    match message {
        Some(Value::String(text)) => Some(SegmentCandidate::Text(text.clone())),
        Some(Value::Array(items)) => {
            let meaningful: Vec<&Value> = items
                .iter()
                .filter(|item| segment_type(item).is_some_and(|typ| typ != "reply"))
                .collect();
            if meaningful.len() != 1 {
                return None;
            }
            candidate_from_segment(meaningful[0])
        }
        _ => raw_message
            .filter(|value| !value.trim().is_empty())
            .map(|value| SegmentCandidate::Text(value.to_string())),
    }
}

fn candidate_from_segment(item: &Value) -> Option<SegmentCandidate> {
    let typ = segment_type(item)?;
    let data = item.get("data");
    match typ {
        "text" => data
            .and_then(|data| data.get("text"))
            .and_then(json_string)
            .map(SegmentCandidate::Text),
        "face" => data
            .and_then(|data| data.get("id"))
            .and_then(json_string)
            .and_then(nonempty)
            .map(|id| SegmentCandidate::Face { id }),
        "mface" => Some(SegmentCandidate::Mface {
            emoji_package_id: data
                .and_then(|data| data.get("emoji_package_id"))
                .and_then(json_string)
                .and_then(nonempty)
                .unwrap_or_default(),
            emoji_id: data
                .and_then(|data| data.get("emoji_id"))
                .and_then(json_string)
                .and_then(nonempty)
                .unwrap_or_default(),
            key: data
                .and_then(|data| data.get("key"))
                .and_then(json_string)
                .and_then(nonempty)
                .unwrap_or_default(),
            summary: data
                .and_then(|data| data.get("summary"))
                .and_then(json_string)
                .and_then(nonempty),
        }),
        "image" => Some(SegmentCandidate::Image {
            file_unique: data
                .and_then(|data| data.get("file_unique"))
                .and_then(json_string)
                .and_then(nonempty),
            source: data.and_then(image_source_from_data),
            summary: data
                .and_then(|data| data.get("summary"))
                .and_then(json_string)
                .and_then(nonempty),
        }),
        _ => None,
    }
}

fn segment_type(item: &Value) -> Option<&str> {
    item.get("type").and_then(|value| value.as_str())
}

fn image_source_from_data(data: &Value) -> Option<String> {
    ["url", "path", "file"]
        .iter()
        .find_map(|key| data.get(*key).and_then(json_string))
        .and_then(nonempty)
        .filter(|value| value != "marketface")
}

async fn fetch_image_bytes(source: &str, client: &Client) -> Result<Vec<u8>, String> {
    if let Some(payload) = source.strip_prefix("base64://") {
        return STANDARD
            .decode(payload)
            .map_err(|err| format!("invalid base64 image: {}", err));
    }
    if let Some(rest) = source.strip_prefix("data:") {
        let Some((_, payload)) = rest.split_once(',') else {
            return Err("invalid data uri image".to_string());
        };
        return STANDARD
            .decode(payload)
            .map_err(|err| format!("invalid data uri image: {}", err));
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        let response = client
            .get(source)
            .send()
            .await
            .map_err(|err| format!("image fetch failed: {}", err))?
            .error_for_status()
            .map_err(|err| format!("image fetch status failed: {}", err))?;
        return response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|err| format!("image read failed: {}", err));
    }
    let path = source.strip_prefix("file://").unwrap_or(source);
    std::fs::read(Path::new(path)).map_err(|err| format!("image file read failed: {}", err))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

pub fn average_hash64(bytes: &[u8]) -> Result<u64, String> {
    let image =
        image::load_from_memory(bytes).map_err(|err| format!("image decode failed: {}", err))?;
    let gray = image
        .grayscale()
        .resize_exact(8, 8, FilterType::Triangle)
        .to_luma8();
    let pixels = gray.as_raw();
    let avg = pixels.iter().map(|value| *value as u32).sum::<u32>() / pixels.len() as u32;
    let mut out = 0u64;
    for (idx, value) in pixels.iter().enumerate() {
        if *value as u32 >= avg {
            out |= 1u64 << (63 - idx);
        }
    }
    Ok(out)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn parse_u64_hex(input: &str) -> Option<u64> {
    let trimmed = input.trim().trim_start_matches("0x");
    u64::from_str_radix(trimmed, 16).ok()
}

fn normalize_hex(input: Option<String>) -> Option<String> {
    input
        .map(|value| value.trim().to_ascii_lowercase())
        .and_then(nonempty)
}

fn json_string(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_string());
    }
    if let Some(value) = value.as_i64() {
        return Some(value.to_string());
    }
    if let Some(value) = value.as_u64() {
        return Some(value.to_string());
    }
    None
}

fn nonempty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MfaceKey {
    emoji_package_id: String,
    emoji_id: String,
    key: String,
}

enum SegmentCandidate {
    Text(String),
    Face {
        id: String,
    },
    Mface {
        emoji_package_id: String,
        emoji_id: String,
        key: String,
        summary: Option<String>,
    },
    Image {
        file_unique: Option<String>,
        source: Option<String>,
        summary: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    face_ids: Vec<RegistryFaceEntry>,
    #[serde(default)]
    mfaces: Vec<RegistryMfaceEntry>,
    #[serde(default)]
    file_uniques: Vec<String>,
    #[serde(default)]
    images: Vec<RegistryImageEntry>,
}

#[derive(Debug, Deserialize)]
struct RegistryFaceEntry {
    id: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    phash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryMfaceEntry {
    #[serde(default)]
    emoji_package_id: Option<String>,
    #[serde(default)]
    emoji_id: Option<String>,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryImageEntry {
    #[serde(default)]
    file_unique: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    phash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ThankYouFilterRuntimeConfig {
        ThankYouFilterRuntimeConfig::with_registry_json(
            true,
            1800,
            16,
            6,
            r#"{
                "face_ids": [{"id": "297"}],
                "mfaces": [{"emoji_package_id": "1", "emoji_id": "2", "key": "k"}],
                "file_uniques": ["file-u"],
                "images": [{"sha256": "001122", "phash": "ff00ff00ff00ff00"}]
            }"#,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn text_exact_matches_thanks() {
        let value = serde_json::json!([
            {"type": "text", "data": {"text": "谢谢！"}}
        ]);
        let matched = evaluate_message(
            &config(),
            ThankYouFeedbackKind::ManualReply,
            Some(&value),
            Some("谢谢！"),
            &Client::new(),
        )
        .await;
        assert_eq!(matched.unwrap().rule, "text_exact");

        let value = serde_json::json!([
            {"type": "text", "data": {"text": "3Q"}}
        ]);
        let matched = evaluate_message(
            &config(),
            ThankYouFeedbackKind::ManualReply,
            Some(&value),
            Some("3Q"),
            &Client::new(),
        )
        .await;
        assert_eq!(matched.unwrap().rule, "text_exact");
    }

    #[tokio::test]
    async fn continuation_text_passes() {
        let value = serde_json::json!([
            {"type": "text", "data": {"text": "谢谢，不过我还想补充"}}
        ]);
        let matched = evaluate_message(
            &config(),
            ThankYouFeedbackKind::SendSucceeded,
            Some(&value),
            Some("谢谢，不过我还想补充"),
            &Client::new(),
        )
        .await;
        assert!(matched.is_none());
    }

    #[tokio::test]
    async fn confirmation_only_requires_terminal_feedback() {
        let value = serde_json::json!([
            {"type": "text", "data": {"text": "收到"}}
        ]);
        assert!(
            evaluate_message(
                &config(),
                ThankYouFeedbackKind::ManualReply,
                Some(&value),
                Some("收到"),
                &Client::new(),
            )
            .await
            .is_none()
        );
        assert_eq!(
            evaluate_message(
                &config(),
                ThankYouFeedbackKind::SendSucceeded,
                Some(&value),
                Some("收到"),
                &Client::new(),
            )
            .await
            .unwrap()
            .rule,
            "text_confirm"
        );
    }

    #[tokio::test]
    async fn face_and_mface_match_registry() {
        let face = serde_json::json!([
            {"type": "face", "data": {"id": "297"}}
        ]);
        assert_eq!(
            evaluate_message(
                &config(),
                ThankYouFeedbackKind::SendSucceeded,
                Some(&face),
                None,
                &Client::new(),
            )
            .await
            .unwrap()
            .rule,
            "face_id"
        );
        let mface = serde_json::json!([
            {"type": "mface", "data": {"emoji_package_id": 1, "emoji_id": "2", "key": "k"}}
        ]);
        assert_eq!(
            evaluate_message(
                &config(),
                ThankYouFeedbackKind::SendSucceeded,
                Some(&mface),
                None,
                &Client::new(),
            )
            .await
            .unwrap()
            .rule,
            "mface_tuple"
        );
    }

    #[tokio::test]
    async fn image_file_unique_matches_without_download() {
        let image = serde_json::json!([
            {"type": "image", "data": {"file_unique": "file-u", "url": "http://127.0.0.1/not-used.png"}}
        ]);
        assert_eq!(
            evaluate_message(
                &config(),
                ThankYouFeedbackKind::SendSucceeded,
                Some(&image),
                None,
                &Client::new(),
            )
            .await
            .unwrap()
            .rule,
            "image_file_unique"
        );
    }
}
