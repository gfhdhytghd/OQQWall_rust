下面给你一套“把 `gotohtml` 那套 CSS 版式，改成 **Skia 直绘**”的落地方案：目标是**渲染出来的 PNG/图片**在视觉上尽可能贴近 `progress-lite-json + gotohtml`（4in 宽、同 padding/阴影/圆角/卡片样式/水印逻辑、cqface 内联、reply 框、forward 缩进等）。

我按你现在脚本的实际输出和 `gotohtml.sh` 里的 CSS 规则来设计（关键点：`gotohtml` 的宽度就是 4in≈384px@96DPI，CSS 变量基本都是固定像素）。

---

## 1）总体实现思路（推荐架构）

### A. 数据侧：把 JSON 转成“可渲染 IR”

你现在 `progress-lite-json` 输出里有两个“为了 HTML”而做的事：

* `face` 被塞进 `text` 里变成 `<img class="cqface" ...>`（内联图片）
* `reply` 被塞进 `text` 里变成 `<div class="reply">...</div>`

Skia 直绘不适合直接跑一套 HTML/CSS 引擎，所以建议做一层 **IR（Intermediate Representation）**：

* `Block::Bubble(RichText)`：气泡文本（支持内联 cqface、换行、以及 bubble 内的 reply 小框）
* `Block::Image` / `Block::VideoThumb`：图片/视频（50% max-width、max-height=300、圆角、阴影）
* `Block::File`：文件块（row-reverse、右侧图标、左侧文件名/大小）
* `Block::Card(Contact/MiniApp/News)`：JSON 卡片（标题、描述、预览图、tag、QR）
* `Block::Forward`：合并转发（标题 + 左侧蓝色 border + 缩进内容）
* `Watermark`：水印层（同 gotohtml 的 tile/rotate/jitter/opacity）

这样你的“渲染器”只需要支持上述有限控件集合，就能做到**几乎像素级复刻** HTML 视觉。

> 兼容做法：如果你暂时不想改 `progress-lite-json`，也可以在渲染前解析 `text` 字符串里的 `<img class="cqface">` / `<div class="reply">` / `<br>`，把它们恢复成 IR。下面代码里我会给“解析器骨架”。

---

## 2）Skia 选型：Rust + skia-safe（支持段落排版/占位符）

要尽量接近 Chrome/HTML 的文字排版效果，关键是用 Skia 的 **skparagraph**（支持换行、行高、fallback、emoji 等）。在 `skia-safe` 里对应 `skia_safe::textlayout` 模块：`ParagraphBuilder` / `Paragraph` / `PlaceholderStyle` 等。([Docs.rs][1])

并且它支持“占位符”（placeholder）——非常适合做 `cqface` 这种**内联图**：你把 cqface 插到 paragraph 里做 placeholder，排版完后再用 `paragraph.get_rects_for_placeholders()` 拿到每个占位符的坐标，把 PNG 画上去即可。([Rust Skia][2])

---

## 3）把 gotohtml 的 CSS 变量固化成渲染常量

从 `gotohtml.sh` 的 CSS（你脚本里写死的）可以直接抽成常量：

* 画布：`width = 4in = 384px`（96 DPI）
* container padding：20
* spacing：xs=4 sm=6 md=8 lg=10 xxl=20
* bubble：pad=(4,8) radius=12 bg=#fff border=#e0
* reply：bg=#fafafa border-left=3 #71a1cc radius=4 pad=6~8
* image：max-width=50%（≈192px），max-height=300，radius=12，shadow-md
* card：max-width=276，radius=12，border=#e0，shadow-sm
* header avatar：50，title 24，meta 12，阴影 lg
* 水印：opacity=0.12、rotate=-24deg、font=40、tile=480、jitter=16（脚本内 JS 写死）

---

## 4）关键实现点

### 4.1 文本/换行/行高：用 Paragraph

* 设置 font families（尽量贴近 CSS：`PingFang SC`/`Microsoft YaHei`/`Noto Sans CJK`…）
* 设置 font size（body 14，meta 12，tag 11）
* 设置 `line-height: 1.5`（SkParagraph 里用 TextStyle height）
* `word-break: break-word`：SkParagraph 本身对 CJK/emoji 换行更接近浏览器（仍建议用同样字体保证观感）

