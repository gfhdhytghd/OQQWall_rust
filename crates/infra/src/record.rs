use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{InfraError, InfraResult};

pub(crate) const HEADER_BYTES: usize = 8;
pub(crate) const FORMAT_VERSION: u8 = 0x02;

pub(crate) struct DecodedRecord<T> {
    pub value: T,
    pub legacy_format: bool,
}

pub(crate) fn versioned_body_len(payload_len: usize) -> InfraResult<usize> {
    payload_len
        .checked_add(1)
        .ok_or_else(|| InfraError::InvalidData("record payload too large".to_string()))
}

pub(crate) fn serialize_record<T>(value: &T) -> InfraResult<Vec<u8>>
where
    T: Serialize,
{
    let payload = bincode::serialize(value).map_err(|err| InfraError::Codec(err.to_string()))?;
    encode_serialized_payload(&payload)
}

pub(crate) fn encode_serialized_payload(payload: &[u8]) -> InfraResult<Vec<u8>> {
    let len = versioned_body_len(payload.len())?;
    if len > u32::MAX as usize {
        return Err(InfraError::InvalidData(format!(
            "record payload too large: {} bytes",
            len
        )));
    }

    let mut body = Vec::with_capacity(len);
    body.push(FORMAT_VERSION);
    body.extend_from_slice(payload);
    let crc = crc32fast::hash(&body);

    let mut buf = Vec::with_capacity(HEADER_BYTES + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&body);
    Ok(buf)
}

pub(crate) fn deserialize_body<T>(body: &[u8], crc: u32) -> Result<DecodedRecord<T>, String>
where
    T: DeserializeOwned,
{
    let body_crc = crc32fast::hash(body);
    let mut versioned_error = None;
    if body.first().copied() == Some(FORMAT_VERSION) && body_crc == crc {
        match bincode::deserialize(&body[1..]) {
            Ok(value) => {
                return Ok(DecodedRecord {
                    value,
                    legacy_format: false,
                });
            }
            Err(err) => {
                versioned_error = Some(err.to_string());
            }
        }
    }

    #[cfg(feature = "legacy_codec")]
    {
        if body_crc == crc {
            match bincode::deserialize(body) {
                Ok(value) => {
                    return Ok(DecodedRecord {
                        value,
                        legacy_format: true,
                    });
                }
                Err(err) => {
                    if let Some(versioned_error) = versioned_error {
                        return Err(format!(
                            "decode failed: versioned={}, legacy={}",
                            versioned_error, err
                        ));
                    }
                    return Err(format!("decode failed: {}", err));
                }
            }
        }
    }

    if body_crc != crc {
        return Err("crc mismatch".to_string());
    }
    if body.is_empty() {
        return Err("empty record body".to_string());
    }
    Err(format!("unsupported format version: 0x{:02x}", body[0]))
}
