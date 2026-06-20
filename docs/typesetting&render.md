# typesetting&render.md — OQQWall_RUST PNG 排版/渲染规范（内部使用 SVG 作为中间表示）

> 目标：用 **Rust 直接输出 PNG**，内部使用 SVG 作为排版/渲染中间表示，在不依赖浏览器排版引擎的情况下，尽可能复刻原版 `gotohtml.sh` 的页面结构、视觉样式与信息密度。  
> 说明：原版 `gotohtml.sh` 生成 HTML + CSS + JS（含动态页高、水印、卡片、合并转发等），再交给外部渲染链路转成图片。  
> Rust 版：**直接输出 PNG**（SVG 仅作为内部中间表示），并保证：可复现、可回放、可落地为单二进制内置资源。

---

## 0. 范围与非目标

### 0.1 覆盖范围（必须实现）
对齐 `gotohtml.sh` 中出现的全部结构/类型：

- 页面结构：背景、container、header（头像+昵称+UID）、content 列表
- 消息段类型：
  - `text`：气泡（支持换行）
  - `image`：图片块（圆角、阴影、尺寸约束）
  - `video`：视频（SVG 无法播放，渲染为“视频占位卡片，抽首帧叠加一个三角播放图标”）
  - `poke`：戳一戳 icon
  - `file`：文件块（按扩展名选择 icon + 文件名 + 文件大小）
  - `json`：QQ 卡片（contact/miniapp/news/通用）
  - `forward`：合并转发（支持嵌套 forward）
- 二维码：对每个卡片的跳转 URL 生成 QR，并显示在卡片右侧
- 水印：按配置的 `watermark_text` 平铺旋转水印（覆盖全页面内容区）
- 匿名：`Header.is_anonymous==true` 时，昵称显示为“匿名”、隐藏 UID、头像替换为匿名头像（归一化阶段填充）

### 0.2 非目标（暂不做/可降级）
- 不做 AI、分段智能、（渲染只消费结构化输入），匿名需求识别只用regex
- 输出为 PNG，不依赖 QQ 客户端 SVG 预览能力
- emoji/彩色字体若缺字体可降级（见字体策略）
---

## 1. 渲染输入（RenderDoc）与节点树（RenderTree）

Rust 渲染层不直接吃 OneBot 原始 JSON；建议先归一化为一个“渲染文档”：

### 1.1 RenderDoc（建议结构）
```rust
struct RenderDoc {
  meta: DocMeta,
  header: Header,
  watermark: Option<WatermarkSpec>,
  items: Vec<MessageItem>,   // 竖向流式布局
}

struct DocMeta {
  post_id: String,          // 用于确定性 jitter/缓存 key
  group_name: String,
  theme_id: String,
}

struct Header {
  is_anonymous: bool,       // 归一化阶段填充，渲染层不做推断
  nickname: String,         // 匿名时已替换为 "匿名"
  user_id_show: Option<String>, // 匿名时为 None
  avatar: AvatarRef,        // 匿名时使用内置匿名头像
}

enum AvatarRef {
  Url(String),              // https://qlogo2.store.qq.com/qzone/{uid}/{uid}/640
  EmbeddedPng(Vec<u8>),     // 匿名头像等内置资源
}

struct MessageItem {
  message_id: String,       // 原版用 message_id 作为 QR key
  segments: Vec<SegmentNode>,
}

enum SegmentNode {
  Text { text: String },
  Image { url: String, blob: Option<BlobId>, width: Option<u32>, height: Option<u32> },
  Video { url: String },
  Poke,
  File { name: String, path_or_url: Option<String>, size_bytes: Option<u64> },
  JsonCard { raw: String, card: Option<CardMeta> }, // 渲染层只消费 card，raw 仅用于调试/重建
  Forward { items: Vec<ForwardItem> } // 合并转发
}

struct CardMeta {
  kind: String,             // contact/miniapp/news/generic
  title: Option<String>,
  desc: Option<String>,
  thumb_url: Option<String>,
  jump_url: Option<String>, // 用于 QR
}

struct ForwardItem {
  message_id: String,
  segments: Vec<SegmentNode>,
}

```

