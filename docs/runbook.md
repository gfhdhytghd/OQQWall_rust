# runbook.md — OQQWall_RUST 单机运行手册（部署/运维/排障）

> 适用范围：单机版 OQQWall_RUST（未来集群未启用）。  
> 目标：给运维与开发一套**能直接照做**的启动/升级/备份/恢复/排障流程。  
> 约束：配置来自 **JSON 文件**（不硬编码），运行时“常规只写不读”，读盘主要发生在重启恢复。

---

## 0. 术语与组件

- **OQQWall_RUST**：主程序（单二进制），包含 engine、drivers、Web API 与 WebView（admin web，可选）。
- **NapCat/OneBot**：QQ 接入层（可被 OQQWall_RUST 管理为子进程或外部自行运行）。
- **Journal**：事件日志（append-only）。
- **Snapshot**：快照（周期生成，用于缩短回放时间）。
- **BlobStore**：产物与附件存储（RAM cache + 异步落盘备份）。
- **审核群**：管理员审稿群，接收预览与指令。

---

## 1. 运行目录与文件布局

默认使用工作目录下的 `data/`：

```
data/
journal/                 # append-only 事件日志（分段）
snapshot/                # 最近快照
blobs/                   # 产物与附件备份（写多读少）
telemetry/               # 遥测缓存（训练样本与聊天对象）
logs/                    # 可选：文件日志
```

建议将 `data/` 放在稳定磁盘（不要放 tmpfs），否则重启恢复会缺失历史。

调试版会把 stderr 调试日志同步写入 `data/logs/debug.log`，可用 `OQQWALL_DEBUG_LOG` 覆盖路径（基于 `OQQWALL_DATA_DIR`）。

遥测启用后会写入：

- `data/telemetry/pending_samples.jsonl`
- `data/telemetry/chat_objects/*.json`

上传成功后会自动删除已上传样本，并清理无引用的聊天对象。

---

## 2. 前置检查（上线前必做）

### 2.1 系统依赖
- Linux 推荐（systemd 运维更方便）
- 网络：能访问 NapCat OneBot 端口（本机/局域网）
- 时间：系统时钟正确（建议开启 NTP）
- 文件权限：`data/` 可写

### 2.2 配置文件
- `config.json` 必须存在并可读
- 必填字段检查：
  - 每个 group 的 `napcat_base_url` / `napcat_access_token`
  - 每个 group 的 `mangroupid / accounts`（且 `accounts[0]` 为主账号）
- 建议先用 OOBE 生成骨架，再按 `docs/config.md` 对照检查

### 2.3 调试配置（仅 debug build 生效）

`devconfig.json` 用于调试选项，**仅在 debug build 下读取**，release build 会忽略。

最小示例：

```json
{
  "use-virt-qzone": false
}
```

字段说明：

* `use-virt-qzone`：true 时启用虚拟 QQ 空间发送（不会真实发布）。

虚拟发送使用内置模拟器（debug build 内置 HTTP 服务）：

* 访问 `http://127.0.0.1:18080/` 查看发送记录
* 数据接口：`http://127.0.0.1:18080/data`
* 记录只保存在内存中（默认保留最近 50 条），重启后清空

### 2.4 NapCat 模式确认
两种模式任选其一：

#### A) managed（推荐）
- `common.manage_napcat_internal=true`
- OQQWall_RUST 会拉起/监控 NapCat 子进程，并自动重启

#### B) external
- `common.manage_napcat_internal=false`
- 运维自己启动 NapCat（或 docker），OQQWall_RUST 只连接 OneBot 地址

### 2.5 代码包/资源包拆分发布
- 从当前仓库执行：
  - `./scripts/package_split_release.sh`
- 会生成两个文件（在 `dist/`）：
  - `OQQWall_RUST-bin-*.tar.gz`（仅主程序）
  - `OQQWall_RUST-res-*.tar.gz`（`res/` 资源目录）
- 部署时把两个包解压到同一目录，目录示例：
  - `./OQQWall_RUST`
  - `./res/...`
- 若程序目录下没有 `res/` 但存在 `OQQWall_RUST-res*.tar.gz` / `res*.tar.gz`（含 `.tgz/.tar`），启动时会先做 SHA256 校验（哈希在编译时计算并内置），校验通过后才自动解压到程序目录。
- 若 `res/` 缺失或关键资源文件缺失，程序会在启动时报错并退出。
- 可通过 `OQQWALL_RES_DIR=/path/to/res` 指定资源目录。

---

## 3. 启动/停止

### 3.1 前台启动（调试）
```bash
./OQQWall_RUST
```

### 3.2 systemd 启动（推荐）

创建 `/etc/systemd/system/OQQWall_RUST.service`：

