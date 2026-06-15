use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use oqqwall_rust_core::StateView;
use oqqwall_rust_core::draft::{IngressAttachment, IngressMessage, MediaKind, MediaReference};
use oqqwall_rust_core::event::{Event, ReviewDecision, ReviewEvent};
use oqqwall_rust_core::ids::{IngressId, PostId, ReviewId};
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::{Duration, interval};

use crate::config::TelemetryConfig;
use crate::engine::EngineHandle;

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        if false {
            oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
        }
    }};
}

macro_rules! telemetry_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

const SAMPLE_SCHEMA_VERSION: u32 = 1;
const CHAT_CODEC: &str = "json";

pub fn spawn_submission_telemetry(
    handle: &EngineHandle,
    telemetry: &TelemetryConfig,
    data_dir: &str,
) -> Option<JoinHandle<()>> {
    if !telemetry.enabled {
        return None;
    }

    let local_dir = resolve_local_dir(data_dir, &telemetry.local_dir);
    let store = match TelemetryStore::open(local_dir, telemetry.upload_batch_size) {
        Ok(store) => store,
        Err(_err) => {
            debug_log!("telemetry disabled: init failed");
            return None;
        }
    };

    let runtime = TelemetryRuntime {
        state: handle.state(),
        rx: handle.subscribe(),
        store,
        config: telemetry.clone(),
        client: Client::new(),
    };
    Some(tokio::spawn(async move {
        runtime.run().await;
    }))
}

fn resolve_local_dir(data_dir: &str, local_dir: &str) -> PathBuf {
    let path = PathBuf::from(local_dir);
    if path.is_absolute() {
        path
    } else {
        Path::new(data_dir).join(path)
    }
}

struct TelemetryRuntime {
    state: Arc<RwLock<StateView>>,
    rx: tokio::sync::broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    store: TelemetryStore,
    config: TelemetryConfig,
    client: Client,
}

impl TelemetryRuntime {
    async fn run(mut self) {
        debug_log!(
            "telemetry started: dir={} upload_enabled={} endpoint_present={} interval_sec={} batch_size={}",
            self.store.root.display(),
            self.config.upload_enabled,
            self.config.upload_endpoint.is_some(),
            self.config.upload_interval_sec,
            self.config.upload_batch_size
        );
        let mut ticker = interval(Duration::from_secs(self.config.upload_interval_sec));
        loop {
            tokio::select! {
                recv = self.rx.recv() => {
                    match recv {
                        Ok(env) => self.handle_event(&env.event).await,
                        Err(RecvError::Closed) => break,
                        Err(RecvError::Lagged(_skipped)) => {
                            debug_log!("telemetry lagged");
                        }
                    }
                }
                _ = ticker.tick() => {
                    if let Err(err) = self.flush_uploads().await {
                        telemetry_log!("telemetry upload failed: {}", err);
                    }
                }
            }
        }
    }