> 备注：原版 `gotohtml.sh` 对 `json.data.data` 会做 `&#44;` 与 `\\/` 处理再解析；Rust 版应在渲染层实现同样“宽容解析”。

---

## 2. 设计 Token（对齐 gotohtml.sh 的 CSS 变量）

原版 `gotohtml.sh` 中 CSS 变量可视为“主题 token”。Rust 版建议抽象为 `ThemeTokens`：

### 2.1 颜色（默认主题）

* primary: `#007aff`
* secondary: `#71a1cc`
* background: `#f2f2f2`
* card_background: `#ffffff`
* text_primary: `#000000`
* text_secondary: `#666666`
* text_muted: `#888888`
* border: `#e0e0e0`

### 2.2 间距/圆角/阴影

* spacing: xs=4, sm=6, md=8, lg=10, xl=12, xxl=20 (px)
* radius: sm=4, md=8, lg=12 (px)
* shadow:

  * sm: `0 0 5 rgba(0,0,0,0.10)`
  * md: `0 0 6 rgba(0,0,0,0.20)`
  * lg: `0 0 10 rgba(0,0,0,0.30)`

### 2.3 字体

* font_family（CSS 等价）：

  * `"PingFang SC", "Microsoft YaHei", Arial, sans-serif`
* font_size: xs=11, sm=12, md=14, lg=24 (px)

### 2.4 布局尺寸（默认与原版一致）

* canvas_width: `4in`（按 96dpi → 384px）
* container_padding: 20px
* avatar_size: 50px
* qr_size: 48px
* file_icon_size: 40px
* card_max_width: 276px
* media_max_width_ratio: 0.5（图片最大宽 = 内容宽 * 0.5）
* media_max_height: 300px

> 说明：原版 body 有 5px padding；SVG 版建议直接把这 5px 合并到 container padding 或外边距策略中，保证整洁可控。

---

## 3. SVG 画布与坐标系

### 3.1 单位与 DPI

* SVG `userUnits = px`
* 默认按 96 DPI 对齐原版 `4in = 384px` 的经验值
* `width` 固定，`height` 动态（由布局结果计算）

### 3.2 SVG 根节点建议

```xml
<svg xmlns="http://www.w3.org/2000/svg"
     width="384" height="{H}"
     viewBox="0 0 384 {H}">
  <defs>...filters/patterns/fonts...</defs>
  <rect x="0" y="0" width="384" height="{H}" fill="#f2f2f2" rx="12" />
  ...content...
</svg>
```

---

## 4. 渲染总体流程（两段式：Layout → Paint）

SVG 没有自动布局引擎；必须手写布局。

### 4.1 Layout Pass（测量与断行）

输入：RenderDoc + ThemeTokens
输出：一个 `LayoutTree`（每个节点包含 bbox：x/y/w/h）

原则：

* 所有坐标尽量用整数 px（避免浮点差异导致“同输入不同输出”）
* 行高用整数：例如 `font_size_md=14`，line-height=1.5 → 行高=21px

### 4.2 Paint Pass（生成中间 SVG）

读取 LayoutTree，把节点画成：

* `<rect>`（背景、气泡、卡片、文件块）
* `<image>`（头像、图片、icon、preview、QR）
* `<text>` + 多行 `<tspan>`（文本）
* `<path>`/`<rect>`（QR code 的模块点阵更建议画 rect，保持矢量）

---

## 5. 文档结构与布局规则（对齐原 HTML）

### 5.1 Container

* container 坐标：`x=0, y=0, w=canvas_width, h=auto`
* 内容区起点：`content_x = padding(20)`, `cursor_y = padding(20)`

### 5.2 Header（头像+昵称+UID）

布局：

