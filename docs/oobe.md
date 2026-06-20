# oobe.md — 从 0 到首次跑通（生成 config.json）

`OQQWall_RUST` 提供一个交互式 OOBE（out-of-box experience），用于在首次部署时生成 `config.json` 骨架，减少手动填错字段的概率。

当直接运行主程序 `OQQWall_RUST` 且配置文件不存在时：
- 如果当前是交互终端，程序会自动进入 OOBE。
- 如果当前无交互终端（如 systemd/容器），程序会报错退出，并提示手动执行 OOBE。

## 用法

在仓库根目录执行：

```bash
cargo run -p OQQWall_RUST -- oobe
```

可选参数：

```bash
cargo run -p OQQWall_RUST -- oobe --config ./config.json
cargo run -p OQQWall_RUST -- oobe --force
```

## 交互项说明

OOBE 会依次询问并写入配置：

- `group id`：逻辑组名（默认 `default`），对应 `groups.<group_id>`。
- `mangroupid`：审核群 ID（必填）。
- `accounts`：账号列表（必填，逗号分隔；第一个为主账号）。
- `napcat_base_url`：NapCat 反向 WS 的 base url（示例：`0.0.0.0:3001/oqqwall/ws`）。
- `napcat_access_token`：NapCat access token（可留空，运行时用环境变量覆盖更安全）。
- `process_waittime_sec`：投稿聚合窗口秒数。
- `tz_offset_minutes`：时区偏移分钟数（中国大陆一般为 `480`）。
- `max_cache_mb`：内存图片缓存上限（MB）。
- `at_unprived_sender`：发件时是否 @ 非匿名投稿人。
- `max_post_stack` / `max_image_number_one_post` / `individual_image_in_posts` / `send_schedule`：发送暂存与排程相关配置。

字段的完整语义与兼容性说明见 `docs/config.md`。

## 下一步

1. 确认 NapCat 已启动并为 `accounts` 中每个账号配置反向 WS，连接到 `ws://<base_url>/<QQ号>`（示例：`ws://127.0.0.1:3001/oqqwall/ws/456787654`）。
2. 运行主程序：

```bash
cargo run -p OQQWall_RUST
```

3. 进入审核群，根据 `docs/command.md` 使用审核指令完成首次“通过/发送”。