Paragraph API：`layout(width)`、`max_intrinsic_width()`、`height()`、`paint()`。([Rust Skia][2])

### 4.2 bubble 的 “fit-content” 宽度

HTML 的 `max-width: fit-content` 实际效果基本是：

* 如果不换行时文本宽度 < 可用宽度：bubble 就贴合文本宽度
* 如果文本太长：bubble 就占满可用宽度并换行

做法：

1. 先用一个最大宽度 `text_max = content_w - pad_x*2` 做 paragraph.layout(text_max)
2. 取 `intrinsic = paragraph.max_intrinsic_width()`
3. bubble 内文本宽度 `w = min(intrinsic, text_max)`
4. 重新 `paragraph.layout(w)` 得到最终高度
5. bubble 宽度 = `w + pad_x*2`

### 4.3 cqface 内联：Placeholder + 二次绘制

* `PlaceholderStyle` 可以指定 width/height/alignment/baseline/baseline_offset。([Rust Skia][3])
* `ParagraphBuilder::add_placeholder()` 插入占位符，排版后用 `Paragraph::get_rects_for_placeholders()` 取每个 placeholder 的 box。([Rust Skia][4])

### 4.4 阴影：用 ImageFilter drop shadow（更像 CSS box-shadow）

CSS `box-shadow: 0 0 5px rgba(0,0,0,0.1)` 对应 Skia 可用 `image_filters::drop_shadow_only` 或 `drop_shadow` 来做。([Rust Skia][5])

思路：先画“只带阴影”的层，再画实体（fill+stroke）。

### 4.5 图片 object-fit: cover + 圆角裁剪

gotohtml 的图片是 `object-fit: cover` + `border-radius:12`：

* 先算 src_rect（居中裁剪）让它 cover dst
* `canvas.clip_rrect()` 做圆角裁剪
* 用 `draw_image_rect_with_sampling_options()`（线性采样）绘制。([Rust Skia][6])

### 4.6 水印：复刻 JS tile/rotate/jitter

gotohtml 的水印逻辑是：按 tile 480 铺满，旋转 -24°，opacity 0.12，每个 tile 随机 jitter。你要“尽可能一致”，建议：

* 用固定 seed（比如 hash(watermarkText)）替代 `Math.random()`，保证同一输入稳定
* blend mode 用 `Multiply` 更接近 `mix-blend-mode: multiply`

---

# 5）关键代码（Rust + skia-safe）

下面只给“关键部件代码”，你把它们嵌入你现有工程即可（比如替换你现在 `SVG -> resvg -> tiny-skia` 那段）。

> 注意：示例里用 `skia-safe` 的 `textlayout`/`Paragraph` API、`Canvas::draw_image_rect_with_sampling_options` 等方法名都来自官方 rust-skia 文档。([Rust Skia][2])

---

## 5.1 Cargo.toml（核心依赖）

```toml
[dependencies]
skia-safe = { version = "0.91", features = ["textlayout"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
regex = "1"
anyhow = "1"
```

Skia 的 text shaping / paragraph 依赖 HarfBuzz + ICU（skia-safe 文档里也说明 text shaping / textlayout 属于 Skia Modules）。([Docs.rs][1])

---

## 5.2 FontCollection 初始化（系统字体 + 自带字体）

尽量复刻浏览器效果的关键是“字体一致”。你可以：

* Linux：打包 `Noto Sans CJK SC` + `Noto Color Emoji` 到 `res/fonts/`
* Windows/macOS：优先系统字体（PingFang/微软雅黑），再 fallback 到 Noto

下面是一个可用的初始化骨架（`FontCollection` 支持 asset/dynamic font manager）。([Rust Skia][7])