* avatar 圆形：`(x=content_x, y=cursor_y, size=50)`
* header gap：10
* 昵称（h1）：font-size 24，font-weight 600
* UID（h2）：font-size 12，color #666
* header 高度：`max(avatar_size, h1+h2+小间距)`，建议固定为 `50`

cursor 更新：

* `cursor_y += header_height + spacing_xxl(20)`

匿名规则：

* header.is_anonymous=true 时：昵称=“匿名”，user_id_show=None，avatar=内置匿名头像

---

## 6. 竖向流式布局（Content 列表）

每个 `MessageItem` 的 segments 顺序渲染，节点之间使用 `margin_bottom = spacing_lg(10)`。

### 6.1 通用规则

* 所有块默认左对齐，从 `x=content_x` 开始
* 最大内容宽：`content_w = canvas_w - 2*padding`
* 文本/卡片/文件块的最大宽不超过 `content_w`
* 图片最大宽：`content_w * 0.5`（对齐原版 `max-width:50%`）
* 图片最大高：300

---

## 7. Segment 渲染规范（逐类型，全量）

### 7.1 Text（气泡 bubble）

对齐原版 `.bubble`：

* 背景白：#fff
* 圆角：12
* padding：上下 4，左右 8
* 阴影：shadow-sm
* `word-break: break-word` 语义：长词可断

#### 7.1.1 换行规则

* 输入文本按 `\n` 分成“硬换行段”
* 每段内再按宽度进行自动换行
* 最终输出多行 `<text>` + `<tspan x=... dy=...>`

#### 7.1.2 自动换行（建议算法）

目标：在不做复杂排版的前提下实现稳定好看的断行。

* 最大文本宽：`max_text_w = bubble_max_w - padding_lr*2`
* bubble_max_w：建议 = `content_w`（可加 “fit-content” 模式，见下）

断行策略（推荐实现顺序）：

1. 将字符串按 **grapheme cluster** 拆分（unicode-segmentation）
2. 维护一个当前行宽 `w`
3. 依次累计宽度（用字体度量，见 §10）
4. 超过 `max_text_w`：

   * 如果本行已有可断点（空格/标点）则回退到断点
   * 否则硬断
5. 支持 `word-break: break-word`：对超长英文单词也允许硬断

#### 7.1.3 “fit-content” 宽度（对齐原版）

原版 bubble `max-width: fit-content`；在 SVG 中可复刻为：

* `bubble_w = min(max_line_w + padding_lr*2, content_w)`
* `bubble_h = lines*line_h + padding_tb*2`

> 推荐：先做 “固定最大宽 content_w + 自动换行”，MVP 跑通后再加入 fit-content。

---

### 7.2 Image（图片块）

对齐原版媒体样式：

* 圆角：12
* 阴影：shadow-md
* margin-bottom：10
* 尺寸约束：

  * `w <= content_w * 0.5`
  * `h <= 300`
  * 保持原始宽高比

#### 7.2.1 宽高来源优先级

1. 若已有 blob 元信息（width/height） → 用真实比例
2. 若 OneBot 段携带宽高 → 用段携带数据
3. 否则使用占位比例（建议 4:3），例如 `w = content_w*0.5`, `h = min(300, w*0.75)`

#### 7.2.2 图片引用策略（推荐支持三档）

* **link 模式（默认）**：SVG `<image href="原始url">`，不下载，最快
* **blob 模式**：href 指向本地 admin web `/blob/{id}`，由浏览器取
* **embed 模式**：把图片 base64 内嵌到 SVG（最可移植，但 SVG 会很大）

---

### 7.3 Video（视频块）

原版 HTML 用 `<video controls autoplay muted>...`。SVG 无法直接播放，建议渲染成“占位卡片”：

* 白底卡片（同 card）
* 左侧一个播放 icon（内置 SVG path 或 png）
* 文本：`视频` + 简短 URL（或文件名）
* 若能拿到 poster（可选）可显示缩略图

---

### 7.4 Poke（戳一戳）

原版用 `poke.png`，SVG 版：

* 固定显示一个 24~32px icon（建议 24）
* 放在一个小白底 bubble 或直接当图片块
* 文字可选：`戳一戳`