```ini
[Unit]
Description=OQQWall_RUST
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/OQQWall_RUST
ExecStart=/opt/OQQWall_RUST/OQQWall_RUST
Restart=always
RestartSec=2
# 当前版本使用环境变量指定配置文件路径
Environment=OQQWALL_CONFIG=/opt/OQQWall_RUST/config.json
# 可选：把 token 放 env，不写入 config.json
Environment=OQQWALL_NAPCAT_TOKEN=REDACTED
# 建议限制资源（按机器情况调整）
# MemoryMax=2G
# CPUQuota=200%
# 日志
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

启用并启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now OQQWall_RUST
```

查看状态：

```bash
sudo systemctl status OQQWall_RUST
journalctl -u OQQWall_RUST -f
```

### 3.3 停止

```bash
sudo systemctl stop OQQWall_RUST
```

---

## 4. 日常运维操作

### 4.1 查看当前队列/状态（推荐途径）

* 若启用 WebView（`common.webview.enabled=true`）：

  * 打开 `http://<host>:<common.webview.port>/` 查看：

    * 待审核 / 待发送 / 发送中 / 失败 / 人工介入
    * 预览 PNG 图
  * 账号权限与登录细节见 `docs/webview.md`
* 若仅群内：

  * 使用全局指令（参见 `docs/command.md`）

    * `@机器人 待处理`
    * `@机器人 信息 <review_code>`
    * `@机器人 自检`

### 4.2 触发“立即发送/flush”

* 审核指令：`立即`（如果实现）
* 或全局：`@机器人 发送暂存区`（如果映射为 flush）

### 4.3 NapCat 重启/修复

* 全局指令（如果实现）：`@机器人 系统修复`
* 或 systemd 重启：`sudo systemctl restart OQQWall_RUST`（managed 模式会带 NapCat 一起重启）

### 4.4 遥测队列检查

用于模型训练的数据缓存可通过以下命令检查：

```bash
wc -l data/telemetry/pending_samples.jsonl
ls -1 data/telemetry/chat_objects | wc -l
```

含义：

- `pending_samples.jsonl` 行数：待上传样本条数
- `chat_objects` 文件数：本地去重后的完整聊天对象数

若 `pending_samples` 长期不下降，优先检查：

- `common.telemetry.upload_enabled` 是否开启
- 内置 telemetry collector 是否可达
- `data/logs/debug.log`（或 `OQQWALL_DEBUG_LOG` 指向的文件）里是否出现 `telemetry upload start/success/failed`

Collector 部署与 API 见：`docs/telemetry_collector.md`。

如需独立部署 collector，请按 `docs/telemetry_collector.md` 中 “6. Docker 运行” 执行；若要让主程序对接自建 collector，需要同时修改客户端内置 telemetry endpoint / token 并重新编译。

---

## 5. 升级流程（安全、可回滚）

> 原则：升级前备份 `data/`，升级后若异常可回滚二进制并恢复 `data/`。

### 5.1 升级步骤

1. 停服务：

```bash
sudo systemctl stop OQQWall_RUST
```

2. 备份数据目录：

```bash
tar -czf OQQWall_RUST-data-$(date +%F_%H%M%S).tar.gz data/
```

3. 替换二进制（与静态资源，如果有）：

```bash
cp OQQWall_RUST /opt/OQQWall_RUST/OQQWall_RUST.new
mv /opt/OQQWall_RUST/OQQWall_RUST /opt/OQQWall_RUST/OQQWall_RUST.old
mv /opt/OQQWall_RUST/OQQWall_RUST.new /opt/OQQWall_RUST/OQQWall_RUST
chmod +x /opt/OQQWall_RUST/OQQWall_RUST
```

4. 启动：

```bash
sudo systemctl start OQQWall_RUST
journalctl -u OQQWall_RUST -n 200 --no-pager
```

### 5.2 回滚

1. 停服务
2. 用旧二进制替换
3. 如数据结构不兼容（极少发生，除非 schema 变更），恢复备份 data/

> 建议：事件与快照增加 `schema_version`，升级时做兼容读取或迁移（最好只增字段，不删字段）。

---

## 6. 备份与恢复

### 6.1 备份策略

最小备份集：

* `data/journal/`
* `data/snapshot/`
* `data/blobs/`（可选但强烈建议：否则历史预览/附件可能缺失）

频率建议：

* journal/snapshot：每天至少一次
* blobs：每天一次或按容量滚动

### 6.2 恢复流程（换机器/重装系统）

1. 安装 OQQWall_RUST 与 config.json
2. 将备份 `data/` 解压到目标目录
3. 启动服务
4. 首次启动会：

   * 读取 snapshot
   * 回放 journal
   * 重建 StateView
   * 继续处理 pending item（会触发一些 retry/publish/send）

### 6.3 恢复验证

* web review（admin web）或 `@机器人 待处理` 检查待审核/待发送是否存在
* 检查 “发送中/失败/人工介入” 列表是否合理

---

## 7. 常见故障与排障

### 7.1 无法接收消息（Ingress 为空）

可能原因：

* OneBot 未连接 / token 错误
* NapCat 未运行（external 模式）
* WS/HTTP 端口不通（防火墙）

排查步骤：