```rust
use skia_safe::textlayout::{FontCollection, TypefaceFontProvider};
use skia_safe::{FontMgr, Typeface};
use anyhow::Result;
use std::path::Path;

pub fn build_font_collection(font_dir: &Path) -> Result<FontCollection> {
    // 1) 你的内置字体（asset）
    let mut asset_mgr = TypefaceFontProvider::new();

    if font_dir.exists() {
        for entry in std::fs::read_dir(font_dir)? {
            let p = entry?.path();
            if p.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("ttf") || s.eq_ignore_ascii_case("otf")).unwrap_or(false) {
                if let Some(tf) = Typeface::from_file(&p, 0) {
                    // alias 传 None 则使用字体内部 family name，也可以给固定 alias
                    asset_mgr.register_typeface(tf, None);
                }
            }
        }
    }

    // 2) 系统字体（dynamic）
    let sys_mgr = FontMgr::new();

    // 3) FontCollection
    let mut fc = FontCollection::new();
    fc.set_asset_font_manager(Some(asset_mgr.into()));
    fc.set_dynamic_font_manager(Some(sys_mgr));
    Ok(fc)
}
```

---

## 5.3 RichText：把 text 解析成 token（兼容你当前 HTML 注入）

你现在 `text` 可能包含 `<img class="cqface" src="file://.../face/123.png">`、`<div class="reply">...</div>`、`<br>`。

先给一个“够用的解析器骨架”：只识别这三类，其余当纯文本（这就能覆盖你脚本生成的情况）。

```rust
#[derive(Debug, Clone)]
pub enum InlineToken {
    Text(String),
    LineBreak,
    Face { path: String }, // 本地 png 路径
}

#[derive(Debug, Clone)]
pub struct ReplyBox {
    pub meta: String, // "Alice · 2025-01-01 12:34"
    pub body: String, // 预览文本
    pub missing: bool,
}

#[derive(Debug, Clone)]
pub struct BubbleContent {
    pub reply: Option<ReplyBox>,
    pub inlines: Vec<InlineToken>,
}

// 只演示关键思路：生产可用版本建议写成状态机 + html entity decode。
pub fn parse_bubble_htmlish(input: &str) -> BubbleContent {
    // 1) 提取 reply div（如果存在）
    //    你脚本生成结构固定：
    //    <div class="reply ..."><div class="reply-meta">...</div><div class="reply-body">...</div></div>
    // 这里用非常保守的“查找片段”方式（建议你后续换成更稳健的 parser）
    let mut s = input.to_string();
    let mut reply: Option<ReplyBox> = None;

    if let Some(pos) = s.find(r#"<div class="reply"#) {
        if let Some(end) = s[pos..].find(r#"</div></div>"#) {
            let block = &s[pos..pos + end + "</div></div>".len()];
            let missing = block.contains("missing");

            let meta = extract_between(block, r#"<div class="reply-meta">"#, "</div>")
                .unwrap_or_default();
            let body = extract_between(block, r#"<div class="reply-body">"#, "</div>")
                .unwrap_or_default();

            reply = Some(ReplyBox { meta: html_unescape(&meta), body: html_unescape(&body), missing });

            // 从文本中移除 reply block，避免后面当作普通文本
            s.replace_range(pos..pos + end + "</div></div>".len(), "");
        }
    }

    // 2) 解析 cqface img & <br>
    let mut inlines = Vec::new();
    let mut rest = s.as_str();

    while let Some(i) = rest.find('<') {
        // 先塞入 '<' 前的纯文本
        if i > 0 {
            inlines.push(InlineToken::Text(html_unescape(&rest[..i])));
        }
        let tail = &rest[i..];

        if tail.starts_with("<br") {
            // <br> / <br/>
            if let Some(gt) = tail.find('>') {
                inlines.push(InlineToken::LineBreak);
                rest = &tail[gt + 1..];
                continue;
            }
        }

        if tail.starts_with("<img") && tail.contains(r#"class="cqface""#) {
            if let Some(src) = extract_attr(tail, "src") {
                // file://... -> path
                let path = src.strip_prefix("file://").unwrap_or(&src).to_string();
                inlines.push(InlineToken::Face { path });
                if let Some(gt) = tail.find('>') {
                    rest = &tail[gt + 1..];
                    continue;
                }
            }
        }

        // 其它标签：当作普通字符 '<'
        inlines.push(InlineToken::Text("<".to_string()));
        rest = &tail[1..];
    }

    if !rest.is_empty() {
        inlines.push(InlineToken::Text(html_unescape(rest)));
    }

    BubbleContent { reply, inlines }
}

// --- 工具函数（你可替换成更完善版本） ---
fn extract_between<'a>(s: &'a str, a: &str, b: &str) -> Option<String> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)? + i;
    Some(s[i..j].to_string())
}

fn extract_attr(tag: &str, key: &str) -> Option<String> {
    // key="..."
    let pat = format!(r#"{key}=""#);
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(tag[i..j].to_string())
}

fn html_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}
```