---

### 7.5 File（文件块）

对齐原版 `.file-block`（row-reverse：icon 在右侧）：

布局：

* 外框：白底 rect，radius 12，padding 7，shadow-sm
* 右侧 icon：40x40（file-icon-size）
* 左侧信息：

  * 文件名：font 14，黑色，可换行，`word-break: break-word`
  * 文件大小：font 11，#888，显示 `MB/KB/B`（与原版一致）

宽度：

* `file_block_w = min(content_w, max(card_max_width, ...))`
  建议：MVP 使用 `w = min(content_w, 320)`，保持观感。

#### 7.5.1 icon 选择（按扩展名映射）

原版映射（等价实现）：

* doc/docx/odt → `doc`
* apk/ipa → `apk`
* dmg/iso → `dmg`
* ppt/pptx/key → `ppt`
* xls/xlsx/numbers → `xls`
* pages → `pages`
* ai/ps/sketch → `ps`
* ttf/otf/woff/woff2/font → `font`
* png/jpg/jpeg/gif/bmp/webp → `image`
* mp3/wav/flac/aac/ogg → `audio`
* mp4/mkv/mov/avi/webm → `video`
* zip/7z → `zip`
* rar → `rar`
* pkg → `pkg`
* pdf → `pdf`
* exe/msi → `exe`
* sh/py/c/cpp/js/ts/go/rs/java/rb/php/lua/code → `code`
* txt/md/note → `txt`
* 其它 → `unknown`

资源打包建议：

* 将 `doc.png/apk.png/...` 等图标用 `include_bytes!` 内置到二进制
* SVG 内以 `<image href="data:image/png;base64,...">` 引用（一次性缓存 base64）

---

### 7.6 JsonCard（QQ 卡片：contact / miniapp / news / 通用）

原版 `gotohtml.sh` 对 `json` 类型做：

* 先将 `.data.data` 做 `&#44; -> ,`、`\\\/ -> /`，再 `fromjson`
* 根据 `view` 与 `meta` 分支渲染不同样式
* 如果能抽取到 `jumpUrl` 则显示 QR（48x48）

Rust 版约定：OneBot json 段在归一化阶段解析为 `CardMeta` 并写入 `JsonCard.card`；渲染层不再解析 `raw`。

#### 7.6.1 卡片 URL 抽取（归一化阶段，必须对齐原版语义）

抽取函数（逻辑等价）：

* view == `contact` 且 `meta.contact`：

  * 优先从 `jumpUrl` 中提取 `uin=数字`
  * 否则从 `contact` 字段里抓取 `>=5位数字`
  * 若有 uin：生成 `https://mp.qzone.qq.com/u/{uin}` 作为 QR 指向
* view == `miniapp` 且 `meta.miniapp`：

  * `jumpUrl` 优先，否则 `doc_url`
* view == `news` 且 `meta.news`：

  * `jumpUrl`
* 其它：

  * 从 `meta` 的第一项里找 `.value.jumpUrl`（如果存在）

> Rust 版：渲染前扫描 RenderDoc（含 forward 内嵌）收集 `card.jump_url`，生成 QR cache（key=message_id）。

#### 7.6.2 contact 卡片布局（横向）

元素：

* 左：联系人头像（48x48，圆角 4）
* 中：nickname（14 bold）、contact 文本（12 灰）、tag（11 灰）可选
* 右：QR（48x48）如果有 url

卡片外框：

* 白底 rect，radius 12，padding 8，shadow-sm
* 最大宽 276（card_max_width）

#### 7.6.3 miniapp 卡片布局（纵向）

元素：

* header：

  * 可选品牌行：brand icon（12）+ source（12 灰）
  * title（14 bold）
  * 右侧 QR（48）可选
* preview：图片宽满卡片（如存在）
* tag row：tag icon（14）+ tag 文本（11 灰）

#### 7.6.4 news 卡片布局

元素：

