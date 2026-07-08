# HTTP API v1

本文档描述当前 Rust 版本已实现的对外 HTTP 审核接口。

## 1. 启用与配置

启用条件：
- `common.web_api.enabled = true`
- `common.web_api.root_token`（或环境变量 `OQQWALL_API_TOKEN`）存在且长度 >= 32

监听地址：
- `0.0.0.0:${common.web_api.port}`
- 默认端口：`10923`

兼容读取（启动时自动迁移）：
- `common.use_web_review` -> `common.web_api.enabled`
- `common.web_review_port` -> `common.web_api.port`
- `common.api_token` -> `common.web_api.root_token`

## 2. 鉴权模型

- 登录接口：`POST /v1/auth/login`
- 其他接口：`Authorization: Bearer <session_id>`
- Session 为临时会话（当前默认 12 小时）
- 子 token 可选设置 `allowed_groups`（组白名单）；root token 不受组限制。

权限枚举：
- `review.read`
- `review.write`
- `send.execute`
- `blacklist.read`
- `blacklist.write`
- `session.manage`
- `token.manage`

## 3. 统一错误返回

```json
{
  "error": {
    "code": "PERMISSION_DENIED",
    "message": "permission denied",
    "request_id": "req_xxx"
  }
}
```

常见状态码：
- `200`：成功
- `204`：成功且无响应体
- `400`：参数错误
- `401`：未认证或 session 失效
- `403`：权限不足
- `404`：资源不存在
- `422`：当前模型不支持该请求语义
- `503`：引擎命令通道不可用

## 4. 接口定义

### 4.1 登录

`POST /v1/auth/login`

请求：
```json
{
  "token": "root_or_sub_token"
}
```

响应：
```json
{
  "session_id": "32hex...",
  "expires_at": 1730000000,
  "permissions": ["review.read", "review.write"]
}
```

### 4.2 登出

`POST /v1/auth/logout`

请求头：`Authorization: Bearer <session_id>`

响应：`204 No Content`

### 4.3 强制下线 Session

`POST /v1/auth/sessions/{session_id}/revoke`

权限：`session.manage`

响应：`204 No Content`

### 4.4 创建子 Token

`POST /v1/auth/tokens`

权限：`token.manage`

请求：
```json
{
  "permissions": ["review.read", "review.write"],
  "expire_at": 1730000000,
  "allowed_groups": ["10001", "10002"]
}
```

说明：
- `allowed_groups` 可选；设置后该 token 仅允许操作这些组（业务接口会按组校验/过滤）。
- `allowed_groups` 必须是配置中存在的组 ID。

响应：
```json
{
  "token": "32hex...",
  "token_id": "tok_2",
  "expire_at": 1730000000,
  "allowed_groups": ["10001", "10002"]
}
```

### 4.5 稿件列表

`GET /v1/posts?stage=review_pending&cursor=0&limit=50`

权限：`review.read`

参数：
- `stage` 可选：
  - `drafted`
  - `render_requested`
  - `rendered`
  - `review_pending`
  - `reviewed`
  - `scheduled`
  - `sending`
  - `sent`
  - `rejected`
  - `skipped`
  - `manual`
  - `failed`
- `cursor` 可选，默认 `0`
- `limit` 可选，默认 `50`，范围 `1..200`

响应：
```json
{
  "items": [
    {
      "post_id": "123",
      "review_id": "456",
      "group_id": "10001",
      "stage": "review_pending",
      "external_code": 1193,
      "internal_code": 102,
      "sender_id": "1050373508",
      "created_at_ms": 1730000000123,
      "last_error": null
    }
  ],
  "next_cursor": 1
}
```

### 4.5.1 创建稿件

`POST /v1/posts/create`

权限：`review.write`

请求头：
- `Authorization: Bearer <session_id>`
- 可选 `Idempotency-Key: <key>`

请求体（示例）：
```json
{
  "target_account": "3391146750",
  "sender_id": "1050373508",
  "sender_name": "Alice",
  "sender_avatar_base64": "iVBORw0KGgoAAAANSUhEUgAA...",
  "messages": [
    {
      "message_id": "171082357",
      "time": 1767094033,
      "message": [
        { "type": "text", "data": { "text": "测试投稿系统" } },
        {
          "type": "image",
          "data": {
            "base64": "iVBORw0KGgoAAAANSUhEUgAA...",
            "mime": "image/png",
            "name": "cover.png"
          }
        },
        { "type": "face", "data": { "id": "5" } }
      ]
    }
  ]
}
```