> 更推荐的长期做法：直接改 `progress-lite-json` 输出结构化 `face/reply`，渲染侧就不需要“解析 HTML”。

---

## 5.4 用 ParagraphBuilder 构建文本 + cqface 占位符

这里是最关键的“像浏览器那样排版”的部分：`ParagraphBuilder` + `PlaceholderStyle` + `get_rects_for_placeholders()`。([Rust Skia][4])

```rust
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle,
    PlaceholderStyle, PlaceholderAlignment, TextBaseline,
};
use skia_safe::{Color4f, Paint, Point};

pub struct BuiltParagraph {
    pub paragraph: Paragraph,
    pub placeholder_paths: Vec<String>, // 顺序与 placeholders rects 一致
}

pub fn build_paragraph_for_bubble(
    fc: &FontCollection,
    inlines: &[InlineToken],
    font_size: f32,
    text_color: Color4f,
    max_width: f32,
) -> BuiltParagraph {
    let mut ps = ParagraphStyle::new();

    // 默认 TextStyle（等价 CSS: font-size/line-height/font-family/color）
    let mut ts = TextStyle::new();
    ts.set_font_size(font_size);
    ts.set_height(1.5);
    ts.set_height_override(true);

    // 贴近 gotohtml 的 font-family（按需增减/按平台调整）
    ts.set_font_families(&[
        "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC",
        "Noto Color Emoji", "Arial", "sans-serif"
    ]);

    let mut fg = Paint::default();
    fg.set_color4f(text_color, None);
    ts.set_foreground_paint(&fg);

    ps.set_text_style(&ts);

    let mut builder = ParagraphBuilder::new(&ps, fc.clone());
    // 如果你需要局部不同 style，可 builder.push_style() / pop()

    let mut placeholder_paths = Vec::new();

    for it in inlines {
        match it {
            InlineToken::Text(t) => builder.add_text(t),
            InlineToken::LineBreak => builder.add_text("\n"),
            InlineToken::Face { path } => {
                // gotohtml：cqface 20x20，并有 translateY(-0.1em)
                let face_w = 20.0;
                let face_h = 20.0;
                let baseline_offset = -(0.1 * font_size);

                let mut ph = PlaceholderStyle::new();
                ph.set_width(face_w);
                ph.set_height(face_h);
                ph.set_alignment(PlaceholderAlignment::Baseline);
                ph.set_baseline(TextBaseline::Alphabetic);
                ph.set_baseline_offset(baseline_offset);

                builder.add_placeholder(&ph);
                placeholder_paths.push(path.clone());
            }
        }
    }

    let mut paragraph = builder.build();

    // 第一次 layout 用 max_width 先 wrap（后面你还会为 fit-content 重新 layout）
    paragraph.layout(max_width);

    BuiltParagraph { paragraph, placeholder_paths }
}
```

---

## 5.5 Bubble 的 layout + draw（含阴影、边框、reply box、cqface）

### a) 阴影（更接近 CSS box-shadow）

用 image filter 的 drop shadow（或 drop_shadow_only），思想：先画 shadow，再画实体。([Rust Skia][5])

### b) cqface：paragraph.paint 后再画占位符图片

`paragraph.paint()` + `get_rects_for_placeholders()`。([Rust Skia][2])

