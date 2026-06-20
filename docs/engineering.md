下面是一份工程化落地指南（偏“怎么做、怎么组织代码、怎么跑起来、怎么测试、怎么交付”）。它与 `docs/dev_guide.md` 的总体指导互补，本文件面向具体落地。

---

# OQQWall_RUST 单机版工程落地文档（未来集群预留）

> 目标：在**单机**完成 OQQWall 全功能链路（接收投稿→时间聚合→成稿→渲染→审核群→指令审核→排程→空间发送→重试/人工介入），并且采用 **Functional Core / Imperative Shell** 架构，最大化函数式与可测试性。
> 限制：本阶段**不做 AI/LLM/embedding**，所有“智能”决策使用**时间 if-else**（窗口聚合、发送窗口、最小间隔、重试退避、账号冷却等）。
> 约束：单 Rust 二进制；尽量不依赖外部服务；磁盘常规只写不读（读只用于重启恢复）；NapCat 可作为被管理子进程。

---

## 1. 工程目标与验收标准

### 1.1 MVP（第一可运行版本）必须满足

* ✅ OneBot 入站（群/私聊投稿）可接入
* ✅ `(chat_id, user_id)` 维度按 `process_waittime_sec` 聚合，受输入状态影响（typing/stop），满足“停更等待”后成稿
  * typing 连续超过 30 分钟视为异常并忽略
  * 未出现 typing 上报则使用 `process_waittime_sec * 2`
* ✅ 默认生成 PNG（不依赖浏览器/外部渲染器）
* ✅ 审核群发布（先文本+链接即可）
* ✅ 审核指令：参考command.md,至少实现 是 否 等 删
* ✅ 调度：发送窗口 + 最小间隔 + 队列上限 + 单写者发送（先用 fake sender 打日志也可）
* ✅ 崩溃重启后：能从 snapshot + journal 恢复，继续处理未完成项

### 1.2 完整单机版本必须满足

* ✅ PNG 输出（预览/最终发送）
* ✅ 真实 Qzone 发送（或通过 NapCat action / 内置 driver）
* ✅ 失败重试 + 账号冷却 + 人工介入态
* ✅ 监控：tracing 日志 + 基础指标（队列深度、发送成功率、NapCat 重启次数）
* ✅ 可选遥测：基于审核结果生成训练样本，支持本地缓存与批量上传（20 条/批）

---

## 2. 总体架构：Functional Core / Imperative Shell

### 2.1 核心原则

* **Reducer 纯函数**：`(StateView, Event) -> StateView'`，无 IO、无 now()、无随机
* **Decider 纯函数**：`(StateView, Command) -> Vec<Event>`，所有“时间 if-else”的结果写入事件字段（例如 `close_at_ms`, `not_before_ms`, `retry_at_ms`）
* **副作用只能出现在 Drivers**：渲染、下载附件、发审核群消息、发空间、NapCat 子进程管理
* **引擎单线程串行处理**：避免锁、避免竞态、保证可重放一致性

### 2.2 请求型事件（Requested → Ready/Failed）必须采用

任何需要 IO 返回值（如 `audit_msg_id`, `qzone_post_id`, `blob_id`）都必须拆成：

* `XxxRequested { ... }`
* `XxxReady { ... }` 或 `XxxFailed { retry_at }`

这样 decider 才能保持纯。

---

## 3. 仓库结构（Workspace，但最终仍产出单二进制）

建议使用 workspace 做内聚隔离（但最后 `crates/app` 编译成一个二进制）：