* header：thumb（48）+ title（14 bold）+ 右侧 QR
* bottom：desc（12 灰）+ tag row（可选）

#### 7.6.5 通用卡片布局（generic）

元素：

* preview（若有）
* title：`value.title` 优先，否则 `prompt`，否则 `view`
* desc：若有
* QR：若有

---

### 7.7 Forward（合并转发聊天记录，支持嵌套）

原版结构：

* 标题：`合并转发聊天记录`
* 下面一个 `forward` 容器：左边框 3px secondary，padding-left 10
* 每个 forward-item 里再渲染其 message 段
* 支持 forward 嵌套：内部 forward 再缩进

SVG 复刻规则：

* 标题文本（12 灰）占一行
* forward 容器：

  * 在左侧画一条竖线（3px，#71a1cc）
  * 内容整体 `indent = 10`（或 12）
  * 每个 item 在内容区继续做竖向流式布局
  * 嵌套 forward：再次加 indent，并重复竖线

---

## 8. 水印（Watermark）实现（对齐原版效果，但需确定性）

原版 HTML 里水印用 JS 生成并包含随机 jitter；Rust 版必须**确定性**，否则同稿件重渲染会变化。

### 8.1 水印参数（默认建议与原版一致）

* text：来自 group config 的 `watermark_text`（为空则不显示）
* opacity：0.12
* angle：24（文本旋转 -24deg）
* font_size：40
* tile：480（平铺间距）
* jitter：10（抖动幅度）

### 8.2 生成算法（确定性版本）

* seed：`seed = blake3(post_id + watermark_text)`
* 用一个简单 PRNG（xorshift/pcg）生成抖动值（[-jitter, +jitter]）
* cols/rows：

  * cols = floor((W - 2*padX)/tile) + 1
  * rows = floor((H - 2*padY)/tile) + 1
* 水平居中：

  * `firstCX = W/2 - ((cols-1)*tile)/2`
* 每个格点输出一个 `<text>`：

  * `x = clamp(0, W-stampW, round(cx + jx - stampW/2))`
  * `y = clamp(0, H-stampH, round(cy + jy - stampH/2))`
  * `transform="rotate(-24 x y)"`（围绕自身中心旋转更美观）

> stampW/stampH 的测量：可用“近似估计”或用字体度量（建议用同一套 font metrics）。

### 8.3 内部 SVG 实现方式

两种方式：

1. **显式生成 text 列表（推荐）**：最接近原版 JS 布点
2. `<pattern>` 平铺（更简洁但不易做 jitter 与边界 clamp）

---

## 9. QR Code 生成与缓存策略（渲染器优先）

### 9.1 生成方式

推荐使用纯 Rust QR 库输出矢量矩形：

* `qrcode` / `qrcodegen`：得到 module matrix
* 在 SVG 里画：

  * 背景白 rect
  * 黑模块：一堆 `<rect x y w h fill="#000"/>`
* 模块尺寸：`qr_size / modules` 向下取整，留出 margin（quiet zone）

### 9.2 缓存

* 渲染一次过程中：`HashMap<(message_id, url), QrSvgGroup>`
* 同一 doc 内同 url 可复用（可选），但原版用 key=message_id；Rust 版保持 key=message_id 更直观。

---

## 10. 字体与文本度量（断行关键）

SVG 不会自动换行；必须断行 + 度量。

### 10.1 MVP 策略（尽快落地）

* 先用“近似等宽”估算宽度（CJK 每字≈font_size，ASCII≈0.6*font_size），快速实现
* 视觉会略有偏差，但能跑通

### 10.2 推荐策略（质量更高）

使用真实字体度量：

* 解析 TTF/OTF（内置或系统）获取 glyph advance
* 建议库组合（择一）：

  * `fontdue`（快，度量方便）
  * `ttf-parser` + 自己计算 advance（更底层）
  * 需要复杂 shaping（阿拉伯/连字）时加 `rustybuzz`（可后置）

### 10.3 字体资源策略（单二进制友好）