```rust
use skia_safe::{
    Canvas, Rect, RRect, Paint, Color4f, EncodedImageFormat, Data, Image,
    ClipOp, BlendMode, image_filters, CropRect,
};
use skia_safe::canvas::SrcRectConstraint;
use skia_safe::{SamplingOptions, FilterMode, MipmapMode};

pub struct BubbleStyle {
    pub pad_x: f32, pub pad_y: f32,
    pub radius: f32,
    pub bg: Color4f,
    pub border: Color4f,
    pub shadow_blur: f32,
    pub shadow_alpha: f32,
}

pub fn draw_bubble(
    canvas: &Canvas,
    fc: &FontCollection,
    x: f32, y: f32,
    content_max_w: f32,
    bubble: &BubbleContent,
) -> (f32 /*w*/, f32 /*h*/) {
    let style = BubbleStyle {
        pad_x: 8.0, pad_y: 4.0,
        radius: 12.0,
        bg: Color4f::new(1.0,1.0,1.0,1.0),
        border: Color4f::new(0xE0 as f32 /255.0, 0xE0 as f32 /255.0, 0xE0 as f32 /255.0, 1.0),
        shadow_blur: 5.0,
        shadow_alpha: 0.10,
    };

    // reply box 先布局（在 bubble 内顶部）
    let mut inner_y = y + style.pad_y;
    let mut reply_h = 0.0;

    if let Some(r) = &bubble.reply {
        // 这里简化：reply 的 meta/body 用两个 paragraph 画，外面套一个小 rrect
        let reply_pad = 6.0;
        let reply_gap = 3.0;
        let reply_max_w = content_max_w - style.pad_x * 2.0 - reply_pad * 2.0;

        let meta_para = {
            let mut bp = build_paragraph_for_bubble(fc, &[InlineToken::Text(r.meta.clone())], 12.0,
                Color4f::new(0x66 as f32/255.0, 0x66 as f32/255.0, 0x66 as f32/255.0, 1.0),
                reply_max_w
            );
            bp.paragraph.layout(reply_max_w);
            bp
        };
        let body_para = {
            let mut bp = build_paragraph_for_bubble(fc, &[InlineToken::Text(r.body.clone())], 12.0,
                Color4f::new(0x33 as f32/255.0, 0x33 as f32/255.0, 0x33 as f32/255.0, 1.0),
                reply_max_w
            );
            bp.paragraph.layout(reply_max_w);
            bp
        };

        let reply_w = reply_max_w + reply_pad * 2.0;
        let reply_h_calc = reply_pad * 2.0 + meta_para.paragraph.height() + reply_gap + body_para.paragraph.height();

        // 背景 + 左侧边框（#71a1cc）
        let rr = RRect::new_rect_xy(
            Rect::from_xywh(x + style.pad_x, inner_y, reply_w, reply_h_calc),
            4.0, 4.0
        );

        let mut bgp = Paint::default();
        bgp.set_color4f(Color4f::new(0xFA as f32/255.0, 0xFA as f32/255.0, 0xFA as f32/255.0, 1.0), None);
        canvas.draw_rrect(rr, &bgp);

        // 左侧 border
        let mut lp = Paint::default();
        lp.set_color4f(Color4f::new(0x71 as f32/255.0, 0xA1 as f32/255.0, 0xCC as f32/255.0, 1.0), None);
        canvas.draw_rect(Rect::from_xywh(x + style.pad_x, inner_y, 3.0, reply_h_calc), &lp);

        meta_para.paragraph.paint(canvas, (x + style.pad_x + reply_pad, inner_y + reply_pad));
        body_para.paragraph.paint(canvas, (x + style.pad_x + reply_pad, inner_y + reply_pad + meta_para.paragraph.height() + reply_gap));

        inner_y += reply_h_calc + 4.0; // margin-bottom: spacing-xs
        reply_h = reply_h_calc + 4.0;
    }

    // 计算 bubble 文本区域最大宽度
    let text_max_w = content_max_w - style.pad_x * 2.0;
    let mut built = build_paragraph_for_bubble(
        fc,
        &bubble.inlines,
        14.0,
        Color4f::new(0.0,0.0,0.0,1.0),
        text_max_w,
    );

    // fit-content：bubble 宽度跟随 max_intrinsic_width
    let intrinsic = built.paragraph.max_intrinsic_width();
    let text_w = intrinsic.min(text_max_w).max(1.0);
    built.paragraph.layout(text_w);

    let bubble_w = text_w + style.pad_x * 2.0;
    let bubble_h = style.pad_y * 2.0 + reply_h + built.paragraph.height();

    let rect = Rect::from_xywh(x, y, bubble_w, bubble_h);
    let rr = RRect::new_rect_xy(rect, style.radius, style.radius);

    // shadow（drop shadow only）
    if let Some(filter) = image_filters::drop_shadow_only(
        (0.0, 0.0),
        (style.shadow_blur, style.shadow_blur),
        Color4f::new(0.0, 0.0, 0.0, style.shadow_alpha),
        None,
        None,
        CropRect::from(None),
    ) {
        let mut sp = Paint::default();
        sp.set_image_filter(filter);
        canvas.draw_rrect(rr, &sp);
    }

    // bubble bg
    let mut bgp = Paint::default();
    bgp.set_color4f(style.bg, None);
    canvas.draw_rrect(rr, &bgp);

    // border
    let mut bp = Paint::default();
    bp.set_color4f(style.border, None);
    bp.set_style(skia_safe::paint::Style::Stroke);
    bp.set_stroke_width(1.0);
    canvas.draw_rrect(rr, &bp);

    // text
    let text_x = x + style.pad_x;
    let text_y = y + style.pad_y + reply_h;
    built.paragraph.paint(canvas, (text_x, text_y));

    // placeholders -> draw face png
    let rects = built.paragraph.get_rects_for_placeholders();
    for (i, tb) in rects.iter().enumerate() {
        if let Some(path) = built.placeholder_paths.get(i) {
            if let Some(img) = load_image(path) {
                let dst = Rect::from_xywh(
                    text_x + tb.rect.left,
                    text_y + tb.rect.top,
                    tb.rect.width(),
                    tb.rect.height(),
                );
                draw_image_cover_rounded(canvas, &img, dst, 3.0);
            }
        }
    }

    (bubble_w, bubble_h)
}

fn load_image(path: &str) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    let data = Data::new_copy(&bytes);
    // skia_safe::Image::from_encoded
    Image::from_encoded(data)  // decode 支持 PNG/JPEG 等 :contentReference[oaicite:13]{index=13}
}

fn draw_image_cover_rounded(canvas: &Canvas, img: &Image, dst: Rect, radius: f32) {
    // object-fit: cover
    let sw = img.width() as f32;
    let sh = img.height() as f32;

    let scale = (dst.width() / sw).max(dst.height() / sh);
    let crop_w = dst.width() / scale;
    let crop_h = dst.height() / scale;
    let crop_x = (sw - crop_w) * 0.5;
    let crop_y = (sh - crop_h) * 0.5;

    let src = Rect::from_xywh(crop_x.max(0.0), crop_y.max(0.0), crop_w.max(1.0), crop_h.max(1.0));
    let rr = RRect::new_rect_xy(dst, radius, radius);

    canvas.save();
    canvas.clip_rrect(rr, Some(ClipOp::Intersect), Some(true));

    let sampling = SamplingOptions {
        filter: FilterMode::Linear,
        mipmap: MipmapMode::None,
        ..SamplingOptions::default()
    };

    let paint = Paint::default();
    canvas.draw_image_rect_with_sampling_options(
        img,
        Some((&src, SrcRectConstraint::Fast)),
        &dst,
        sampling,
        &paint,
    );
    canvas.restore();
}
```