```
OQQWall_RUST/
  Cargo.toml                # workspace
  crates/
    core/                   # 纯函数：Event/State/Reducer/Deciders
      src/
        event.rs
        state.rs
        reduce/
          mod.rs
          system.rs
          ingress.rs
          session.rs
          draft.rs
          render.rs
          review.rs
          schedule.rs
          send.rs
          gc.rs
        decide/
          mod.rs
          ingest.rs          # OneBotMessage -> events（含指令解析）
          tick.rs            # TimerTick -> events（close session / retry / start send）
          scheduler.rs       # compute_not_before 等纯函数
          sender.rs          # choose_account 等纯函数
          builder.rs         # build_post_from_session（纯）
      tests/
        reducer_replay.rs
        decide_tick.rs

    infra/                  # 本地实现：journal/blob/bus/lease/snapshot
      src/
        journal.rs
        snapshot.rs
        bus.rs
        blob.rs
        lease.rs
        clock.rs            # 仅 shell 用（now），core 禁止使用
        config.rs

    drivers/                # IO 实现：onebot/renderer/audit/qzone/media/napcatd
      src/
        onebot.rs
        napcatd.rs
        media_fetcher.rs
        renderer.rs
        audit_publisher.rs
        qzone_sender.rs
        admin_web.rs         # 可选
        metrics.rs           # 可选

    app/                    # 二进制入口：装配引擎、启动 tasks
      src/
        main.rs
        engine.rs            # 单线程引擎 actor
        wiring.rs            # 依赖注入/装配
        commands.rs          # Command 定义与输入适配
  docs/
    xxx
  data/                      # 默认运行目录（.gitignore）
```

> 为什么用 workspace：让开发组可以并行（core/infra/drivers/app），并强制约束 core 不引入 IO 依赖。
> 最终交付仍是一个 `OQQWall_RUST` 二进制（`crates/app`）。

---

## 4. 核心数据与接口约定（必须统一）

### 4.1 ID 与时间的约定

* `TimestampMs`：毫秒时间戳（i64）
* EventEnvelope：

  * `id` 由引擎分配 ULID（允许非确定性，因为 event payload 内的实体 ID 必须确定性）
  * `ts_ms` 由 shell 的时钟提供，但一旦写入事件就固定（replay 不依赖 now）

### 4.2 实体 ID 的确定性派生（非常重要：幂等与重放）

推荐：

* `IngressId = hash(profile + chat + user + platform_msg_id)`（确定性）
* `SessionId = hash(chat + user + first_ingress_id)`（确定性）
* `PostId = hash(session_id)`（确定性）

实现建议：

* 使用 `blake3` / `xxhash` 把输入 bytes hash 成 128-bit，再映射到 `Ulid` 或自定义 `Id128`

> 这样即使 tick 重复或事件重放，都不会产生不同的 post_id/session_id。

---

## 5. 引擎（Engine）实现建议：单线程 Actor，保证顺序一致性

### 5.1 引擎输入/输出通道

* 输入：`Command`（来自 OneBot inbound、Timer tick、Driver 完成事件）
* 输出：

  * 事件写 Journal（append-only）
  * reducer 更新 StateView
  * publish 到 LocalBus（让 drivers 订阅 `XxxRequested`、`SendStarted` 等）

### 5.2 关键顺序（强一致建议）

**append journal → reduce state → publish bus**

理由：

* crash 后 journal 是真相；state 可从 journal 重新构建
* drivers 订阅到的事件一定已落盘

### 5.3 Engine 伪代码（可直接落地）

```rust
loop {
  let cmd = cmd_rx.recv().await;
  let events = core::decide(&state, &cmd); // 纯函数
  for e in events {
    let env = envelope(e, actor, ts_ms, corr_id);
    journal.append(&env)?;           // 只写
    state = state.reduce(&env)?;     // 纯
    bus.publish(env.clone()).await;  // 通知 drivers
  }
}
```

---

## 6. Drivers：副作用执行器的统一规范（非常重要）

### 6.1 统一 Driver 形态

每个 driver：

* 订阅特定事件类型（通常是 `XxxRequested`）
* 执行 IO
* 产出完成/失败事件，通过 `cmd_tx.send(Command::DriverEvent(env))` 发回引擎

### 6.2 Driver 幂等与“避免重复执行”

driver 不应依赖自己的内存去重，应该依赖 state 的 request 状态或事件本身的确定性：

* `MediaFetchRequested(ingress, idx, attempt)`：同一个 `(ingress, idx, attempt)` 如果重复出现，driver 可以允许重复执行，但结果事件要幂等（同值 no-op）
* 更推荐：decider 保证不会对同一键在 Requested 状态下再发请求

### 6.3 blocking 任务隔离

* PNG 渲染、附件下载、发送空间，都必须：

  * 使用专用 `tokio::task::spawn_blocking` 或单独线程池
  * 队列限长（例如 16），超限则丢弃/降级（例如审核只发摘要文本）

---

## 7. 配置与 CLI（工程落地必须具备）

### 7.1 配置格式（JSON）

建议 `config.json`：
请参考 `docs/config.md`


