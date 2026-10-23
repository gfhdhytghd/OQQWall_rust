//! Experimental PublishFeed adapter. Image bytes still use the CGI uploader.
//! Reference: https://qzone.apifox.cn/522044942e0.md (partially redacted schema).
use super::*;

const TRPC_BASE: &str = "https://h5.qzone.qq.com/http2rpc/gotrpc/v1";

impl QzoneClient {
    async fn trpc_request(
        &self,
        method: &str,
        command: &str,
        token_name: &str,
        body: &Value,
    ) -> Result<Value, QzoneError> {
        let token = if token_name == "bkn" {
            generate_gtk(
                self.cookies
                    .get("skey")
                    .ok_or_else(|| QzoneError::account("TRPC upload requires skey"))?,
            )
        } else {
            self.gtk.clone()
        };
        let response = self
            .client
            .post(format!("{TRPC_BASE}/{method}"))
            .query(&[(token_name, token)])
            .header("cookie", build_cookie_header(&self.cookies))
            .header("origin", "https://h5.qzone.qq.com")
            .header("referer", "https://h5.qzone.qq.com/")
            .header("x-web-qua", "V1_HT5_QZ_9.9.32_001_IDC_B")
            .header(
                "x-oidb",
                json!({"uint32_command": command, "uint32_service_type": "1"}).to_string(),
            )
            .json(body)
            .send()
            .await
            .map_err(|err| classify_reqwest_error("TRPC request", err))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| classify_reqwest_error("TRPC read body", err))?;
        if !status.is_success() {
            return Err(classify_http_status_with_body(
                "TRPC",
                status.as_u16(),
                &body,
            ));
        }
        let response: Value = serde_json::from_str(&body)
            .map_err(|_| QzoneError::unknown("invalid TRPC JSON response"))?;
        trpc_data(&response).cloned()
    }

    pub(super) async fn publish_trpc(
        &self,
        content: &str,
        images: &[Vec<u8>],
    ) -> Result<String, QzoneError> {
        if self.uin == 0 {
            return Err(QzoneError::account("TRPC publishing requires a valid uin"));
        }
        let begin = now_secs();
        let client_key = format!(
            "{}_{}",
            self.uin,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut media_items = Vec::new();
        let mut batch_id = String::new();
        let mut album_id = String::new();
        let mut upload_size = 0usize;
        if !images.is_empty() {
            let batch = self
                .trpc_request(
                    "trpc.qzone.media_upload_logic.BatchUpload/ApplyBatch",
                    "0x9773",
                    "bkn",
                    &json!({"userId": self.uin.to_string(), "batchNum": 1,
                    "photoNum": images.len(), "videoNum": 0, "bizType": 1,
                    "qzoneAlbumType": 7}),
                )
                .await?;
            batch_id = required_string(&batch, "batch_id")?;
            album_id = required_string(&batch, "album_id")?;
            for image in images {
                let (upload, size) = self
                    .upload_image_in_batch(image, Some((&album_id, &batch_id)))
                    .await?;
                upload_size += size;
                media_items.push(trpc_media_item(&upload, &batch_id, begin)?);
            }
            self.trpc_request(
                "trpc.qzone.media_upload_logic.BatchUpload/FinishBatch",
                "0x9774",
                "bkn",
                &json!({"userId": self.uin.to_string(), "albumId": album_id,
                    "batchId": batch_id, "clientKey": client_key,
                    "beginUploadTime": begin, "finishUploadTime": now_secs(),
                    "uploadSize": upload_size,
                    "succPhotoNum": images.len(), "succVideoNum": 0, "bizType": 1}),
            )
            .await?;
        }
        let payload = publish_payload(
            self.uin,
            content,
            begin,
            &client_key,
            &batch_id,
            &album_id,
            media_items,
        )?;
        let data = self
            .trpc_request(
                "trpc.qzone.feeds_writer.FeedsWriter/PublishFeed",
                "0x9775",
                "g_tk",
                &payload,
            )
            .await?;
        // Keep feed_id rather than substituting ugc_id. CGI edit compatibility
        // still needs verification with a real TRPC publication.
        trpc_remote_id(&data)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn required_string(data: &Value, key: &str) -> Result<String, QzoneError> {
    let value = field_to_string(data, key)?;
    if value.trim().is_empty() {
        return Err(QzoneError::unknown(format!("TRPC missing {key}")));
    }
    Ok(value)
}

fn trpc_data(response: &Value) -> Result<&Value, QzoneError> {
    let retcode = response
        .get("retcode")
        .and_then(Value::as_i64)
        .ok_or_else(|| QzoneError::unknown("TRPC response missing retcode"))?;
    for code in [
        Some(retcode),
        response.pointer("/error/code").and_then(Value::as_i64),
        response.pointer("/data/ret").and_then(Value::as_i64),
    ]
    .into_iter()
    .flatten()
    {
        if code != 0 {
            let message = response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .filter(|message| !message.is_empty())
                .or_else(|| response.get("message").and_then(Value::as_str))
                .unwrap_or("TRPC request failed");
            return Err(classify_json_response_error(code, message, response));
        }
    }
    response
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| QzoneError::unknown("TRPC response missing data"))
}

fn trpc_remote_id(data: &Value) -> Result<String, QzoneError> {
    let common = data
        .pointer("/feed/cell_common")
        .ok_or_else(|| QzoneError::unknown("TRPC publish response missing cell_common"))?;
    let tid = common
        .pointer("/union_id/feed_id")
        .and_then(Value::as_str)
        .and_then(usable_remote_id)
        .or_else(|| {
            common
                .get("feed_id")
                .and_then(Value::as_str)
                .and_then(usable_remote_id)
        });
    tid.ok_or_else(|| {
        QzoneError::unknown("TRPC publish response missing feed_id; publication outcome uncertain")
    })
}

fn trpc_media_item(upload: &Value, batch_id: &str, time: u64) -> Result<Value, QzoneError> {
    // Experimental mapping: the public schema redacts the nested image fields.
    // Validate the same upload result used by the stable CGI publisher.
    get_picbo_and_richval(upload)?;
    let data = &upload["data"];
    Ok(json!({"type": 1, "batch_id": batch_id, "upload_time": time,
        "image": {"sloc": required_string(data, "sloc")?,
            "lloc": required_string(data, "lloc")?,
            "width": required_string(data, "width")?.parse::<u32>()
                .map_err(|_| QzoneError::unknown("invalid TRPC image width"))?,
            "height": required_string(data, "height")?.parse::<u32>()
                .map_err(|_| QzoneError::unknown("invalid TRPC image height"))?,
            "url": required_string(data, "url")?}}))
}

fn publish_payload(
    uin: u64,
    content: &str,
    time: u64,
    client_key: &str,
    batch_id: &str,
    album_id: &str,
    media_items: Vec<Value>,
) -> Result<Value, QzoneError> {
    let batch_number = if media_items.is_empty() {
        0
    } else {
        batch_id
            .parse::<u64>()
            .map_err(|_| QzoneError::unknown("invalid TRPC batch_id"))?
    };
    Ok(json!({"feed": {
        "cell_common": {"time": time, "client_key": client_key,
            "feed_type": 1, "feed_sub_type": 1, "feed_biz_type": 0,
            "union_id": {"uf_key": "", "ugc_id": "", "feed_id": ""}},
        "cell_user_info": {"user": {"uin": uin.to_string(), "uid": "", "nick": ""}},
        "cell_summary": {"summary": [{"content": content, "type": 0}]},
        "cell_permission": {"right": {"flag": [5]}},
        "cell_media": {"upload_num": media_items.len(), "media_items": media_items,
            "batch_id": batch_number, "album_id": album_id}}, "from": 3}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_or_nested_failure_status() {
        for response in [
            json!({"data": {}}),
            json!({"retcode": -1, "data": {}}),
            json!({"retcode": 0, "error": {"code": -3000}, "data": {}}),
            json!({"retcode": 0, "data": {"ret": -1}}),
            json!({"retcode": 0}),
        ] {
            assert!(trpc_data(&response).is_err(), "{response}");
        }
        assert!(
            trpc_data(&json!({"retcode": 0, "error": {"code": 0}, "data": {"ret": 0}})).is_ok()
        );
    }

    #[test]
    fn requires_feed_id_not_ugc_id() {
        assert!(
            trpc_remote_id(&json!({"feed": {"cell_common": {"union_id": {"ugc_id": "other"}}}}))
                .is_err()
        );
        assert_eq!(
            trpc_remote_id(&json!({"feed": {"cell_common": {"union_id": {"feed_id": "tid"}}}}))
                .unwrap(),
            "tid"
        );
    }

    #[test]
    fn payload_preserves_text_and_batch_precision() {
        let body = publish_payload(
            10001,
            "中文\n\"test\"",
            123,
            "key",
            "1700000000000001",
            "album",
            vec![json!({"type": 1})],
        )
        .unwrap();
        assert_eq!(
            body.pointer("/feed/cell_summary/summary/0/content"),
            Some(&json!("中文\n\"test\""))
        );
        assert_eq!(
            body.pointer("/feed/cell_media/batch_id"),
            Some(&json!(1700000000000001_u64))
        );
        assert!(publish_payload(1, "", 1, "", "bad", "", vec![json!({})]).is_err());
    }
}