---

## 5.6 画图片块（max-width=50%、max-height=300、shadow-md）

复刻 gotohtml 的图片块样式：圆角 12、阴影 md、max-height 300、max-width 50%。绘制思路与上面的 `draw_image_cover_rounded` 一样，只是多一层 shadow。`Canvas::draw_image_rect_with_sampling_options` 的签名如文档所示。([Rust Skia][6])

---

## 5.7 水印层（tile/rotate/jitter/opacity/multiply）

gotohtml 里的 JS：opacity=0.12、angle=24、fontSize=40、tile=480、jitter=16。你在 Skia 里这么做：

* `paint.set_alpha_f(0.12)`（或 Color4f alpha）
* `paint.set_blend_mode(BlendMode::Multiply)`
* `canvas.save(); canvas.rotate(-24, Some(center))` / 或先 translate 再 rotate
* 计算网格点 + jitter（建议用固定 seed）

伪代码（不展开所有细节）：

```rust
fn draw_watermark(canvas: &Canvas, text: &str, w: f32, h: f32, seed: u64) {
    let opacity = 0.12;
    let angle_deg = -24.0;
    let tile = 480.0;
    let jitter = 16.0;
    let font_size = 40.0;

    let mut paint = Paint::default();
    paint.set_color4f(Color4f::new(0.0,0.0,0.0,opacity), None);
    paint.set_blend_mode(BlendMode::Multiply);

    // 这里用 Paragraph 或 draw_str 都行；想贴近 HTML 建议 Paragraph（同字体同抗锯齿）
    // 关键：用 seed 固定 jitter，而不是随机漂移导致每次渲染都不一样
}
```