### 7.2 CLI 约定（建议）

* `OQQWALL_CONFIG=./config.json OQQWall_RUST`（当前实现）
  * 若配置文件不存在且有交互终端：自动进入 OOBE
  * 若配置文件不存在且无交互终端：报错退出并提示手动 OOBE
* `OQQWall_RUST oobe --config ./config.json`（生成/覆盖配置，当前实现）
* `OQQWall_RUST replay ...`（规划：调试回放）
* `OQQWall_RUST doctor ...`（规划：自检 NapCat/端口/目录权限）
* `OQQWall_RUST export ...`（规划：导出稿件与产物用于排查）

---

## 8. 功能链路落地：Command → decide → Event → Driver → Event

下面给开发组一个“最小闭环”的事件流图（落地优先级顺序）。

### 8.1 投稿接入与聚合（必须第一周完成）

1. OneBot inbound → `Command::OneBotMessage`
2. decider：

   * 去重：`IngressMessageAccepted` 或 `IngressMessageIgnored`
   * session 聚合：`DraftSessionOpened/Appended`（计算 close_at）
3. TimerTick：

   * `now>=close_at` → `DraftSessionClosed`
   * `PostDraftCreated`（由 builder 纯函数构造 blocks）
   * `RenderRequested`

### 8.2 渲染与审核发布（第二阶段）

4. Renderer driver 消费 `RenderRequested` → `RenderPngReady`
5. decider（看到 PngReady）：

   * `ReviewItemCreated`（分配 review_code）
   * `ReviewPublishRequested`
6. AuditPublisher driver 执行 IO → `ReviewPublished`（带 audit_msg_id）

### 8.3 审核指令（核心玩法）

7. 审核群消息 → `Command::OneBotMessage`
8. decider 解析：

   * `是` → `ReviewApproved`
   * `否` → 标记已处理 + 外部编号+1 + 进入“人工发送”状态
   * `等` → `ReviewDelayed`
   * `立即` → `SendPlanCreated/Rescheduled`

### 8.4 调度与发送（最复杂，后做但要一次写对）

9. ReviewApproved → decider 计算 `not_before` → `SendPlanCreated`
10. TimerTick：

* 找 due 且无 in-flight → `SendStarted`（选择账号）

11. QzoneSender driver IO：

* `SendSucceeded` 或 `SendFailed { retry_at }`

12. 失败：

* retryable → `SendPlanRescheduled`（由 driver 或 decider 生成，但建议 driver 直接给出 retry_at）
* 超过 max_retry → `SendGaveUp` + `ManualInterventionRequired`

---

## 9. 实现细节建议（减少返工/踩坑）

### 9.1 核心禁止项（写进团队规范）

* core crate 禁止引入：

  * `tokio`, `reqwest`, `std::time::SystemTime`, `rand`, 任何 IO
* core 里禁止直接调用 `now()`

  * 所有时间点必须从 command/tick 输入或事件字段传入

### 9.2 “纯函数 builder”建议（从 session 构造 draft）

* 每条 ingress 消息生成一个 Paragraph block
* 空行切段
* 超长（>800字）按标点切（。！？；）
* 这不是 AI，是确定性文本处理，必须纯函数

### 9.3 PNG 渲染建议（落地策略）

* `resvg/usvg + tiny-skia`（尽量避免外部浏览器）
* blocking 线程池 size=1（先保守）
* 大量积压时降级：审核只发摘要文本

### 9.4 审核预览策略（可配置）

* `png_low`：审核群发 720px 预览（更友好，但 CPU 增加）
* `png_full`：重（不推荐默认）

### 9.5 发送的“单写者”策略（单机）

* 引擎层保证：全局 `sending` 集合非空则不再产生 `SendStarted`
* 这样不需要 lock/lease 也能避免并发发送
* 未来集群才需要分布式 lease

### 9.6 backoff（重试退避）统一函数

写在 core 的纯函数里（供 driver/decider 共用）：

```text
delay = min(base * 2^(attempt-1), max_delay)
retry_at = now + delay
```

同时允许“错误码分层”：

* 网络波动：base=5s
* 风控/频率：base=60s
* 账号异常：base=10min + cooldown

---

## 10. 测试与 CI（工程交付的硬指标）

### 10.1 单测优先级（必须写）

