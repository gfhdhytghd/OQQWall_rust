# OQQWall_RUST 开放 QQ 校园墙自动运营系统（生锈版）
# 👍 稳定运行数十万秒 👍

## 简介
Rust 版的目标是把原版脚本链路重构成可测试、可回放、可演进的事件驱动系统（并尽量保持原版的使用体验与指令语义）。

本系统用于“校园墙”日常运营：收稿 → 聚合成稿 → 渲染预览 → 审核群指令 → 排程发送 QQ 空间。  

本系统专注于“墙”本身，适用于用户量五十万以下的情况，致力于给用户提供QQ校园墙的无感的交互

本系统的技术实现由GPT进行，核心为纯函数式，总体上速度极快
<br/>RUST版本的目标是尽可能优化性能，相比原版OQQWall,他快了至少一百倍
<br/>编写和测试平台是主线ArchLinux，作者目前使用的生产环境是阿里云的ubuntu 22.04 x64 UEFI版本，最低兼容glibc版本为glibc v2.31。

本系统拥有处理并发的能力，允许的最小投稿时间间隔是无限小，最大并行处理能力取决于你的电脑内存大小和管理员响应速度。

本系统支持如下类型的消息：文本，表情，表情包，图片，视频，文件，暂不支持卡片和聊天记录
技术交流请加群1056259167

# <div align=center>文档</div>
## <div align=center > [快速开始](https://github.com/gfhdhytghd/OQQWall_rust/wiki/%E5%BF%AB%E9%80%9F%E5%BC%80%E5%A7%8B) | [全部文档](https://github.com/gfhdhytghd/OQQWall_rust/wiki)</div>
<div align="center">

遥测相关文档：
- 投稿端遥测：`docs/telemetry.md`
- 遥测服务端（collector）：`docs/telemetry_collector.md`

## 开源项目列表

本项目使用或参考了以下开源项目：

- [Campux](https://github.com/idoknow/Campux)
- [NapCat（napneko）](https://napneko.github.io/zh-CN/)
- [LiteLoaderQQNT](https://liteloaderqqnt.github.io/)
- [OneBot](https://github.com/botuniverse/onebot)
- [go-cqhttp](https://github.com/Mrs4s/go-cqhttp)
- [LLOneBot](https://github.com/LLOneBot/LLOneBot/)
- [Lagrange.OneBot](https://github.com/LSTM-Kirigaya/Lagrange.OneBot)
- [Stapxs QQ Lite](https://github.com/Stapxs/Stapxs-QQ-Lite-2.0)
- [Axum](https://github.com/tokio-rs/axum)
- [Tokio](https://github.com/tokio-rs/tokio)
- [Reqwest](https://github.com/seanmonstar/reqwest)
- [Serde](https://github.com/serde-rs/serde)
- [Ratatui](https://github.com/ratatui-org/ratatui)
- [Crossterm](https://github.com/crossterm-rs/crossterm)
- [Tokio Tungstenite](https://github.com/snapview/tokio-tungstenite)
- [Rust](https://github.com/rust-lang/rust)
- [Skia](https://skia.org/)
- [skia-safe（rust-skia）](https://github.com/rust-skia/rust-skia)

感谢各位对自由软件与本项目作出的贡献！

</div>

以下用来骗搜索引擎：

QQ校园墙 | 校园墙自动运营 | 自动校园墙 | 自动化校园墙 | QQ群机器人 | 大语言模型 | AI自动审核 | 匿名投稿系统 | 校园社交平台 | QQ空间自动发布 | 群消息管理 | 智能助手 | 多账号运营 | 消息渲染 | 并发处理 | 投稿审核系统 | 校园墙管理系统 | 表情包处理 | 图片渲染 | 视频处理 | 文件管理 | 聊天记录转发 | 校园墙运营工具 | 自动化运营 | 校园社交 | 智能审核 | 校园墙API | automatic qq wall |