---

# 6）怎么把它接到你现有链路里

你现在链路是：

1. `progress-lite-json.sh` -> 产出 JSON + 下载媒体到本地
2. `gotohtml.sh` -> 产出 HTML（再截图/展示）

替换“渲染部分”的最小改动方案：

* 保留 `progress-lite-json` 作为输入
* 新增 `skia_render`（Rust 二进制/库）：

  * 读 JSON
  * 组 IR（顺手解析 `<img class=cqface>`、`<div class=reply>`、`<br>`）
  * layout（按 gotohtml 常量）
  * draw（Skia）
  * 输出 PNG

`gotohtml` 可以保留作 debug 对照（同一份 JSON：HTML 截图 vs Skia PNG）。

---

# 7）想做到“尽可能一致”的几个决定性细节

1. **字体一致**：这是最大差异源。建议在渲染容器里固定一套字体（Noto CJK + emoji），并确保 HTML 截图也用同样字体（或对齐系统字体）。
2. **阴影参数**：CSS blur 半径与 Skia sigma 不是完全同一标尺，你需要对 `shadow-sm/md/lg` 的 blur 做一次标定。
3. **fit-content**：用 `max_intrinsic_width()` + 二次 layout 才能接近浏览器。([Rust Skia][2])
4. **cqface 基线偏移**：HTML 是 `translateY(-0.1em)`，你也要做 `baseline_offset = -0.1 * font_size`（我上面这么写了）。`PlaceholderStyle` 支持 baseline_offset。([Rust Skia][3])
5. **水印 jitter**：gotohtml 用 `Math.random()`，你如果不固定 seed，会导致“同一输入每次水印不同”，看起来就“不一致”。建议 deterministic。

---

如果你愿意把你现在“渲染入口”那段代码（你是怎么从 `gotohtml` 变成最终图片的，比如 puppeteer 截图/Chrome headless/wkhtmltoimage 之类）贴一下，我可以直接按你的工程结构给你一份**更贴近你目录/模块的替换方案**（比如：在哪个函数里替换、原有 SVG 渲染段怎么删、Skia surface 输出怎么接到现有文件命名/分段逻辑）。

[1]: https://docs.rs/crate/skia-safe/latest "skia-safe 0.91.1 - Docs.rs"
[2]: https://rust-skia.github.io/doc/skia_safe/textlayout/paragraph/type.Paragraph.html "Paragraph in skia_safe::textlayout::paragraph - Rust"
[3]: https://rust-skia.github.io/doc/skia_safe/textlayout/struct.PlaceholderStyle.html "PlaceholderStyle in skia_safe::textlayout - Rust"
[4]: https://rust-skia.github.io/doc/skia_safe/textlayout/type.ParagraphBuilder.html "ParagraphBuilder in skia_safe::textlayout - Rust"
[5]: https://rust-skia.github.io/doc/skia_safe/image_filters/index.html?utm_source=chatgpt.com "skia_safe::image_filters - Rust"
[6]: https://rust-skia.github.io/doc/skia_safe/canvas/struct.Canvas.html "Canvas in skia_safe::canvas - Rust"
[7]: https://rust-skia.github.io/doc/skia_safe/textlayout/type.FontCollection.html "FontCollection in skia_safe::textlayout - Rust"
