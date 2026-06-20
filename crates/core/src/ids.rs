use serde::{Deserialize, Serialize};

pub type TimestampMs = i64;
pub type GroupId = String;
pub type AccountId = String;
pub type ReviewCode = u32;
pub type ExternalCode = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Id128(pub u128);

impl Id128 {
    pub const ZERO: Self = Self(0);

    pub fn from_u128(value: u128) -> Self {
        Self(value)
    }

    pub fn to_be_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }
}

pub type EventId = Id128;
pub type ActorId = Id128;
pub type CorrelationId = Id128;
pub type IngressId = Id128;
pub type SessionId = Id128;
pub type PostId = Id128;
pub type DraftId = Id128;
pub type ReviewId = Id128;
pub type BlobId = Id128;

pub type AuditMsgId = String;
pub type RemotePostId = String;

pub const TAG_INGRESS_ID: &[u8] = b"ingress_id";
pub const TAG_SESSION_ID: &[u8] = b"session_id";
pub const TAG_POST_ID: &[u8] = b"post_id";
pub const TAG_DRAFT_ID: &[u8] = b"draft_id";
pub const TAG_REVIEW_ID: &[u8] = b"review_id";
pub const TAG_BLOB_ID: &[u8] = b"blob_id";

pub fn derive_id128(tag: &[u8], parts: &[&[u8]]) -> Id128 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(tag);
    hasher.update(&[0u8]);
    for part in parts {
        hasher.update(part);
        hasher.update(&[0u8]);
    }
    let hash = hasher.finalize();
    id128_from_hash(hash)
}

pub fn derive_ingress_id(parts: &[&[u8]]) -> IngressId {
    derive_id128(TAG_INGRESS_ID, parts)
}

pub fn derive_session_id(parts: &[&[u8]]) -> SessionId {
    derive_id128(TAG_SESSION_ID, parts)
}

pub fn derive_post_id(parts: &[&[u8]]) -> PostId {
    derive_id128(TAG_POST_ID, parts)
}

pub fn derive_draft_id(parts: &[&[u8]]) -> DraftId {
    derive_id128(TAG_DRAFT_ID, parts)
}

pub fn derive_review_id(parts: &[&[u8]]) -> ReviewId {
    derive_id128(TAG_REVIEW_ID, parts)
}

pub fn derive_blob_id(parts: &[&[u8]]) -> BlobId {
    derive_id128(TAG_BLOB_ID, parts)
}

fn id128_from_hash(hash: blake3::Hash) -> Id128 {
    let bytes = hash.as_bytes();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    Id128::from_u128(u128::from_be_bytes(buf))
}