* reducer replay：给定 event 序列，最终 StateView 符合预期
* decider tick：给定 StateView + now，输出事件集合正确且幂等
* 指令解析：各种输入 是/否/等 的解析稳定

### 10.2 属性测试（推荐）

* “幂等性”：同一个 tick 重复 N 次，不会产生额外 `PostDraftCreated/SendStarted`
* “不变量”：stage index 与 posts.stage 永远一致

### 10.3 集成测试（推荐）

* Fake OneBot：记录 outbound 请求；验证审核发布与指令处理
* Fake Renderer/Fake QzoneSender：快速返回成功/失败，验证重试与冷却逻辑

### 10.4 CI 建议

* `cargo fmt --check`
* `cargo clippy -D warnings`
* `cargo test`
* `cargo test -p core`（纯函数部分最关键）

---

## 11. 运行与交付（给运维/部署）

### 11.1 systemd（建议模板）

* 以 `Restart=always` 运行
* data_dir 目录持久化
* 日志输出到 journald + 文件

### 11.2 升级策略

* 停止服务
* 备份 `data/`（尤其 journal + snapshot + blobs）
* 替换二进制
* 启动，自动从 snapshot+journal 恢复

### 11.3 故障排查入口

* `OQQWall_RUST replay`：回放指定范围事件，复现 bug
* `admin web`：查看队列、查看 post 状态、查看 last_error

---

## 12. 任务拆分（开发组可直接开 Jira/Tapd）

### M1（1–2 周）：跑通链路

* [ ] core：Event/StateView/reducer 框架 + stage index 工具函数
* [ ] infra：journal append + snapshot + replay
* [ ] app：engine actor（单线程）+ cmd channel + bus
* [ ] drivers：OneBot inbound/outbound（可先 mock）
* [ ] core.decide：ingress 去重 + session 聚合 + tick close → PostDraftCreated
* [ ] renderer：先实现 PNG 生成（简单模板）
* [ ] audit：发布审核消息（先文本+link）
* [ ] command parser：是/否/等
* [ ] scheduler：ReviewApproved → SendPlanCreated（not_before 基础 if-else）
* [ ] sender：fake 发送（日志），确保状态机完整

**DoD**：从投稿到审核通过到“发送成功事件”全链路事件可回放恢复。

### M2（2–4 周）：完整单机

* [ ] MediaFetcher：附件下载与 blob 引用
* [ ] PNG on-demand：审核 png_low 或发送 png_final
* [ ] QzoneSender 真发送（对接 NapCat获取cookies和逆向的接口（参考原版OQQWall的~/data/OQQWall/SendQzone/send.py））
* [ ] 重试/冷却/人工介入态完善
* [ ] admin web：列表/详情/手工 resend/手工 route（可选）
* [ ] metrics + tracing 完整化

### M3（持续）：硬化与性能

* [ ] GC/归档策略（事件化或 runtime 缓存清理）
* [ ] 更完善的发送窗口逻辑（跨天、多个窗口）
* [ ] 演练：kill -9、NapCat 重启风暴、网络失败、发送失败
* [ ] 性能压测：大量投稿、渲染队列、发送队列压力测试

---

## 13. 给开发组的“最重要约定”（请写进 CONTRIBUTING.md）

1. **core 永远纯函数**：任何 IO/now/rand 不许进 core
2. 所有 IO 都必须通过 `Requested → Ready/Failed` 事件回写，不直接改 state
3. 引擎串行处理：append journal → reduce → publish，保持可恢复一致性
4. 所有事件字段必须包含决策结果时间点（close_at/not_before/retry_at/cooldown_until）
5. 任何状态变化必须有事件（不要“偷偷 mutate”）

---

## 附：建议你要求开发组先交付的两份文档（用于对齐）

* `docs/command.md`：审核群指令语法、示例、边界行为（幂等规则）
* `docs/runbook.md`：如何启动/停止/升级/备份/恢复/排障

---

如果你愿意，我可以再补两样“更能直接开干”的东西（仍然是工程落地风格）：

1. **一份可复制的 `docs/command.md`**（含指令语法、示例消息、错误提示、权限模型）
2. **一份可复制的 `crates/app/src/engine.rs` 代码骨架**（带 channels、bus、journal、driver wiring 的完整结构）

你希望先要哪一个？
