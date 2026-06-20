# OQQWall TUI 功能清单

本文档基于 `oqqwall_tui.py` 的实现整理，描述 Linux 终端 TUI 管理器的实际功能与交互细节。

## 运行环境与依赖
- 仅支持 Linux 终端运行。
- 依赖 `textual>=0.30`（缺失会提示安装）。
- 读取运行目录下的 `oqqwall.config`、`AcountGroupcfg.json`、`cache/OQQWall.db`、`cache/prepost`、`logs/` 等数据。

## 总体界面结构
- 左侧导航栏，右侧内容区。
- 四个页面：主页 / 全局配置 / 组配置 / Log。
- 顶部 Header 与底部 Footer，支持快捷键退出：`q` 或 `Ctrl+C`。

## 主页（运行与状态概览）
### 操作按钮
- 启动 OQQWall：调用 `./main.sh`，以独立进程组启动，输出转发到日志页面。
- 停止 OQQWall：终止进程组；清理核心子服务；若 `manage_napcat_internal=true` 则尝试停止 QQ/NapCat 进程。
- 检查 NapCat：逐实例探测 NapCat 端口与登录信息。
- 检查子服务：探测接收/审核/QZone/WebReview 子服务运行状态。

### 状态显示
- OQQWall 运行状态：本 TUI 启动的进程或外部进程均可识别。
- NapCat 状态：汇总所有实例健康度，标识在线/异常/离线，并提示异常账号列表。
- QQ 登录状态：展示各实例的 QQ 与端口，逐条标记健康状态。
- 子服务状态：
  - 接收服务 `getmsgserv/serv.py`
  - 审核服务 `Sendcontrol/sendcontrol.sh`
  - QZone 发送服务 `SendQzone/qzone-serv-UDS.py`（结合进程检测与 UDS 探测）
  - WebReview（若启用）：`web_review.py`/`web_review/web_review.py` 并显示端口

### 指标卡片
- 待审核数量：统计 `cache/prepost` 下数字目录，排除已进入暂存区的 tag。
- 当前内部编号：读取数据库 `preprocess` 表的最大 tag。

## 全局配置页（`oqqwall.config`）
### 表单特性
- 以固定顺序展示核心配置；未列出的键按字母序追加。
- 布尔型值展示为开关；其他值使用文本输入框。
- 各项带提示信息（悬浮 “?” 或字段名显示说明）。
- 支持滚动浏览。

### 操作
- 重新加载：丢弃未保存更改并重建表单。
- 保存：覆盖写回 `oqqwall.config`（key=value，统一加引号输出）。

## 组配置页（`AcountGroupcfg.json`）
### 顶栏与组管理
- 展示已有组按钮（按组名排序）。
- 新增组：输入组名（仅字母/数字/下划线），可确认/取消。
- 删除组：进入确认删除模式，可确认/取消。

### 组内字段编辑
基础字段（文本输入）：
- `mangroupid`（群号）
- `accounts`（账号列表，首项为主账号）
- `max_post_stack`
- `max_image_number_one_post`
- `watermark_text`
- `friend_add_message`

发送计划：
- `send_schedule` 以时间字符串列表维护（HH:MM），支持新增与删除。

快捷回复：
- `quick_replies` 以 “指令 -> 文本” 列表维护，支持新增与删除。

快捷指令：
- `review_shortcuts` 以 “指令 -> 步骤 DSL” 列表维护，支持新增与删除。
- `global_shortcuts` 以 “指令 -> 步骤 DSL” 列表维护，支持新增与删除。
- DSL 步骤支持 `|` 分隔，例如 `匿 | 是`。

网页审核管理员：
- `admins` 以用户名/密码列表维护，支持新增与删除。
- 允许 `sha256:` 前缀的密码格式。

布尔开关：
- `individual_image_in_posts`：发件时是否同时发送原图。

### 保存时校验
保存前会执行校验，错误会阻止写入，警告会提示但允许保存：
- 组名仅允许字母、数字、下划线。
- `mangroupid` 必须为数字。
- `accounts` 必须存在且每项为数字；账号跨组唯一。
- `max_post_stack` / `max_image_number_one_post` 若填写必须为数字。
- `friend_add_message` / `watermark_text` 若存在必须为字符串。
- `send_schedule` 必须为数组且时间格式为 HH:MM。
- `quick_replies` 必须为对象，键/值为字符串，内容不能为空，且不能与审核指令冲突（如“是/否/删/拒”等）。
- `review_shortcuts` / `global_shortcuts` 必须为对象，键不能包含空白、不能是 `原始`。
- `review_shortcuts` 的键不能与 `quick_replies` 重名。
- 快捷指令 DSL 必须能解析成至少一个对应作用域的原始内置步骤；非法占位符会阻止保存。

### 保存输出
- 写回 JSON 时按固定键顺序输出，并对 `quick_replies`、`review_shortcuts`、`global_shortcuts` 的键排序。
- 组本身按组名排序写回。

## Log 页面
- 文件列表：自动包含 `OQQWallmsgserv.log`、`NapCatlog` 与 `logs/*.log`。
- 选择日志后读取尾部约 500 行并显示。
- 支持跟随/暂停跟随（默认跟随）。
- 支持刷新日志文件列表。
- 记忆上次查看的日志文件与跟随状态（写入 `cache/tui_state.json`）。
- 当主页启动 OQQWall 时，`main.sh` 的输出会实时转发到日志面板。

## 运行状态持久化
- `cache/tui_state.json` 保存日志页的跟随状态和最后一次查看的日志文件。

## 备注与限制
- 仅适配 Linux。
- 依赖本地 `NapCat` HTTP 接口（`/get_status`、`/get_login_info`），并支持 `napcat_access_token` 或 `NAPCAT_TOKEN` 环境变量。