* MVP：依赖系统字体（PingFang/微软雅黑），SVG 里写 font-family 即可
* 增强：内置一个apple-pingfang和apple-color-emoji，并通过：

  * A) admin web 静态资源 `/assets/font.ttf`，SVG 用 `@font-face src:url(...)`
  * B) 直接 base64 嵌入 SVG（如果选择这条，要做子集化）
  * C)文字路径化(推荐)

---

## 11. 高度控制与分页（避免极端长稿件）

原版会把页面高度限制在 `4in~24in`（高度超过 2304px 时固定 24in）。

Rust SVG 建议：

* 默认允许动态高度
* 可配置 `max_height_px`（默认 2304）
* 超过则分页输出：

  * `post_id_p1.png`, `post_id_p2.png`…

---

## 12. 资源打包清单（建议与原版对齐）

必须内置：

* 匿名头像：`Anonymous_avatar.png`
* poke icon：`poke.png`
* 文件 icon 集合：`doc/apk/dmg/ppt/xls/pages/ps/font/image/audio/video/zip/rar/pkg/pdf/exe/code/txt/unknown.png`

打包方式：

* `include_bytes!()` 进二进制
* 渲染时 base64 编码并缓存（避免重复编码）

---

## 13. 安全与文本转义

SVG 是 XML，所有用户文本必须：

* XML escape：`& < > " '`
* 过滤控制字符（如 `\u0000`）
* URL：只允许 `http/https/file`（或按需白名单），避免 `javascript:` 等

---

## 14. Rust 实现建议（工程落地）

### 14.1 推荐 crate

* 内部 SVG 构建：`svg`（DOM 风格）或手写字符串 builder（更快）
* QR：`qrcode` / `qrcodegen`
* base64：`base64`
* unicode 分词/换行：`unicode-segmentation`
* 字体度量：`fontdue`（建议）
* 图片读取（可选）：`image`（仅当你要从 blob 解码得到宽高）

### 14.2 Renderer 模块接口（建议）

```rust
struct RenderOptions {
  canvas_width_px: u32,     // default 384
  max_height_px: u32,       // default 2304
  embed_icons: bool,        // default true
  image_href_mode: ImageHrefMode, // Link|Blob|Embed
}

fn render_svg(doc: &RenderDoc, theme: &ThemeTokens, opt: &RenderOptions) -> Vec<u8>;
```

### 14.3 两段式实现骨架

* `layout::layout_doc(doc, theme, opt) -> LayoutDoc`
* `paint::paint_svg(layout_doc, theme, opt) -> String/Bytes`

---

## 15. 渲染一致性（必须满足事件溯源回放）

同一 `RenderDoc` 输入应输出**字节级稳定**（至少结构稳定）：

* 坐标用整数
* 水印 jitter 使用 seed PRNG（禁止随机）
* 节点遍历顺序稳定（按输入顺序）
* base64 编码固定（标准 base64，无换行）

---

## 16. 验收清单（渲染模块）

* [ ] header：头像圆形+阴影、昵称、UID（匿名隐藏 UID）
* [ ] bubble：换行、自动断行、fit-content（可后置）
* [ ] image：圆角阴影，max-width 50%，max-height 300
* [ ] file：扩展名 icon + 文件名换行 + 大小 MB/KB/B
* [ ] json card：contact/miniapp/news/通用 四类布局正确
* [ ] QR：卡片有 url 则显示 QR，无则不显示
* [ ] forward：标题+左竖线+缩进，支持嵌套
* [x] watermark：按配置平铺旋转，且确定性
* [ ] 超长稿件分页（可配置）

---

## 17. 兼容说明（与原 gotohtml 的差异）

* 原版 HTML 使用浏览器完成布局、图片自然尺寸、video 播放；SVG 版需要显式给尺寸与占位策略
* 原版水印 jitter 随机；SVG 版必须确定性
* 原版 CSS 的 hover 效果（card:hover）在 SVG 中可忽略或用简单 filter 切换（非必须）

---
