use crate::ids::BlobId;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

const REPLY_MARKER_PREFIX: &str = "[[reply:";
const JSON_CARD_MARKER_PREFIX: &str = "[[jsoncard:";
const FORWARD_MARKER_PREFIX: &str = "[[forward:";
const MARKER_SUFFIX: &str = "]]";
const POKE_MARKER: &str = "[[poke]]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub blocks: Vec<DraftBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftBlock {
    Paragraph {
        text: String,
    },
    Attachment {
        kind: MediaKind,
        #[serde(default)]
        name: Option<String>,
        reference: MediaReference,
        #[serde(default)]
        size_bytes: Option<u64>,
    },
    Reply {
        preview: ReplyPreview,
    },
    Poke,
    JsonCard {
        raw: String,
    },
    Forward {
        items: Vec<ForwardItem>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyPreview {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub meta: Option<String>,
    pub body: String,
    #[serde(default)]
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardItem {
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub blocks: Vec<DraftBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressMessage {
    pub text: String,
    pub attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IngressRouteMeta {
    #[serde(default)]
    pub source_webhook: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressAttachment {
    pub kind: MediaKind,
    pub name: Option<String>,
    pub reference: MediaReference,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaReference {
    RemoteUrl { url: String },
    Blob { blob_id: BlobId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Video,
    File,
    Audio,
    Other,
    Sticker,
}

pub fn reply_marker(preview: &ReplyPreview) -> String {
    let payload = serde_json::to_vec(preview).unwrap_or_default();
    format!(
        "{}{}{}",
        REPLY_MARKER_PREFIX,
        URL_SAFE_NO_PAD.encode(payload),
        MARKER_SUFFIX
    )
}

pub fn poke_marker() -> &'static str {
    POKE_MARKER
}

pub fn json_card_marker(raw: &str) -> String {
    format!(
        "{}{}{}",
        JSON_CARD_MARKER_PREFIX,
        URL_SAFE_NO_PAD.encode(raw.as_bytes()),
        MARKER_SUFFIX
    )
}

pub fn forward_marker(items: &[ForwardItem]) -> String {
    let payload = serde_json::to_vec(items).unwrap_or_default();
    format!(
        "{}{}{}",
        FORWARD_MARKER_PREFIX,
        URL_SAFE_NO_PAD.encode(payload),
        MARKER_SUFFIX
    )
}

pub fn parse_special_marker(input: &str) -> Option<(DraftBlock, usize)> {
    if input.starts_with(POKE_MARKER) {
        return Some((DraftBlock::Poke, POKE_MARKER.len()));
    }
    parse_payload_marker(input, REPLY_MARKER_PREFIX)
        .and_then(|(payload, consumed)| {
            serde_json::from_slice::<ReplyPreview>(&payload)
                .ok()
                .map(|preview| (DraftBlock::Reply { preview }, consumed))
        })
        .or_else(|| {
            parse_payload_marker(input, JSON_CARD_MARKER_PREFIX).and_then(|(payload, consumed)| {
                String::from_utf8(payload)
                    .ok()
                    .map(|raw| (DraftBlock::JsonCard { raw }, consumed))
            })
        })
        .or_else(|| {
            parse_payload_marker(input, FORWARD_MARKER_PREFIX).and_then(|(payload, consumed)| {
                serde_json::from_slice::<Vec<ForwardItem>>(&payload)
                    .ok()
                    .map(|items| (DraftBlock::Forward { items }, consumed))
            })
        })
}

fn parse_payload_marker(input: &str, prefix: &str) -> Option<(Vec<u8>, usize)> {
    let rest = input.strip_prefix(prefix)?;
    let close = rest.find(MARKER_SUFFIX)?;
    let payload = &rest[..close];
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    Some((bytes, prefix.len() + close + MARKER_SUFFIX.len()))
}
