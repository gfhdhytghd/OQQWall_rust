OQQWall_RUST（开放QQ校园墙自动运营系统 - Rust版）是一个高性能、事件驱动的校园墙自动化运营平台。本系统专注于为50万用户量级以下的校园墙提供无感交互体验，通过事件溯源架构与函数式核心设计，实现了投稿接收→时间聚合→稿件生成→渲染预览→审核群指令→排程发送QQ空间的全自动化流程。相比原版OQQWall脚本系统，Rust版在性能上提升了至少100倍，同时保持了相同的指令语义与使用体验。

## 核心架构

本系统采用 **Functional Core / Imperative Shell** 架构模式，将业务逻辑与副作用执行分离，确保系统的可测试性、可恢复性与幂等性。核心引擎是单线程串行处理的Actor模型，保证事件顺序一致性与可重放性。

```mermaid
graph TB
    subgraph "Imperative Shell (副作用层)"
        A[OneBot/NapCat] -->|消息| B[Ingress Driver]
        C[定时器] -->|Tick| B
        D[管理员指令] -->|Command| B
        
        B -->|Command| E[Engine Actor]
        
        E -->|Event| F[Journal Writer]
        E -->|Event| G[LocalBus]
        
        G -->|XxxRequested| H[Media Fetcher]
        G -->|XxxRequested| I[Renderer]
        G -->|XxxRequested| J[Audit Publisher]
        G -->|XxxRequested| K[Qzone Sender]
        
        H -->|XxxReady/Failed| E
        I -->|XxxReady/Failed| E
        J -->|XxxReady/Failed| E
        K -->|XxxReady/Failed| E
    end
    
    subgraph "Functional Core (纯函数层)"
        L[StateView] --> M[Reducer]
        M -->|StateView'| L
        
        N[Command] --> O[Decider]
        O -->|Vec<Event>| N
        
        M -->|纯函数| P[(State, Event) → State']
        O -->|纯函数| Q[(State, Command) → Events]
    end
    
    subgraph "持久化层"
        F -->|Append-Only| R[Journal]
        R -->|恢复| L
        S[Snapshot Store] -->|定期快照| L
    end
```

### 架构设计原则

**事件溯源（Event Sourcing）**：所有状态变化以事件形式追加写入日志，运行时状态在内存中维护，恢复时通过快照+日志回放重建。这种设计天然支持崩溃恢复、幂等处理与未来集群扩展。

**请求型事件模式（Requested → Ready/Failed）**：任何需要IO返回值的操作（如`audit_msg_id`、`qzone_post_id`、`blob_id`）都拆分为`XxxRequested`、`XxxReady`或`XxxFailed`事件，确保Decider保持纯函数特性。

**确定性ID派生**：关键实体ID（如`IngressId`、`SessionId`、`PostId`）通过`blake3`/`xxhash`从输入确定性派生，保证事件重放时不会产生不同ID，是幂等性的基础。

## 核心特性

| 特性类别 | 具体能力 | 技术实现 |
|---------|---------|---------|
| **投稿处理** | 支持文本、表情、表情包、图片、视频、文件 | `IngressMessage`多态类型系统 |
| **聚合策略** | 按`(chat_id, user_id)`维度时间窗口聚合 | `SessionEvent`开/关/追加，支持typing状态感知 |
| **渲染引擎** | PNG-only输出，统一预览与最终发送 | Skia渲染器，`RenderRequested`→`PngReady` |
| **审核系统** | 群指令审核，支持通过/拒绝/延后/改路由等 | `ReviewEvent`全生命周期管理 |
| **调度排程** | 发送窗口+最小间隔+队列上限+账号冷却 | `ScheduleEvent`与`SendEvent`状态机 |
| **多账号组** | 不同组可配不同审核群、发送窗口、账号列表 | `GroupId`隔离与`CoreConfig`组配置 |
| **QQ空间发送** | 单写者模式，失败重试与人工介入 | `QzoneSender`Driver与`ManualEvent` |
| **并发能力** | 允许无限小投稿间隔，内存限制并行处理 | 单线程引擎+异步Drivers |
| **崩溃恢复** | 快照+日志回放，继续处理未完成项 | `SnapshotStore`+`LocalJournal` |
| **可观测性** | tracing日志+基础指标+遥测样本 | `metrics.rs`+`telemetry.rs` |

## 项目结构

