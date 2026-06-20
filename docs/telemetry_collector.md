# telemetry_collector.md — 遥测采集服务端（独立进程）

> 目标：接收 OQQWall_RUST 上传的投稿训练样本，做幂等入库、对象落盘、样本管理和 Parquet 导出。

---

## 1. 架构与职责

`crates/telemetry-collector` 是独立二进制，不与主程序 `OQQWall_RUST` 共享进程。

- 上传入口：`POST /telemetry/v1/submission/batch`
- 存储：PostgreSQL（元数据）+ 本地对象目录（chat object 原文）
- 认证：Bearer Token + RBAC
- 管理：批次/样本查询、样本修订、token 管理、导出任务管理

---

## 2. 环境变量

- `COLLECTOR_HTTP_ADDR`：监听地址（默认 `0.0.0.0:10925`）
- `COLLECTOR_PG_DSN`：Postgres 连接串（必填）
- `COLLECTOR_OBJECT_DIR`：聊天对象目录（默认 `data/collector/objects`）
- `COLLECTOR_EXPORT_DIR`：导出目录（默认 `data/collector/exports`）
- `COLLECTOR_BOOTSTRAP_ROOT_TOKEN`：启动时写入/更新 `root` token（必填，>=16）
- `COLLECTOR_MAX_BODY_MB`：请求体上限（默认 10）
- `RUST_LOG`：日志级别

说明：

- `root` token 每次启动都会按 `COLLECTOR_BOOTSTRAP_ROOT_TOKEN` 覆盖更新。
- 生产环境建议把 `COLLECTOR_BOOTSTRAP_ROOT_TOKEN` 放入 secret 管理，不写入仓库。

---

## 3. 上传协议与幂等

与主程序当前上传协议兼容：

- Header
  - `Authorization: Bearer <token>`
  - `Idempotency-Key: <key>`
- Body
  - `batch_id`
  - `schema_version`（当前只接受 `1`）
  - `chat_objects[]`
  - `samples[]`

幂等规则：

- 相同 `Idempotency-Key` + 相同 payload hash：返回 `200`，`duplicate=true`
- 相同 key + 不同 payload：返回 `409 IDEMPOTENCY_CONFLICT`
- 首次成功写入：返回 `201`

客户端仍按既有语义处理：只要收到 `2xx` 就视为整批 ACK。

### 3.1 上传前校验（服务端）

- `schema_version == 1`
- `samples` 非空且不超过上限
- `label` 只能是 `0/1`
- `sample.chat_record_hash` 必须能在本批 `chat_objects` 或历史对象中解析
- `chat_record_hash` 必须与 `payload` 计算结果一致

### 3.2 上传响应示例

首次成功：

```json
{
  "ingested": true,
  "duplicate": false,
  "batch_id": "b_test_001",
  "accepted_samples": 20,
  "accepted_chat_objects": 14,
  "request_id": "..."
}
```

幂等重试：

```json
{
  "ingested": false,
  "duplicate": true,
  "batch_id": "b_test_001",
  "accepted_samples": 0,
  "accepted_chat_objects": 0,
  "request_id": "..."
}
```

---

## 4. 权限与 API

### 4.1 权限矩阵

- `ingest.write`：允许上传批次
- `batches.read`：允许查看批次列表/详情
- `samples.read`：允许查看样本与聊天对象
- `samples.write`：允许修订样本（排除/纠正标签/备注）
- `exports.manage`：允许创建与下载导出任务
- `tokens.manage`：允许创建/删除 API token

### 4.2 接口清单

- `GET /telemetry/v1/healthz`
- `POST /telemetry/v1/submission/batch`
- `GET /telemetry/v1/batches`
- `GET /telemetry/v1/batches/{batch_id}`
- `GET /telemetry/v1/samples`
- `GET /telemetry/v1/samples/{sample_id}`
- `PATCH /telemetry/v1/samples/{sample_id}`
- `GET /telemetry/v1/chat_objects/{chat_record_hash}`
- `POST /telemetry/v1/exports`
- `GET /telemetry/v1/exports`
- `GET /telemetry/v1/exports/{job_id}`
- `GET /telemetry/v1/exports/{job_id}/manifest`
- `GET /telemetry/v1/exports/{job_id}/files/{name}`
- `POST /telemetry/v1/admin/tokens`
- `DELETE /telemetry/v1/admin/tokens/{token_id}`

