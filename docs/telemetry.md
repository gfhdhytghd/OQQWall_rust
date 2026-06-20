# telemetry.md — OQQWall_RUST 投稿完整性遥测与训练样本上传

> 适用范围：当前 `crates/app/src/telemetry.rs` 实现。  
> 目标：把“审核结果”转成可训练样本，并支持本地缓存 + 批量上传（20 条/批）。
>
> 服务端落地见 `docs/telemetry_collector.md`（独立 `telemetry-collector` 二进制）。

说明：当前主程序二进制只包含“样本生成 + 上传客户端”，不包含遥测接收 API。

---

## 1. 触发点与标签定义

遥测监听事件总线，仅在收到 `ReviewDecisionRecorded` 时生成样本：

- 正样本（`label=1`）：`ReviewDecision::Approved`
- 负样本（`label=0`）：`ReviewDecision::Rejected` / `ReviewDecision::Deleted`
- 忽略：`Deferred` / `Skipped`

样本基础上下文来自状态快照中的：

- `reviews[review_id] -> post_id/review_code`
- `posts[post_id] -> group_id`
- `post_ingress[post_id]` + `ingress_meta` + `ingress_messages`

---

## 2. 负样本构造策略（当前实现）

当决策为 `Approved` 时，除了 1 条正样本外，还会额外生成负样本：

1. `truncate_tail`
- 对完整聊天记录截断尾部（优先删最后一条消息）。
- 若只有一条消息，退化为文本截断（保留前半段）。

2. `append_offtopic`
- 选取同 `group_id + sender_id`、时间晚于当前投稿最后消息、且不属于本投稿的后续消息拼接。
- 最多拼接 `max_append_messages` 条（配置项控制，默认 2）。

当决策为 `Rejected/Deleted` 时，仅生成 1 条负样本（`augmentation=none`）。

---

## 3. 本地落盘结构

默认目录（相对 `OQQWALL_DATA_DIR`）：

```
data/telemetry/
  pending_samples.jsonl
  chat_objects/
    <chat_record_hash>.json
```

说明：

- `pending_samples.jsonl`：待上传样本队列（按行 JSON）。
- `chat_objects/*.json`：完整聊天对象，按 `chat_record_hash` 去重存储。
- 上传成功后：
  - 已上传样本会从 `pending_samples.jsonl` 删除
  - 无引用的 `chat_objects` 文件会被清理

---

## 4. 上传行为

上传线程按 `upload_interval_sec` 周期执行：

- 仅当 `upload_enabled=true` 时尝试上传
- 仅当待上传样本数 `>= upload_batch_size` 才发送
- 当前实现 `upload_batch_size` 固定为 `20`（配置会被钳制到 20）
- 上传开始、成功、失败都会写入运行日志

请求特征：

- `POST <builtin upload endpoint>`
- Header:
  - `Idempotency-Key: b<timestamp>_<random>`
  - 附加内置固定的 `Authorization: Bearer <builtin token>`
- Body:
  - `batch_id`
  - `schema_version`
  - `chat_objects[]`（去重后的完整聊天对象）
  - `samples[]`（正好 20 条样本）

ACK 语义（当前实现）：

- 只要 HTTP 状态码是 2xx，视为整批成功，删除该批样本
- 非 2xx 或网络错误，整批保留，等待下次重试

---

## 5. 样本字段（核心）

每条 `sample` 主要字段：

- `sample_id`
- `schema_version`
- `label`
- `augmentation`（`none` / `truncate_tail` / `append_offtopic`）
- `base_sample_id`（增强样本指向基样本）
- `label_source`（`approved/rejected/deleted`）
- `decision_at_ms`
- `review_id` / `review_code` / `post_id`
- `group_id` / `sender_id`
- `chat_record_hash`
- `message_count`

每条 `chat_object` 主要字段：

- `chat_record_hash`
- `codec`（当前固定 `json`）
- `message_count`
- `payload.messages[]`（完整聊天原文 + 附件元信息）

---

## 6. 配置

参见 `docs/config.md` 的 `common.telemetry.*` 小节。关键项：

- `common.telemetry.enabled`
- `common.telemetry.local_dir`
- `common.telemetry.upload_enabled`
- `common.telemetry.upload_interval_sec`
- `common.telemetry.upload_batch_size`（当前实现固定 20）
- `common.telemetry.max_append_messages`

上传 endpoint / token 为客户端内置固定值，不通过配置文件或环境变量暴露。

---

## 7. 验证与测试

本仓库内已覆盖的关键单测：

- `truncate_tail_removes_last_message`
- `append_offtopic_appends_following_messages_from_same_sender`

Web API 相关回归测试也已修复并通过（`create_rendered_post_*`）。

---

## 8. 服务端部署建议

当前客户端会上传到程序内置的 collector 目标；如需调整目标地址或 token，需要改代码后重新编译。
服务端启动与运维细节参见 `docs/telemetry_collector.md`。
