# config.md - OQQWall_RUST 配置说明

本文只记录当前 `OQQWall_RUST` 实际读取并会影响运行行为的配置项。未列出的字段不需要手写；程序或 TUI 可能会在保存时归一化配置文件。

## 配置文件

- 默认路径：`./config.json`
- 覆盖路径：`OQQWALL_CONFIG=/path/to/config.json`
- 生成配置：`cargo run -p OQQWall_RUST -- oobe`
- 编辑配置：`cargo run -p OQQWall_RUST -- --tui`

主程序启动时读取配置文件。配置文件不存在时，交互终端会自动进入 OOBE；非交互环境会退出并提示手动生成配置。

推荐结构：

```json
{
  "common": {},
  "groups": {
    "default": {}
  },
  "webview_global_admins": []
}
```

## 环境变量覆盖

| 环境变量 | 覆盖目标 | 说明 |
| --- | --- | --- |
| `OQQWALL_CONFIG` | 配置文件路径 | 默认 `config.json` |
| `OQQWALL_NAPCAT_BASE_URL` | NapCat base url | 覆盖所有组的 `napcat_base_url` |
| `OQQWALL_NAPCAT_TOKEN` | NapCat access token | 覆盖所有组的 `napcat_access_token` |
| `OQQWALL_API_TOKEN` | `common.web_api.root_token` | 覆盖 Web API root token |
| `OQQWALL_PROCESS_WAITTIME_MS` | 全局默认投稿聚合窗口 | 单位是毫秒，优先于 `common.process_waittime_sec`；组内 `process_waittime_sec` 仍可覆盖 |
| `OQQWALL_MAX_CACHE_MB` | `common.max_cache_mb` | 图片/blob 内存缓存上限 |
| `OQQWALL_DATA_DIR` | 运行数据目录 | 默认 `data`，影响日志、遥测、本地 blob 等 |

## common

`common` 是全局默认值。组内同名发送参数会覆盖全局默认值。

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

### Web API

`common.web_api` 控制 `/v1/*` HTTP API。开启时必须提供长度至少 32 的 root token，否则 API 不会启动。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `web_api.enabled` | bool/string/number | `false` | 是否启用 `/v1/*` API |
| `web_api.port` | number/string | `10923` | API 监听端口，监听地址为 `0.0.0.0` |
| `web_api.root_token` | string | 无 | root token，建议用 `OQQWALL_API_TOKEN` 注入 |

### WebView

`common.webview` 控制内置审核前端。开启时必须配置至少一个 `webview_global_admins` 或组内 `webview_admins`，否则 WebView 不会启动。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `webview.enabled` | bool/string/number | `false` | 是否启用内置 WebView 审核前端 |
| `webview.host` | string | `0.0.0.0` | WebView 绑定地址，例如 `127.0.0.1` 或 `0.0.0.0` |
| `webview.port` | number/string | `10924` | WebView 监听端口 |
| `webview.session_ttl_sec` | number/string | `43200` | 登录会话有效期，范围会钳制到 300 秒至 7 天 |

### Telemetry

`common.telemetry` 控制投稿遥测与训练样本本地缓存/上传。上传 endpoint 和 token 为程序内置固定值，不通过配置文件暴露。

| Key | 类型 | 默认值 | 作用 |
| --- | --- | --- | --- |
| `telemetry.enabled` | bool/string/number | `true` | 是否在审核完成后生成训练样本 |
| `telemetry.local_dir` | string | `telemetry` | 本地遥测目录，相对 `OQQWALL_DATA_DIR` 解析 |
| `telemetry.upload_enabled` | bool/string/number | `true` | 是否启用批量上传 |
| `telemetry.upload_interval_sec` | number/string | `30` | 上传轮询间隔，范围会钳制到 1..86400 秒 |
| `telemetry.max_append_messages` | number/string | `2` | `append_offtopic` 负样本最多追加消息数，范围会钳制到 1..10 |

## groups

`groups` 是账号组配置。每个 key 是逻辑组名；每个组必须能解析出审核群、账号列表和 NapCat base url。

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
| `quick_replies` | object | `{}` | 快捷回复，格式为 `{ "指令": "回复文本" }` |
| `review_shortcuts` | object | `{}` | 审核快捷指令，格式为 `{ "指令": "步骤 DSL" }` |
| `global_shortcuts` | object | `{}` | 全局快捷指令，格式为 `{ "指令": "步骤 DSL" }` |
| `webview_admins` | array | `[]` | 本组 WebView 管理员账号 |

NapCat 反向 WS 的完整 URL 是：

```text
ws://<napcat_base_url>/<QQ号>
```

例如 `napcat_base_url = "127.0.0.1:3001/oqqwall/ws"` 且账号为 `3995477265`，则 NapCat 中填写：

```text
ws://127.0.0.1:3001/oqqwall/ws/3995477265
```

`send_schedule` 命中本地时间分钟时会触发本组暂存内容 flush；同一日期同一时间点只触发一次。

## WebView 管理员

全局管理员写在顶层 `webview_global_admins`，可访问所有组。组管理员写在 `groups.<id>.webview_admins`，只访问对应组。

管理员条目：

| Key | 类型 | 说明 |
| --- | --- | --- |
| `username` | string | 登录用户名 |
| `password` | string | 登录密码；推荐写 `sha256:<hex64>` |

如果配置里写入明文密码，程序加载时会改写为 `sha256:` 哈希。

## 快捷指令

`review_shortcuts` / `global_shortcuts` 的 value 是步骤 DSL：

- 步骤用 `|` 或换行分隔，例如 `匿 | 是`、`拒 | 拉黑 广告`。
- 指令名不能为空，不能包含空白，不能命名为 `原始`。
- `quick_replies` 的指令名不能与内置审核指令冲突。
- `review_shortcuts` 不能与 `quick_replies` 重名。
- 审核快捷指令支持 `{args}`、`{review_code}`、`{sender_id}`、`{group_id}`。
- 全局快捷指令支持 `{args}`、`{group_id}`。
- 快捷指令可以覆盖同作用域内置指令；群内输入 `原始 <指令>` 可调用被覆盖的内置指令。

## 示例

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
