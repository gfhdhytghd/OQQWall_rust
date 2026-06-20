> 版本：v0.1（单机 MVP → 完整单机 → 未来集群预留）
> 范围：**单机**实现 OQQWall 全功能链路；**不实现 AI/LLM**；“智能处理”全部用**时间驱动 if-else**（聚合/排程/重试/冷却）。
> 约束：单 Rust 二进制；尽量避免外部服务；尽量避免硬盘读；硬盘主要用于 **append-only 日志/WAL 与产物备份**。
> 定位：总体指导；具体落地细节见 `docs/engineering.md`。

---
注：原版OQQWall仓库在～/data/OQQWall

# 目录

1. 背景与目标
2. 非目标与约束
3. 路线选择原理（为什么选事件溯源 + 函数式 core）
4. 总体架构（Functional Core / Imperative Shell）
5. 模块划分与职责
6. 数据模型与状态机（事件、StateView、Reducer）
7. Command → Events 决策层（纯函数）
8. Drivers（副作用执行器：渲染/发审核/发空间/附件下载）
9. 存储与 IO 策略（“常规只写不读”）
10. NapCat/OneBot 集成与自愈策略
11. 渲染体系（PNG-only）
12. 审核体系（群指令 + 幂等）
13. 调度与发送（时间 if-else 排程）
14. 可观测性、运维与故障演练
15. 测试策略（单元/属性/集成）
16. 未来集群扩展点（现在不做，但接口预留）
17. 推荐里程碑与任务拆分

---

# 1. 背景与目标

## 1.1 单机版要完成的功能（覆盖 OQQWall 全链路）

* 从 OneBot（NapCat）接收投稿（群/私聊）。
* **按时间窗口聚合投稿**（`process_waittime_sec` 语义）。
* 生成稿件（最小分段：按消息/空行/标点），不做 AI/内容识别。
* 渲染稿件：**默认输出 PNG**，用于审核预览与最终发送。
* 把预览发到审核群，并生成短码 `review_code`（仅审核内部使用，不对外展示）。
* 管理员在审核群用指令：通过/拒绝/延后/改路由/立即发/查看。
* 调度系统按 **发送窗口/最小间隔/队列上限/账号冷却** 做排队与发送。
* 多账号组支持：不同账号组可有不同的审核群、发送窗口与账号列表。
* QQ 空间发送：**单写者**（同一时刻只发一条），失败重试与人工介入。

## 1.2 关键质量目标

* **可恢复**：崩溃/重启后，能通过快照+日志回放恢复状态，继续跑。
* **幂等**：重复消息、重复指令、重复 tick 不会造成重复发稿/重复发送。
* **低 IO**：运行期主要在内存中推进；磁盘以“追加写”为主；读盘主要发生在启动恢复。
* **未来可集群**：现在只做单机，但核心接口与边界要为未来复制日志/分布式 lease/多副本 blob 做准备。

---

# 2. 非目标与约束

## 2.1 非目标（本阶段明确不做）

* 不做 LLM / embedding / 贝叶斯 / 正则智能处理（后续可加，但现在不包含）。
* 不做真正的多机集群（只做接口预留与结构兼容）。
* 不保证 QQ/NapCat 的“多机同号多活”可行（这受平台限制；单机先落地）。

## 2.2 约束

* **单 Rust 二进制**：业务逻辑、渲染、web UI、调度都在一个产物里。
* 尽量不依赖外部服务（NATS/etcd/MinIO/Kafka 等不引入）。
* 尽量避免磁盘读；磁盘主要作 WAL/快照/产物备份（写多读少）。
* 兼容未来集群：关键组件必须有 trait 边界（Bus/Journal/MetaStore/BlobStore/Lease）。

---

# 3. 路线选择原理（为什么这样设计）

## 3.1 为什么选“事件溯源（Event Sourcing）”

因为你明确希望：

* 常规流程不读盘，硬盘只作为恢复数据源；
* 未来加集群要无单点、可重放。

事件溯源天然适配：