### 4.3 常用 API 示例

创建 token：

```bash
curl -X POST http://127.0.0.1:10925/telemetry/v1/admin/tokens \
  -H "Authorization: Bearer <root_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "token_id": "ingest_bot",
    "permissions": ["ingest.write"],
    "note": "oqqwall uploader"
  }'
```

查询样本：

```bash
curl "http://127.0.0.1:10925/telemetry/v1/samples?limit=50&label=1" \
  -H "Authorization: Bearer <token>"
```

修订样本：

```bash
curl -X PATCH "http://127.0.0.1:10925/telemetry/v1/samples/<sample_id>" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "excluded": true,
    "corrected_label": 0,
    "note": "human corrected"
  }'
```

创建导出任务：

```bash
curl -X POST "http://127.0.0.1:10925/telemetry/v1/exports" \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "format": "parquet",
    "from_decision_at_ms": 1700000000000,
    "to_decision_at_ms": 1800000000000,
    "labels": [0, 1],
    "include_excluded": false
  }'
```

---

## 5. 导出格式

导出任务默认 `parquet`，按目录分区：

```text
<export_root>/<job_id>/
  manifest.json
  decision_date=YYYY-MM-DD/
    label=0/part-*.parquet
    label=1/part-*.parquet
```

`manifest.json` 包含：

- `job_id`
- `schema_version`
- `row_count`
- `filters`
- `files[]`（path + row_count + sha256）

训练端建议以 `manifest.json` 作为任务入口，不直接硬编码文件路径。

---

## 6. Docker 运行

`Dockerfile.telemetry-collector` 采用“宿主机先编译、镜像只打包二进制”的方式。

### 6.0 先构建 Linux 兼容二进制

```bash
docker run --rm --network host \
  -v "$PWD:/work" -w /work \
  -v "$HOME/.cargo/registry:/root/.cargo/registry" \
  -v "$HOME/.cargo/git:/root/.cargo/git" \
  rust-glibc231:20.04-oqqwall \
  bash -lc 'CARGO_TARGET_DIR=/work/out-target cargo build --release -p oqqwall_telemetry_collector --bin telemetry-collector && cp /work/out-target/release/telemetry-collector /work/out/telemetry-collector'
```

### 6.1 使用 compose（推荐）

```bash
docker compose -f docker-compose.telemetry.yml up -d --build
```

服务：

- Postgres：`postgres:16`
- Collector：`telemetry-collector`（端口 `10925`）

### 6.2 健康检查

```bash
curl http://127.0.0.1:10925/telemetry/v1/healthz
```

### 6.3 与主程序对接

在 OQQWall_RUST 的 `config.json`：

- `common.telemetry.upload_enabled = true`
- 上传 endpoint / token 为主程序内置固定值，不再通过 `config.json` 暴露
- 如需改成自建 collector，请调整主程序内置 telemetry endpoint / token 后重新编译

---

## 7. 运维与排障

### 7.1 常见错误

- `401 UNAUTHORIZED`：token 缺失/无效/过期
- `403 FORBIDDEN`：token 权限不足
- `409 IDEMPOTENCY_CONFLICT`：同一 `Idempotency-Key` 对应了不同请求体
- `400 BAD_REQUEST`：样本字段或 hash 校验失败

### 7.2 快速自检命令

```bash
# collector 存活
curl -sS http://127.0.0.1:10925/telemetry/v1/healthz

# 批次是否持续写入
curl -sS -H "Authorization: Bearer <token>" \
  http://127.0.0.1:10925/telemetry/v1/batches

# 查看最近日志
docker logs --tail 200 oqqwall-telemetry-collector
```

### 7.3 备份建议

建议同时备份：

- Postgres 数据卷
- `COLLECTOR_OBJECT_DIR`
- `COLLECTOR_EXPORT_DIR`

说明：`samples` 依赖 `chat_objects` 原文，不能只备份数据库。

---

## 8. 数据保留

当前实现不做自动 TTL 清理（永久保留）。如需清理，请通过外部任务按业务策略执行。
