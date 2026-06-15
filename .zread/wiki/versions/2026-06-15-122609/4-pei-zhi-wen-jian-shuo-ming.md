本文档详细说明 OQQWall_RUST 的配置文件结构、各项参数含义及最佳实践。配置文件采用 JSON 格式，支持环境变量覆盖，程序启动时会自动验证并归一化配置。

## 配置文件基础

OQQWall_RUST 的配置文件采用 JSON 格式，默认路径为 `config.json`，可通过环境变量 `OQQWALL_CONFIG` 覆盖。当配置文件不存在时，交互终端会自动进入 OOBE（开箱体验）引导生成；非交互环境会报错退出并提示手动生成。

**配置文件生成方式**：
- 交互式 OOBE：`cargo run -p OQQWall_RUST -- oobe`
- TUI 编辑器：`cargo run -p OQQWall_RUST -- --tui`
- 手动创建：参考本文档结构编写 JSON 文件

推荐配置文件结构包含三个顶层字段：`common`（全局默认值）、`groups`（账号组配置）和 `webview_global_admins`（全局管理员）。程序启动时会验证配置有效性，若发现不兼容字段会自动归一化并写回配置文件。

Sources: [config.rs](crates/app/src/config.rs#L1-L50), [oobe.rs](crates/app/src/oobe.rs#L1-L100), [main.rs](crates/app/src/main.rs#L50-L100)

## 环境变量覆盖

OQQWall_RUST 支持通过环境变量覆盖关键配置项，这在容器化部署或敏感信息管理时特别有用。环境变量优先级高于配置文件中的对应字段。

| 环境变量 | 覆盖目标 | 说明 |
| --- | --- | --- |
| `OQQWALL_CONFIG` | 配置文件路径 | 默认 `config.json` |
| `OQQWALL_NAPCAT_BASE_URL` | NapCat base url | 覆盖所有组的 `napcat_base_url` |
| `OQQWALL_NAPCAT_TOKEN` | NapCat access token | 覆盖所有组的 `napcat_access_token` |
| `OQQWALL_API_TOKEN` | `common.web_api.root_token` | 覆盖 Web API root token |
| `OQQWALL_PROCESS_WAITTIME_MS` | 全局默认投稿聚合窗口 | 单位是毫秒，优先于 `common.process_waittime_sec`；组内 `process_waittime_sec` 仍可覆盖 |
| `OQQWALL_MAX_CACHE_MB` | `common.max_cache_mb` | 图片/blob 内存缓存上限 |
| `OQQWALL_DATA_DIR` | 运行数据目录 | 默认 `data`，影响日志、遥测、本地 blob 等 |

环境变量覆盖逻辑在配置加载时实现，优先级顺序为：环境变量 > 组内配置 > 全局默认值。例如 `OQQWALL_NAPCAT_BASE_URL` 会覆盖所有组的 NapCat 连接地址，但组内显式配置的 `napcat_base_url` 仍会被保留。

Sources: [config.rs](crates/app/src/config.rs#L200-L250), [config.rs](crates/app/src/config.rs#L500-L550)

## common 全局配置

`common` 字段包含全局默认值，组内同名配置会覆盖这些默认值。配置采用分层结构，支持 `web_api`、`webview`、`renderer` 和 `telemetry` 四个子模块。

### 基础参数

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `process_waittime_sec` | number/string | `20` | 投稿聚合等待时间，单位秒 |
| `min_interval_ms` | number/string | `0` | 同组发送的最小间隔，单位毫秒 |
| `max_image_number_one_post` | number/string | `30` | 单条说说最大图片数，组内可覆盖 |
| `send_timeout_ms` | number/string | `300000` | 单次发送超时，单位毫秒 |
| `send_max_attempts` | number/string | `3` | 发送失败最大尝试次数 |
| `tz_offset_minutes` | number/string | `0` | 时区偏移分钟数，中国大陆通常为 `480` |
| `max_cache_mb` | number/string | `256` | 图片/blob 内存缓存上限 |
| `napcat_base_url` | string | 无 | 所有组默认 NapCat 反向 WS base url |
| `napcat_access_token` | string | 无 | 所有组默认 NapCat access token |
| `at_unprived_sender` | bool/string/number | `false` | 发件时是否 @ 非匿名且空间不可访问的投稿人 |
| `friend_request_window_sec` | number/string | `300` | 好友申请去重/限频窗口，组内可覆盖 |
| `friend_add_message` | string | 无 | 通过好友申请后自动发送的默认文本，组内可覆盖 |

这些基础参数控制投稿处理的核心流程。`process_waittime_sec` 决定了投稿聚合的时间窗口，在此窗口内的消息会被合并处理。`tz_offset_minutes` 影响定时发送和日志时间戳的显示，中国大陆用户应设置为 `480`（UTC+8）。

Sources: [config.rs](crates/app/src/config.rs#L100-L150), [config.rs](crates/app/src/config.rs#L300-L350)

### Web API 配置

`common.web_api` 控制 `/v1/*` HTTP API 的启用与配置。该 API 提供 RESTful 接口，支持第三方系统集成。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `web_api.enabled` | bool/string/number | `false` | 是否启用 `/v1/*` API |
| `web_api.port` | number/string | `10923` | API 监听端口，监听地址为 `0.0.0.0` |
| `web_api.root_token` | string | 无 | root token，建议用 `OQQWALL_API_TOKEN` 注入 |

启用 Web API 时必须提供长度至少 32 的 root token，否则 API 不会启动。root token 用于初始认证和创建子 token，建议通过环境变量注入以避免明文存储。

Sources: [config.rs](crates/app/src/config.rs#L150-L180), [api_v1.md](docs/api_v1.md#L1-L50)

### WebView 配置

`common.webview` 控制内置审核前端的启用与配置。该前端提供图形化审核界面，支持账号密码登录和权限管理。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `webview.enabled` | bool/string/number | `false` | 是否启用内置 WebView 审核前端 |
| `webview.host` | string | `0.0.0.0` | WebView 绑定地址，例如 `127.0.0.1` 或 `0.0.0.0` |
| `webview.port` | number/string | `10924` | WebView 监听端口 |
| `webview.session_ttl_sec` | number/string | `43200` | 登录会话有效期，范围会钳制到 300 秒至 7 天 |

启用 WebView 时必须配置至少一个管理员账号（全局或组级），否则 WebView 不会启动。会话有效期默认为 12 小时，可通过 `session_ttl_sec` 调整，但会被限制在 5 分钟到 7 天之间。

Sources: [config.rs](crates/app/src/config.rs#L180-L210), [webview.md](docs/webview.md#L1-L50)

### Renderer 配置

`common.renderer` 控制投稿预览 PNG 的渲染尺寸。这些参数影响输出图片的质量和文件大小。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `renderer.canvas_width_px` | number/string | `1152` | 渲染 PNG 的画布宽度，影响正文排版宽度和输出图片宽度 |
| `renderer.max_height_px` | number/string | `6912` | 渲染 PNG 的最大高度，超出时按渲染器截断策略处理 |

渲染器使用 Skia 图形库生成 PNG 图片。画布宽度决定了消息气泡的排版宽度，最大高度限制了单张图片的高度上限。这些参数需要根据审核群的显示需求和网络传输限制进行调整。

Sources: [config.rs](crates/app/src/config.rs#L210-L230), [renderer.rs](crates/drivers/src/renderer.rs#L70-L80)

### Telemetry 配置

`common.telemetry` 控制投稿遥测与训练样本的本地缓存和上传。遥测功能用于收集审核决策数据，支持机器学习模型的训练。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `telemetry.enabled` | bool/string/number | `true` | 是否在审核完成后生成训练样本 |
| `telemetry.local_dir` | string | `telemetry` | 本地遥测目录，相对 `OQQWALL_DATA_DIR` 解析 |
| `telemetry.upload_enabled` | bool/string/number | `true` | 是否启用批量上传 |
| `telemetry.upload_interval_sec` | number/string | `30` | 上传轮询间隔，范围会钳制到 1..86400 秒 |
| `telemetry.max_append_messages` | number/string | `2` | `append_offtopic` 负样本最多追加消息数，范围会钳制到 1..10 |

遥测功能默认启用，会收集审核决策并生成训练样本。上传 endpoint 和 token 为程序内置固定值，不通过配置文件暴露。上传批次大小固定为 20 条样本。

Sources: [config.rs](crates/app/src/config.rs#L230-L260), [telemetry.md](docs/telemetry.md#L1-L50)

## groups 账号组配置

`groups` 字段包含账号组配置，每个 key 是逻辑组名。每个组必须配置审核群 ID、账号列表和 NapCat 连接信息。组内配置会覆盖 `common` 中的同名默认值。

### 基础组参数

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `mangroupid` | string/number | 必填 | 审核群 ID |
| `accounts` | array/string | 必填 | QQ 账号列表，首项为主账号；运行时要求非空且均为数字 |
| `napcat_base_url` | string | 继承 `common` | 本组 NapCat 反向 WS base url |
| `napcat_access_token` | string | 继承 `common` | 本组 NapCat access token |
| `process_waittime_sec` | number/string | 继承 `common` | 本组投稿聚合等待时间，单位秒 |
| `min_interval_ms` | number/string | 继承 `common` | 本组发送最小间隔，单位毫秒 |
| `max_post_stack` | number/string | `1` | 暂存区上限；`1` 表示通过后直接进入发送流程 |
| `max_image_number_one_post` | number/string | 继承 `common` | 本组单条说说最大图片数 |
| `send_timeout_ms` | number/string | 继承 `common` | 本组发送超时，单位毫秒 |
| `send_max_attempts` | number/string | 继承 `common` | 本组发送失败最大尝试次数 |
| `send_schedule` | array[string] | `[]` | 每日定时 flush，格式 `HH:MM` |
| `individual_image_in_posts` | bool/string/number | `true` | 发送时是否附带原图 |
| `watermark_text` | string | 无 | 渲染图水印文本，空值不绘制 |
| `friend_request_window_sec` | number/string | 继承 `common` | 本组好友申请去重/限频窗口 |
| `friend_add_message` | string | 继承 `common` | 本组通过好友申请后自动发送的文本 |

每个账号组代表一个独立的运营单元，可以有自己的审核群、账号列表和发送策略。`mangroupid` 和 `accounts` 是必填字段，缺少会导致启动失败。

Sources: [config.rs](crates/app/src/config.rs#L260-L320), [config.rs](crates/app/src/config.rs#L400-L450)

### 快捷指令配置

组内支持配置三种快捷指令：`quick_replies`、`review_shortcuts` 和 `global_shortcuts`。这些指令可以简化审核操作流程。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `quick_replies` | object | `{}` | 快捷回复，格式为 `{ "指令": "回复文本" }` |
| `review_shortcuts` | object | `{}` | 审核快捷指令，格式为 `{ "指令": "步骤 DSL" }` |
| `global_shortcuts` | object | `{}` | 全局快捷指令，格式为 `{ "指令": "步骤 DSL" }` |
| `webview_admins` | array | `[]` | 本组 WebView 管理员账号 |

快捷指令 DSL 使用 `|` 或换行分隔步骤，例如 `匿 | 是`、`拒 | 拉黑 广告`。指令名不能为空，不能包含空白，不能命名为 `原始`。`quick_replies` 的指令名不能与内置审核指令冲突，`review_shortcuts` 不能与 `quick_replies` 重名。

审核快捷指令支持 `{args}`、`{review_code}`、`{sender_id}`、`{group_id}` 占位符，全局快捷指令支持 `{args}`、`{group_id}`。快捷指令可以覆盖同作用域内置指令，群内输入 `原始 <指令>` 可调用被覆盖的内置指令。

Sources: [config.rs](crates/app/src/config.rs#L800-L850), [shortcut.rs](crates/drivers/src/shortcut.rs#L1-L50)

### NapCat 连接配置

NapCat 反向 WS 的完整 URL 构造规则为：`ws://<napcat_base_url>/<QQ号>`。例如 `napcat_base_url = "127.0.0.1:3001/oqqwall/ws"` 且账号为 `3995477265`，则 NapCat 中应填写 `ws://127.0.0.1:3001/oqqwall/ws/3995477265`。

配置加载时会自动规范化 `napcat_base_url`，移除协议前缀（`ws://`、`wss://`、`http://`、`https://`）和尾部斜杠。`send_schedule` 命中本地时间分钟时会触发本组暂存内容 flush，同一日期同一时间点只触发一次。

Sources: [config.rs](crates/app/src/config.rs#L1000-L1050), [napcat.rs](crates/drivers/src/napcat.rs#L50-L80)

## WebView 管理员配置

WebView 管理员分为全局管理员和组管理员两种角色，配置位置不同但格式相同。

### 管理员条目格式

| Key | 类型 | 说明 |
| --- | --- | --- |
| `username` | string | 登录用户名 |
| `password` | string | 登录密码；推荐写 `sha256:<hex64>` |

如果配置里写入明文密码，程序加载时会自动改写为 `sha256:` 哈希并写回配置文件。密码哈希使用 SHA-256 算法，生成 64 位十六进制字符串。

### 管理员配置位置

- **全局管理员**：写在顶层 `webview_global_admins` 数组，可访问所有组
- **组管理员**：写在 `groups.<id>.webview_admins` 数组，只访问对应组

管理员账号在配置加载时会被合并处理，相同用户名和密码的条目会被合并其组权限。全局管理员和组管理员的权限在运行时通过 RBAC 模型控制。

Sources: [config.rs](crates/app/src/config.rs#L900-L950), [webview.md](docs/webview.md#L30-L50)

## 配置归一化与迁移

OQQWall_RUST 在启动时会自动检测并归一化不兼容的配置字段，确保向后兼容性。归一化过程会修改配置文件并写回磁盘。

### 自动迁移字段

| 旧字段 | 新字段 | 说明 |
| --- | --- | --- |
| `common.use_web_review` | `common.web_api.enabled` | Web API 启用开关 |
| `common.web_review_port` | `common.web_api.port` | Web API 端口 |
| `common.api_token` / `common.token` | `common.web_api.root_token` | Web API 根 token |
| `common.canvas_width_px` | `common.renderer.canvas_width_px` | 渲染画布宽度 |
| `common.max_height_px` | `common.renderer.max_height_px` | 渲染最大高度 |
| `groups.<id>.admins` | `groups.<id>.webview_admins` | 组管理员列表 |
| `groups.<id>.acount` | `groups.<id>.accounts` | 账号列表拼写修正 |
| `groups.<id>.mainqqid` + `minorqqid` | `groups.<id>.accounts` | 旧版账号字段迁移 |

归一化过程是幂等的，多次运行不会产生副作用。程序会保留未识别的字段，但会移除已废弃的字段（如 `mainqq_http_port`、`minorqq_http_port`）。

Sources: [config.rs](crates/app/src/config.rs#L600-L700), [config.rs](crates/app/src/config.rs#L750-L800)

## 完整配置示例

以下是一个包含常用配置的完整示例，展示了推荐的配置结构和参数设置：

```json
{
  "common": {
    "process_waittime_sec": 20,
    "tz_offset_minutes": 480,
    "max_cache_mb": 256,
    "at_unprived_sender": false,
    "friend_request_window_sec": 300,
    "web_api": {
      "enabled": false,
      "port": 10923,
      "root_token": ""
    },
    "webview": {
      "enabled": true,
      "host": "127.0.0.1",
      "port": 10924,
      "session_ttl_sec": 43200
    },
    "renderer": {
      "canvas_width_px": 1152,
      "max_height_px": 6912
    },
    "telemetry": {
      "enabled": true,
      "local_dir": "telemetry",
      "upload_enabled": true,
      "upload_interval_sec": 30,
      "max_append_messages": 2
    }
  },
  "groups": {
    "default": {
      "mangroupid": "123456789",
      "accounts": ["3995477265"],
      "napcat_base_url": "127.0.0.1:3001/oqqwall/ws",
      "napcat_access_token": "REDACTED",
      "max_post_stack": 1,
      "max_image_number_one_post": 30,
      "individual_image_in_posts": true,
      "send_schedule": ["08:30", "22:10"],
      "watermark_text": "",
      "friend_add_message": "",
      "quick_replies": {
        "补充信息": "请补充时间地点"
      },
      "review_shortcuts": {
        "匿": "匿 | 是"
      },
      "global_shortcuts": {
        "清队列": "删除待处理 | 删除暂存区"
      },
      "webview_admins": [
        {
          "username": "op",
          "password": "sha256:REDACTED"
        }
      ]
    }
  },
  "webview_global_admins": [
    {
      "username": "root",
      "password": "sha256:REDACTED"
    }
  ]
}
```

该示例配置启用了 WebView 审核前端，禁用了 Web API，设置了中国大陆时区偏移，并配置了基本的快捷指令。生产环境建议通过环境变量注入敏感信息（如 `napcat_access_token` 和 `web_api.root_token`）。

Sources: [config.md](docs/config.md#L200-L234), [config.rs](crates/app/src/config.rs#L1400-L1450)

## 最佳实践

### 配置管理建议

1. **敏感信息管理**：NapCat access token 和 Web API root token 建议通过环境变量注入，避免明文存储在配置文件中。

2. **配置备份**：定期备份 `config.json` 文件，特别是在修改配置前。配置文件包含所有运行时参数，丢失会导致服务中断。

3. **配置验证**：修改配置后建议先用 `cargo run -p OQQWall_RUST -- --tui` 检查配置有效性，TUI 编辑器会提供实时验证。

4. **分环境配置**：开发、测试和生产环境使用不同的配置文件，通过 `OQQWALL_CONFIG` 环境变量切换。

### 性能调优参数

1. **投稿聚合窗口**：`process_waittime_sec` 默认 20 秒，可根据投稿频率调整。高频投稿场景可适当增大以减少处理次数。

2. **内存缓存上限**：`max_cache_mb` 默认 256MB，根据服务器内存容量调整。图片密集型投稿需要更大的缓存。

3. **发送超时**：`send_timeout_ms` 默认 5 分钟，网络不稳定环境可适当增大。

4. **渲染尺寸**：`renderer.canvas_width_px` 和 `renderer.max_height_px` 影响输出图片质量，需根据审核群显示需求调整。

### 故障排查配置

配置问题通常表现为启动失败或功能异常，常见排查步骤：

1. **配置文件语法错误**：使用 JSON 验证工具检查 `config.json` 语法。

2. **必填字段缺失**：检查每个组的 `mangroupid` 和 `accounts` 是否配置。

3. **NapCat 连接失败**：验证 `napcat_base_url` 和 `napcat_access_token` 是否正确，检查 NapCat 服务是否运行。

4. **WebView 无法访问**：确认 `webview.enabled` 为 `true`，且至少配置了一个管理员账号。

5. **权限问题**：检查 `data/` 目录的写入权限，确保程序可以创建日志和遥测文件。

Sources: [runbook.md](docs/runbook.md#L50-L100), [oobe.md](docs/oobe.md#L1-L50)

## 下一步

配置文件准备完成后，建议按以下顺序进行后续操作：

1. **[首次运行与初始化](5-shou-ci-yun-xing-yu-chu-shi-hua)**：了解程序首次启动的初始化流程和验证步骤
2. **[开发环境搭建](3-kai-fa-huan-jing-da-jian)**：配置开发环境，准备进行代码修改和调试
3. **[审核指令系统](6-shen-he-zhi-ling-xi-tong)**：学习审核群内的指令操作，开始实际使用系统
4. **[生产环境部署](20-sheng-chan-huan-jing-bu-shu)**：了解生产环境的部署要求和最佳实践

配置文件是系统运行的基础，正确的配置能确保系统稳定运行。建议仔细阅读本文档，并根据实际需求调整各项参数。