* 运行时：状态在内存 `StateView`；每次状态变化只需**追加写事件日志**；
* 恢复时：读快照 + 回放日志重建；
* 未来集群：把“本地追加写日志”升级成“复制日志”即可。

## 3.2 为什么选“Functional Core / Imperative Shell”

你希望尽可能函数式。将系统分为：

* **Functional Core（纯函数）**：`StateView + reducer + deciders`

  * `(state, command) -> events`
  * `(state, event) -> state'`
  * 无 IO，无 now()，可重放、可单测、可属性测试
* **Imperative Shell（副作用）**：渲染/发消息/下载附件/发空间

  * 只消费请求事件，执行 IO，产出完成/失败事件

这样能确保：

* 正确性与可测试性强；
* 幂等更容易保证；
* 将来集群化时，把 IO 任务分发出去不会改动 core 的逻辑。

## 3.3 为什么默认 PNG

渲染是单机 CPU/内存大头。PNG-only 的路线：

* 渲染产物与审核预览一致，避免 SVG/PNG 双路径带来的偏差；
* 统一走 PNG 输出，便于下游直接发送；
* PNG 渲染放入单独 blocking 线程池，队列限长，避免拖垮主流程。

---

# 4. 总体架构（单进程，模块化）

```
┌─────────────────────────────── OQQWall_RUST ───────────────────────────────┐
│ Functional Core                                                        │
│  - Event types + Envelope                                              │
│  - Reducer: (StateView, Event) -> StateView'                           │
│  - Deciders: (StateView, Command) -> Vec<Event>                         │
│                                                                         │
│ Imperative Shell                                                       │
│  - Journal writer (append-only)                                        │
│  - LocalBus (in-proc subscriptions)                                    │
│  - Drivers: MediaFetcher / Renderer / AuditPublisher / QzoneSender      │
│  - NapCat daemon + OneBot I/O                                          │
│  - Admin Web UI (optional)                                             │
└─────────────────────────────────────────────────────────────────────────┘
```

---

# 5. 模块划分与职责（单机）

## 5.1 核心（core）

* `core/event.rs`：所有事件类型枚举 + Envelope + ID 类型
* `core/state.rs`：StateView（内存索引结构）+ reducer（纯）
* `core/decide/*.rs`：deciders（纯），处理 Command/tick/指令并产出事件

## 5.2 基础设施（infra）

* `infra/journal.rs`：追加写日志 + 分段 + flush 策略 + snapshot
* `infra/bus.rs`：in-proc pub/sub（mpsc + fanout）
* `infra/blob.rs`：RAM cache + 异步持久化备份（写多读少）
* `infra/lease.rs`：单机 lease（互斥 + TTL，未来可换分布式）

## 5.3 Drivers（副作用）

* `drivers/onebot.rs`：OneBot 客户端（收/发）
* `drivers/napcatd.rs`：NapCat 子进程守护（拉起/健康检查/重启退避）
* `drivers/media_fetcher.rs`：下载附件 URL → blob
* `drivers/renderer.rs`：draft → PNG
* `drivers/audit_publisher.rs`：发审核群消息（请求→完成/失败）
* `drivers/qzone_sender.rs`：发空间（请求→完成/失败）

## 5.4 API/运维（可选）

* `admin/web.rs`：本地 web UI（列出稿件、展示 PNG、手工操作）
* `metrics.rs`：指标与 tracing

---

# 6. 数据模型与状态机（事件、StateView、Reducer）

## 6.1 事件分组（建议保持 “Requested/Done/Failed” 对称）

* System/Config/Snapshot
* NapCat/OneBot health
* Blob registry（引用计数、持久化、GC）
* Ingress（消息接入、去重、落入 ingress store）
* Aggregator（session open/append/close）
* Draft（PostDraftCreated/Edited/Routed）
* Render（RenderRequested/PngReady/Failed）
* Review（ReviewItemCreated/ReviewPublishRequested/Published/Decision…）
* Scheduling/Queue（SendPlanCreated/Rescheduled/…）
* Sending（SendStarted/Succeeded/Failed/GaveUp）
* Manual intervention（Required/Resolved）
* Account runtime（cooldown/enable/disable/last_send）
* GC/Archive/Delete