本项目采用Rust workspace结构，最终编译为单个二进制文件。模块化设计确保了职责分离与依赖约束：

```
OQQWall_RUST/
├── Cargo.toml              # Workspace配置
├── crates/
│   ├── core/               # 纯函数核心层（无IO依赖）
│   │   └── src/
│   │       ├── event.rs    # 事件类型定义（20+种事件）
│   │       ├── state.rs    # StateView内存索引结构
│   │       ├── reduce/     # Reducer纯函数实现
│   │       └── decide/     # Decider纯函数实现
│   │
│   ├── infra/              # 基础设施层（本地IO实现）
│   │   └── src/
│   │       ├── journal.rs  # 追加写日志+分段+flush策略
│   │       └── snapshot.rs # 快照存储与恢复
│   │
│   ├── drivers/            # 副作用执行器层
│   │   └── src/
│   │       ├── napcat.rs   # NapCat/OneBot客户端
│   │       ├── renderer.rs # Skia渲染器（PNG生成）
│   │       ├── qzone.rs    # QQ空间发送
│   │       └── media_fetcher.rs # 附件下载
│   │
│   ├── app/                # 应用入口层
│   │   ├── src/
│   │   │   ├── main.rs     # 主程序入口
│   │   │   ├── engine.rs   # Engine Actor实现
│   │   │   ├── config.rs   # 配置加载与解析
│   │   │   ├── webview.rs  # 内置WebView审核前端
│   │   │   └── web_api.rs  # HTTP API服务
│   │   └── webview-ui/     # Vue前端（审核界面）
│   │
│   └── telemetry-collector/ # 遥测收集器（独立服务）
│
├── docs/                   # 项目文档
├── res/                    # 资源文件（表情、字体、图标）
└── scripts/                # 构建与部署脚本
```

### 关键模块职责

**core层**：包含所有业务逻辑的纯函数实现。`StateView`是内存中的完整状态索引，`Reducer`负责`(State, Event) → State'`的状态转换，`Decider`负责`(State, Command) → Events`的决策逻辑。该层无IO依赖，完全可测试。

**infra层**：提供本地基础设施实现。`LocalJournal`实现追加写日志，支持分段与flush策略；`SnapshotStore`负责定期快照，优化恢复时间。该层只写磁盘，读取仅发生在启动恢复阶段。

**drivers层**：执行所有副作用的驱动程序。每个Driver订阅特定事件类型（如`RenderRequested`），执行IO操作，产出完成/失败事件。采用`Requested → Ready/Failed`模式确保核心层保持纯函数。

**app层**：应用组装与启动。`Engine`是单线程Actor，接收Command、调用Decider、执行Reducer、写入Journal、发布到LocalBus。`main.rs`负责依赖注入与启动序列。

## 数据流与状态机

系统的数据流遵循严格的顺序：**Command → Decider → Events → Journal → Reducer → StateView → Bus → Drivers**。每个环节都有明确的职责与约束。

```mermaid
stateDiagram-v2
    [*] --> Drafted: PostDraftCreated
    Drafted --> RenderRequested: RenderRequested
    RenderRequested --> Rendered: PngReady/PngBatchReady
    RenderRequested --> Failed: RenderFailed(超过重试)
    Rendered --> ReviewPending: ReviewItemCreated
    ReviewPending --> Reviewed: ReviewDecisionRecorded
    ReviewPending --> Deferred: ReviewDelayed
    Deferred --> ReviewPending: 定时触发
    Reviewed --> Scheduled: SendPlanCreated
    Reviewed --> Rejected: ReviewDecision::Rejected
    Reviewed --> Deleted: ReviewDecision::Deleted
    Scheduled --> Sending: SendStarted
    Sending --> Sent: SendSucceeded
    Sending --> Failed: SendFailed(超过重试)
    Sending --> Manual: ManualInterventionRequired
    Manual --> Sending: ManualInterventionResolved
    Sent --> Withdrawn: QzonePostWithdrawRequested
```

### 事件分组

系统定义了20+种事件类型，按功能领域分组：

