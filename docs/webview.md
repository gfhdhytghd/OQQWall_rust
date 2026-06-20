# webview.md — WebView 审核前端使用与运维

本文档说明 Rust 版内置 WebView 审核前端（Vue + Rust）的部署、登录、权限模型、构建与排障。

## 1. 功能边界

- `/v1/*`：对外 API（Token + Bearer Session），用于第三方接入。
- `/auth/*` + `/api/*` + `/`：内置 WebView（账号密码 + Cookie Session），用于内部审核页面。
- 两者鉴权体系独立：WebView 登录不会产生 `/v1` token session，反之亦然。

## 2. 配置项

参考 `docs/config.md`，最小示例：

```json
{
  "common": {
    "web_api": {
      "enabled": true,
      "port": 10923,
      "root_token": "REDACTED_32+"
    },
    "webview": {
      "enabled": true,
      "host": "127.0.0.1",
      "port": 10924,
      "session_ttl_sec": 43200
    }
  },
  "groups": {
    "GroupA": {
      "mangroupid": "123456",
      "accounts": ["3995477265"],
      "napcat_base_url": "127.0.0.1:3001/oqqwall/ws",
      "napcat_access_token": "REDACTED",
      "webview_admins": [
        { "username": "op_a", "password": "sha256:REDACTED", "role": "group_admin" }
      ]
    }
  },
  "webview_global_admins": [
    { "username": "root", "password": "sha256:REDACTED", "role": "global_admin" }
  ]
}
```

兼容迁移（启动时自动）：

- `common.use_web_review` -> `common.web_api.enabled`
- `common.web_review_port` -> `common.web_api.port`
- `common.api_token` / `common.token` -> `common.web_api.root_token`
- `groups.<id>.admins` -> `groups.<id>.webview_admins`

## 3. 登录与会话

- 登录接口：`POST /auth/login`
- 登出接口：`POST /auth/logout`
- 当前用户：`GET /auth/me`
- 会话保存于 `HttpOnly` Cookie：`oqqwall_webview_session`
- 会话 TTL：`common.webview.session_ttl_sec`（默认 12 小时）

密码规则：

- 推荐写入 `sha256:<hex64>`。
- 若配置里是明文，加载时会归一化为 `sha256:`（并写回配置文件）。

## 4. RBAC 权限模型

- `global_admin`：可查看和操作所有组。
- `group_admin`：仅可查看和操作其授权组。

授权来源：

- 组管理员：`groups.<id>.webview_admins`
- 全局管理员：`webview_global_admins`

后端会在读取帖子、详情、blob、审核动作、批量动作时执行组级权限校验。

## 5. WebView 接口（内部）

- `GET /api/posts?stage=review_pending&limit=200`
- `GET /api/posts/{post_id}`
- `GET /api/blobs/{blob_id}`
- `POST /api/reviews/{review_id}/decision`
- `POST /api/reviews/batch`

审核动作支持集合与 `/v1/reviews/{review_id}/decision` 一致（见 `docs/api_v1.md`）。

## 6. 前端构建与嵌入

WebView 前端目录：`crates/app/webview-ui`

开发：

```bash
cd crates/app/webview-ui
bun install
bun run dev
```

开发服务器只提供前端资源；`/auth/*` 和 `/api/*` 会代理到内置 WebView 后端，默认目标为
`http://127.0.0.1:10924`。如果后端端口不同：

```bash
VITE_WEBVIEW_BACKEND=http://127.0.0.1:<port> bun run dev
```

生产构建（生成 dist）：

```bash
cd crates/app/webview-ui
bun run build
```

Rust 构建时会读取 `crates/app/webview-ui/dist` 并嵌入二进制（`crates/app/build.rs`）。

若未构建 dist，WebView 会返回提示页：`webview-ui dist not found`。

## 7. 部署建议

- API 与 WebView 使用分离端口（默认 `10923` / `10924`）。
- WebView 主机可通过 `common.webview.host` 指定：
  - `127.0.0.1`：仅本机访问
  - `0.0.0.0`：监听所有网卡
- WebView 仅开放内网；公网请放在反向代理后并加额外访问控制。
- root token 建议使用环境变量 `OQQWALL_API_TOKEN` 注入。

## 8. 常见排障

1. 页面打不开
- 检查 `common.webview.enabled=true`
- 检查监听端口 `common.webview.port`
- 查看服务日志是否有 `webview bind failed`

2. 登录失败
- 检查账号是否存在于 `webview_global_admins` 或 `groups.*.webview_admins`
- 检查密码 hash 格式是否为 `sha256:<hex64>`

3. 能看列表但操作 403
- 账号为 `group_admin` 且目标稿件不在授权组
- 检查该账号是否需改为 `global_admin`

4. 图片/附件预览失败
- 检查 `data/blobs/` 是否存在对应文件
- 检查帖子/blob 是否属于当前账号授权组

5. 构建后页面仍旧
- 重新执行 `bun run build`
- 重新 `cargo build`，确保新 dist 被重新嵌入