1. 看日志是否出现 `OneBotConnected/Disconnected`（或类似）
2. 检查 token：`OQQWALL_NAPCAT_TOKEN` 与 config 是否一致
3. external 模式下，用 curl 测 OneBot 端口
4. managed 模式下，检查 NapCat 子进程是否被拉起（日志中有 pid）

### 7.2 投稿不成稿（一直不 close session）

可能原因：

* `process_waittime_sec` 设置过大
* Timer tick 未运行（engine 卡死）

排查步骤：

1. 查看日志：是否有 `DraftSessionAppended` 与 `DraftSessionClosed`
2. 调整 waittime 为 20~60 秒测试
3. 检查 CPU/内存是否打满（渲染/下载阻塞）

### 7.3 审核群不发预览/不发消息

可能原因：

* audit_group_id 配错
* preview 模式为 png_full，渲染队列卡住
* OneBot 发消息失败

排查：

1. 看是否有 `ReviewPublishRequested` 事件
2. 若有但没有 `ReviewPublished`，看失败原因（会有 retry）
3. 临时关闭预览（只发摘要文本），验证链路

### 7.4 发送队列积压不动

可能原因：

* 发送窗口限制（不在 send_windows）
* min_interval 太大
* 所有账号 cooldown
* sending 单写者卡死（in-flight 不释放）

排查：

1. 看 SendPlan 的 not_before 是否在未来
2. 看 group_runtime.last_send_at 与 min_interval
3. 看 account.cooldown_until
4. 检查是否存在 `SendStarted` 但无 `SendSucceeded/SendFailed`（driver 卡死）
5. 必要时重启服务（事件溯源保证不丢）

### 7.5 NapCat 重启风暴

表现：

* 日志反复出现 NapCatProcessStarted/Exited

可能原因：

* 配置/端口冲突
* token 不对导致连接不停失败
* NapCat 本身崩溃

处理：

1. 临时切换 `common.manage_napcat_internal=false` 用手工方式起 NapCat
2. 降低自动重启频率（加冷却窗口）
3. 收集 NapCat stderr 日志

### 7.6 journal 查看（TUI）

用于快速浏览事件，定位某段错误/缺失或确认回放顺序：

```bash
cargo run -p OQQWall_RUST --bin journal_tui -- [data_dir]
```

常用按键：`q/esc` 退出，`r` 重载，`t` 切换视图，`u` 用户视图，`a` 全量视图，方向键或 `j/k` 移动，`PgUp/PgDn` 翻页，`g/G` 或 `Home/End` 跳转，`Tab` 或 `h/l` 切换焦点（用户视图），`Ctrl+u/d` 滚动详情。鼠标：点击选择/切换，详情面板左键拖拽复制（OSC52），滚轮滚动列表/详情。

OSC52 复制注意事项：

* 终端需要支持 OSC52（Konsole/kitty/WezTerm/iTerm2 等）。
* tmux 内需要允许剪贴板转发：

```tmux
set -g set-clipboard on
set -as terminal-features ',xterm-256color:clipboard'
set -as terminal-features ',tmux-256color:clipboard'
```

* Konsole 需在设置中允许 OSC52 写剪贴板。

### 7.7 渲染失败（PNG）

可能原因：

* 字体缺失（度量/布局失败）
* 资源图标缺失（匿名头像、file icons）
* 渲染库问题（resvg/usvg）

处理：

1. 优先切回“仅摘要文本”以确保审核不断
2. 检查资源是否随二进制正确打包
3. 对文本做 XML escape，避免非法字符导致渲染失败

---

## 8. 数据损坏与一致性策略

### 8.1 journal 损坏

* 典型原因：非正常断电 + 没有 flush
* 策略：

  * journal 分段，每段带 CRC
  * 读取回放时遇到坏段：截断到最后一条完整事件

### 8.2 snapshot 损坏

* 读取 snapshot 失败则忽略 snapshot，从 journal 全量回放（会慢）

---

## 9. 运行参数建议（单机）

### 9.1 对低配机（2c2g）

* 默认审核预览：`png_low`
* PNG 渲染线程池：1
* 下载附件并发：2（或 1）
* journal flush：每 50ms 或 256KB
* blob RAM cache：<= 512MB

### 9.2 对中配机

* 可启用 `png_low` 作为审核预览
* 下载附件并发可提高到 4

---

## 10. 安全建议

* `napcat_access_token` 建议用环境变量提供，避免落盘（`OQQWALL_NAPCAT_TOKEN`）
* WebView（admin web）只绑定内网（或加反向代理鉴权）
* 日志中避免打印 token、cookie、完整私密内容（做脱敏）

---

## 11. 附：最小排障信息收集清单（发给开发）

当出现问题，请提供：

* `config.json`（脱敏 token/cookie）
* `data/journal/` 最近 1~2 个分段
* `data/snapshot/latest.snap`（如有）
* `journalctl -u OQQWall_RUST -n 500`
* 问题发生的时间点、对应 `review_code/post_id`

---