字段约束：
- 顶层必填：`target_account`、`sender_id`、`messages`
- `target_account` 必须映射到已配置组；`group_id` 由服务端根据 `target_account` 推导，不再由客户端传入。
- token 若配置了 `allowed_groups`，推导出的 `group_id` 必须在白名单内。
- `sender_name` 可选；`sender_avatar_base64` 可选。
- `sender_id` 允许任意非空字符串；仅当 `sender_id` 为纯数字时才会尝试 QQ 资料兜底。
- `messages[*]` 必填：`message_id`、`time`、`message`
- 图片段 `type=image`：必须提供 `data.base64`；`data.mime`/`data.name` 可选。
- 支持的段类型最小字段：
  - `text`: `data.text`
  - `image`: `data.base64`（必须）
  - `face`: `data.id`
  - `reply`: `data.id`（可选）
  - `forward`: `data.id`（可选）
  - `json`/`poke`: 无必填字段
  - `video`/`file`/`record`: `data.base64` 或 `data.url`/`data.file`/`data.path` 至少一个
- 其他多余字段会被忽略，不需要传 NapCat 原始裸消息里的全部字段。

无效数据处理：
- 未知段：折叠为占位文本（如 `[未知段:xxx]`），不使整单失败。
- 段字段缺失或 base64 非法：折叠为占位文本（如 `[image:invalid]`）。
- 通过响应里的 `warnings` 和 `normalization` 返回归一化结果。

昵称与头像兜底：
- 若 `sender_name` 缺失且 `sender_id` 为纯数字：尝试调用 `get_stranger_info` 获取昵称。
- 若昵称仍无法获取：使用 `sender_name = "未知"`。
- 若传入 `sender_avatar_base64` 且合法：优先使用传入头像。
- 若头像未传且“上一步昵称成功通过 `get_stranger_info` 获取”：尝试现有头像获取链路；失败则使用匿名默认头像。
- 若头像未传且不满足上一步条件：直接使用匿名默认头像。

响应（示例）：
```json
{
  "request_id": "req_xxx",
  "post_id": "18089374114424392123",
  "review_code": 102,
  "accepted_messages": 1,
  "normalization": {
    "unknown_segments": 0,
    "invalid_segments_folded": 0
  },
  "warnings": []
}
```

返回时机：
- 接口立即返回 `post_id`。
- 会在最多 3 秒内尝试附带 `review_code`；超时则 `review_code = null`（后续可通过 `/v1/posts/{post_id}` 查询）。
- 若传 `Idempotency-Key`，同一 session + 组 + 账号 + key 的重复请求返回同一个创建结果。

### 4.5.2 创建无需渲染消息记录的稿件

`POST /v1/posts/create_rendered`

权限：`review.write`

请求头：
- `Authorization: Bearer <session_id>`
- 可选 `Idempotency-Key: <key>`

请求体（示例）：
```json
{
  "target_account": "3391146750",
  "image_base64": "iVBORw0KGgoAAAANSUhEUgAA...",
  "image_mime": "image/png",
  "sender_id": "1050373508",
  "sender_name": "Alice"
}
```

字段约束：
- 必填：`target_account`、`image_base64`、`image_mime`
- `image_mime` 仅支持：`image/png`、`image/jpeg`、`image/jpg`、`image/webp`
- `sender_id` 可选：
  - 不传或空：按匿名投稿处理
  - 纯数字：可在发送阶段按配置 `at_unprived_sender` 触发 `@`
  - 非数字：允许创建，但禁用 `@`（会返回 warning）
- `sender_name` 可选；当 `sender_id` 为纯数字且 `sender_name` 缺失时，会尝试 `get_stranger_info`，失败回退 `"未知"`。

处理流程：
- 接口直接接收“已渲染图片”，落盘为 blob，并写入投稿草稿 + 渲染完成事件。
- 创建后进入待审核流程，返回 `post_id`，并在最多 3 秒内尝试附带 `review_code`。
- 支持幂等：同一 session + 组 + 账号 + key 重复请求返回同一个创建结果。

响应（示例）：
```json
{
  "request_id": "req_xxx",
  "post_id": "18089374114424392123",
  "review_code": 3021,
  "accepted_messages": 1,
  "normalization": {
    "unknown_segments": 0,
    "invalid_segments_folded": 0
  },
  "warnings": []
}
```

### 4.6 稿件详情

`GET /v1/posts/{post_id}`

权限：`review.read`

响应（示例）：
```json
{
  "post_id": "123",
  "review_id": "456",
  "review_code": 102,
  "group_id": "10001",
  "stage": "review_pending",
  "external_code": 1193,
  "sender_id": "1050373508",
  "session_id": "999",
  "created_at_ms": 1730000000123,
  "is_anonymous": false,
  "is_safe": true,
  "blocks": [
    {"kind":"text","text":"正文"},
    {
      "kind":"attachment",
      "media_kind":"image",
      "reference_type":"blob_id",
      "reference":"777",
      "size_bytes": 12345
    }
  ],
  "render_png_blob_id": "888",
  "last_error": null
}
```