> **原则**：任何涉及 IO 返回值（msg_id、qzone_post_id、下载后的 blob_id）必须走 **Requested → Ready/Failed**，否则无法保持 decider 纯函数。

## 6.2 StateView 内存索引结构（可恢复）

核心思想：StateView 是事件回放结果（可重建），运行期所有查询都只读 StateView。

必备索引：

* `ingress_seen`（去重）+ `ingress store`（消息内容/附件状态）
* `sessions` + `session_by_key` + `session_close_index`
* `input_status`（按 `(chat,user)` 记录最新状态与时间戳，用于延迟成稿）
* `posts` + `posts_by_stage`
* `drafts` / `render` / `review` / `send_plans` / `sending`
* `review_by_code` / `review_by_audit_msg`
* `send_due_index`（not_before + seq 排序）
* `accounts` / `group_runtime`（last_send 等）
* `blobs`（refcount/persisted_path）

## 6.3 Reducer 的关键不变量（必须严格）

* 幂等：重复 event replay 不应导致结构崩坏（必要时允许 “同值 no-op”）
* session：关闭后不可 append；close_index 必须同步移除
* post：同 post_id 不可重复创建 draft；stage index 同步更新
* review_code 全局唯一；audit_msg_id 不可映射到多个 post
* sending：单机先限定“全局单 in-flight”，避免并发发送风险

---

# 7. Command → Events 决策层（纯函数）

## 7.1 Command 类型

* `OneBotMessage{ profile, chat, user, msg_id, ts, text, attachments }`
* `InputStatus{ profile, chat, user, status, ts }`（来自 `notice.notify.input_status`）
* `TimerTick{ now_ms }`
* Driver 产出 `EventEnvelope`，并通过 `Command::DriverEvent(EventEnvelope)` 送回引擎（Driver 不直接写 journal/改 state）

## 7.2 decide_on_onebot_message（纯）

输出事件组合：

1. 去重：seen → `IngressMessageIgnored`；否则 `IngressMessageAccepted`
2. session 聚合：按 `(chat,user)` open/append，计算 `close_at = last_activity + waittime`
   * `last_activity` 取消息时间与输入状态时间的较新值
   * 若 `input_status=typing`，在 waittime 内不允许成稿（持续延后 close_at）
   * 若 `input_status=stopped`，从停止时间开始重新计时 waittime
   * 若 `input_status` 连续超过 30 分钟仍为 typing，则判定异常并忽略输入状态，直接成稿
   * 若收到消息前从未出现 `input_status`，认为终端不支持 typing，上述 waittime 改为 `waittime * 2`
3. 审核指令解析：仅审核群 + 命令格式 → `ReviewApproved/Rejected/Delayed/Route/Send…`
4. 附件下载请求：对有 URL 且尚未请求的 attachment → `MediaFetchRequested`

> 任何“时间 if-else”产生的结果（close_at、delay_until）必须写进事件字段。

## 7.3 decide_on_tick（纯）

tick 做三类事情：

1. **关闭到期 session**：`now>=close_at` 且 `input_status` 非 typing → `DraftSessionClosed`
   然后对刚关闭 session：创建 `PostDraftCreated` + `RenderRequested`
2. **审核发布重试**：`review_publish.failed && now>=retry_at` → `ReviewPublishRequested`
3. **发送启动**：

   * 找到 due_index 最早 `not_before<=now` 的 `SendPlan`
   * 选择可用账号（enabled 且 cooldown<=now）
   * emit `SendStarted`
   * 若无账号可用：emit `SendPlanRescheduled`（推迟到 next_available 或 now+30s）

---

# 8. Drivers（副作用执行器）

## 8.1 MediaFetcher

* 消费 `MediaFetchRequested`：

  * 下载 URL（限速/超时/重试）
  * 写入 BlobStore（RAM + 异步备份）
  * emit `MediaFetched` 或 `MediaFetchFailed{retry_at}`

## 8.2 Renderer（PNG-only）