    async fn handle_event(&mut self, event: &Event) {
        let Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision,
            decided_at_ms,
            ..
        }) = event
        else {
            return;
        };
        let samples = self.build_samples(*review_id, *decision, *decided_at_ms);
        if samples.is_empty() {
            return;
        }
        for (sample, chat_record) in samples {
            if let Err(_err) = self.store.enqueue(sample, &chat_record) {
                debug_log!("telemetry enqueue failed");
            }
        }
    }

    fn build_samples(
        &self,
        review_id: ReviewId,
        decision: ReviewDecision,
        decided_at_ms: i64,
    ) -> Vec<(PendingSample, ChatRecord)> {
        let state_guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let Some(base) = build_base_context(&state_guard, review_id, decided_at_ms) else {
            return Vec::new();
        };

        match decision {
            ReviewDecision::Approved => {
                let mut out = Vec::new();
                let positive = PendingSample::new(
                    &base,
                    1,
                    "none",
                    None,
                    &base.chat_record,
                    "approved",
                    decided_at_ms,
                );
                out.push((positive, base.chat_record.clone()));

                if let Some(truncated) = truncate_tail(&base.chat_record) {
                    let neg = PendingSample::new(
                        &base,
                        0,
                        "truncate_tail",
                        out.first().map(|(s, _)| s.sample_id.clone()),
                        &truncated,
                        "approved",
                        decided_at_ms,
                    );
                    out.push((neg, truncated));
                }

                if let Some(offtopic) =
                    append_offtopic(&state_guard, &base, self.config.max_append_messages)
                {
                    let neg = PendingSample::new(
                        &base,
                        0,
                        "append_offtopic",
                        out.first().map(|(s, _)| s.sample_id.clone()),
                        &offtopic,
                        "approved",
                        decided_at_ms,
                    );
                    out.push((neg, offtopic));
                }

                out
            }
            ReviewDecision::Rejected | ReviewDecision::Deleted => {
                let sample = PendingSample::new(
                    &base,
                    0,
                    "none",
                    None,
                    &base.chat_record,
                    match decision {
                        ReviewDecision::Rejected => "rejected",
                        _ => "deleted",
                    },
                    decided_at_ms,
                );
                vec![(sample, base.chat_record)]
            }
            ReviewDecision::Deferred | ReviewDecision::Skipped => Vec::new(),
        }
    }

    async fn flush_uploads(&mut self) -> Result<(), String> {
        if !self.config.upload_enabled {
            return Ok(());
        }
        let endpoint = match self.config.upload_endpoint.as_ref() {
            Some(endpoint) => endpoint,
            None => return Ok(()),
        };
        while self
            .store
            .upload_one_batch(endpoint, self.config.upload_token.as_deref(), &self.client)
            .await?
        {}
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct BaseSampleContext {
    review_id: ReviewId,
    review_code: u32,
    post_id: PostId,
    group_id: String,
    sender_id: String,
    chat_record: ChatRecord,
    post_ingress_set: HashSet<IngressId>,
    latest_message_ms: i64,
}

fn build_base_context(
    state: &StateView,
    review_id: ReviewId,
    decided_at_ms: i64,
) -> Option<BaseSampleContext> {
    let review = state.reviews.get(&review_id)?;
    let post = state.posts.get(&review.post_id)?;
    let ingress_ids = state.post_ingress.get(&review.post_id)?;
    if ingress_ids.is_empty() {
        return None;
    }

    let mut messages = Vec::new();
    let mut sender_id = None;
    let mut latest_message_ms = decided_at_ms;
    let mut ingress_set = HashSet::new();
    for ingress_id in ingress_ids {
        ingress_set.insert(*ingress_id);
        let Some(meta) = state.ingress_meta.get(ingress_id) else {
            continue;
        };
        let Some(message) = state.ingress_messages.get(ingress_id) else {
            continue;
        };
        sender_id = sender_id.or_else(|| Some(meta.user_id.clone()));
        if meta.received_at_ms > latest_message_ms {
            latest_message_ms = meta.received_at_ms;
        }
        messages.push(ChatMessage::from_ingress(
            *ingress_id,
            meta.platform_msg_id.clone(),
            meta.received_at_ms,
            message,
        ));
    }
    messages.sort_by_key(|msg| (msg.received_at_ms, msg.ingress_id.clone()));
    if messages.is_empty() {
        return None;
    }

    Some(BaseSampleContext {
        review_id,
        review_code: review.review_code,
        post_id: review.post_id,
        group_id: post.group_id.clone(),
        sender_id: sender_id.unwrap_or_else(|| "unknown".to_string()),
        chat_record: ChatRecord { messages },
        post_ingress_set: ingress_set,
        latest_message_ms,
    })
}

fn truncate_tail(chat: &ChatRecord) -> Option<ChatRecord> {
    if chat.messages.len() >= 2 {
        let mut shortened = chat.clone();
        shortened.messages.pop();
        return Some(shortened);
    }
    let first = chat.messages.first()?;
    if first.text.chars().count() <= 8 {
        return None;
    }
    let mut shortened = chat.clone();
    let keep = first.text.chars().count() / 2;
    shortened.messages[0].text = first.text.chars().take(keep.max(1)).collect();
    Some(shortened)
}

fn append_offtopic(
    state: &StateView,
    base: &BaseSampleContext,
    max_append_messages: usize,
) -> Option<ChatRecord> {
    if max_append_messages == 0 {
        return None;
    }
    let mut candidates: Vec<(i64, IngressId, ChatMessage)> = Vec::new();
    for (ingress_id, meta) in &state.ingress_meta {
        if meta.group_id != base.group_id || meta.user_id != base.sender_id {
            continue;
        }
        if meta.received_at_ms <= base.latest_message_ms {
            continue;
        }
        if base.post_ingress_set.contains(ingress_id) {
            continue;
        }
        let Some(message) = state.ingress_messages.get(ingress_id) else {
            continue;
        };
        candidates.push((
            meta.received_at_ms,
            *ingress_id,
            ChatMessage::from_ingress(
                *ingress_id,
                meta.platform_msg_id.clone(),
                meta.received_at_ms,
                message,
            ),
        ));
    }
    candidates.sort_by_key(|entry| (entry.0, entry.1.0));
    if candidates.is_empty() {
        return None;
    }

    let mut merged = base.chat_record.clone();
    for (_, _, message) in candidates.into_iter().take(max_append_messages) {
        merged.messages.push(message);
    }
    Some(merged)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatRecord {
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    ingress_id: String,
    platform_msg_id: String,
    received_at_ms: i64,
    text: String,
    attachments: Vec<ChatAttachment>,
}

impl ChatMessage {
    fn from_ingress(
        ingress_id: IngressId,
        platform_msg_id: String,
        received_at_ms: i64,
        message: &IngressMessage,
    ) -> Self {
        let attachments = message
            .attachments
            .iter()
            .map(ChatAttachment::from_ingress)
            .collect();
        Self {
            ingress_id: ingress_id.0.to_string(),
            platform_msg_id,
            received_at_ms,
            text: message.text.clone(),
            attachments,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatAttachment {
    kind: String,
    name: Option<String>,
    reference_type: String,
    reference: String,
    size_bytes: Option<u64>,
}

impl ChatAttachment {
    fn from_ingress(attachment: &IngressAttachment) -> Self {
        let (reference_type, reference) = match &attachment.reference {
            MediaReference::RemoteUrl { url } => ("remote_url".to_string(), url.clone()),
            MediaReference::Blob { blob_id } => ("blob_id".to_string(), blob_id.0.to_string()),
        };
        Self {
            kind: media_kind_name(attachment.kind).to_string(),
            name: attachment.name.clone(),
            reference_type,
            reference,
            size_bytes: attachment.size_bytes,
        }
    }
}

fn media_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::File => "file",
        MediaKind::Audio => "audio",
        MediaKind::Other => "other",
        MediaKind::Sticker => "sticker",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingSample {
    sample_id: String,
    schema_version: u32,
    label: u8,
    augmentation: String,
    base_sample_id: Option<String>,
    label_source: String,
    decision_at_ms: i64,
    review_id: String,
    review_code: u32,
    post_id: String,
    group_id: String,
    sender_id: String,
    chat_record_hash: String,
    message_count: usize,
}

impl PendingSample {
    fn new(
        ctx: &BaseSampleContext,
        label: u8,
        augmentation: &str,
        base_sample_id: Option<String>,
        chat_record: &ChatRecord,
        label_source: &str,
        decision_at_ms: i64,
    ) -> Self {
        let chat_record_hash = hash_chat_record(chat_record);
        let sample_id = hash_text(&format!(
            "{}:{}:{}:{}:{}:{}:{}",
            ctx.review_id.0,
            ctx.post_id.0,
            label,
            augmentation,
            label_source,
            decision_at_ms,
            chat_record_hash
        ));
        Self {
            sample_id,
            schema_version: SAMPLE_SCHEMA_VERSION,
            label,
            augmentation: augmentation.to_string(),
            base_sample_id,
            label_source: label_source.to_string(),
            decision_at_ms,
            review_id: ctx.review_id.0.to_string(),
            review_code: ctx.review_code,
            post_id: ctx.post_id.0.to_string(),
            group_id: ctx.group_id.clone(),
            sender_id: ctx.sender_id.clone(),
            chat_record_hash,
            message_count: chat_record.messages.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatObjectEntry {
    chat_record_hash: String,
    codec: String,
    message_count: usize,
    payload: ChatRecord,
}

impl ChatObjectEntry {
    fn from_record(chat_record_hash: String, payload: ChatRecord) -> Self {
        Self {
            message_count: payload.messages.len(),
            chat_record_hash,
            codec: CHAT_CODEC.to_string(),
            payload,
        }
    }
}

#[derive(Debug, Serialize)]
struct UploadBatchRequest {
    batch_id: String,
    schema_version: u32,
    chat_objects: Vec<ChatObjectEntry>,
    samples: Vec<PendingSample>,
}

struct TelemetryStore {
    root: PathBuf,
    objects_dir: PathBuf,
    pending_samples_file: PathBuf,
    batch_size: usize,
}

impl TelemetryStore {
    fn open(root: PathBuf, batch_size: usize) -> Result<Self, String> {
        let objects_dir = root.join("chat_objects");
        fs::create_dir_all(&objects_dir)
            .map_err(|err| format!("create telemetry objects dir failed: {}", err))?;
        fs::create_dir_all(&root).map_err(|err| format!("create telemetry dir failed: {}", err))?;
        let pending_samples_file = root.join("pending_samples.jsonl");
        if !pending_samples_file.exists() {
            File::create(&pending_samples_file)
                .map_err(|err| format!("create pending sample file failed: {}", err))?;
        }
        Ok(Self {
            root,
            objects_dir,
            pending_samples_file,
            batch_size,
        })
    }

    fn enqueue(&self, sample: PendingSample, chat_record: &ChatRecord) -> Result<(), String> {
        self.persist_chat_object_if_missing(&sample.chat_record_hash, chat_record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.pending_samples_file)
            .map_err(|err| format!("open pending sample file failed: {}", err))?;
        let line = serde_json::to_string(&sample)
            .map_err(|err| format!("encode pending sample failed: {}", err))?;
        file.write_all(line.as_bytes())
            .map_err(|err| format!("write pending sample failed: {}", err))?;
        file.write_all(b"\n")
            .map_err(|err| format!("write pending sample newline failed: {}", err))?;
        Ok(())
    }

    fn persist_chat_object_if_missing(
        &self,
        chat_record_hash: &str,
        chat_record: &ChatRecord,
    ) -> Result<(), String> {
        let object_path = self.object_path(chat_record_hash);
        if object_path.exists() {
            return Ok(());
        }
        let entry = ChatObjectEntry::from_record(chat_record_hash.to_string(), chat_record.clone());
        let bytes = serde_json::to_vec(&entry)
            .map_err(|err| format!("encode chat object failed: {}", err))?;
        let tmp_path = object_path.with_extension("json.tmp");
        fs::write(&tmp_path, bytes)
            .map_err(|err| format!("write chat object tmp failed: {}", err))?;
        fs::rename(&tmp_path, &object_path)
            .map_err(|err| format!("move chat object file failed: {}", err))?;
        Ok(())
    }

    async fn upload_one_batch(
        &mut self,
        endpoint: &str,
        upload_token: Option<&str>,
        client: &Client,
    ) -> Result<bool, String> {
        let all_samples = self.read_pending_samples()?;
        if all_samples.len() < self.batch_size {
            return Ok(false);
        }
        let mut batch_samples = Vec::with_capacity(self.batch_size);
        for sample in all_samples.iter().take(self.batch_size) {
            batch_samples.push(sample.clone());
        }
        let remaining_samples: Vec<PendingSample> =
            all_samples.into_iter().skip(self.batch_size).collect();

        let mut hashes = HashSet::new();
        for sample in &batch_samples {
            hashes.insert(sample.chat_record_hash.clone());
        }
        let mut chat_objects = Vec::new();
        for hash in hashes {
            let object = self.read_chat_object(&hash)?;
            chat_objects.push(object);
        }

        let batch_id = format!("b{}_{}", now_ms(), random_u64());
        let request = UploadBatchRequest {
            batch_id: batch_id.clone(),
            schema_version: SAMPLE_SCHEMA_VERSION,
            chat_objects,
            samples: batch_samples,
        };
        telemetry_log!(
            "telemetry upload start: batch_id={} endpoint={} samples={} chat_objects={}",
            batch_id,
            endpoint,
            request.samples.len(),
            request.chat_objects.len()
        );
        let mut req = client
            .post(endpoint)
            .header("Idempotency-Key", batch_id)
            .json(&request);
        if let Some(token) = upload_token {
            req = req.bearer_auth(token);
        }
        let response = req
            .send()
            .await
            .map_err(|err| format!("telemetry post request failed: {}", err))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<read body failed>".to_string());
            return Err(format!(
                "telemetry post failed: status={} body={}",
                status, body
            ));
        }

        self.rewrite_pending_samples(&remaining_samples)?;
        self.cleanup_unreferenced_objects(&remaining_samples)?;
        telemetry_log!(
            "telemetry upload success: batch_id={} samples={} remaining_samples={}",
            request.batch_id,
            request.samples.len(),
            remaining_samples.len()
        );
        Ok(true)
    }

    fn read_pending_samples(&self) -> Result<Vec<PendingSample>, String> {
        let file = File::open(&self.pending_samples_file)
            .map_err(|err| format!("open pending sample file failed: {}", err))?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|err| format!("read pending sample line failed: {}", err))?;
            if line.trim().is_empty() {
                continue;
            }
            let sample: PendingSample = serde_json::from_str(&line)
                .map_err(|err| format!("decode pending sample line failed: {}", err))?;
            out.push(sample);
        }
        Ok(out)
    }

    fn rewrite_pending_samples(&self, samples: &[PendingSample]) -> Result<(), String> {
        let tmp_path = self.pending_samples_file.with_extension("jsonl.tmp");
        let mut file = File::create(&tmp_path)
            .map_err(|err| format!("create pending sample temp failed: {}", err))?;
        for sample in samples {
            let line = serde_json::to_string(sample)
                .map_err(|err| format!("encode pending sample line failed: {}", err))?;
            file.write_all(line.as_bytes())
                .map_err(|err| format!("write pending sample line failed: {}", err))?;
            file.write_all(b"\n")
                .map_err(|err| format!("write pending sample newline failed: {}", err))?;
        }
        file.sync_all()
            .map_err(|err| format!("sync pending sample temp failed: {}", err))?;
        fs::rename(&tmp_path, &self.pending_samples_file)
            .map_err(|err| format!("replace pending sample file failed: {}", err))?;
        Ok(())
    }

    fn cleanup_unreferenced_objects(&self, samples: &[PendingSample]) -> Result<(), String> {
        let mut needed = HashSet::new();
        for sample in samples {
            needed.insert(sample.chat_record_hash.clone());
        }
        let entries = fs::read_dir(&self.objects_dir)
            .map_err(|err| format!("read objects dir failed: {}", err))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("read object entry failed: {}", err))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            if needed.contains(stem) {
                continue;
            }
            if let Err(_err) = fs::remove_file(&path) {
                debug_log!("cleanup telemetry object failed: {}", path.display());
            }
        }
        Ok(())
    }

    fn read_chat_object(&self, chat_record_hash: &str) -> Result<ChatObjectEntry, String> {
        let path = self.object_path(chat_record_hash);
        let bytes = fs::read(&path)
            .map_err(|err| format!("read chat object {} failed: {}", path.display(), err))?;
        let object = serde_json::from_slice::<ChatObjectEntry>(&bytes)
            .map_err(|err| format!("decode chat object {} failed: {}", path.display(), err))?;
        Ok(object)
    }

    fn object_path(&self, chat_record_hash: &str) -> PathBuf {
        self.objects_dir.join(format!("{}.json", chat_record_hash))
    }
}

fn hash_chat_record(chat_record: &ChatRecord) -> String {
    let bytes = serde_json::to_vec(chat_record).unwrap_or_default();
    hash_bytes(&bytes)
}

fn hash_text(text: &str) -> String {
    hash_bytes(text.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn random_u64() -> u64 {
    let mut rng = rand::thread_rng();
    rng.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oqqwall_rust_core::ids::Id128;
    use oqqwall_rust_core::state::IngressMeta;

    fn sample_chat_record() -> ChatRecord {
        ChatRecord {
            messages: vec![
                ChatMessage {
                    ingress_id: "1".to_string(),
                    platform_msg_id: "m1".to_string(),
                    received_at_ms: 1000,
                    text: "first".to_string(),
                    attachments: Vec::new(),
                },
                ChatMessage {
                    ingress_id: "2".to_string(),
                    platform_msg_id: "m2".to_string(),
                    received_at_ms: 2000,
                    text: "second".to_string(),
                    attachments: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn truncate_tail_removes_last_message() {
        let chat = sample_chat_record();
        let truncated = truncate_tail(&chat).expect("truncated");
        assert_eq!(truncated.messages.len(), 1);
        assert_eq!(truncated.messages[0].text, "first");
    }

    #[test]
    fn append_offtopic_appends_following_messages_from_same_sender() {
        let mut state = StateView::default();
        state.ingress_meta.insert(
            Id128(11),
            IngressMeta {
                profile_id: "p".to_string(),
                chat_id: "c".to_string(),
                user_id: "sender".to_string(),
                sender_name: Some("S".to_string()),
                group_id: "10001".to_string(),
                platform_msg_id: "m11".to_string(),
                received_at_ms: 3000,
            },
        );
        state.ingress_messages.insert(
            Id128(11),
            IngressMessage {
                text: "offtopic".to_string(),
                attachments: Vec::new(),
            },
        );
        state.ingress_meta.insert(
            Id128(12),
            IngressMeta {
                profile_id: "p".to_string(),
                chat_id: "c".to_string(),
                user_id: "other".to_string(),
                sender_name: Some("O".to_string()),
                group_id: "10001".to_string(),
                platform_msg_id: "m12".to_string(),
                received_at_ms: 4000,
            },
        );
        state.ingress_messages.insert(
            Id128(12),
            IngressMessage {
                text: "should_not_append".to_string(),
                attachments: Vec::new(),
            },
        );

        let base = BaseSampleContext {
            review_id: Id128(1),
            review_code: 101,
            post_id: Id128(2),
            group_id: "10001".to_string(),
            sender_id: "sender".to_string(),
            chat_record: sample_chat_record(),
            post_ingress_set: HashSet::new(),
            latest_message_ms: 2500,
        };

        let merged = append_offtopic(&state, &base, 2).expect("merged");
        assert_eq!(merged.messages.len(), 3);
        assert_eq!(merged.messages[2].text, "offtopic");
    }
}
