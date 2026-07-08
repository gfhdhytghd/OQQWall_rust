# renderTodo

对比对象：
- 老版本（`getmsgserv/LM_work/progress-lite-json.sh` + `getmsgserv/HTMLwork/gotohtml.sh`）
- 当前仓库 PNG 渲染器（`crates/drivers/src/renderer.rs`）

说明：Rust 渲染器处理的是 `Draft`（text + attachment），不是直接渲染 OneBot 原始段；Draft 由 ingress 消息拼接，文本按空行分段，附件追加在该条消息文本之后（不保留段内顺序）。Napcat 会把 CQ face 转成图片附件（`res/face/<id>.png` 可用时），reply 仅用于审核逻辑。

## 内容类型支持

| 项目 | 老版本 | Rust PNG 渲染器 | 备注 |
| --- | --- | --- | --- |
| text | 支持 | 支持 | Rust 以气泡渲染 `DraftBlock::Paragraph` |
| face/表情 | 支持（转成 text 内嵌 img） | 部分支持 | CQ face 在 `res/face/` 可用时转成 data URI 图片附件，否则保留文本占位 |
| reply/引用 | 支持（生成预览 HTML） | 不支持 | reply 仅用于审核/快捷回复判断，不进入 Draft |
| image | 支持 | 支持 | 支持 data URI / file 路径 / blob / http(s) 拉取 |
| video | 支持 | 部分支持 | 媒体卡片 + 图标，占位无缩略图，可能显示文件名 |
| file | 支持 | 部分支持 | 文件卡片，文件名取自 URL（blob 无名），大小未知 |
| poke/戳一戳 | 支持 | 不支持 | Rust 无 poke 组件 |
| json/card | 支持 | 不支持 | Rust 无卡片渲染 |
| forward/合并转发 | 支持 | 不支持 | Rust 仅接收扁平 Draft |
| audio/语音 | 不支持 | 部分支持 | 媒体卡片 + 图标，占位无音轨 |
| 其它 OneBot 类型 | 不支持 | 不支持 | ingress 侧多被忽略（仅支持 text/face/image/video/file/record/reply） |

## 处理/渲染能力

| 项目 | 老版本 | Rust PNG 渲染器 | 备注 |
| --- | --- | --- | --- |
| 合并纯 text message | 支持 | 不支持 | Rust 仅按消息顺序追加段落，长文本按 800/1000 字符拆分 |
| 合并相邻 text 段 | 支持 | 不支持 | Rust 仅基于空行分段，不做段内合并 |
| 转发内容拉取 | 支持 | 不支持 | Rust 不拉取 forward 内容 |
| file→image 转换 | 支持 | 不支持 | Rust 不处理 file->image  |
| 图片本地化缓存 | 支持 | 部分支持 | 媒体抓取器会下载远程附件到 `data/blobs/<kind>/`，renderer 本身不做缓存 |
| 视频本地化 + H.264 | 支持 | 不支持 | Rust 不转码 |
| QR 码（卡片） | 支持 | 不支持 | 老版本生成二维码 |
| 水印 | 支持 | 不支持 | 老版本基于 `watermark_text` |
| 匿名/隐私处理 | 支持 | 不支持 | 老版本处理 `needpriv` |
| 版面/样式渲染 | 支持 | 支持 | 老版本输出 HTML/CSS，Rust 输出 PNG |
