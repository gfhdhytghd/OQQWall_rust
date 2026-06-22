OQQWall_rust 采用了多层次的性能优化策略，从架构设计到具体实现形成了一套完整的优化体系。这些策略涵盖了内存管理、I/O 优化、并发处理、缓存机制和资源调度等关键领域，确保系统在高负载场景下仍能保持稳定高效的运行。

## 事件溯源架构与内存优先设计

系统的性能基石在于**事件溯源架构**与**Functional Core / Imperative Shell**的分离设计。整个运行时状态存储在内存中的 `StateView` 结构体里，所有业务查询都只读内存，避免了频繁的磁盘 I/O 操作。状态变化通过追加写事件日志实现，磁盘仅作为恢复数据源而非运行时数据源。

`StateView` 包含了 `ingress_seen`、`sessions`、`posts`、`send_plans`、`blobs` 等 30 多个 HashMap/HashSet 索引，覆盖了从消息接入到发送完成的完整生命周期。这种设计使得状态查询的时间复杂度保持在 O(1) 级别。

[engine.rs](crates/app/src/engine.rs#L105-L143)

引擎的主循环采用**串行处理模型**：接收 Command → decide 产出事件 → 追加写日志 → reduce 更新状态 → 广播事件。这种设计虽然限制了单次处理的并发度，但保证了可恢复一致性，避免了并发状态竞争带来的复杂性。

[engine.rs](crates/app/src/engine.rs#L220-L238)

快照机制采用**双条件触发策略**：每 1000 个事件或每 5 分钟（取先满足者）自动创建快照。快照包含完整的 `StateView` 和日志游标，启动恢复时只需加载最新快照并回放少量增量事件，大幅缩短了冷启动时间。

[engine.rs](crates/app/src/engine.rs#L23-L24)

## 多级缓存体系

系统构建了**五级缓存体系**，针对不同数据类型和访问模式采用差异化的缓存策略：

### Blob 缓存（全局共享）

Blob 缓存是系统最核心的缓存层，采用全局单例模式，通过 `OnceLock<Mutex<CacheState>>` 实现线程安全访问。缓存容量默认 256MB，可通过配置文件的 `max_cache_mb` 参数调整。

[blob_cache.rs](crates/drivers/src/blob_cache.rs#L7-L8)

缓存条目分为两种保留策略：`RenderOnly`（仅渲染期间保留）和 `UntilSend`（发送完成后释放）。图片数据保留至发送，而表情贴纸仅在渲染期间缓存，渲染完成后立即释放，优化了内存使用效率。

[blob_cache.rs](crates/drivers/src/blob_cache.rs#L17-L21)

淘汰策略采用**大对象优先淘汰**算法：当缓存超过容量限制时，按对象大小降序排列，优先淘汰最大的对象，直到总大小回到限制以内。这种策略避免了频繁的小对象淘汰操作。

[blob_cache.rs](crates/drivers/src/blob_cache.rs#L159-L177)

### 头像缓存（请求去重）

头像缓存实现了**请求去重机制**，通过 `in_flight` 状态追踪避免了同一用户头像的重复网络请求。当多个渲染任务同时请求同一用户头像时，只有第一个请求会触发实际的网络下载，后续请求通过 `Notify` 机制等待首个请求完成。

[avatar_cache.rs](crates/drivers/src/avatar_cache.rs#L61-L85)

`ensure_in_flight` 函数返回 `(Arc<Notify>, bool)` 元组，其中 `bool` 标识当前调用者是否为首个请求者。非首个请求者只需等待通知即可获取缓存结果，无需重复发起网络请求。

[avatar_cache.rs](crates/drivers/src/avatar_cache.rs#L87-L107)

### 渲染器图像缓存（渲染会话级）

渲染器维护了独立的 `ImageMemoryCache`，采用 `ImageCacheKey` 枚举（支持 BlobId 和 Source 两种键类型）实现图像数据的快速查找。缓存通过事件驱动方式预热：当接收到 `MessageAccepted` 或 `MediaFetchSucceeded` 事件时，自动将相关图像数据加载到缓存中。

[renderer.rs](crates/drivers/src/renderer.rs#L332-L341)

渲染完成后，通过 `release_keys` 方法主动释放已使用的缓存条目，避免缓存无限增长。对于仅用于渲染的表情贴纸，调用 `blob_cache::release_render_only` 进行针对性释放。

[renderer.rs](crates/drivers/src/renderer.rs#L1673-L1679)

### 文本测量缓存（渲染器内部）

文本宽度测量是排版计算的高频操作，`TextMeasurer` 内部维护了 `HashMap<TextMeasureKey, u32>` 缓存，键由 `font_size`、`font_weight` 和 `text` 组成。相同的文本样式组合只需测量一次，后续查询直接返回缓存结果。

[renderer.rs](crates/drivers/src/renderer.rs#L343-L416)

### 状态视图缓存（驱动层预加载）

渲染器和 QZone 发送器在初始化时通过 `OnceLock` 预加载完整的 `StateView`，避免了每次渲染请求都需要从引擎获取状态的开销。预加载过程包括加载最新快照和回放增量日志，确保缓存状态的完整性。

[renderer.rs](crates/drivers/src/renderer.rs#L1854-L1896)

## 并发处理与异步架构

系统基于 Tokio 多线程运行时构建，通过 `broadcast` 和 `mpsc` 两种 channel 实现组件间的解耦通信。

[drivers/Cargo.toml](crates/drivers/Cargo.toml#L20)

### 事件广播机制

`broadcast::channel(1024)` 创建了容量为 1024 的事件总线，支持多个消费者（渲染器、媒体获取器、QZone 发送器等）并行接收事件。当消费者处理速度跟不上生产者时，会触发 `Lagged` 错误，消费者通过 `continue` 跳过丢失的消息继续处理后续事件，避免了背压传播。

[media_fetcher.rs](crates/drivers/src/media_fetcher.rs#L76-L80)

### CPU 密集型任务隔离

PNG 渲染是系统中最耗时的 CPU 密集型操作，通过 `tokio::task::spawn_blocking` 将其调度到专用的阻塞线程池执行，避免阻塞 Tokio 的异步任务调度器。

[renderer.rs](crates/drivers/src/renderer.rs#L8357-L8370)

每个渲染请求在独立的异步任务中处理，通过 `tokio::spawn` 实现并发渲染。渲染任务完成后，通过 `mpsc::Sender<Command>` 将结果事件发送回引擎。

[renderer.rs](crates/drivers/src/renderer.rs#L1804)

### 媒体获取并发

媒体获取器为每个下载请求创建独立的异步任务，实现了网络 I/O 的并发执行。头像下载和附件下载分别在不同的 `tokio::spawn` 任务中进行，互不阻塞。

[media_fetcher.rs](crates/drivers/src/media_fetcher.rs#L98-L103)

## 日志与快照策略

### 分段日志

日志采用**分段存储**设计，每个段文件默认 64MB。当日志写入达到段大小限制时，自动创建新段文件，避免了单个文件过大导致的管理困难。

[journal.rs](crates/infra/src/journal.rs#L13)

写入采用**双条件刷新策略**：当累积写入字节数达到 256KB 或距上次刷新超过 50ms 时，执行一次 `flush` 操作。这种设计在写入吞吐量和数据持久性之间取得了平衡。

[journal.rs](crates/infra/src/journal.rs#L14-L15)

每条日志记录包含 8 字节头部（4 字节长度 + 4 字节 CRC32 校验和），确保了数据完整性。读取时自动验证校验和，发现损坏时记录错误位置并支持从损坏点之前的状态恢复。

[journal.rs](crates/infra/src/journal.rs#L136-L141)

### 快照持久化

快照采用**原子写入**策略：先写入临时文件 `latest.snap.tmp`，然后通过 `fs::rename` 原子性地替换正式文件 `latest.snap`，避免了写入过程中断导致的快照损坏。

[snapshot.rs](crates/infra/src/snapshot.rs#L85-L89)

## 重试与退避策略

系统针对不同类型的失败实现了差异化的退避策略：

### 渲染失败退避

渲染失败采用**指数退避**策略，基础延迟 10 秒，最大延迟 5 分钟。每次重试延迟翻倍（左移 1 位），最多偏移 10 次（约 170 分钟理论值，但被 max 限制）。

[renderer.rs](crates/drivers/src/renderer.rs#L7896-L7902)

### QZone 发送退避

QZone 发送根据错误类型采用不同的退避参数：

| 错误类型 | 基础延迟 | 最大延迟 | 适用场景 |
|---------|---------|---------|---------|
| Network | 5 秒 | 60 秒 | 网络连接超时 |
| RiskControl | 60 秒 | 30 分钟 | 风控触发 |
| Account | 10 分钟 | 60 分钟 | 账号异常 |
| Unknown | 60 秒 | 10 分钟 | 其他未知错误 |

[qzone.rs](crates/drivers/src/qzone.rs#L2248-L2258)

### 账号冷却机制

发送调度器实现了**账号冷却**机制：当账号发送失败时，设置 `cooldown_until_ms` 时间戳，在冷却期间该账号不会被选中发送。选择账号时优先选择最近最少使用的可用账号（LRU 策略），实现了负载均衡。

[sender.rs](crates/core/src/decide/sender.rs#L12-L61)

### Cookie 缓存与刷新

QZone 发送器维护了 Cookie 缓存，有效期 5 分钟。在有效期内的请求直接使用缓存 Cookie，避免了重复的认证请求。发送失败时自动触发 Cookie 刷新。

[qzone.rs](crates/drivers/src/qzone.rs#L2260-L2286)

## 调度优化

### 时间窗口调度

发送调度器支持**时间窗口**配置，确保消息只在指定时间段内发送。调度计算通过 `minute_of_day` 和 `next_window_start` 函数实现，支持跨天窗口和多窗口配置。

[scheduler.rs](crates/core/src/decide/scheduler.rs#L4-L34)

### 最小间隔控制

同一群组的连续发送之间强制执行最小间隔（`min_interval_ms`），防止短时间内大量发送触发平台风控。当队列深度超过上限时，还会在下一个时间窗口基础上额外增加退避延迟。

[scheduler.rs](crates/core/src/decide/scheduler.rs#L18-L31)

### 优先级排序

发送计划按 `(priority, seq, post_id)` 三元组排序，确保高优先级消息优先发送，同优先级按提交顺序发送。

[flush.rs](crates/core/src/decide/flush.rs#L15)

## 图像处理优化

### 上传尺寸限制

QZone 图片上传限制为 4MB，系统在上传前自动进行**多级压缩处理**：

[qzone.rs](crates/drivers/src/qzone.rs#L64)

1. **尺寸限制**：超过 1080p 分辨率的图片自动缩放
2. **格式转换**：PNG 转 JPEG、GIF 转 JPEG（保留首帧）
3. **质量递减**：JPEG 质量从 90% 逐步降至 50%，直到文件大小满足要求

[qzone.rs](crates/drivers/src/qzone.rs#L2628-L2714)

### 透明图片处理

对于带透明通道的 PNG 图片，优先尝试保持 PNG 格式（仅缩放），仅在缩放后仍超过 4MB 时才转换为 JPEG，最大程度保留了图像质量。

[qzone.rs](crates/drivers/src/qzone.rs#L2658-L2674)

## Release 构建优化

Cargo 配置启用了极致的二进制优化：

[Cargo.toml](Cargo.toml#L11-L16)

- `opt-level = "z"`：优先优化二进制体积
- `lto = "fat"`：全程序链接时优化，消除跨 crate 冗余
- `codegen-units = 1`：单代码生成单元，最大化优化机会
- `strip = "symbols"`：剥离调试符号，减小二进制体积

## 配置化的性能参数

系统提供了多个可配置的性能参数，允许根据部署环境调整：

| 参数 | 默认值 | 说明 |
|-----|-------|------|
| `max_cache_mb` | 256 | Blob 缓存上限（MB） |
| `process_waittime_sec` | 20 | 消息聚合等待时间（秒） |
| `min_interval_ms` | - | 最小发送间隔（毫秒） |
| `max_queue` | - | 发送队列上限 |
| `send_timeout_ms` | - | 发送超时时间 |
| `send_max_attempts` | - | 最大重试次数 |
| `renderer_canvas_width_px` | 1152 | 渲染画布宽度 |
| `renderer_max_height_px` | 6912 | 渲染最大高度 |

[config.rs](crates/app/src/config.rs#L84-L104)

## 总结

OQQWall_rust 的性能优化策略体现了**内存优先、缓存分层、异步并发、智能退避**的设计理念。通过事件溯源架构减少了磁盘 I/O，多级缓存体系降低了重复计算和网络请求，异步并发模型充分利用了多核 CPU 资源，差异化的退避策略提高了系统在异常情况下的恢复能力。这些策略共同构成了一个高效、可靠的投稿处理系统。