* 消费 `RenderRequested`：生成 PNG bytes → Blob → `RenderPngReady`
* PNG 放 `spawn_blocking`
* 队列限长（例如 16），超限则降级策略（审核只发摘要文本）

## 8.3 AuditPublisher

* 消费 `ReviewPublishRequested`：

  * 构造审核文本 + 预览策略（png_low / png_full）
  * 发审核群消息（OneBot action）
  * 成功：emit `ReviewPublished{ audit_msg_id }`
  * 失败：emit `ReviewPublishFailed{ retry_at_ms = now + backoff(attempt) }`

## 8.4 QzoneSender（单机先做单写者）

* 消费 `SendStarted`：

  * 确保素材（PNG）
  * 调用发空间接口（OneBot 或你的 qzone driver）
  * 成功：`SendSucceeded` + `AccountLastSendUpdated` + `GroupLastSendUpdated`
  * 失败：`SendFailed{ retryable, retry_at }`；必要时 `AccountCooldownSet`
  * 达到 max_retry：`SendGaveUp` + `ManualInterventionRequired`

---

# 9. 存储与 IO 策略（“常规只写不读”）

## 9.1 目录结构建议

```
data/
  journal/
    00000001.log
    00000002.log
  snapshot/
    latest.snap
  blobs/
    png/
    image/
    file/
  logs/
```

## 9.2 Journal（事件日志）

* 追加写（BufWriter + 分段）
* flush 策略：

  * 默认：每 50ms 或累计 256KB flush（不 fsync，靠 OS page cache）
  * 关键事件可配置强耐久（Approved/Sent）：可选择 fsync（可选）
* 恢复：启动时读 snapshot，再回放 journal tail

## 9.3 Snapshot（快照）

* 每 N 分钟 or 每 N 条事件写一次
* 内容：StateView 的必要子集（不含大 blob bytes）
* 目的：缩短恢复回放时间

## 9.4 BlobStore（RAM 为主）

* `put_bytes`：立即放入 RAM cache（LRU/容量上限），并异步写盘备份
* 常规读取：只读 RAM（不读盘）
* 恢复期：必要时允许从磁盘读回（例如重启后要发某条积压稿）

---

# 10. NapCat/OneBot 集成与自愈策略

## 10.1 NapCat daemon（managed 模式）

* 拉起子进程（按 profile）
* 健康检查三层：

  1. 进程存活
  2. OneBot 连接状态（WS/HTTP）
  3. 业务心跳（get_status/noop action）
* 重启策略（time if-else）：

  * `if now-last_ok > 10s` → 重连/重启
  * `if 10min 内重启 > 3` → 冷却 5min + 告警（避免风暴）

## 10.2 多账号/多 profile

* 配置里定义 account_groups 与 profiles 的映射
* Sender 按 account_group 选择 profile/account 执行发送
* 单机先支持多 profile（多个 NapCat 实例）

---

# 11. 渲染体系（PNG-only）

## 11.1 PNG 生成
* 参考 ./typesetting&render.md
* 输入：`PostDraft.blocks + theme`
* 输出：PNG bytes（包含字体/排版/图片占位）
* 附件图片可：

  * 审核阶段：只显示缩略/占位符（避免下载未完成阻塞）
  * 发送阶段：需要则使用 blob 图片嵌入（或贴图）

## 11.2 渲染执行原则

* 放入 blocking 线程池
* 队列限长
* 失败可降级：审核只发摘要文本；发送失败进入人工介入

---

# 12. 审核体系（群指令 + 幂等）

## 12.1 审核消息内容建议

请参考./command.md

# 13. 调度与发送（时间 if-else 排程）

## 13.1 not_before 计算（纯 if-else）

输入：now、send_windows、min_interval、queue_depth、max_stack、delay_until、last_send

规则示例：

1. candidate = max(now, delay_until)
2. 若不在窗口 → next_window_start(candidate)
3. 若 candidate < last_send + min_interval → last_send + min_interval
4. 若 queue_depth >= max_stack → next_window_start + overflow_backoff
   输出写入 `SendPlanCreated.not_before_ms`