| 事件组 | 事件类型 | 职责 |
|-------|---------|------|
| **System/Config** | `Booted`, `SnapshotLoaded`, `Applied` | 系统生命周期与配置 |
| **Ingress** | `MessageAccepted`, `MessageSynced`, `InputStatusUpdated` | 消息接入与状态 |
| **Session** | `Opened`, `Appended`, `Closed` | 投稿聚合会话 |
| **Draft** | `PostDraftCreated` | 稿件生成 |
| **Media** | `AvatarFetchRequested`, `MediaFetchRequested` | 头像与附件获取 |
| **Render** | `RenderRequested`, `PngReady`, `PngBatchReady` | PNG渲染 |
| **Review** | `ReviewItemCreated`, `ReviewPublished`, `ReviewDecisionRecorded` | 审核流程 |
| **Schedule** | `SendPlanCreated`, `SendPlanRescheduled` | 发送调度 |
| **Send** | `SendStarted`, `SendSucceeded`, `SendFailed`, `SendGaveUp` | 空间发送 |
| **Account** | `AccountEnabled`, `AccountCooldownSet` | 账号状态管理 |
| **Manual** | `ManualInterventionRequired`, `ManualInterventionResolved` | 人工介入 |

## 配置与部署

系统采用JSON配置文件，默认路径为`./config.json`，可通过环境变量`OQQWALL_CONFIG`覆盖。配置分为`common`（全局默认）与`groups`（组级覆盖）两部分。

### 关键配置项

| 配置类别 | 配置项 | 默认值 | 说明 |
|---------|-------|-------|------|
| **聚合** | `process_waittime_sec` | `20` | 投稿聚合等待时间（秒） |
| **发送** | `min_interval_ms` | `0` | 同组发送最小间隔（毫秒） |
| **渲染** | `renderer.canvas_width_px` | `384` | PNG画布宽度（像素） |
| **Web API** | `web_api.enabled` | `false` | 是否启用HTTP API |
| **WebView** | `webview.enabled` | `false` | 是否启用内置审核前端 |
| **遥测** | `telemetry.enabled` | `true` | 是否生成训练样本 |

### 部署模式

1. **单机模式**：单个Rust二进制，包含所有功能模块
2. **Docker模式**：支持容器化部署，提供`Dockerfile`与`docker-compose`
3. **多架构支持**：通过GitHub Actions构建`linux/amd64`与`linux/arm64`镜像

## 技术栈与依赖

| 组件 | 技术选型 | 用途 |
|------|---------|------|
| **运行时** | Tokio | 异步运行时，支持高并发IO |
| **渲染** | Skia (rust-skia) | 高性能2D图形库，生成PNG |
| **前端** | Vue 3 + Vite | WebView审核界面 |
| **协议** | OneBot v11 | 与NapCat通信标准 |
| **序列化** | Serde | JSON/事件序列化与反序列化 |
| **日志** | tracing | 结构化日志与指标 |
| **测试** | 单元测试+属性测试+集成测试 | 多层次质量保证 |

## 阅读建议

根据您的角色与需求，建议按以下路径阅读文档：

### 快速上手路径
1. **[快速开始](2-kuai-su-kai-shi)** - 5分钟部署与运行
2. **[配置文件说明](4-pei-zhi-wen-jian-shuo-ming)** - 理解配置结构
3. **[审核指令系统](6-shen-he-zhi-ling-xi-tong)** - 掌握日常操作

### 开发者路径
1. **[项目架构详解](8-xiang-mu-jia-gou-xiang-jie)** - 深入架构设计
2. **[事件溯源架构](9-shi-jian-su-yuan-jia-gou)** - 理解核心模式
3. **[Skia 渲染引擎](12-skia-xuan-ran-yin-qing)** - 渲染层实现
4. **[HTTP API v1](19-http-api-v1)** - API集成开发

### 运维路径
1. **[生产环境部署](20-sheng-chan-huan-jing-bu-shu)** - 部署与运维
2. **[监控与遥测](21-jian-kong-yu-yao-ce)** - 可观测性配置
3. **[故障排查手册](22-gu-zhang-pai-cha-shou-ce)** - 问题诊断

### 高级主题路径
1. **[性能优化策略](23-xing-neng-you-hua-ce-lue)** - 性能调优
2. **[并发处理机制](24-bing-fa-chu-li-ji-zhi)** - 并发模型
3. **[未来集群扩展](26-wei-lai-ji-qun-kuo-zhan)** - 分布式演进

## 相关资源

- **源代码**：[GitHub仓库](https://github.com/gfhdhytghd/OQQWall_rust)
- **技术交流群**：1056259167
- **原版OQQWall**：参考实现，Rust版保持指令语义兼容
- **NapCat文档**：[napneko.github.io](https://napneko.github.io/zh-CN/)