### 4.7 审核决策

`POST /v1/reviews/{review_id}/decision`

权限：`review.write`

请求：
```json
{
  "action": "approve|reject|delete|defer|skip|immediate|refresh|rerender|select_all|toggle_anonymous|expand_audit|show|comment|reply|blacklist|quick_reply|merge",
  "comment": "可选",
  "delay_ms": 60000,
  "text": "可选（comment/reply）",
  "quick_reply_key": "可选（quick_reply）",
  "target_review_code": 1234
}
```

说明：
- `blacklist` 时 `comment` 作为拉黑理由。
- `defer` 时可传 `delay_ms`。
- `comment/reply` 需要 `text`（兼容读取 `comment`）。
- `quick_reply` 需要 `quick_reply_key`。
- `merge` 需要 `target_review_code`。
- 支持幂等请求头：`Idempotency-Key: <key>`。

响应：
```json
{
  "review_id": "456",
  "status": "applied"
}
```

### 4.8 批量审核决策

`POST /v1/reviews/batch`

权限：`review.write`

请求：
```json
{
  "review_ids": ["456", "789"],
  "action": "approve"
}
```

响应：
```json
{
  "accepted": 2,
  "failed": []
}
```

### 4.9 Blob 读取

`GET /v1/blobs/{blob_id}`

权限：`review.read`

响应：二进制流（`Content-Type` 按文件扩展名推断）。

### 4.10 黑名单列表

`GET /v1/blacklist?group_id=10001&cursor=0&limit=50`

权限：`blacklist.read`

响应：
```json
{
  "items": [
    {
      "group_id": "10001",
      "sender_id": "1050373508",
      "reason": "广告"
    }
  ],
  "next_cursor": null
}
```

### 4.11 新增黑名单

`POST /v1/blacklist`

权限：`blacklist.write`

请求：
```json
{
  "group_id": "10001",
  "sender_id": "1050373508",
  "reason": "广告"
}
```

响应：`204 No Content`

### 4.12 删除黑名单

`DELETE /v1/blacklist/{group_id}/{sender_id}`

权限：`blacklist.write`

响应：`204 No Content`

### 4.13 触发发件

`POST /v1/posts/send`

权限：`send.execute`

请求：
```json
{
  "post_ids": ["123", "124"],
  "mode": "immediate|scheduled",
  "schedule_at": null
}
```

响应：
```json
{
  "accepted": 1,
  "failed": [
    {
      "post_id": "124",
      "reason": "post has no review_id"
    }
  ]
}
```

`schedule_at` 语义：
- 预期用途：当 `mode = scheduled` 时指定未来发送时间（Unix 秒/毫秒时间戳）。
- 当前实现状态：尚未接入调度链路，传入 `schedule_at` 会返回 `422`。

### 4.14 指定账号发送私信

`POST /v1/messages/private/send`

权限：`send.execute`

请求：
```json
{
  "target_account": "3391146750",
  "group_id": "10001",
  "user_id": "123456789",
  "message": [
    { "type": "text", "data": { "text": "hello" } },
    { "type": "face", "data": { "id": "14" } }
  ]
}
```

字段约束：
- 必填：`target_account`、`user_id`、`message`
- `message` 必须是非空数组，且每个元素必须是对象；服务端不限制段类型，按 NapCat 段原样透传。
- `group_id` 可选：
  - 传入时：必须是已配置组，且 `target_account` 必须属于该组，且 token 必须有该组权限。
  - 未传时：先筛出 token 可访问且包含 `target_account` 的组：
    - 若仅 1 个候选组：直接使用；
    - 若多个候选组：优先尝试“当前在线主账号恰好为 `target_account`”唯一命中；否则要求显式传 `group_id`。
- 无论是否传 `group_id`，`target_account` 本身必须在线；离线会返回错误。

响应：
```json
{
  "request_id": "req_xxx",
  "status": "ok",
  "target_account": "3391146750",
  "group_id": "10001",
  "user_id": "123456789",
  "message_id": "1873219",
  "raw": {
    "status": "ok",
    "retcode": 0,
    "data": {
      "message_id": 1873219
    },
    "echo": "echo-1"
  }
}
```

错误语义：
- `400`：参数错误（如空消息、未知账号、`group_id` 与账号不匹配）
- `403`：无组权限
- `409`：未传 `group_id` 且命中多个在线候选组（需显式指定 `group_id`）
- `503`：目标账号离线或 NapCat 调用失败

## 5. 兼容性说明

- `chooseall` 属于前端交互状态，不提供独立后端接口。
- 发送能力仍复用现有审核/调度事件链；接口是命令入口，不直接绕过引擎状态机。