## 13.2 账号选择（纯 if-else）

* 过滤 enabled 且 cooldown<=now 的账号
* 策略（优先建议 LRU）：

  * last_send 最久未用者优先
* 无可用账号：

  * reschedule 到最早 cooldown 或 now+30s

---

# 14. 可观测性、运维与故障演练

## 14.1 tracing（必须）

* 每个 post_id 的链路日志：Ingress → Session → Draft → Render → Review → Send
* 关键点打 INFO：

  * session close
  * review publish
  * approve/reject
  * send start/success/fail

## 14.2 指标（可选但强烈建议）

* session 数量、队列深度、send 成功率、render 耗时、napcat 重启次数

## 14.3 故障演练清单

* kill -9 主进程：重启后能恢复 pending/retry/queue
* NapCat 卡死：daemon 自动重启并继续接收/发送
* 渲染失败：不影响审核队列推进（可降级为摘要文本）
* 发送失败：按 backoff 重试，超过次数进入 manual

---

# 15. 测试策略（强烈建议按 Functional Core 优先）

## 15.1 reducer 单测（必须）

* 给定一串事件回放，StateView 与预期一致
* stage index 不变量（move_stage 统一处理）
* 幂等：重复 event（同值）不崩

## 15.2 decider 单测（必须）

* 给定 state + now + 输入消息，输出事件序列符合预期（close_at/not_before/retry_at）

## 15.3 属性测试（推荐）

* “回放一致性”：`reduce(reduce(s,e1),e2)… == replay(snapshot+events)`
* “不会产生重复发送”：在随机 tick 顺序下，SendStarted 不会并发出现两次

## 15.4 集成测试（推荐）

* 用 Fake OneBot driver 记录发送请求，验证审核与发送行为
* 用 Fake Renderer 快速返回

---

# 16. 未来集群扩展点（现在不做，但接口预留）

保持这四个 trait 边界不破：

* `Bus`：LocalBus → ReplicatedBus（复制日志/多节点订阅）
* `Journal`：LocalJournal → ReplicatedJournal（Raft）
* `MetaStore`：Local（内存+快照）→ Raft KV
* `LeaseManager`：Local mutex → 分布式 lease（保证全局单写者）
* `BlobStore`：Local RAM+备份 → 多副本/内容寻址复制

核心业务层（deciders/reducer）不需要改。

---

# 17. 推荐里程碑与任务拆分（交付给团队）

## M1（最小可跑链路）

* Ingress + 去重
* Session 聚合（waittime）
* Draft 生成（最小分段）
* PNG 渲染（内置主题）
* 审核群发布（摘要 + 渲染图）
* 指令解析（是/否/等）
* SendPlan + Sender（先用 fake sender 打日志）

## M2（完整单机）

* PNG 渲染（预览/发送）
* QzoneSender 真接口
* 重试/backoff/冷却
* snapshot + 恢复回放
* admin web UI（可选）
* 监控与告警（最小）

## M3（硬化）

* 故障演练与修复
* GC/归档策略
* 兼容多个账号组、多个 profile
* 性能压测与内存上限策略

---

# 附：开发实现建议（让团队少走弯路）

## A. 事件写入与状态更新顺序（推荐）

* **建议顺序**：`append journal → reduce state → publish bus`

  * 即使 crash，也能通过 journal 恢复到一致状态
* journal 不必每条 fsync，默认 OS cache 足够；关键事件可配置强一致

## B. 单线程“状态机 actor”模式（推荐）

* 用一个 tokio task 作为“引擎”，串行处理 Command/Events
* 这样 state 不需要锁，天然确定性、易测

## C. 渲染与网络 IO 必须隔离

* PNG 渲染、下载附件、发送空间都用 `spawn_blocking` 或专用线程池
* 队列限长 + 超限降级，保证系统不会因为重任务堵塞入口

## D. 配置热更新（可选）

* ConfigApplied 写事件
* reducer 更新 config_version
* 业务层从 StateView 或共享 config 读取（注意保持决定结果写入事件字段）
