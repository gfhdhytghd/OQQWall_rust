use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command as ProcessCommand;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::avatar_cache;
use crate::blob_cache::{self, CacheKind, CacheRetention};
use crate::napcat::{
    NapCatConfig, extract_message_lite, napcat_account_for_group, napcat_ws_request,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use oqqwall_rust_core::decide::builder::build_draft_from_messages;
use oqqwall_rust_core::event::{
    BlobEvent, Event, IngressEvent, MediaEvent, RenderEvent, SendEvent,
};
use oqqwall_rust_core::{
    BlobId, Command, Draft, DraftBlock, ForwardItem, IngressId, IngressMessage, MediaKind,
    MediaReference, PostId, ReplyPreview, StateView, derive_blob_id,
};
use oqqwall_rust_infra::{LocalJournal, SnapshotStore};
use qrcode::{
    Color as QrColor, EcLevel, QrCode, bits,
    canvas::{Canvas as QrCanvas, MaskPattern},
    ec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use skia_safe::canvas::SrcRectConstraint;
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle, TypefaceFontProvider,
};
use skia_safe::utils::OrderedFontMgr;
use skia_safe::{
    Canvas, ClipOp, Color4f, Data, EncodedImageFormat, FontStyle, Image, Paint, PathBuilder, RRect,
    Rect, SamplingOptions, Typeface, image_filters,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

mod embedded_resources {
    include!(concat!(env!("OUT_DIR"), "/embedded_resources.rs"));
}

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

const FORWARD_PREFIX: &str = "[合并转发:";
const MAX_FORWARD_DEPTH: u32 = 4;
const MEASURE_MAX_WIDTH: f32 = 10_000.0;
const FONT_FAMILIES: [&str; 1] = ["PingFang SC"];
const DEFAULT_AVATAR_PATH: &str = "res/Anonymous_avatar.png";
const AVATAR_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const CARD_REMOTE_IMAGE_TIMEOUT: Duration = Duration::from_secs(8);
const CARD_REMOTE_IMAGE_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_CANVAS_WIDTH_PX: u32 = 1152;
pub const DEFAULT_MAX_HEIGHT_PX: u32 = 6912;
const JSON_CARD_MAX_WIDTH: u32 = 276;
const JSON_CARD_QR_SIZE: u32 = 48;
const JSON_CARD_SIDE_GAP: u32 = 8;
const JSON_CARD_SOURCE_ICON_SIZE: u32 = 12;
const JSON_CARD_TAG_ICON_SIZE: u32 = 14;
const JSON_CARD_BOTTOM_QR_RAISE_PX: u32 = 8;
const JSON_CARD_BOTTOM_QR_DRAW_HEIGHT_SHRINK_PX: u32 = 2;
const JSON_CARD_CONTACT_WIDTH_EXTRA_PX: u32 = 1;
const JSON_CARD_MINIAPP_SOURCE_BASELINE_DOWN_PX: u32 = 4;
const JSON_CARD_MINIAPP_TITLE_BASELINE_DOWN_PX: u32 = 6;
const JSON_CARD_NEWS_WITH_FOOTER_WIDTH_SHRINK_PX: u32 = 1;
const JSON_CARD_VERTICAL_TEXT_BASELINE_RAISE_PX: u32 = 5;
const JSON_CARD_PREVIEW_MIN_HEIGHT: u32 = 72;
const REPLY_BODY_BASELINE_OFFSET_PX: u32 = 4;
const REPLY_INNER_Y_RAISE_PX: u32 = 2;
const REPLY_WRAP_WIDTH_COMPENSATION_PX: u32 = 8;
const SHADOW_SM_ALPHA: f32 = 0.10;
const SHADOW_SM_BLUR_PX: f32 = 2.5;
const TEXT_RASTER_STROKE_PX: f32 = 0.01;
const TOP_LEVEL_TEXT_BUBBLE_WIDTH_SHRINK_PX: u32 = 6;
const FORWARD_TEXT_BUBBLE_WIDTH_SHRINK_PX: u32 = 1;
const FORWARD_FILE_CARD_WIDTH_SHRINK_PX: u32 = 1;
const FORWARD_SINGLE_LINE_TEXT_HEIGHT_SHRINK_PX: u32 = 2;
const REQUIRED_RES_FILES: &[&str] = &[
    "Anonymous_avatar.png",
    "fonts/PingFangSC-Regular.otf",
    "face/default_config.json",
    "emoji_png/apple_color_emoji/metadata.json",
];
static FONT_BYTES_CACHE: OnceLock<Vec<FontBytes>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct RendererRuntimeConfig {
    pub blob_root: PathBuf,
    pub canvas_width_px: u32,
    pub max_height_px: u32,
    pub napcat_by_group: HashMap<String, NapCatConfig>,
    pub default_napcat: Option<NapCatConfig>,
    pub watermark_text_by_group: HashMap<String, String>,
}

impl Default for RendererRuntimeConfig {
    fn default() -> Self {
        let blob_root = std::env::var("OQQWALL_BLOB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/blobs"));
        Self {
            blob_root,
            canvas_width_px: DEFAULT_CANVAS_WIDTH_PX,
            max_height_px: DEFAULT_MAX_HEIGHT_PX,
            napcat_by_group: HashMap::new(),
            default_napcat: None,
            watermark_text_by_group: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPreviewHeader {
    pub group_id: String,
    pub user_id: String,
    pub post_id_hex: String,
    pub sender_name: Option<String>,
    pub is_anonymous: bool,
}

impl Default for RenderPreviewHeader {
    fn default() -> Self {
        Self {
            group_id: "fixture-group".to_string(),
            user_id: "fixture-user".to_string(),
            post_id_hex: "00000000000000000000000000000000".to_string(),
            sender_name: Some("fixture sender".to_string()),
            is_anonymous: false,
        }
    }
}

#[derive(Debug, Clone)]
struct HeaderInfo {
    group_id: String,
    user_id: String,
    post_id_hex: String,
    sender_name: Option<String>,
    is_anonymous: bool,
}

impl From<RenderPreviewHeader> for HeaderInfo {
    fn from(value: RenderPreviewHeader) -> Self {
        Self {
            group_id: value.group_id,
            user_id: value.user_id,
            post_id_hex: value.post_id_hex,
            sender_name: value.sender_name,
            is_anonymous: value.is_anonymous,
        }
    }
}

#[derive(Debug, Clone)]
enum BlockKind {
    Text {
        lines: Vec<InlineLine>,
    },
    Image {
        image: Option<ResolvedImage>,
    },
    VideoPreview {
        image: Option<ResolvedImage>,
    },
    MediaCard {
        lines: Vec<String>,
        icon_text: String,
        media_kind: oqqwall_rust_core::MediaKind,
    },
    FileCard {
        name_lines: Vec<String>,
        meta_line: Option<String>,
        icon_path: Option<&'static str>,
        icon_text: String,
    },
    Reply {
        meta_lines: Vec<String>,
        body_lines: Vec<String>,
    },
    Poke {
        image: Option<ResolvedImage>,
    },
    JsonCard {
        view: JsonCardView,
        title_lines: Vec<String>,
        desc_lines: Vec<String>,
        footer_line: Option<String>,
        media: Option<ResolvedImage>,
        tag_icon: Option<ResolvedImage>,
        brand_icon: Option<ResolvedImage>,
        qr_url: Option<String>,
    },
    Forward {
        items: Vec<ForwardLayoutItem>,
    },
}

#[derive(Debug, Clone)]
struct InlineLine {
    runs: Vec<InlineRun>,
    width: u32,
}

#[derive(Debug, Clone)]
enum InlineRun {
    Text(String),
    Face { id: String },
    Emoji { glyph_id: u16 },
}

#[derive(Debug, Clone)]
enum InlineAtom {
    Char(char),
    Face(String),
    Emoji(u16),
}

#[derive(Debug, Clone)]
struct BlockLayout {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    kind: BlockKind,
}

#[derive(Debug, Clone)]
struct ResolvedImage {
    bytes: Option<Arc<[u8]>>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Clone, Default)]
struct JsonCardView {
    view_kind: String,
    title: String,
    desc: Option<String>,
    footer: Option<String>,
    jump_url: Option<String>,
    media_source: Option<String>,
    tag_icon_source: Option<String>,
    brand_icon_source: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct JsonCardResolvedImages {
    media: Option<ResolvedImage>,
    tag_icon: Option<ResolvedImage>,
    brand_icon: Option<ResolvedImage>,
}

#[derive(Debug, Clone)]
struct ForwardLayoutItem {
    blocks: Vec<BlockLayout>,
}

fn assign_legacy_json_card_qr_urls(blocks: &mut [BlockLayout]) {
    let mut run_indices: Vec<usize> = Vec::new();
    let mut last_url: Option<String> = None;

    fn flush(
        blocks: &mut [BlockLayout],
        run_indices: &mut Vec<usize>,
        last_url: &mut Option<String>,
    ) {
        if let Some(url) = last_url.clone() {
            for idx in run_indices.drain(..) {
                if let BlockKind::JsonCard { qr_url, .. } = &mut blocks[idx].kind {
                    *qr_url = Some(url.clone());
                }
            }
        } else {
            run_indices.clear();
        }
        *last_url = None;
    }

    for idx in 0..blocks.len() {
        let jump_url = match &blocks[idx].kind {
            BlockKind::JsonCard { view, .. } => view.jump_url.clone(),
            _ => {
                flush(blocks, &mut run_indices, &mut last_url);
                continue;
            }
        };

        if let Some(url) = jump_url {
            run_indices.push(idx);
            last_url = Some(url);
        } else {
            flush(blocks, &mut run_indices, &mut last_url);
        }
    }
    flush(blocks, &mut run_indices, &mut last_url);
}

#[derive(Debug, Clone)]
struct RenderImageSources {
    avatar: Option<ResolvedImage>,
    block_images: Vec<Option<ResolvedImage>>,
    block_tag_icons: Vec<Option<ResolvedImage>>,
    block_brand_icons: Vec<Option<ResolvedImage>>,
    block_labels: Vec<Option<String>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum ImageCacheKey {
    Blob(BlobId),
    Source(String),
}

#[derive(Default)]
struct ImageMemoryCache {
    entries: HashMap<ImageCacheKey, ResolvedImage>,
    attachment_sources: HashMap<(IngressId, usize), ImageCacheKey>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct TextMeasureKey {
    font_size: u32,
    font_weight: u32,
    text: String,
}

#[derive(Debug)]
struct TextMeasurer {
    font_collection: FontCollection,
    cache: HashMap<TextMeasureKey, u32>,
}

impl ResolvedImage {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let size = image_size_from_bytes(&bytes);
        let (width, height) = size.map_or((None, None), |(w, h)| (Some(w), Some(h)));
        Self {
            bytes: Some(Arc::from(bytes)),
            width,
            height,
        }
    }

    fn from_arc(bytes: Arc<[u8]>) -> Self {
        let size = image_size_from_bytes(&bytes);
        let (width, height) = size.map_or((None, None), |(w, h)| (Some(w), Some(h)));
        Self {
            bytes: Some(bytes),
            width,
            height,
        }
    }

    fn has_bytes(&self) -> bool {
        self.bytes.as_ref().map(|b| !b.is_empty()).unwrap_or(false)
    }
}

impl TextMeasurer {
    fn new(font_collection: FontCollection) -> Self {
        Self {
            font_collection,
            cache: HashMap::new(),
        }
    }

    fn measure_text_width(&mut self, text: &str, font_size: u32, font_weight: u32) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let key = TextMeasureKey {
            font_size,
            font_weight,
            text: text.to_string(),
        };
        if let Some(width) = self.cache.get(&key) {
            return *width;
        }
        let width_px = if let Some((paragraph, _)) = build_line_paragraph(
            &self.font_collection,
            text,
            font_size,
            font_weight,
            Color4f::new(0.0, 0.0, 0.0, 1.0),
        ) {
            paragraph.max_intrinsic_width().ceil().max(0.0) as u32
        } else {
            0
        };
        self.cache.insert(key, width_px);
        width_px
    }
}

fn reply_meta_text(preview: &ReplyPreview) -> String {
    if let Some(meta) = preview
        .meta
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return meta.to_string();
    }
    if let Some(id) = preview
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("id:{}", id);
    }
    "回复".to_string()
}

fn parse_json_card_view(raw: &str) -> JsonCardView {
    let normalized = raw.replace("&#44;", ",").replace("\\/", "/");
    let value = serde_json::from_str::<Value>(&normalized).ok();
    if let Some(value) = value.as_ref() {
        let view_kind = json_field_string(value, &["view", "type", "template"])
            .unwrap_or_else(|| "json".to_string());
        if view_kind == "contact" {
            if let Some(contact) = value.pointer("/meta/contact") {
                return JsonCardView {
                    view_kind,
                    title: json_field_string(contact, &["nickname", "title", "name"])
                        .unwrap_or_else(|| "联系人".to_string()),
                    desc: json_field_string(contact, &["contact", "desc", "description"]),
                    footer: json_field_string(contact, &["tag", "source", "footer"]),
                    jump_url: json_field_string(contact, &["jumpUrl", "jump_url", "url", "link"])
                        .or_else(|| contact_uin_url(contact)),
                    media_source: json_field_string(contact, &["avatar", "icon", "preview"]),
                    tag_icon_source: json_field_string(contact, &["tagIcon", "tag_icon"]),
                    brand_icon_source: None,
                };
            }
        }
        if view_kind == "miniapp" {
            if let Some(miniapp) = value.pointer("/meta/miniapp") {
                return JsonCardView {
                    view_kind,
                    title: json_field_string(miniapp, &["title", "name"])
                        .unwrap_or_else(|| "小程序卡片".to_string()),
                    desc: json_field_string(miniapp, &["source"]),
                    footer: json_field_string(miniapp, &["tag", "footer"]),
                    jump_url: json_field_string(
                        miniapp,
                        &["jumpUrl", "jump_url", "doc_url", "url", "link"],
                    ),
                    media_source: json_field_string(miniapp, &["preview", "cover", "image"]),
                    tag_icon_source: json_field_string(miniapp, &["tagIcon", "tag_icon"]),
                    brand_icon_source: json_field_string(miniapp, &["sourcelogo", "sourceLogo"]),
                };
            }
        }
        if view_kind == "news" {
            if let Some(news) = value.pointer("/meta/news") {
                return JsonCardView {
                    view_kind,
                    title: json_field_string(news, &["title", "name"])
                        .unwrap_or_else(|| "分享".to_string()),
                    desc: json_field_string(news, &["desc", "description"]),
                    footer: json_field_string(news, &["tag", "source", "footer"]),
                    jump_url: json_field_string(news, &["jumpUrl", "jump_url", "url", "link"]),
                    media_source: json_field_string(news, &["preview", "cover", "image", "thumb"]),
                    tag_icon_source: json_field_string(news, &["tagIcon", "tag_icon"]),
                    brand_icon_source: None,
                };
            }
        }
        if let Some(generic) = first_meta_value(value) {
            return JsonCardView {
                view_kind,
                title: json_field_string(generic, &["title", "name"])
                    .or_else(|| json_field_string(value, &["prompt", "summary"]))
                    .unwrap_or_else(|| "[卡片]".to_string()),
                desc: json_field_string(generic, &["desc", "description", "text", "content"]),
                footer: json_field_string(generic, &["source", "tag", "app", "footer"]),
                jump_url: json_field_string(
                    generic,
                    &["jumpUrl", "jump_url", "url", "targetUrl", "link"],
                ),
                media_source: json_field_string(
                    generic,
                    &[
                        "preview",
                        "image",
                        "icon",
                        "cover",
                        "picture",
                        "thumbnail",
                        "img_url",
                    ],
                ),
                tag_icon_source: json_field_string(generic, &["tagIcon", "tag_icon"]),
                brand_icon_source: json_field_string(generic, &["sourcelogo", "sourceLogo"]),
            };
        }
    }

    let title = value
        .as_ref()
        .and_then(|value| find_json_string(value, &["title", "nickname", "prompt", "name"]))
        .unwrap_or_else(|| "[卡片]".to_string());
    let desc = value
        .as_ref()
        .and_then(|value| find_json_string(value, &["desc", "description", "text", "content"]));
    let footer = value
        .as_ref()
        .and_then(|value| find_json_string(value, &["source", "app", "tag", "footer"]));
    let jump_url = value.as_ref().and_then(|value| {
        find_json_string(
            value,
            &[
                "url",
                "jump_url",
                "jumpUrl",
                "qqdocurl",
                "targetUrl",
                "link",
            ],
        )
    });
    let media_source = value.as_ref().and_then(|value| {
        find_json_string(
            value,
            &[
                "preview",
                "avatar",
                "image",
                "icon",
                "cover",
                "picture",
                "thumbnail",
                "img_url",
            ],
        )
    });
    let view_kind = value
        .as_ref()
        .and_then(|value| json_field_string(value, &["type", "view", "template"]))
        .unwrap_or_else(|| "json".to_string());
    JsonCardView {
        view_kind,
        title,
        desc,
        footer,
        jump_url,
        media_source,
        tag_icon_source: value
            .as_ref()
            .and_then(|value| find_json_string(value, &["tagIcon", "tag_icon"])),
        brand_icon_source: value
            .as_ref()
            .and_then(|value| find_json_string(value, &["sourcelogo", "sourceLogo"])),
    }
}

fn json_field_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn first_meta_value(value: &Value) -> Option<&Value> {
    value
        .get("meta")
        .and_then(Value::as_object)
        .and_then(|meta| meta.values().next())
}

fn contact_uin_url(contact: &Value) -> Option<String> {
    let contact_text = json_field_string(contact, &["contact"])?;
    let uin = contact_text
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|part| part.len() >= 5)?;
    Some(format!("https://mp.qzone.qq.com/u/{}", uin))
}

fn json_card_uses_vertical_media(view: &JsonCardView) -> bool {
    view.media_source.is_some() && !matches!(view.view_kind.as_str(), "contact" | "news")
        || json_card_uses_bottom_qr(view)
}

fn json_card_uses_bottom_qr(view: &JsonCardView) -> bool {
    view.jump_url.is_some() && !matches!(view.view_kind.as_str(), "contact" | "news" | "miniapp")
}

fn json_card_side_media_space(_view: &JsonCardView, has_side_media_slot: bool) -> u32 {
    if !has_side_media_slot {
        0
    } else {
        JSON_CARD_QR_SIZE + JSON_CARD_SIDE_GAP
    }
}

fn json_card_has_side_media_slot(view: &JsonCardView, media: Option<&ResolvedImage>) -> bool {
    matches!(view.view_kind.as_str(), "contact" | "news")
        && (view.media_source.is_some() || media.map(|img| img.has_bytes()).unwrap_or(false))
}

fn json_card_top_qr_space(view: &JsonCardView) -> u32 {
    if view.jump_url.is_some() && !json_card_uses_bottom_qr(view) {
        JSON_CARD_QR_SIZE + JSON_CARD_SIDE_GAP
    } else {
        0
    }
}

fn json_card_preview_height(
    view: &JsonCardView,
    media: Option<&ResolvedImage>,
    width: u32,
    card_padding: u32,
) -> u32 {
    if !json_card_uses_vertical_media(view) || media.is_none() {
        return 0;
    }
    let inner_width = width.saturating_sub(card_padding * 2).max(1);
    media
        .and_then(|img| img.width.zip(img.height))
        .filter(|(w, h)| *w > 0 && *h > 0)
        .map(|(w, h)| {
            ((inner_width as f32 * h as f32 / w as f32).round() as u32)
                .max(JSON_CARD_PREVIEW_MIN_HEIGHT)
        })
        .unwrap_or(JSON_CARD_PREVIEW_MIN_HEIGHT)
}

fn json_card_tag_row_height(has_tag: bool) -> u32 {
    if has_tag { JSON_CARD_TAG_ICON_SIZE } else { 0 }
}

fn json_card_miniapp_header_height(
    source_lines: usize,
    title_lines: usize,
    has_qr: bool,
    file_meta_height: u32,
    card_line_height: u32,
) -> u32 {
    let source_height = if source_lines > 0 {
        source_lines as u32 * file_meta_height + JSON_CARD_SIDE_GAP.saturating_sub(2)
    } else {
        0
    };
    let text_height = source_height + title_lines as u32 * card_line_height;
    text_height.max(if has_qr { JSON_CARD_QR_SIZE } else { 0 })
}

fn json_card_resolved_images(
    view: &JsonCardView,
    block_idx: usize,
    image_sources: &RenderImageSources,
) -> JsonCardResolvedImages {
    JsonCardResolvedImages {
        media: image_sources
            .block_images
            .get(block_idx)
            .and_then(|value| value.as_ref().cloned())
            .or_else(|| {
                view.media_source
                    .as_deref()
                    .filter(|source| !is_remote_http(source))
                    .and_then(resolve_source_to_image)
            }),
        tag_icon: image_sources
            .block_tag_icons
            .get(block_idx)
            .and_then(|value| value.as_ref().cloned())
            .or_else(|| {
                view.tag_icon_source
                    .as_deref()
                    .filter(|source| !is_remote_http(source))
                    .and_then(resolve_source_to_image)
            }),
        brand_icon: image_sources
            .block_brand_icons
            .get(block_idx)
            .and_then(|value| value.as_ref().cloned())
            .or_else(|| {
                view.brand_icon_source
                    .as_deref()
                    .filter(|source| !is_remote_http(source))
                    .and_then(resolve_source_to_image)
            }),
    }
}

fn resolve_json_card_local_images(view: &JsonCardView) -> JsonCardResolvedImages {
    let load = |source: &Option<String>| {
        source
            .as_deref()
            .filter(|source| !is_remote_http(source))
            .and_then(resolve_source_to_image)
    };
    JsonCardResolvedImages {
        media: load(&view.media_source),
        tag_icon: load(&view.tag_icon_source),
        brand_icon: load(&view.brand_icon_source),
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_json_card_block(
    x: u32,
    y: u32,
    content_width: u32,
    view: JsonCardView,
    images: JsonCardResolvedImages,
    font_size: u32,
    font_weight_title: u32,
    font_weight_body: u32,
    meta_size: u32,
    card_padding: u32,
    card_line_height: u32,
    file_meta_height: u32,
    file_meta_gap: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> BlockLayout {
    let card_title_size = font_size.saturating_sub(2).max(meta_size);
    let max_card_width = content_width.min(JSON_CARD_MAX_WIDTH).max(1);
    let mut width = max_card_width;
    let bottom_qr = json_card_uses_bottom_qr(&view);
    let qr_space = json_card_top_qr_space(&view);
    let vertical_media = json_card_uses_vertical_media(&view);
    let side_media_slot =
        !vertical_media && json_card_has_side_media_slot(&view, images.media.as_ref());
    let media_space = json_card_side_media_space(&view, side_media_slot);
    let text_max_w = width
        .saturating_sub(card_padding * 2 + qr_space + media_space)
        .max(1);
    let body_text_max_w = if view.view_kind == "news" {
        width.saturating_sub(card_padding * 2).max(1)
    } else {
        text_max_w
    };
    let title_lines = limit_lines(
        wrap_text(
            &view.title,
            text_max_w,
            card_title_size,
            font_weight_title,
            measurer,
            emoji_cache,
        ),
        2,
        text_max_w,
        card_title_size,
        font_weight_title,
        measurer,
        emoji_cache,
    );
    let desc_lines = view
        .desc
        .as_deref()
        .map(|desc| {
            let limit = if view.view_kind == "miniapp" {
                1
            } else {
                usize::MAX
            };
            limit_lines(
                wrap_text(
                    desc,
                    body_text_max_w,
                    meta_size,
                    font_weight_body,
                    measurer,
                    emoji_cache,
                ),
                limit,
                body_text_max_w,
                meta_size,
                font_weight_body,
                measurer,
                emoji_cache,
            )
        })
        .unwrap_or_default();
    let footer_line = view.footer.as_deref().map(|footer| {
        truncate_text(
            footer,
            body_text_max_w,
            meta_size,
            font_weight_body,
            measurer,
            emoji_cache,
        )
    });

    if matches!(view.view_kind.as_str(), "contact" | "news") {
        let title_width = title_lines
            .iter()
            .map(|line| {
                measure_inline_text_width(
                    line,
                    card_title_size,
                    font_weight_title,
                    measurer,
                    emoji_cache,
                )
            })
            .max()
            .unwrap_or(0);
        let desc_width = desc_lines
            .iter()
            .map(|line| {
                measure_inline_text_width(line, meta_size, font_weight_body, measurer, emoji_cache)
            })
            .max()
            .unwrap_or(0);
        let footer_width = footer_line
            .as_ref()
            .map(|line| {
                let icon_space = if images.tag_icon.is_some() {
                    JSON_CARD_TAG_ICON_SIZE + 4
                } else {
                    0
                };
                icon_space.saturating_add(measure_inline_text_width(
                    line,
                    meta_size,
                    font_weight_body,
                    measurer,
                    emoji_cache,
                ))
            })
            .unwrap_or(0);
        let side_space = qr_space + media_space;
        let content_fit = if view.view_kind == "news" {
            title_width
                .saturating_add(side_space)
                .max(desc_width)
                .max(footer_width)
        } else {
            side_space.saturating_add(title_width.max(desc_width).max(footer_width))
        };
        width = card_padding
            .saturating_mul(2)
            .saturating_add(content_fit)
            .saturating_add(if view.view_kind == "contact" {
                JSON_CARD_CONTACT_WIDTH_EXTRA_PX
            } else {
                0
            })
            .min(max_card_width)
            .max(JSON_CARD_QR_SIZE);
        if view.view_kind == "news" && footer_line.is_some() {
            width = width.saturating_sub(JSON_CARD_NEWS_WITH_FOOTER_WIDTH_SHRINK_PX);
        }
    }

    let has_footer = footer_line.as_ref().is_some_and(|line| !line.is_empty());
    let text_height = title_lines.len() as u32 * card_line_height
        + desc_lines.len() as u32 * file_meta_height
        + if has_footer { file_meta_height } else { 0 };
    let height = if vertical_media {
        if view.view_kind == "miniapp" {
            let header_height = json_card_miniapp_header_height(
                desc_lines.len(),
                title_lines.len(),
                view.jump_url.is_some(),
                file_meta_height,
                card_line_height,
            );
            let preview_height =
                json_card_preview_height(&view, images.media.as_ref(), width, card_padding);
            card_padding * 2
                + header_height
                + if preview_height > 0 {
                    JSON_CARD_SIDE_GAP.saturating_sub(2) + preview_height
                } else {
                    0
                }
                + if has_footer {
                    JSON_CARD_SIDE_GAP.saturating_sub(2) + json_card_tag_row_height(true)
                } else {
                    0
                }
        } else {
            let header_height = text_height.max(if qr_space > 0 { JSON_CARD_QR_SIZE } else { 0 });
            let bottom_qr_height = if bottom_qr {
                JSON_CARD_QR_SIZE + card_padding
            } else {
                0
            };
            let preview_height =
                json_card_preview_height(&view, images.media.as_ref(), width, card_padding);
            card_padding * 2
                + header_height
                + bottom_qr_height
                + if preview_height > 0 {
                    card_padding + preview_height
                } else {
                    0
                }
        }
    } else {
        let side_height = if qr_space > 0 || media_space > 0 {
            JSON_CARD_QR_SIZE
        } else {
            0
        };
        if view.view_kind == "news" {
            let body_height = desc_lines.len() as u32 * file_meta_height
                + if has_footer {
                    file_meta_gap + json_card_tag_row_height(true)
                } else {
                    0
                };
            card_padding * 2
                + side_height.max(card_line_height)
                + if body_height > 0 {
                    JSON_CARD_SIDE_GAP.saturating_sub(2)
                        + JSON_CARD_SIDE_GAP.saturating_sub(4)
                        + body_height
                } else {
                    0
                }
        } else {
            card_padding * 2 + text_height.max(side_height).max(card_line_height)
        }
    };

    BlockLayout {
        x,
        y,
        width,
        height,
        kind: BlockKind::JsonCard {
            qr_url: view.jump_url.clone(),
            view,
            title_lines,
            desc_lines,
            footer_line,
            media: images.media,
            tag_icon: images.tag_icon,
            brand_icon: images.brand_icon,
        },
    }
}

fn find_json_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(found.to_string());
                }
            }
            for child in map.values() {
                if let Some(found) = find_json_string(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_json_string(item, keys)),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn layout_forward_items(
    items: &[ForwardItem],
    x: u32,
    start_y: u32,
    content_width: u32,
    font_size: u32,
    line_height: u32,
    face_size: u32,
    font_weight_title: u32,
    font_weight_body: u32,
    meta_size: u32,
    card_padding: u32,
    card_line_height: u32,
    card_icon_size: u32,
    card_icon_gap: u32,
    file_padding: u32,
    file_line_height: u32,
    file_meta_height: u32,
    file_meta_gap: u32,
    file_icon_size: u32,
    file_icon_gap: u32,
    bubble_pad_left: u32,
    bubble_pad_right: u32,
    bubble_pad_top: u32,
    bubble_pad_bottom: u32,
    spacing: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> (Vec<ForwardLayoutItem>, u32) {
    let mut cursor_y = start_y;
    let mut out = Vec::new();
    for item in items {
        let mut blocks = Vec::new();
        for block in &item.blocks {
            if let Some(layout) = layout_forward_child_block(
                block,
                x,
                cursor_y,
                content_width,
                font_size,
                line_height,
                face_size,
                font_weight_title,
                font_weight_body,
                meta_size,
                card_padding,
                card_line_height,
                card_icon_size,
                card_icon_gap,
                file_padding,
                file_line_height,
                file_meta_height,
                file_meta_gap,
                file_icon_size,
                file_icon_gap,
                bubble_pad_left,
                bubble_pad_right,
                bubble_pad_top,
                bubble_pad_bottom,
                measurer,
                emoji_cache,
            ) {
                cursor_y = cursor_y
                    .saturating_add(layout.height)
                    .saturating_add(spacing);
                blocks.push(layout);
            }
        }
        if blocks.is_empty() {
            let fallback = DraftBlock::Paragraph {
                text: "[空消息]".to_string(),
            };
            if let Some(layout) = layout_forward_child_block(
                &fallback,
                x,
                cursor_y,
                content_width,
                font_size,
                line_height,
                face_size,
                font_weight_title,
                font_weight_body,
                meta_size,
                card_padding,
                card_line_height,
                card_icon_size,
                card_icon_gap,
                file_padding,
                file_line_height,
                file_meta_height,
                file_meta_gap,
                file_icon_size,
                file_icon_gap,
                bubble_pad_left,
                bubble_pad_right,
                bubble_pad_top,
                bubble_pad_bottom,
                measurer,
                emoji_cache,
            ) {
                cursor_y = cursor_y
                    .saturating_add(layout.height)
                    .saturating_add(spacing);
                blocks.push(layout);
            }
        }
        assign_legacy_json_card_qr_urls(&mut blocks);
        out.push(ForwardLayoutItem { blocks });
    }
    let bottom = if cursor_y > start_y {
        cursor_y.saturating_sub(spacing)
    } else {
        start_y
    };
    (out, bottom)
}

fn layout_forward_child_block(
    block: &DraftBlock,
    x: u32,
    y: u32,
    content_width: u32,
    font_size: u32,
    line_height: u32,
    face_size: u32,
    font_weight_title: u32,
    font_weight_body: u32,
    meta_size: u32,
    card_padding: u32,
    card_line_height: u32,
    card_icon_size: u32,
    card_icon_gap: u32,
    file_padding: u32,
    file_line_height: u32,
    file_meta_height: u32,
    file_meta_gap: u32,
    file_icon_size: u32,
    file_icon_gap: u32,
    bubble_pad_left: u32,
    bubble_pad_right: u32,
    bubble_pad_top: u32,
    bubble_pad_bottom: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> Option<BlockLayout> {
    match block {
        DraftBlock::Paragraph { text } => {
            let max_text_w = content_width
                .saturating_sub(bubble_pad_left + bubble_pad_right)
                .max(1);
            let lines = wrap_inline_text(
                text,
                max_text_w,
                font_size,
                face_size,
                font_weight_body,
                measurer,
                emoji_cache,
            );
            let max_line_w = lines.iter().map(|line| line.width).max().unwrap_or(0);
            let width = (max_line_w + bubble_pad_left + bubble_pad_right)
                .saturating_sub(FORWARD_TEXT_BUBBLE_WIDTH_SHRINK_PX)
                .min(content_width)
                .max(1);
            let height = (bubble_pad_top + bubble_pad_bottom + line_height * lines.len() as u32)
                .saturating_sub(if lines.len() == 1 {
                    FORWARD_SINGLE_LINE_TEXT_HEIGHT_SHRINK_PX
                } else {
                    0
                });
            Some(BlockLayout {
                x,
                y,
                width,
                height,
                kind: BlockKind::Text { lines },
            })
        }
        DraftBlock::Attachment {
            kind,
            name,
            reference,
            size_bytes,
        } => match *kind {
            oqqwall_rust_core::MediaKind::Image | oqqwall_rust_core::MediaKind::Sticker => {
                let image = match reference {
                    MediaReference::RemoteUrl { url } if !is_remote_http(url) => {
                        resolve_source_to_image(url)
                    }
                    _ => None,
                };
                let (width, height) = match image
                    .as_ref()
                    .and_then(|img| img.width.zip(img.height))
                    .filter(|(w, h)| *w > 0 && *h > 0)
                {
                    Some((orig_w, orig_h)) => {
                        let max_width = (content_width / 2).max(1);
                        let max_height = 300u32;
                        let scale_w = max_width as f32 / orig_w as f32;
                        let scale_h = max_height as f32 / orig_h as f32;
                        let scale = scale_w.min(scale_h).min(1.0);
                        (
                            (orig_w as f32 * scale).round().max(1.0) as u32,
                            (orig_h as f32 * scale).round().max(1.0) as u32,
                        )
                    }
                    None => image_preview_fallback_size(content_width),
                };
                Some(BlockLayout {
                    x,
                    y,
                    width,
                    height,
                    kind: BlockKind::Image { image },
                })
            }
            oqqwall_rust_core::MediaKind::Video => {
                let image = match reference {
                    MediaReference::RemoteUrl { url } if !is_remote_http(url) => {
                        resolve_source_to_video_preview(url)
                    }
                    _ => None,
                };
                let (fallback_width, fallback_height) = image_preview_fallback_size(content_width);
                let (width, height) = image_like_preview_size(
                    image.as_ref(),
                    content_width,
                    fallback_width,
                    fallback_height,
                );
                Some(BlockLayout {
                    x,
                    y,
                    width,
                    height,
                    kind: BlockKind::VideoPreview { image },
                })
            }
            oqqwall_rust_core::MediaKind::File => {
                let file_name_size = font_size.saturating_sub(2).max(meta_size);
                let max_width = content_width.min(320).max(1);
                let text_max_w = max_width
                    .saturating_sub(file_padding * 2 + file_icon_size + file_icon_gap)
                    .max(1);
                let filename = name
                    .clone()
                    .or_else(|| {
                        media_reference_label(reference).and_then(|url| extract_filename(&url))
                    })
                    .unwrap_or_else(|| "Unknown file".to_string());
                let name_lines = wrap_text(
                    &filename,
                    text_max_w,
                    file_name_size,
                    font_weight_body,
                    measurer,
                    emoji_cache,
                );
                let meta_line = size_bytes.as_ref().and_then(|size| format_size_line(*size));
                let width = file_card_width(
                    max_width,
                    &name_lines,
                    meta_line.as_deref(),
                    file_padding,
                    file_icon_size,
                    file_icon_gap,
                    file_name_size,
                    11,
                    font_weight_body,
                    measurer,
                    emoji_cache,
                )
                .saturating_sub(FORWARD_FILE_CARD_WIDTH_SHRINK_PX)
                .max(1);
                let name_height = name_lines.len() as u32 * file_line_height;
                let meta_height = if meta_line.is_some() {
                    file_meta_height + file_meta_gap
                } else {
                    0
                };
                let text_height = name_height + meta_height;
                let content_height = file_icon_size.max(text_height);
                Some(BlockLayout {
                    x,
                    y,
                    width,
                    height: content_height + file_padding * 2,
                    kind: BlockKind::FileCard {
                        name_lines,
                        meta_line,
                        icon_path: file_icon_path(&filename),
                        icon_text: file_icon_text(&filename),
                    },
                })
            }
            _ => {
                let height = 90;
                let width = content_width.min(320).max(1);
                let text_max_w = width
                    .saturating_sub(card_padding * 2 + card_icon_size + card_icon_gap)
                    .max(1);
                let mut lines = vec![media_label(*kind).to_string()];
                if let Some(detail) =
                    media_reference_label(reference).and_then(|url| extract_filename(&url))
                {
                    let detail_line = truncate_text(
                        &detail,
                        text_max_w,
                        font_size,
                        font_weight_body,
                        measurer,
                        emoji_cache,
                    );
                    if !detail_line.is_empty() {
                        lines.push(detail_line);
                    }
                }
                Some(BlockLayout {
                    x,
                    y,
                    width,
                    height,
                    kind: BlockKind::MediaCard {
                        lines,
                        icon_text: media_icon_text(*kind),
                        media_kind: *kind,
                    },
                })
            }
        },
        DraftBlock::Reply { preview } => {
            let width = content_width.max(1);
            let wrap_width = width.min(320).max(1);
            let reply_accent_width = 3u32;
            let reply_inner_pad_x = card_padding;
            let reply_inner_pad_y = 6u32;
            let reply_meta_size = font_size.saturating_sub(2).max(meta_size);
            let reply_body_size = font_size;
            let reply_meta_line_height = file_line_height;
            let reply_body_line_height = line_height;
            let reply_outer_extra_height = 6u32;
            let text_max_w = wrap_width
                .saturating_sub(
                    bubble_pad_left + bubble_pad_right + reply_accent_width + reply_inner_pad_x * 2,
                )
                .saturating_add(REPLY_WRAP_WIDTH_COMPENSATION_PX)
                .max(1);
            let meta = reply_meta_text(preview);
            let meta_lines = limit_lines(
                wrap_text(
                    &meta,
                    text_max_w,
                    reply_meta_size,
                    font_weight_body,
                    measurer,
                    emoji_cache,
                ),
                1,
                text_max_w,
                reply_meta_size,
                font_weight_body,
                measurer,
                emoji_cache,
            );
            let body_lines = wrap_text(
                &preview.body,
                text_max_w,
                reply_body_size,
                font_weight_body,
                measurer,
                emoji_cache,
            );
            let body_gap = if body_lines.is_empty() {
                0
            } else {
                file_meta_gap
            };
            let inner_height = reply_inner_pad_y * 2
                + meta_lines.len() as u32 * reply_meta_line_height
                + body_gap
                + body_lines.len() as u32 * reply_body_line_height;
            let height =
                bubble_pad_top + bubble_pad_bottom + inner_height + reply_outer_extra_height;
            Some(BlockLayout {
                x,
                y,
                width,
                height,
                kind: BlockKind::Reply {
                    meta_lines,
                    body_lines,
                },
            })
        }
        DraftBlock::Poke => {
            let image = resolve_source_to_image("res/poke.png");
            let (width, height) = image
                .as_ref()
                .and_then(|img| img.width.zip(img.height))
                .map(|(orig_w, orig_h)| {
                    let max_width = (content_width / 2).max(48);
                    let max_height = 120u32;
                    let scale_w = max_width as f32 / orig_w.max(1) as f32;
                    let scale_h = max_height as f32 / orig_h.max(1) as f32;
                    let scale = scale_w.min(scale_h).min(1.0);
                    (
                        (orig_w as f32 * scale).round().max(1.0) as u32,
                        (orig_h as f32 * scale).round().max(1.0) as u32,
                    )
                })
                .unwrap_or((64, 64));
            Some(BlockLayout {
                x,
                y,
                width,
                height,
                kind: BlockKind::Poke { image },
            })
        }
        DraftBlock::JsonCard { raw } => {
            let view = parse_json_card_view(raw);
            Some(layout_json_card_block(
                x,
                y,
                content_width,
                view.clone(),
                resolve_json_card_local_images(&view),
                font_size,
                font_weight_title,
                font_weight_body,
                meta_size,
                card_padding,
                card_line_height,
                file_meta_height,
                file_meta_gap,
                measurer,
                emoji_cache,
            ))
        }
        DraftBlock::Forward { items } => {
            let title_h = meta_size + file_meta_gap;
            let forward_child_indent = 17u32;
            let child_x = x + forward_child_indent;
            let child_y = y + title_h + file_meta_gap + 12;
            let child_width = content_width.saturating_sub(forward_child_indent).max(1);
            let (layout_items, bottom) = layout_forward_items(
                items,
                child_x,
                child_y,
                child_width,
                font_size,
                line_height,
                face_size,
                font_weight_title,
                font_weight_body,
                meta_size,
                card_padding,
                card_line_height,
                card_icon_size,
                card_icon_gap,
                file_padding,
                file_line_height,
                file_meta_height,
                file_meta_gap,
                file_icon_size,
                file_icon_gap,
                bubble_pad_left,
                bubble_pad_right,
                bubble_pad_top,
                bubble_pad_bottom,
                10,
                measurer,
                emoji_cache,
            );
            let height = bottom.saturating_sub(y).saturating_add(card_padding);
            Some(BlockLayout {
                x,
                y,
                width: content_width,
                height,
                kind: BlockKind::Forward {
                    items: layout_items,
                },
            })
        }
    }
}

fn media_reference_label(reference: &MediaReference) -> Option<String> {
    match reference {
        MediaReference::RemoteUrl { url } => Some(url.clone()),
        MediaReference::Blob { .. } => None,
    }
}

fn file_card_width(
    content_width: u32,
    name_lines: &[String],
    meta_line: Option<&str>,
    file_padding: u32,
    file_icon_size: u32,
    file_icon_gap: u32,
    font_size: u32,
    meta_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> u32 {
    let name_width = name_lines
        .iter()
        .map(|line| measure_inline_text_width(line, font_size, font_weight, measurer, emoji_cache))
        .max()
        .unwrap_or(0);
    let meta_width = meta_line
        .map(|line| measure_inline_text_width(line, meta_size, font_weight, measurer, emoji_cache))
        .unwrap_or(0);
    let text_width = name_width.max(meta_width);
    let desired = text_width
        .saturating_add(file_icon_size)
        .saturating_add(file_icon_gap)
        .saturating_add(file_padding * 2);
    desired
        .min(content_width)
        .max(file_icon_size + file_padding * 2)
}

impl ImageMemoryCache {
    fn on_event(&mut self, state: &StateView, event: &Event) {
        match event {
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id,
                message,
                ..
            })
            | Event::Ingress(IngressEvent::MessageSynced {
                ingress_id,
                message,
                ..
            }) => {
                self.prime_from_message(*ingress_id, message);
            }
            Event::Media(MediaEvent::MediaFetchSucceeded {
                ingress_id,
                attachment_index,
                blob_id,
            }) => {
                self.prime_from_blob(state, *ingress_id, *attachment_index, *blob_id);
            }
            Event::Ingress(IngressEvent::MessageIgnored { ingress_id, .. }) => {
                self.clear_for_ingress(*ingress_id);
            }
            Event::Ingress(IngressEvent::MessageRecalled { ingress_id, .. }) => {
                self.clear_for_ingress(*ingress_id);
            }
            _ => {}
        }
    }

    fn prime_from_message(&mut self, ingress_id: IngressId, message: &IngressMessage) {
        for (idx, attachment) in message.attachments.iter().enumerate() {
            if !is_renderable_image(attachment.kind) {
                continue;
            }
            let MediaReference::RemoteUrl { url } = &attachment.reference else {
                continue;
            };
            if is_remote_http(url) {
                continue;
            }
            let key = ImageCacheKey::Source(url.clone());
            if !self.entries.contains_key(&key) {
                if let Some(image) = resolve_source_to_image(url) {
                    self.entries.insert(key.clone(), image);
                }
            }
            self.attachment_sources.insert((ingress_id, idx), key);
        }
    }

    fn prime_from_blob(
        &mut self,
        state: &StateView,
        ingress_id: IngressId,
        attachment_index: usize,
        blob_id: BlobId,
    ) {
        if !attachment_is_image(state, ingress_id, attachment_index) {
            return;
        }
        if let Some(key) = self
            .attachment_sources
            .remove(&(ingress_id, attachment_index))
        {
            self.entries.remove(&key);
        }
        let key = ImageCacheKey::Blob(blob_id);
        if self.entries.contains_key(&key) {
            return;
        }
        if let Some(image) = resolve_blob_image(state, blob_id) {
            self.entries.insert(key, image);
        }
    }

    fn clear_for_ingress(&mut self, ingress_id: IngressId) {
        let keys = self
            .attachment_sources
            .iter()
            .filter_map(|((id, _), key)| (*id == ingress_id).then(|| key.clone()))
            .collect::<Vec<_>>();
        for key in keys {
            self.entries.remove(&key);
        }
        self.attachment_sources
            .retain(|(id, _), _| *id != ingress_id);
    }

    fn release_keys(&mut self, used_keys: &HashSet<ImageCacheKey>) {
        for key in used_keys {
            self.entries.remove(key);
        }
        self.attachment_sources
            .retain(|_, key| !used_keys.contains(key));
    }

    fn get_or_load_source(&mut self, source: &str) -> Option<ResolvedImage> {
        let key = ImageCacheKey::Source(source.to_string());
        if let Some(image) = self.entries.get(&key) {
            return Some(image.clone());
        }
        let image = resolve_source_to_image(source)?;
        self.entries.insert(key, image.clone());
        Some(image)
    }

    async fn get_or_fetch_remote_source(&mut self, source: &str) -> Option<ResolvedImage> {
        let key = ImageCacheKey::Source(source.to_string());
        if let Some(image) = self.entries.get(&key) {
            return Some(image.clone());
        }
        let image = fetch_remote_image(source).await?;
        self.entries.insert(key, image.clone());
        Some(image)
    }

    fn get_or_load_video_preview_source(&mut self, source: &str) -> Option<ResolvedImage> {
        let key = ImageCacheKey::Source(format!("video-preview:{}", source));
        if let Some(image) = self.entries.get(&key) {
            return Some(image.clone());
        }
        let image = resolve_source_to_video_preview(source)?;
        self.entries.insert(key, image.clone());
        Some(image)
    }

    fn get_or_load_video_preview_blob(
        &mut self,
        state: &StateView,
        blob_id: BlobId,
    ) -> Option<ResolvedImage> {
        let key = ImageCacheKey::Source(format!("video-preview:blob:{}", id128_hex(blob_id.0)));
        if let Some(image) = self.entries.get(&key) {
            return Some(image.clone());
        }
        let image = resolve_blob_video_preview(state, blob_id)?;
        self.entries.insert(key, image.clone());
        Some(image)
    }

    fn get_or_load_blob(&mut self, state: &StateView, blob_id: BlobId) -> Option<ResolvedImage> {
        let key = ImageCacheKey::Blob(blob_id);
        if let Some(image) = self.entries.get(&key) {
            return Some(image.clone());
        }
        let image = resolve_blob_image(state, blob_id)?;
        self.entries.insert(key, image.clone());
        Some(image)
    }
}

fn is_renderable_image(kind: MediaKind) -> bool {
    matches!(kind, MediaKind::Image | MediaKind::Sticker)
}

fn is_video_media(kind: MediaKind) -> bool {
    matches!(kind, MediaKind::Video)
}

fn image_preview_fallback_size(content_width: u32) -> (u32, u32) {
    let width = (content_width / 2).max(1);
    let height = (width.saturating_mul(3) / 4).min(300).max(1);
    (width, height)
}

fn image_like_preview_size(
    image: Option<&ResolvedImage>,
    content_width: u32,
    fallback_width: u32,
    fallback_height: u32,
) -> (u32, u32) {
    let max_width = (content_width / 2).max(1);
    let max_height = 300u32;
    match image
        .and_then(|img| img.width.zip(img.height))
        .filter(|(w, h)| *w > 0 && *h > 0)
    {
        Some((orig_w, orig_h)) => {
            let scale_w = max_width as f32 / orig_w as f32;
            let scale_h = max_height as f32 / orig_h as f32;
            let scale = scale_w.min(scale_h).min(1.0);
            (
                (orig_w as f32 * scale).round().max(1.0) as u32,
                (orig_h as f32 * scale).round().max(1.0) as u32,
            )
        }
        None => (fallback_width.max(1), fallback_height.max(1)),
    }
}

fn attachment_is_image(state: &StateView, ingress_id: IngressId, attachment_index: usize) -> bool {
    state
        .ingress_messages
        .get(&ingress_id)
        .and_then(|message| message.attachments.get(attachment_index))
        .map(|attachment| is_renderable_image(attachment.kind))
        .unwrap_or(false)
}

fn purge_avatar_for_post(state: &StateView, post_id: PostId) {
    let Some(ingress_ids) = state.post_ingress.get(&post_id) else {
        return;
    };
    for ingress_id in ingress_ids {
        if let Some(meta) = state.ingress_meta.get(ingress_id) {
            avatar_cache::remove_avatar(&meta.user_id);
        }
    }
}

pub fn spawn_renderer(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    config: RendererRuntimeConfig,
) -> Result<JoinHandle<()>, String> {
    validate_renderer_resources()?;
    let font_dir = resolve_font_dir();
    init_font_bytes_cache(&font_dir);
    let _ = emoji_png_store();
    Ok(tokio::spawn(async move {
        debug_log!(
            "renderer task start: blob_root={} canvas_width={} max_height={}",
            config.blob_root.display(),
            config.canvas_width_px,
            config.max_height_px
        );
        let mut state = load_state_view_cached();
        let mut bus_rx = bus_rx;
        let mut image_cache = ImageMemoryCache::default();

        loop {
            let env = match bus_rx.recv().await {
                Ok(env) => env,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };

            state = state.reduce(&env);
            image_cache.on_event(&state, &env.event);

            if let Event::Send(SendEvent::SendSucceeded { post_id, .. })
            | Event::Send(SendEvent::SendGaveUp { post_id, .. }) = env.event
            {
                purge_avatar_for_post(&state, post_id);
            }

            if let Event::Render(RenderEvent::RenderRequested {
                post_id, attempt, ..
            }) = env.event
            {
                if let Err(_err) = handle_render_request(
                    &cmd_tx,
                    &state,
                    post_id,
                    attempt,
                    &config,
                    &mut image_cache,
                )
                .await
                {
                    debug_log!("render failed: post_id={} err={}", post_id.0, _err);
                }
            }
        }

        debug_log!("renderer task end");
    }))
}

fn load_state_view_cached() -> StateView {
    static CACHE: OnceLock<StateView> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let data_dir = std::env::var("OQQWALL_DATA_DIR").unwrap_or_else(|_| "data".to_string());
            let journal = match LocalJournal::open(&data_dir) {
                Ok(journal) => journal,
                Err(_err) => {
                    debug_log!("renderer preload skipped: journal open failed: {}", _err);
                    return StateView::default();
                }
            };
            let snapshot = match SnapshotStore::open(&data_dir) {
                Ok(snapshot) => snapshot,
                Err(_err) => {
                    debug_log!("renderer preload skipped: snapshot open failed: {}", _err);
                    return StateView::default();
                }
            };

            let mut state = StateView::default();
            let mut cursor = None;
            match snapshot.load() {
                Ok(Some(loaded)) => {
                    state = loaded.state;
                    cursor = loaded.journal_cursor;
                }
                Ok(None) => {}
                Err(_err) => {
                    debug_log!("renderer preload: snapshot load failed: {}", _err);
                }
            }

            if let Err(_err) = journal.replay(cursor, |env| {
                state = state.reduce(env);
            }) {
                debug_log!("renderer preload: journal replay failed: {}", _err);
            }

            state
        })
        .clone()
}

async fn handle_render_request(
    cmd_tx: &mpsc::Sender<Command>,
    state: &StateView,
    post_id: PostId,
    attempt: u32,
    config: &RendererRuntimeConfig,
    image_cache: &mut ImageMemoryCache,
) -> Result<(), String> {
    let draft = match rebuild_draft_from_state(state, post_id)
        .or_else(|| state.drafts.get(&post_id).cloned())
    {
        Some(draft) => draft,
        None => {
            return send_render_failed(
                cmd_tx,
                post_id,
                attempt,
                "missing draft for render".to_string(),
            )
            .await;
        }
    };

    let header = extract_header(state, post_id);
    let draft = resolve_forward_draft(&draft, &header, config).await;
    let (image_sources, used_keys) =
        resolve_image_sources(state, &draft, &header, cmd_tx, image_cache).await;
    let render_result = render_png_async(&draft, &header, &image_sources, config).await;
    drop(image_sources);
    image_cache.release_keys(&used_keys);
    let pages = match render_result {
        Ok(pages) => {
            let render_only_blob_ids = collect_post_blob_ids(state, post_id);
            blob_cache::release_render_only(render_only_blob_ids);
            pages
        }
        Err(err) => {
            return send_render_failed(cmd_tx, post_id, attempt, err).await;
        }
    };

    if pages.is_empty() {
        return send_render_failed(
            cmd_tx,
            post_id,
            attempt,
            "render produced no pages".to_string(),
        )
        .await;
    }

    let mut ready_blob_ids = Vec::new();
    for (page_index, bytes) in pages.into_iter().enumerate() {
        let blob_id = render_blob_id(post_id, page_index);
        let bytes = blob_cache::store_bytes(
            blob_id,
            bytes,
            CacheKind::Image,
            CacheRetention::UntilSend,
            Some("image/png".to_string()),
        );
        let (path, size_bytes) =
            persist_blob(&config.blob_root, "png", "png", blob_id, bytes.as_ref())?;

        send_event(
            cmd_tx,
            Event::Blob(BlobEvent::BlobRegistered {
                blob_id,
                size_bytes,
            }),
        )
        .await?;
        send_event(
            cmd_tx,
            Event::Blob(BlobEvent::BlobPersisted { blob_id, path }),
        )
        .await?;

        ready_blob_ids.push(blob_id);
    }

    send_event(
        cmd_tx,
        Event::Render(RenderEvent::PngBatchReady {
            post_id,
            blob_ids: ready_blob_ids,
        }),
    )
    .await?;

    Ok(())
}

fn rebuild_draft_from_state(state: &StateView, post_id: PostId) -> Option<Draft> {
    let ingress_ids = state.post_ingress.get(&post_id)?;
    let mut messages = Vec::new();
    for ingress_id in ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            messages.push(message.clone());
        }
    }
    if messages.is_empty() {
        return None;
    }
    Some(build_draft_from_messages(&messages))
}

fn collect_post_blob_ids(state: &StateView, post_id: PostId) -> Vec<BlobId> {
    let Some(ingress_ids) = state.post_ingress.get(&post_id) else {
        return Vec::new();
    };
    let mut blob_ids = Vec::new();
    for ingress_id in ingress_ids {
        if let Some(message) = state.ingress_messages.get(ingress_id) {
            for attachment in &message.attachments {
                if let MediaReference::Blob { blob_id } = attachment.reference {
                    blob_ids.push(blob_id);
                }
            }
        }
    }
    blob_ids
}

struct ForwardContext {
    account_id: String,
    cache: HashMap<String, Vec<DraftBlock>>,
    seen: HashSet<String>,
}

async fn resolve_forward_draft(
    draft: &Draft,
    header: &HeaderInfo,
    _config: &RendererRuntimeConfig,
) -> Draft {
    let mut context = napcat_account_for_group(&header.group_id).map(|account_id| ForwardContext {
        account_id,
        cache: HashMap::new(),
        seen: HashSet::new(),
    });

    let blocks = resolve_forward_blocks(&draft.blocks, &mut context, 0).await;
    Draft { blocks }
}

fn resolve_forward_blocks<'a>(
    source_blocks: &'a [DraftBlock],
    context: &'a mut Option<ForwardContext>,
    depth: u32,
) -> Pin<Box<dyn Future<Output = Vec<DraftBlock>> + Send + 'a>> {
    Box::pin(async move {
        let mut blocks = Vec::new();
        for block in source_blocks {
            match block {
                DraftBlock::Paragraph { text }
                    if context.is_some() && text.contains(FORWARD_PREFIX) =>
                {
                    let mut expanded = expand_forward_in_text(text, context, depth).await;
                    blocks.append(&mut expanded);
                }
                DraftBlock::Forward { items } => {
                    if depth >= MAX_FORWARD_DEPTH {
                        blocks.push(DraftBlock::Paragraph {
                            text: "[合并转发:层级过深]".to_string(),
                        });
                    } else {
                        let items = resolve_forward_items(items, context, depth + 1).await;
                        blocks.push(DraftBlock::Forward { items });
                    }
                }
                _ => blocks.push(block.clone()),
            }
        }
        blocks
    })
}

fn resolve_forward_items<'a>(
    source_items: &'a [ForwardItem],
    context: &'a mut Option<ForwardContext>,
    depth: u32,
) -> Pin<Box<dyn Future<Output = Vec<ForwardItem>> + Send + 'a>> {
    Box::pin(async move {
        let mut items = Vec::with_capacity(source_items.len());
        for item in source_items {
            let blocks = resolve_forward_blocks(&item.blocks, context, depth).await;
            items.push(ForwardItem {
                sender_name: item.sender_name.clone(),
                blocks,
            });
        }
        items
    })
}

fn normalize_embedded_forward_draft(draft: &Draft) -> Draft {
    let blocks = draft
        .blocks
        .iter()
        .cloned()
        .map(|block| normalize_embedded_forward_block(block, 0))
        .collect();
    Draft { blocks }
}

fn normalize_embedded_forward_block(block: DraftBlock, depth: u32) -> DraftBlock {
    match block {
        DraftBlock::Forward { items } if depth < MAX_FORWARD_DEPTH => DraftBlock::Forward {
            items: normalize_forward_items(items, depth + 1),
        },
        DraftBlock::Forward { .. } => DraftBlock::Paragraph {
            text: "[合并转发:层级过深]".to_string(),
        },
        _ => block,
    }
}

fn normalize_forward_items(items: Vec<ForwardItem>, depth: u32) -> Vec<ForwardItem> {
    items
        .into_iter()
        .map(|item| {
            let blocks = item
                .blocks
                .into_iter()
                .map(|block| normalize_embedded_forward_block(block, depth))
                .collect();
            ForwardItem {
                sender_name: item.sender_name,
                blocks,
            }
        })
        .collect()
}

fn forward_placeholder(id: &str) -> String {
    if id.is_empty() {
        "[合并转发]".to_string()
    } else {
        format!("[合并转发:{}]", id)
    }
}

fn push_text_block(blocks: &mut Vec<DraftBlock>, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    blocks.push(DraftBlock::Paragraph {
        text: trimmed.to_string(),
    });
}

fn expand_forward_in_text<'a>(
    text: &'a str,
    context: &'a mut Option<ForwardContext>,
    depth: u32,
) -> Pin<Box<dyn Future<Output = Vec<DraftBlock>> + Send + 'a>> {
    Box::pin(async move {
        let Some(context) = context.as_mut() else {
            let mut blocks = Vec::new();
            push_text_block(&mut blocks, text);
            return blocks;
        };
        expand_forward_in_text_with_context(text, context, depth).await
    })
}

fn expand_forward_in_text_with_context<'a>(
    text: &'a str,
    context: &'a mut ForwardContext,
    depth: u32,
) -> Pin<Box<dyn Future<Output = Vec<DraftBlock>> + Send + 'a>> {
    Box::pin(async move {
        if depth >= MAX_FORWARD_DEPTH {
            let mut blocks = Vec::new();
            push_text_block(&mut blocks, text);
            return blocks;
        }

        let mut blocks = Vec::new();
        let mut remaining = text;
        while let Some(start) = remaining.find(FORWARD_PREFIX) {
            let (before, rest) = remaining.split_at(start);
            push_text_block(&mut blocks, before);

            let after_prefix = &rest[FORWARD_PREFIX.len()..];
            let Some(end) = after_prefix.find(']') else {
                push_text_block(&mut blocks, rest);
                return blocks;
            };
            let id = after_prefix[..end].trim();
            let mut resolved = forward_blocks_for_id(id, context, depth).await;
            blocks.append(&mut resolved);
            remaining = &after_prefix[end + 1..];
        }
        push_text_block(&mut blocks, remaining);
        blocks
    })
}

async fn forward_blocks_for_id(
    forward_id: &str,
    context: &mut ForwardContext,
    depth: u32,
) -> Vec<DraftBlock> {
    if forward_id.is_empty() || depth >= MAX_FORWARD_DEPTH {
        return vec![DraftBlock::Paragraph {
            text: forward_placeholder(forward_id),
        }];
    }

    if let Some(cached) = context.cache.get(forward_id) {
        return cached.clone();
    }
    if context.seen.contains(forward_id) {
        return vec![DraftBlock::Paragraph {
            text: forward_placeholder(forward_id),
        }];
    }
    context.seen.insert(forward_id.to_string());

    let resolved = match fetch_forward_messages(context, forward_id).await {
        Ok(messages) => {
            let items = forward_messages_to_items(&messages, context, depth + 1).await;
            vec![DraftBlock::Forward { items }]
        }
        Err(_err) => {
            debug_log!("forward resolve failed: id={} err={}", forward_id, _err);
            vec![DraftBlock::Paragraph {
                text: forward_placeholder(forward_id),
            }]
        }
    };
    context
        .cache
        .insert(forward_id.to_string(), resolved.clone());
    resolved
}

async fn fetch_forward_messages(
    context: &ForwardContext,
    forward_id: &str,
) -> Result<Vec<Value>, String> {
    let body = napcat_ws_request(
        &context.account_id,
        "get_forward_msg",
        json!({ "message_id": forward_id }),
        Duration::from_secs(6),
    )
    .await?;
    let messages = body
        .get("data")
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing forward messages".to_string())?;
    Ok(messages.to_vec())
}

async fn forward_messages_to_items(
    messages: &[Value],
    context: &mut ForwardContext,
    depth: u32,
) -> Vec<ForwardItem> {
    let mut items = Vec::new();
    for message in messages {
        let payload = message.get("message").or_else(|| message.get("content"));
        let extracted = extract_message_lite(payload);
        let mut blocks = expand_forward_in_text_with_context(&extracted.text, context, depth).await;
        for attachment in extracted.attachments {
            blocks.push(DraftBlock::Attachment {
                kind: attachment.kind,
                name: attachment.name,
                reference: attachment.reference,
                size_bytes: attachment.size_bytes,
            });
        }
        items.push(ForwardItem {
            sender_name: forward_sender_name(message),
            blocks,
        });
    }
    items
}

fn forward_sender_name(message: &Value) -> Option<String> {
    message
        .get("sender")
        .and_then(Value::as_object)
        .and_then(|sender| {
            sender
                .get("nickname")
                .or_else(|| sender.get("card"))
                .and_then(|value| value.as_str())
        })
        .or_else(|| message.get("sender_name").and_then(Value::as_str))
        .or_else(|| message.get("nickname").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn extract_header(state: &StateView, post_id: PostId) -> HeaderInfo {
    let mut group_id = "unknown".to_string();
    let mut user_id = "unknown".to_string();
    let mut sender_name = None;
    let is_anonymous = state
        .posts
        .get(&post_id)
        .map(|meta| meta.is_anonymous)
        .unwrap_or(false);
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            if let Some(meta) = state.ingress_meta.get(ingress_id) {
                group_id = meta.group_id.clone();
                user_id = meta.user_id.clone();
                sender_name = meta
                    .sender_name
                    .clone()
                    .filter(|name| !name.trim().is_empty());
                break;
            }
        }
    }
    HeaderInfo {
        group_id,
        user_id,
        post_id_hex: id128_hex(post_id.0),
        sender_name,
        is_anonymous,
    }
}

fn render_png(
    draft: &Draft,
    header: &HeaderInfo,
    image_sources: &RenderImageSources,
    config: &RendererRuntimeConfig,
) -> Result<Vec<u8>, String> {
    render_png_pages(draft, header, image_sources, config)?
        .into_iter()
        .next()
        .ok_or_else(|| "render produced no pages".to_string())
}

fn render_png_pages(
    draft: &Draft,
    header: &HeaderInfo,
    image_sources: &RenderImageSources,
    config: &RendererRuntimeConfig,
) -> Result<Vec<Vec<u8>>, String> {
    let padding = 20u32;
    let spacing_lg = 10u32;
    let spacing_xxl = 5u32;
    let bubble_pad_left = 8u32;
    let bubble_pad_right = 8u32;
    let bubble_pad_top = 6u32;
    let bubble_pad_bottom = 6u32;
    let font_size = 16u32;
    let line_height = 22u32;
    let face_size = 16u32;
    let title_size = 32u32;
    let meta_size = 12u32;
    let font_weight_title = 600u32;
    let font_weight_body = 400u32;
    let header_gap = 10u32;
    let avatar_size = 50u32;
    let radius_lg = 12u32;
    let card_padding = 8u32;
    let card_title_size = font_size.saturating_sub(2).max(meta_size);
    let card_line_height = 18u32;
    let card_icon_size = 24u32;
    let card_icon_gap = 8u32;
    let forward_line_gap = 6u32;
    let file_padding = 7u32;
    let file_line_height = 18u32;
    let file_meta_height = 14u32;
    let file_meta_gap = 2u32;
    let file_icon_size = 40u32;
    let file_icon_gap = 6u32;
    let font_dir = resolve_font_dir();
    let font_collection = build_font_collection(&font_dir);
    let mut text_measurer = TextMeasurer::new(font_collection.clone());
    let mut emoji_cache = EmojiRenderCache::new();

    let scale = 3u32;
    let canvas_width_px = (config.canvas_width_px / scale).max(1);
    let max_height_px = (config.max_height_px / scale).max(1);
    debug_log!(
        "render start: blocks={} output_canvas_width={} output_max_height={} logical_canvas_width={} logical_max_height={} scale={}",
        draft.blocks.len(),
        config.canvas_width_px,
        config.max_height_px,
        canvas_width_px,
        max_height_px,
        scale
    );
    let content_padding = 15u32;
    let header_content_width = canvas_width_px.saturating_sub(padding.saturating_mul(2));
    let content_width = canvas_width_px.saturating_sub(content_padding.saturating_mul(2));
    let header_x = padding;
    let header_y = padding;
    let header_text_x = header_x + avatar_size + header_gap;
    let header_text_width = header_content_width.saturating_sub(avatar_size + header_gap);

    let title_text = if header.is_anonymous {
        "匿名".to_string()
    } else {
        header.sender_name.clone().unwrap_or_else(|| {
            if header.user_id == "unknown" {
                "OQQWall".to_string()
            } else {
                format!("User {}", header.user_id)
            }
        })
    };
    let title_text = truncate_text(
        &title_text,
        header_text_width,
        title_size,
        font_weight_title,
        &mut text_measurer,
        &emoji_cache,
    );
    let meta_text = if header.is_anonymous {
        String::new()
    } else {
        let meta_source = if header.user_id == "unknown" {
            header.post_id_hex.clone()
        } else {
            header.user_id.clone()
        };
        truncate_text(
            &format!("QQ {}", meta_source),
            header_text_width,
            meta_size,
            font_weight_body,
            &mut text_measurer,
            &emoji_cache,
        )
    };
    debug_log!(
        "render header: title={} meta={} header_text_width={}",
        title_text,
        meta_text,
        header_text_width
    );
    let title_y = header_y + title_size;
    let meta_y = title_y + meta_size + 4;
    let header_height = avatar_size.max(meta_y + meta_size - header_y);

    let mut cursor_y = header_y + header_height + spacing_xxl;
    let mut blocks = Vec::new();

    for (block_idx, block) in draft.blocks.iter().enumerate() {
        let layout = match block {
            DraftBlock::Paragraph { text } => {
                let max_text_w = content_width
                    .saturating_sub(bubble_pad_left + bubble_pad_right)
                    .max(1);
                let lines = wrap_inline_text(
                    text,
                    max_text_w,
                    font_size,
                    face_size,
                    font_weight_body,
                    &mut text_measurer,
                    &emoji_cache,
                );
                let mut max_line_w = 0u32;
                for line in &lines {
                    max_line_w = max_line_w.max(line.width);
                }
                let bubble_w = (max_line_w + bubble_pad_left + bubble_pad_right)
                    .saturating_sub(TOP_LEVEL_TEXT_BUBBLE_WIDTH_SHRINK_PX)
                    .min(content_width)
                    .max(1);
                let height = bubble_pad_top + bubble_pad_bottom + line_height * lines.len() as u32;
                BlockLayout {
                    x: content_padding,
                    y: cursor_y,
                    width: bubble_w,
                    height,
                    kind: BlockKind::Text { lines },
                }
            }
            DraftBlock::Attachment {
                kind,
                name,
                reference: _,
                size_bytes,
            } => {
                let image = image_sources
                    .block_images
                    .get(block_idx)
                    .and_then(|value| value.as_ref());
                let label_href = image_sources
                    .block_labels
                    .get(block_idx)
                    .and_then(|value| value.clone());
                match *kind {
                    oqqwall_rust_core::MediaKind::Image | oqqwall_rust_core::MediaKind::Sticker => {
                        let (fallback_width, fallback_height) =
                            image_preview_fallback_size(content_width);
                        let (width, height) = image_like_preview_size(
                            image,
                            content_width,
                            fallback_width,
                            fallback_height,
                        );
                        BlockLayout {
                            x: content_padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::Image {
                                image: image.cloned(),
                            },
                        }
                    }
                    oqqwall_rust_core::MediaKind::Video => {
                        let (fallback_width, fallback_height) =
                            image_preview_fallback_size(content_width);
                        let (width, height) = image_like_preview_size(
                            image,
                            content_width,
                            fallback_width,
                            fallback_height,
                        );
                        BlockLayout {
                            x: content_padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::VideoPreview {
                                image: image.cloned(),
                            },
                        }
                    }
                    oqqwall_rust_core::MediaKind::File => {
                        let file_name_size = font_size.saturating_sub(2).max(meta_size);
                        let max_width = content_width.min(320).max(1);
                        let text_max_w = max_width
                            .saturating_sub(file_padding * 2 + file_icon_size + file_icon_gap)
                            .max(1);
                        let filename = label_href
                            .as_deref()
                            .and_then(extract_filename)
                            .or_else(|| name.clone())
                            .unwrap_or_else(|| "Unknown file".to_string());
                        let name_lines = wrap_text(
                            &filename,
                            text_max_w,
                            file_name_size,
                            font_weight_body,
                            &mut text_measurer,
                            &emoji_cache,
                        );
                        let meta_line =
                            size_bytes.as_ref().and_then(|size| format_size_line(*size));
                        let width = file_card_width(
                            max_width,
                            &name_lines,
                            meta_line.as_deref(),
                            file_padding,
                            file_icon_size,
                            file_icon_gap,
                            file_name_size,
                            11,
                            font_weight_body,
                            &mut text_measurer,
                            &emoji_cache,
                        );
                        let name_height = name_lines.len() as u32 * file_line_height;
                        let meta_height = if meta_line.is_some() {
                            file_meta_height + file_meta_gap
                        } else {
                            0
                        };
                        let text_height = name_height + meta_height;
                        let content_height = file_icon_size.max(text_height);
                        let height = content_height + file_padding * 2;
                        BlockLayout {
                            x: content_padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::FileCard {
                                name_lines,
                                meta_line,
                                icon_path: file_icon_path(&filename),
                                icon_text: file_icon_text(&filename),
                            },
                        }
                    }
                    _ => {
                        let height = 90u32;
                        let width = content_width.min(320).max(1);
                        let text_max_w = width
                            .saturating_sub(card_padding * 2 + card_icon_size + card_icon_gap)
                            .max(1);
                        let label = media_label(*kind);
                        let mut lines = vec![label.to_string()];
                        if let Some(detail) = label_href.as_deref().and_then(extract_filename) {
                            let detail_line = truncate_text(
                                &detail,
                                text_max_w,
                                font_size,
                                font_weight_body,
                                &mut text_measurer,
                                &emoji_cache,
                            );
                            if !detail_line.is_empty() {
                                lines.push(detail_line);
                            }
                        }
                        BlockLayout {
                            x: content_padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::MediaCard {
                                lines,
                                icon_text: media_icon_text(*kind),
                                media_kind: *kind,
                            },
                        }
                    }
                }
            }
            DraftBlock::Reply { preview } => {
                let width = content_width.max(1);
                let wrap_width = width.min(320).max(1);
                let reply_accent_width = 3u32;
                let reply_inner_pad_x = card_padding;
                let reply_inner_pad_y = 6u32;
                let reply_meta_size = font_size.saturating_sub(2).max(meta_size);
                let reply_body_size = font_size;
                let reply_meta_line_height = file_line_height;
                let reply_body_line_height = line_height;
                let reply_outer_extra_height = 6u32;
                let text_max_w = wrap_width
                    .saturating_sub(
                        bubble_pad_left
                            + bubble_pad_right
                            + reply_accent_width
                            + reply_inner_pad_x * 2,
                    )
                    .saturating_add(REPLY_WRAP_WIDTH_COMPENSATION_PX)
                    .max(1);
                let meta = reply_meta_text(preview);
                let mut meta_lines = wrap_text(
                    &meta,
                    text_max_w,
                    reply_meta_size,
                    font_weight_body,
                    &mut text_measurer,
                    &emoji_cache,
                );
                meta_lines = limit_lines(
                    meta_lines,
                    1,
                    text_max_w,
                    reply_meta_size,
                    font_weight_body,
                    &mut text_measurer,
                    &emoji_cache,
                );
                let body_lines = wrap_text(
                    &preview.body,
                    text_max_w,
                    reply_body_size,
                    font_weight_body,
                    &mut text_measurer,
                    &emoji_cache,
                );
                let body_gap = if body_lines.is_empty() {
                    0
                } else {
                    file_meta_gap
                };
                let inner_height = reply_inner_pad_y * 2
                    + meta_lines.len() as u32 * reply_meta_line_height
                    + body_gap
                    + body_lines.len() as u32 * reply_body_line_height;
                let height =
                    bubble_pad_top + bubble_pad_bottom + inner_height + reply_outer_extra_height;
                BlockLayout {
                    x: content_padding,
                    y: cursor_y,
                    width,
                    height,
                    kind: BlockKind::Reply {
                        meta_lines,
                        body_lines,
                    },
                }
            }
            DraftBlock::Poke => {
                let image = resolve_source_to_image("res/poke.png");
                let (width, height) = image
                    .as_ref()
                    .and_then(|img| img.width.zip(img.height))
                    .map(|(orig_w, orig_h)| {
                        let max_width = (content_width / 2).max(48);
                        let max_height = 120u32;
                        let scale_w = max_width as f32 / orig_w.max(1) as f32;
                        let scale_h = max_height as f32 / orig_h.max(1) as f32;
                        let scale = scale_w.min(scale_h).min(1.0);
                        (
                            (orig_w as f32 * scale).round().max(1.0) as u32,
                            (orig_h as f32 * scale).round().max(1.0) as u32,
                        )
                    })
                    .unwrap_or((64, 64));
                BlockLayout {
                    x: content_padding,
                    y: cursor_y,
                    width,
                    height,
                    kind: BlockKind::Poke { image },
                }
            }
            DraftBlock::JsonCard { raw } => {
                let view = parse_json_card_view(raw);
                layout_json_card_block(
                    content_padding,
                    cursor_y,
                    content_width,
                    view.clone(),
                    json_card_resolved_images(&view, block_idx, image_sources),
                    font_size,
                    font_weight_title,
                    font_weight_body,
                    meta_size,
                    card_padding,
                    card_line_height,
                    file_meta_height,
                    file_meta_gap,
                    &mut text_measurer,
                    &emoji_cache,
                )
            }
            DraftBlock::Forward { items } => {
                let width = content_width.min(340).max(1);
                let forward_child_indent = 17u32;
                let child_x = content_padding + forward_child_indent;
                let child_start_y = cursor_y + meta_size + file_meta_gap + forward_line_gap + 6;
                let child_width = width.saturating_sub(forward_child_indent).max(1);
                let (layout_items, bottom) = layout_forward_items(
                    items,
                    child_x,
                    child_start_y,
                    child_width,
                    font_size,
                    line_height,
                    face_size,
                    font_weight_title,
                    font_weight_body,
                    meta_size,
                    card_padding,
                    card_line_height,
                    card_icon_size,
                    card_icon_gap,
                    file_padding,
                    file_line_height,
                    file_meta_height,
                    file_meta_gap,
                    file_icon_size,
                    file_icon_gap,
                    bubble_pad_left,
                    bubble_pad_right,
                    bubble_pad_top,
                    bubble_pad_bottom,
                    spacing_lg,
                    &mut text_measurer,
                    &emoji_cache,
                );
                let height = bottom.saturating_sub(cursor_y).saturating_add(card_padding);
                BlockLayout {
                    x: content_padding,
                    y: cursor_y,
                    width,
                    height,
                    kind: BlockKind::Forward {
                        items: layout_items,
                    },
                }
            }
        };
        match &layout.kind {
            BlockKind::Text { lines: _lines } => {
                debug_log!(
                    "layout block: idx={} kind=text width={} height={} lines={}",
                    block_idx,
                    layout.width,
                    layout.height,
                    _lines.len()
                );
            }
            BlockKind::Image { image } => {
                let _size = image.as_ref().and_then(|img| img.width.zip(img.height));
                debug_log!(
                    "layout block: idx={} kind=image width={} height={} image_size={:?}",
                    block_idx,
                    layout.width,
                    layout.height,
                    _size
                );
            }
            BlockKind::VideoPreview { image } => {
                let _size = image.as_ref().and_then(|img| img.width.zip(img.height));
                debug_log!(
                    "layout block: idx={} kind=video width={} height={} image_size={:?}",
                    block_idx,
                    layout.width,
                    layout.height,
                    _size
                );
            }
            BlockKind::MediaCard {
                lines: _lines,
                media_kind: _media_kind,
                ..
            } => {
                debug_log!(
                    "layout block: idx={} kind=media width={} height={} lines={} media_kind={:?}",
                    block_idx,
                    layout.width,
                    layout.height,
                    _lines.len(),
                    _media_kind
                );
            }
            BlockKind::FileCard {
                name_lines: _name_lines,
                ..
            } => {
                debug_log!(
                    "layout block: idx={} kind=file width={} height={} name_lines={}",
                    block_idx,
                    layout.width,
                    layout.height,
                    _name_lines.len()
                );
            }
            BlockKind::Reply { body_lines, .. } => {
                debug_log!(
                    "layout block: idx={} kind=reply width={} height={} body_lines={}",
                    block_idx,
                    layout.width,
                    layout.height,
                    body_lines.len()
                );
            }
            BlockKind::Poke { image } => {
                let _size = image.as_ref().and_then(|img| img.width.zip(img.height));
                debug_log!(
                    "layout block: idx={} kind=poke width={} height={} image_size={:?}",
                    block_idx,
                    layout.width,
                    layout.height,
                    _size
                );
            }
            BlockKind::JsonCard { view, .. } => {
                debug_log!(
                    "layout block: idx={} kind=json_card view={} width={} height={}",
                    block_idx,
                    view.view_kind,
                    layout.width,
                    layout.height
                );
            }
            BlockKind::Forward { items } => {
                debug_log!(
                    "layout block: idx={} kind=forward width={} height={} items={}",
                    block_idx,
                    layout.width,
                    layout.height,
                    items.len()
                );
            }
        }

        let layout_height = layout.height;
        blocks.push(layout);
        cursor_y = cursor_y
            .saturating_add(layout_height)
            .saturating_add(spacing_lg);
    }

    if !blocks.is_empty() {
        cursor_y = cursor_y.saturating_sub(spacing_lg);
    }
    assign_legacy_json_card_qr_urls(&mut blocks);

    let canvas_bottom = if blocks.is_empty() {
        header_y + header_height + spacing_xxl
    } else {
        cursor_y
    };
    let canvas_height = canvas_bottom.saturating_add(padding);
    let full_background_height = canvas_height.max(canvas_width_px).max(1);
    let max_page_height = max_height_px.max(1);
    let min_page_height = canvas_width_px.min(max_page_height).max(1);
    let page_ranges = paginate_render_pages(
        &blocks,
        full_background_height,
        max_page_height,
        min_page_height,
        padding,
        spacing_lg,
    );
    debug_log!(
        "render canvas: full_background_height={} pages={} max_page_height={}",
        full_background_height,
        page_ranges.len(),
        max_page_height
    );

    let color_bg = color_from_hex(0xF2F2F2);
    let color_white = Color4f::new(1.0, 1.0, 1.0, 1.0);
    let color_border = color_from_hex(0xE0E0E0);
    let color_text = Color4f::new(0.0, 0.0, 0.0, 1.0);
    let color_meta = color_from_hex(0x666666);
    let color_muted = color_from_hex(0x888888);

    let mut pages = Vec::with_capacity(page_ranges.len());
    for (page_index, (page_y, background_height)) in page_ranges.iter().copied().enumerate() {
        let output_width = canvas_width_px.saturating_mul(scale);
        let output_height = background_height.saturating_mul(scale);
        debug_log!(
            "render page: index={} y={} height={} output={}x{}",
            page_index,
            page_y,
            background_height,
            output_width,
            output_height
        );

        let mut surface =
            skia_safe::surfaces::raster_n32_premul((output_width as i32, output_height as i32))
                .ok_or_else(|| "surface alloc failed".to_string())?;
        let canvas = surface.canvas();
        canvas.scale((scale as f32, scale as f32));

        let mut bg_paint = Paint::default();
        bg_paint.set_color4f(color_bg, None);
        bg_paint.set_anti_alias(true);
        canvas.draw_rect(
            Rect::from_xywh(0.0, 0.0, canvas_width_px as f32, background_height as f32),
            &bg_paint,
        );
        canvas.save();
        canvas.translate((0.0, -(page_y as f32)));

        let avatar_rect = Rect::from_xywh(
            header_x as f32,
            header_y as f32,
            avatar_size as f32,
            avatar_size as f32,
        );
        let avatar_rr = RRect::new_rect_xy(
            avatar_rect,
            avatar_size as f32 / 2.0,
            avatar_size as f32 / 2.0,
        );
        draw_shadowed_rrect(canvas, avatar_rr, 4.0, 0.30);
        let mut avatar_bg = Paint::default();
        avatar_bg.set_color4f(color_white, None);
        avatar_bg.set_anti_alias(true);
        canvas.draw_rrect(avatar_rr, &avatar_bg);
        let mut avatar_border = Paint::default();
        avatar_border.set_color4f(color_border, None);
        avatar_border.set_style(skia_safe::paint::Style::Stroke);
        avatar_border.set_stroke_width(1.0);
        avatar_border.set_anti_alias(true);
        canvas.draw_rrect(avatar_rr, &avatar_border);

        if let Some(avatar) = image_sources.avatar.as_ref().filter(|img| img.has_bytes()) {
            if let Some(image) = decode_image(avatar) {
                draw_image_cover_rounded(canvas, &image, avatar_rect, avatar_size as f32 / 2.0);
            }
        }

        let title_baseline = if header.is_anonymous {
            let center_y = header_y as f32 + avatar_size as f32 / 2.0;
            if let Some((_, metrics)) = build_line_paragraph(
                &font_collection,
                &title_text,
                title_size,
                font_weight_title,
                color_text,
            ) {
                center_baseline(center_y, &metrics)
            } else {
                title_y as f32
            }
        } else {
            title_y as f32
        };

        draw_text_line(
            canvas,
            &font_collection,
            &mut emoji_cache,
            &title_text,
            header_text_x as f32,
            title_baseline,
            title_size,
            font_weight_title,
            color_text,
        );
        draw_text_line(
            canvas,
            &font_collection,
            &mut emoji_cache,
            &meta_text,
            header_text_x as f32,
            meta_y as f32,
            meta_size,
            font_weight_body,
            color_meta,
        );

        let mut face_cache: HashMap<String, ResolvedImage> = HashMap::new();
        let mut face_image_cache: HashMap<String, Option<Image>> = HashMap::new();
        let mut file_icon_cache: HashMap<&'static str, Option<Image>> = HashMap::new();

        let page_bottom = page_y.saturating_add(background_height);
        for block in blocks.clone() {
            let block_bottom = block.y.saturating_add(block.height);
            if block_bottom <= page_y || block.y >= page_bottom {
                continue;
            }
            match block.kind {
                BlockKind::Text { lines } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                    draw_shadowed_rrect(canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);

                    let mut bubble_bg = Paint::default();
                    bubble_bg.set_color4f(color_white, None);
                    bubble_bg.set_anti_alias(true);
                    canvas.draw_rrect(rr, &bubble_bg);

                    let line_x = block.x + bubble_pad_left;
                    let line_y = block.y + bubble_pad_top + font_size;
                    for (idx, line) in lines.iter().enumerate() {
                        let baseline_y = line_y + line_height.saturating_mul(idx as u32);
                        let mut cursor_x = line_x;
                        for run in &line.runs {
                            match run {
                                InlineRun::Text(text) => {
                                    if !text.is_empty() {
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            text,
                                            cursor_x as f32,
                                            baseline_y as f32,
                                            font_size,
                                            font_weight_body,
                                            color_text,
                                        );
                                        cursor_x =
                                            cursor_x.saturating_add(measure_inline_text_width(
                                                text,
                                                font_size,
                                                font_weight_body,
                                                &mut text_measurer,
                                                &emoji_cache,
                                            ));
                                    }
                                }
                                InlineRun::Face { id } => {
                                    if let Some(face) = resolve_face_image(id, &mut face_cache) {
                                        let line_top = baseline_y.saturating_sub(font_size);
                                        let face_y = line_top.saturating_add(
                                            line_height.saturating_sub(face_size) / 2,
                                        );
                                        let face_x = cursor_x;
                                        let face_image = face_image_cache
                                            .entry(id.clone())
                                            .or_insert_with(|| decode_image(&face));
                                        if let Some(image) = face_image.as_ref() {
                                            draw_image_cover_rounded(
                                                canvas,
                                                image,
                                                Rect::from_xywh(
                                                    face_x as f32,
                                                    face_y as f32,
                                                    face_size as f32,
                                                    face_size as f32,
                                                ),
                                                3.0,
                                            );
                                        } else {
                                            let fallback = format!("[face:{}]", id);
                                            draw_text_line(
                                                canvas,
                                                &font_collection,
                                                &mut emoji_cache,
                                                &fallback,
                                                cursor_x as f32,
                                                baseline_y as f32,
                                                font_size,
                                                font_weight_body,
                                                color_text,
                                            );
                                        }
                                        cursor_x = cursor_x.saturating_add(face_size);
                                    } else {
                                        let fallback = format!("[face:{}]", id);
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            &fallback,
                                            cursor_x as f32,
                                            baseline_y as f32,
                                            font_size,
                                            font_weight_body,
                                            color_text,
                                        );
                                        cursor_x =
                                            cursor_x.saturating_add(measure_inline_text_width(
                                                &fallback,
                                                font_size,
                                                font_weight_body,
                                                &mut text_measurer,
                                                &emoji_cache,
                                            ));
                                    }
                                }
                                InlineRun::Emoji { glyph_id } => {
                                    let line_top = baseline_y.saturating_sub(font_size);
                                    let emoji_y = line_top
                                        .saturating_add(line_height.saturating_sub(face_size) / 2);
                                    let emoji_x = cursor_x;
                                    if let Some(image) = emoji_cache.resolve_image_by_gid(*glyph_id)
                                    {
                                        draw_image_stretch(
                                            canvas,
                                            &image,
                                            Rect::from_xywh(
                                                emoji_x as f32,
                                                emoji_y as f32,
                                                face_size as f32,
                                                face_size as f32,
                                            ),
                                        );
                                    }
                                    cursor_x = cursor_x.saturating_add(face_size);
                                }
                            }
                        }
                    }
                }
                BlockKind::Image { image } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    draw_image_preview_frame(
                        canvas,
                        image.as_ref(),
                        rect,
                        radius_lg as f32,
                        color_white,
                        color_border,
                    );
                    if image.as_ref().filter(|img| img.has_bytes()).is_none() {
                        let text_x = block.x + bubble_pad_left;
                        let text_y = block.y + bubble_pad_top + font_size;
                        draw_text_line(
                            canvas,
                            &font_collection,
                            &mut emoji_cache,
                            "Image",
                            text_x as f32,
                            text_y as f32,
                            meta_size,
                            font_weight_body,
                            color_muted,
                        );
                    }
                }
                BlockKind::VideoPreview { image } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    let bg = if image.as_ref().filter(|img| img.has_bytes()).is_some() {
                        color_white
                    } else {
                        color_from_hex(0x262626)
                    };
                    draw_image_preview_frame(
                        canvas,
                        image.as_ref(),
                        rect,
                        radius_lg as f32,
                        bg,
                        color_border,
                    );
                    draw_video_play_triangle(canvas, rect);
                }
                BlockKind::MediaCard {
                    lines,
                    icon_text,
                    media_kind,
                } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                    if matches!(media_kind, oqqwall_rust_core::MediaKind::Video) {
                        draw_image_preview_frame(
                            canvas,
                            None,
                            rect,
                            radius_lg as f32,
                            color_from_hex(0x262626),
                            color_border,
                        );
                        draw_video_play_triangle(canvas, rect);
                    } else {
                        draw_shadowed_rrect(canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);

                        let mut card_bg = Paint::default();
                        card_bg.set_color4f(color_white, None);
                        card_bg.set_anti_alias(true);
                        canvas.draw_rrect(rr, &card_bg);
                        let mut card_border = Paint::default();
                        card_border.set_color4f(color_border, None);
                        card_border.set_style(skia_safe::paint::Style::Stroke);
                        card_border.set_stroke_width(1.0);
                        card_border.set_anti_alias(true);
                        canvas.draw_rrect(rr, &card_border);

                        let icon_x = block.x + card_padding;
                        let icon_y = block.y + (block.height - card_icon_size) / 2;
                        let icon_rect = Rect::from_xywh(
                            icon_x as f32,
                            icon_y as f32,
                            card_icon_size as f32,
                            card_icon_size as f32,
                        );
                        let icon_rr = RRect::new_rect_xy(icon_rect, 4.0, 4.0);
                        let mut icon_bg = Paint::default();
                        icon_bg.set_color4f(color_bg, None);
                        icon_bg.set_anti_alias(true);
                        canvas.draw_rrect(icon_rr, &icon_bg);
                        let mut icon_border = Paint::default();
                        icon_border.set_color4f(color_border, None);
                        icon_border.set_style(skia_safe::paint::Style::Stroke);
                        icon_border.set_stroke_width(1.0);
                        icon_border.set_anti_alias(true);
                        canvas.draw_rrect(icon_rr, &icon_border);

                        let icon_center_x = icon_x + card_icon_size / 2;
                        let icon_center_y = icon_y + card_icon_size / 2;
                        if let Some((paragraph, metrics)) = build_line_paragraph(
                            &font_collection,
                            &icon_text,
                            11,
                            font_weight_body,
                            color_meta,
                        ) {
                            let icon_baseline = center_baseline(icon_center_y as f32, &metrics);
                            let icon_x = icon_center_x as f32 - metrics.width * 0.5;
                            let top_y = icon_baseline - metrics.baseline;
                            paragraph.paint(canvas, (icon_x, top_y));
                        }

                        let text_x = block.x + card_padding + card_icon_size + card_icon_gap;
                        let text_y = block.y + card_padding + font_size;
                        for (idx, line) in lines.iter().enumerate() {
                            let baseline = text_y + card_line_height.saturating_mul(idx as u32);
                            draw_text_line(
                                canvas,
                                &font_collection,
                                &mut emoji_cache,
                                line,
                                text_x as f32,
                                baseline as f32,
                                font_size,
                                font_weight_body,
                                color_text,
                            );
                        }
                    }
                }
                BlockKind::FileCard {
                    name_lines,
                    meta_line,
                    icon_path,
                    icon_text,
                } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                    draw_shadowed_rrect(canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);

                    let mut card_bg = Paint::default();
                    card_bg.set_color4f(color_white, None);
                    card_bg.set_anti_alias(true);
                    canvas.draw_rrect(rr, &card_bg);

                    let icon_x =
                        block.x + block.width.saturating_sub(file_padding + file_icon_size);
                    let icon_y = block.y + (block.height - file_icon_size) / 2;
                    let mut drew_icon = false;
                    if let Some(path) = icon_path {
                        let icon_image = file_icon_cache.entry(path).or_insert_with(|| {
                            resolved_image_from_path(path).and_then(|img| decode_image(&img))
                        });
                        if let Some(image) = icon_image.as_ref() {
                            let icon_dst = Rect::from_xywh(
                                icon_x as f32,
                                icon_y as f32,
                                file_icon_size as f32,
                                file_icon_size as f32,
                            );
                            draw_image_stretch(canvas, image, icon_dst);
                            drew_icon = true;
                        }
                    }
                    if !drew_icon {
                        let icon_center_x = icon_x + file_icon_size / 2;
                        let icon_center_y = icon_y + file_icon_size / 2;
                        if let Some((paragraph, metrics)) = build_line_paragraph(
                            &font_collection,
                            &icon_text,
                            11,
                            font_weight_body,
                            color_meta,
                        ) {
                            let icon_baseline = center_baseline(icon_center_y as f32, &metrics);
                            let icon_x = icon_center_x as f32 - metrics.width * 0.5;
                            let top_y = icon_baseline - metrics.baseline;
                            paragraph.paint(canvas, (icon_x, top_y));
                        }
                    }

                    let text_x = block.x + file_padding;
                    let name_height = name_lines.len() as u32 * file_line_height;
                    let meta_height = if meta_line.is_some() {
                        file_meta_height + file_meta_gap
                    } else {
                        0
                    };
                    let text_height = name_height + meta_height;
                    let content_height = file_icon_size.max(text_height);
                    let text_top = block.y + file_padding + (content_height - text_height) / 2 - 4;
                    let file_name_size = font_size.saturating_sub(2).max(meta_size);
                    let text_baseline = text_top + file_name_size;

                    for (idx, line) in name_lines.iter().enumerate() {
                        let baseline = text_baseline + file_line_height.saturating_mul(idx as u32);
                        draw_text_line(
                            canvas,
                            &font_collection,
                            &mut emoji_cache,
                            line,
                            text_x as f32,
                            baseline as f32,
                            file_name_size,
                            font_weight_body,
                            color_text,
                        );
                    }
                    if let Some(meta) = meta_line {
                        let meta_font_size = 11u32;
                        let meta_y = text_top + name_height + file_meta_gap + meta_font_size + 6;
                        draw_text_line(
                            canvas,
                            &font_collection,
                            &mut emoji_cache,
                            &meta,
                            text_x as f32,
                            meta_y as f32,
                            meta_font_size,
                            font_weight_body,
                            color_muted,
                        );
                    }
                }
                BlockKind::Reply {
                    meta_lines,
                    body_lines,
                } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                    draw_shadowed_rrect(canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);
                    let mut bubble_bg = Paint::default();
                    bubble_bg.set_color4f(color_white, None);
                    bubble_bg.set_anti_alias(true);
                    canvas.draw_rrect(rr, &bubble_bg);

                    let reply_accent_width = 3u32;
                    let reply_inner_pad_x = card_padding;
                    let reply_inner_pad_y = 6u32;
                    let reply_meta_size = font_size.saturating_sub(2).max(meta_size);
                    let reply_body_size = font_size;
                    let reply_body_draw_size = font_size;
                    let reply_meta_line_height = file_line_height;
                    let reply_body_line_height = line_height;
                    let inner_x = block.x + bubble_pad_left;
                    let inner_y = block
                        .y
                        .saturating_add(bubble_pad_top)
                        .saturating_sub(REPLY_INNER_Y_RAISE_PX);
                    let inner_width = block
                        .width
                        .saturating_sub(bubble_pad_left + bubble_pad_right);
                    let inner_height = block
                        .height
                        .saturating_sub(bubble_pad_top + bubble_pad_bottom);
                    let inner_rect = Rect::from_xywh(
                        inner_x as f32,
                        inner_y as f32,
                        inner_width as f32,
                        inner_height as f32,
                    );
                    let inner_rr = RRect::new_rect_xy(inner_rect, 4.0, 4.0);
                    let mut bg = Paint::default();
                    bg.set_color4f(color_from_hex(0xFAFAFA), None);
                    bg.set_anti_alias(true);
                    canvas.draw_rrect(inner_rr, &bg);
                    let mut accent = Paint::default();
                    accent.set_color4f(color_border, None);
                    accent.set_anti_alias(true);
                    canvas.draw_rect(
                        Rect::from_xywh(
                            inner_x as f32,
                            inner_y as f32,
                            reply_accent_width as f32,
                            inner_height as f32,
                        ),
                        &accent,
                    );
                    let text_x = inner_x + reply_accent_width + reply_inner_pad_x;
                    let mut baseline = inner_y + reply_inner_pad_y + reply_meta_size;
                    for line in &meta_lines {
                        draw_text_line(
                            canvas,
                            &font_collection,
                            &mut emoji_cache,
                            line,
                            text_x as f32,
                            baseline as f32,
                            reply_meta_size,
                            font_weight_body,
                            color_meta,
                        );
                        baseline = baseline.saturating_add(reply_meta_line_height);
                    }
                    baseline = inner_y
                        + reply_inner_pad_y
                        + meta_lines.len() as u32 * reply_meta_line_height
                        + file_meta_gap
                        + reply_body_size
                        + REPLY_BODY_BASELINE_OFFSET_PX;
                    for line in &body_lines {
                        draw_text_line(
                            canvas,
                            &font_collection,
                            &mut emoji_cache,
                            line,
                            text_x as f32,
                            baseline as f32,
                            reply_body_draw_size,
                            font_weight_body,
                            color_from_hex(0x333333),
                        );
                        baseline = baseline.saturating_add(reply_body_line_height);
                    }
                }
                BlockKind::Poke { image } => {
                    let rect = Rect::from_xywh(
                        block.x as f32,
                        block.y as f32,
                        block.width as f32,
                        block.height as f32,
                    );
                    if let Some(img) = image.as_ref().filter(|img| img.has_bytes()) {
                        if let Some(decoded) = decode_image(img) {
                            draw_image_stretch(canvas, &decoded, rect);
                        }
                    } else {
                        draw_text_line(
                            canvas,
                            &font_collection,
                            &mut emoji_cache,
                            "[戳一戳]",
                            block.x as f32,
                            (block.y + font_size) as f32,
                            font_size,
                            font_weight_body,
                            color_text,
                        );
                    }
                }
                BlockKind::JsonCard {
                    qr_url,
                    view,
                    title_lines,
                    desc_lines,
                    footer_line,
                    media,
                    tag_icon,
                    brand_icon,
                } => {
                    draw_json_card_frame(
                        canvas,
                        &font_collection,
                        &mut emoji_cache,
                        BlockFrame {
                            x: block.x,
                            y: block.y,
                            width: block.width,
                            height: block.height,
                        },
                        radius_lg,
                        card_padding,
                        font_size,
                        meta_size,
                        card_title_size,
                        card_line_height,
                        file_meta_height,
                        file_meta_gap,
                        font_weight_title,
                        font_weight_body,
                        color_white,
                        color_text,
                        color_meta,
                        color_muted,
                        qr_url.as_deref(),
                        &view,
                        &title_lines,
                        &desc_lines,
                        footer_line.as_deref(),
                        media.as_ref(),
                        tag_icon.as_ref(),
                        brand_icon.as_ref(),
                    );
                }
                BlockKind::Forward { items } => {
                    let title = "合并转发聊天记录";
                    let title_y = block.y.saturating_add(meta_size).saturating_sub(1);
                    draw_text_line(
                        canvas,
                        &font_collection,
                        &mut emoji_cache,
                        title,
                        block.x as f32,
                        title_y as f32,
                        meta_size,
                        font_weight_body,
                        color_meta,
                    );
                    let left = block.x;
                    let first_child_y = items
                        .iter()
                        .flat_map(|item| item.blocks.iter().map(|child| child.y))
                        .min()
                        .unwrap_or(block.y);
                    let top = first_child_y.saturating_sub(forward_line_gap);
                    let bottom = block.y.saturating_add(block.height).saturating_add(2);
                    let mut accent = Paint::default();
                    accent.set_color4f(color_from_hex(0x71A1CC), None);
                    accent.set_anti_alias(true);
                    canvas.draw_rect(
                        Rect::from_xywh(
                            left as f32,
                            top as f32,
                            3.0,
                            bottom.saturating_sub(top) as f32,
                        ),
                        &accent,
                    );
                    for item in items {
                        for child in item.blocks {
                            match child.kind {
                                BlockKind::Text { lines } => {
                                    let rect = Rect::from_xywh(
                                        child.x as f32,
                                        child.y as f32,
                                        child.width as f32,
                                        child.height as f32,
                                    );
                                    let rr = RRect::new_rect_xy(
                                        rect,
                                        radius_lg as f32,
                                        radius_lg as f32,
                                    );
                                    draw_shadowed_rrect(
                                        canvas,
                                        rr,
                                        SHADOW_SM_BLUR_PX,
                                        SHADOW_SM_ALPHA,
                                    );

                                    let mut bubble_bg = Paint::default();
                                    bubble_bg.set_color4f(color_white, None);
                                    bubble_bg.set_anti_alias(true);
                                    canvas.draw_rrect(rr, &bubble_bg);

                                    let line_x = child.x + bubble_pad_left;
                                    let line_y = child.y + bubble_pad_top + font_size;
                                    for (idx, line) in lines.iter().enumerate() {
                                        let baseline_y =
                                            line_y + line_height.saturating_mul(idx as u32);
                                        let mut cursor_x = line_x;
                                        for run in &line.runs {
                                            match run {
                                                InlineRun::Text(text) => {
                                                    if !text.is_empty() {
                                                        draw_text_line(
                                                            canvas,
                                                            &font_collection,
                                                            &mut emoji_cache,
                                                            text,
                                                            cursor_x as f32,
                                                            baseline_y as f32,
                                                            font_size,
                                                            font_weight_body,
                                                            color_text,
                                                        );
                                                        cursor_x = cursor_x.saturating_add(
                                                            measure_inline_text_width(
                                                                text,
                                                                font_size,
                                                                font_weight_body,
                                                                &mut text_measurer,
                                                                &emoji_cache,
                                                            ),
                                                        );
                                                    }
                                                }
                                                InlineRun::Face { id } => {
                                                    if let Some(face) =
                                                        resolve_face_image(id, &mut face_cache)
                                                    {
                                                        let line_top =
                                                            baseline_y.saturating_sub(font_size);
                                                        let face_y = line_top.saturating_add(
                                                            line_height.saturating_sub(face_size)
                                                                / 2,
                                                        );
                                                        let face_x = cursor_x;
                                                        let face_image = face_image_cache
                                                            .entry(id.clone())
                                                            .or_insert_with(|| decode_image(&face));
                                                        if let Some(image) = face_image.as_ref() {
                                                            draw_image_cover_rounded(
                                                                canvas,
                                                                image,
                                                                Rect::from_xywh(
                                                                    face_x as f32,
                                                                    face_y as f32,
                                                                    face_size as f32,
                                                                    face_size as f32,
                                                                ),
                                                                3.0,
                                                            );
                                                        }
                                                        cursor_x =
                                                            cursor_x.saturating_add(face_size);
                                                    }
                                                }
                                                InlineRun::Emoji { glyph_id } => {
                                                    let line_top =
                                                        baseline_y.saturating_sub(font_size);
                                                    let emoji_y = line_top.saturating_add(
                                                        line_height.saturating_sub(face_size) / 2,
                                                    );
                                                    if let Some(image) =
                                                        emoji_cache.resolve_image_by_gid(*glyph_id)
                                                    {
                                                        draw_image_stretch(
                                                            canvas,
                                                            &image,
                                                            Rect::from_xywh(
                                                                cursor_x as f32,
                                                                emoji_y as f32,
                                                                face_size as f32,
                                                                face_size as f32,
                                                            ),
                                                        );
                                                    }
                                                    cursor_x = cursor_x.saturating_add(face_size);
                                                }
                                            }
                                        }
                                    }
                                }
                                BlockKind::FileCard {
                                    name_lines,
                                    meta_line,
                                    icon_path,
                                    icon_text,
                                } => {
                                    let rect = Rect::from_xywh(
                                        child.x as f32,
                                        child.y as f32,
                                        child.width as f32,
                                        child.height as f32,
                                    );
                                    let rr = RRect::new_rect_xy(
                                        rect,
                                        radius_lg as f32,
                                        radius_lg as f32,
                                    );
                                    draw_shadowed_rrect(
                                        canvas,
                                        rr,
                                        SHADOW_SM_BLUR_PX,
                                        SHADOW_SM_ALPHA,
                                    );

                                    let mut card_bg = Paint::default();
                                    card_bg.set_color4f(color_white, None);
                                    card_bg.set_anti_alias(true);
                                    canvas.draw_rrect(rr, &card_bg);

                                    let icon_x = child.x
                                        + child.width.saturating_sub(file_padding + file_icon_size);
                                    let icon_y = child.y + (child.height - file_icon_size) / 2;
                                    let mut drew_icon = false;
                                    if let Some(path) = icon_path {
                                        let icon_image =
                                            file_icon_cache.entry(path).or_insert_with(|| {
                                                resolved_image_from_path(path)
                                                    .and_then(|img| decode_image(&img))
                                            });
                                        if let Some(image) = icon_image.as_ref() {
                                            draw_image_stretch(
                                                canvas,
                                                image,
                                                Rect::from_xywh(
                                                    icon_x as f32,
                                                    icon_y as f32,
                                                    file_icon_size as f32,
                                                    file_icon_size as f32,
                                                ),
                                            );
                                            drew_icon = true;
                                        }
                                    }
                                    if !drew_icon {
                                        let icon_center_x = icon_x + file_icon_size / 2;
                                        let icon_center_y = icon_y + file_icon_size / 2;
                                        if let Some((paragraph, metrics)) = build_line_paragraph(
                                            &font_collection,
                                            &icon_text,
                                            11,
                                            font_weight_body,
                                            color_meta,
                                        ) {
                                            let icon_baseline =
                                                center_baseline(icon_center_y as f32, &metrics);
                                            let icon_x = icon_center_x as f32 - metrics.width * 0.5;
                                            paragraph.paint(
                                                canvas,
                                                (icon_x, icon_baseline - metrics.baseline),
                                            );
                                        }
                                    }

                                    let text_x = child.x + file_padding;
                                    let name_height = name_lines.len() as u32 * file_line_height;
                                    let meta_height = if meta_line.is_some() {
                                        file_meta_height + file_meta_gap
                                    } else {
                                        0
                                    };
                                    let text_height = name_height + meta_height;
                                    let content_height = file_icon_size.max(text_height);
                                    let text_top =
                                        child.y + file_padding + (content_height - text_height) / 2
                                            - 4;
                                    let file_name_size = font_size.saturating_sub(2).max(meta_size);
                                    let text_baseline = text_top + file_name_size;

                                    for (idx, line) in name_lines.iter().enumerate() {
                                        let baseline = text_baseline
                                            + file_line_height.saturating_mul(idx as u32);
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            line,
                                            text_x as f32,
                                            baseline as f32,
                                            file_name_size,
                                            font_weight_body,
                                            color_text,
                                        );
                                    }
                                    if let Some(meta) = meta_line {
                                        let meta_font_size = 11u32;
                                        let meta_y = text_top
                                            + name_height
                                            + file_meta_gap
                                            + meta_font_size
                                            + 6;
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            &meta,
                                            text_x as f32,
                                            meta_y as f32,
                                            meta_font_size,
                                            font_weight_body,
                                            color_muted,
                                        );
                                    }
                                }
                                BlockKind::JsonCard {
                                    qr_url,
                                    view,
                                    title_lines,
                                    desc_lines,
                                    footer_line,
                                    media,
                                    tag_icon,
                                    brand_icon,
                                } => {
                                    draw_json_card_frame(
                                        canvas,
                                        &font_collection,
                                        &mut emoji_cache,
                                        BlockFrame {
                                            x: child.x,
                                            y: child.y,
                                            width: child.width,
                                            height: child.height,
                                        },
                                        radius_lg,
                                        card_padding,
                                        font_size,
                                        meta_size,
                                        card_title_size,
                                        card_line_height,
                                        file_meta_height,
                                        file_meta_gap,
                                        font_weight_title,
                                        font_weight_body,
                                        color_white,
                                        color_text,
                                        color_meta,
                                        color_muted,
                                        qr_url.as_deref(),
                                        &view,
                                        &title_lines,
                                        &desc_lines,
                                        footer_line.as_deref(),
                                        media.as_ref(),
                                        tag_icon.as_ref(),
                                        brand_icon.as_ref(),
                                    );
                                }
                                BlockKind::Image { image } => {
                                    let rect = Rect::from_xywh(
                                        child.x as f32,
                                        child.y as f32,
                                        child.width as f32,
                                        child.height as f32,
                                    );
                                    draw_image_preview_frame(
                                        canvas,
                                        image.as_ref(),
                                        rect,
                                        radius_lg as f32,
                                        color_white,
                                        color_border,
                                    );
                                }
                                BlockKind::VideoPreview { image } => {
                                    let rect = Rect::from_xywh(
                                        child.x as f32,
                                        child.y as f32,
                                        child.width as f32,
                                        child.height as f32,
                                    );
                                    let bg =
                                        if image.as_ref().filter(|img| img.has_bytes()).is_some() {
                                            color_white
                                        } else {
                                            color_from_hex(0x262626)
                                        };
                                    draw_image_preview_frame(
                                        canvas,
                                        image.as_ref(),
                                        rect,
                                        radius_lg as f32,
                                        bg,
                                        color_border,
                                    );
                                    draw_video_play_triangle(canvas, rect);
                                }
                                BlockKind::MediaCard { media_kind, .. } => {
                                    draw_text_line(
                                        canvas,
                                        &font_collection,
                                        &mut emoji_cache,
                                        media_label(media_kind),
                                        child.x as f32,
                                        (child.y + font_size) as f32,
                                        font_size,
                                        font_weight_body,
                                        color_text,
                                    );
                                }
                                BlockKind::Reply {
                                    meta_lines,
                                    body_lines,
                                } => {
                                    let rect = Rect::from_xywh(
                                        child.x as f32,
                                        child.y as f32,
                                        child.width as f32,
                                        child.height as f32,
                                    );
                                    let rr = RRect::new_rect_xy(
                                        rect,
                                        radius_lg as f32,
                                        radius_lg as f32,
                                    );
                                    draw_shadowed_rrect(
                                        canvas,
                                        rr,
                                        SHADOW_SM_BLUR_PX,
                                        SHADOW_SM_ALPHA,
                                    );
                                    let mut bubble_bg = Paint::default();
                                    bubble_bg.set_color4f(color_white, None);
                                    bubble_bg.set_anti_alias(true);
                                    canvas.draw_rrect(rr, &bubble_bg);

                                    let reply_accent_width = 3u32;
                                    let reply_inner_pad_x = card_padding;
                                    let reply_inner_pad_y = 6u32;
                                    let reply_meta_size =
                                        font_size.saturating_sub(2).max(meta_size);
                                    let reply_body_size = font_size;
                                    let reply_meta_line_height = file_line_height;
                                    let reply_body_line_height = line_height;
                                    let inner_x = child.x + bubble_pad_left;
                                    let inner_y = child
                                        .y
                                        .saturating_add(bubble_pad_top)
                                        .saturating_sub(REPLY_INNER_Y_RAISE_PX);
                                    let inner_width = child
                                        .width
                                        .saturating_sub(bubble_pad_left + bubble_pad_right);
                                    let inner_height = child
                                        .height
                                        .saturating_sub(bubble_pad_top + bubble_pad_bottom);
                                    let inner_rect = Rect::from_xywh(
                                        inner_x as f32,
                                        inner_y as f32,
                                        inner_width as f32,
                                        inner_height as f32,
                                    );
                                    let inner_rr = RRect::new_rect_xy(inner_rect, 4.0, 4.0);
                                    let mut bg = Paint::default();
                                    bg.set_color4f(color_from_hex(0xFAFAFA), None);
                                    bg.set_anti_alias(true);
                                    canvas.draw_rrect(inner_rr, &bg);
                                    let mut accent = Paint::default();
                                    accent.set_color4f(color_border, None);
                                    accent.set_anti_alias(true);
                                    canvas.draw_rect(
                                        Rect::from_xywh(
                                            inner_x as f32,
                                            inner_y as f32,
                                            reply_accent_width as f32,
                                            inner_height as f32,
                                        ),
                                        &accent,
                                    );
                                    let text_x = inner_x + reply_accent_width + reply_inner_pad_x;
                                    let mut baseline =
                                        inner_y + reply_inner_pad_y + reply_meta_size;
                                    for line in &meta_lines {
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            line,
                                            text_x as f32,
                                            baseline as f32,
                                            reply_meta_size,
                                            font_weight_body,
                                            color_meta,
                                        );
                                        baseline = baseline.saturating_add(reply_meta_line_height);
                                    }
                                    baseline = inner_y
                                        + reply_inner_pad_y
                                        + meta_lines.len() as u32 * reply_meta_line_height
                                        + file_meta_gap
                                        + reply_body_size
                                        + REPLY_BODY_BASELINE_OFFSET_PX;
                                    for line in &body_lines {
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            line,
                                            text_x as f32,
                                            baseline as f32,
                                            reply_body_size,
                                            font_weight_body,
                                            color_from_hex(0x333333),
                                        );
                                        baseline = baseline.saturating_add(reply_body_line_height);
                                    }
                                }
                                BlockKind::Poke { image } => {
                                    let rect = Rect::from_xywh(
                                        child.x as f32,
                                        child.y as f32,
                                        child.width as f32,
                                        child.height as f32,
                                    );
                                    if let Some(img) = image.as_ref().filter(|img| img.has_bytes())
                                    {
                                        if let Some(decoded) = decode_image(img) {
                                            draw_image_stretch(canvas, &decoded, rect);
                                        }
                                    } else {
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            "[戳一戳]",
                                            child.x as f32,
                                            (child.y + font_size) as f32,
                                            font_size,
                                            font_weight_body,
                                            color_text,
                                        );
                                    }
                                }
                                BlockKind::Forward { items } => {
                                    let frame = BlockFrame {
                                        x: child.x,
                                        y: child.y,
                                        width: child.width,
                                        height: child.height,
                                    };
                                    let mut painter = ForwardDrawContext {
                                        canvas,
                                        font_collection: &font_collection,
                                        emoji_cache: &mut emoji_cache,
                                        text_measurer: &mut text_measurer,
                                        face_cache: &mut face_cache,
                                        face_image_cache: &mut face_image_cache,
                                        file_icon_cache: &mut file_icon_cache,
                                        bubble_pad_left,
                                        bubble_pad_right,
                                        bubble_pad_top,
                                        bubble_pad_bottom,
                                        font_size,
                                        line_height,
                                        face_size,
                                        font_weight_title,
                                        font_weight_body,
                                        meta_size,
                                        card_title_size,
                                        card_padding,
                                        card_line_height,
                                        file_padding,
                                        file_line_height,
                                        file_meta_height,
                                        file_meta_gap,
                                        file_icon_size,
                                        radius_lg,
                                        forward_line_gap,
                                        color_white,
                                        color_border,
                                        color_text,
                                        color_meta,
                                        color_muted,
                                    };
                                    painter.draw_forward(frame, &items);
                                }
                            }
                        }
                    }
                }
            }
        }

        canvas.restore();

        if let Some(watermark_text) = config
            .watermark_text_by_group
            .get(&header.group_id)
            .map(|text| text.trim())
            .filter(|text| !text.is_empty())
        {
            draw_watermark_layer(
                canvas,
                &font_collection,
                &mut emoji_cache,
                header,
                canvas_width_px,
                background_height,
                watermark_text,
            );
        }

        let image = surface.image_snapshot();
        let data = image
            .encode(None, EncodedImageFormat::PNG, None)
            .ok_or_else(|| "encode png failed".to_string())?;
        pages.push(data.as_bytes().to_vec());
    }

    Ok(pages)
}

fn paginate_render_pages(
    blocks: &[BlockLayout],
    full_height: u32,
    max_page_height: u32,
    min_page_height: u32,
    padding: u32,
    spacing: u32,
) -> Vec<(u32, u32)> {
    let full_height = full_height.max(1);
    let max_page_height = max_page_height.max(1);
    let min_page_height = min_page_height.min(max_page_height).max(1);
    let mut pages = Vec::new();
    let mut start = 0u32;

    while start < full_height {
        let hard_end = start.saturating_add(max_page_height).min(full_height);
        if hard_end >= full_height {
            pages.push((start, hard_end.saturating_sub(start).max(min_page_height)));
            break;
        }

        let mut soft_end = None;
        for block in blocks {
            let block_top = block.y;
            if block_top <= start {
                continue;
            }
            let block_bottom = block.y.saturating_add(block.height).saturating_add(padding);
            if block_bottom > hard_end {
                let break_y = block_top.saturating_sub(spacing / 2);
                if break_y > start {
                    soft_end = Some(break_y);
                }
                break;
            }
            soft_end = Some(block_bottom.min(hard_end));
        }

        let candidate_end = soft_end.unwrap_or(hard_end);
        let end = if candidate_end <= start
            || candidate_end.saturating_sub(start) < min_page_height.saturating_div(2).max(1)
        {
            hard_end
        } else {
            candidate_end.min(hard_end)
        };
        let height = end.saturating_sub(start).max(min_page_height);
        pages.push((start, height.min(max_page_height)));
        start = end.max(start.saturating_add(1));
    }

    if pages.is_empty() {
        pages.push((0, min_page_height));
    }
    pages
}

pub fn render_preview_png(
    draft: &Draft,
    header: RenderPreviewHeader,
    config: &RendererRuntimeConfig,
) -> Result<Vec<u8>, String> {
    let draft = normalize_embedded_forward_draft(draft);
    let header = HeaderInfo::from(header);
    let image_sources = render_preview_image_sources(&draft);
    render_png(&draft, &header, &image_sources, config)
}

pub fn render_preview_png_pages(
    draft: &Draft,
    header: RenderPreviewHeader,
    config: &RendererRuntimeConfig,
) -> Result<Vec<Vec<u8>>, String> {
    let draft = normalize_embedded_forward_draft(draft);
    let header = HeaderInfo::from(header);
    let image_sources = render_preview_image_sources(&draft);
    render_png_pages(&draft, &header, &image_sources, config)
}

fn color_from_hex(hex: u32) -> Color4f {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;
    Color4f::new(r, g, b, 1.0)
}

#[derive(Clone, Copy)]
struct BlockFrame {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl BlockFrame {
    fn from_layout(block: &BlockLayout) -> Self {
        Self {
            x: block.x,
            y: block.y,
            width: block.width,
            height: block.height,
        }
    }

    fn rect(self) -> Rect {
        Rect::from_xywh(
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        )
    }
}

fn draw_json_card_image(canvas: &Canvas, image: Option<&ResolvedImage>, rect: Rect, radius: f32) {
    let Some(img) = image.filter(|img| img.has_bytes()) else {
        return;
    };
    let Some(decoded) = decode_image(img) else {
        return;
    };
    if radius > 0.0 {
        draw_image_cover_rounded(canvas, &decoded, rect, radius);
    } else {
        draw_image_stretch(canvas, &decoded, rect);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_json_card_tag_row(
    canvas: &Canvas,
    font_collection: &FontCollection,
    emoji_cache: &mut EmojiRenderCache,
    x: u32,
    top_y: u32,
    text: &str,
    icon: Option<&ResolvedImage>,
    font_size: u32,
    font_weight: u32,
    text_color: Color4f,
) {
    let mut text_x = x;
    if icon.is_some_and(|img| img.has_bytes()) {
        draw_json_card_image(
            canvas,
            icon,
            Rect::from_xywh(
                x as f32,
                top_y as f32,
                JSON_CARD_TAG_ICON_SIZE as f32,
                JSON_CARD_TAG_ICON_SIZE as f32,
            ),
            3.0,
        );
        text_x = text_x.saturating_add(JSON_CARD_TAG_ICON_SIZE + 4);
    }
    draw_text_line(
        canvas,
        font_collection,
        emoji_cache,
        text,
        text_x as f32,
        (top_y + font_size) as f32,
        font_size,
        font_weight,
        text_color,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_json_card_frame(
    canvas: &Canvas,
    font_collection: &FontCollection,
    emoji_cache: &mut EmojiRenderCache,
    frame: BlockFrame,
    radius_lg: u32,
    card_padding: u32,
    font_size: u32,
    meta_size: u32,
    card_title_size: u32,
    card_line_height: u32,
    file_meta_height: u32,
    file_meta_gap: u32,
    font_weight_title: u32,
    font_weight_body: u32,
    color_white: Color4f,
    color_text: Color4f,
    color_meta: Color4f,
    color_muted: Color4f,
    qr_url: Option<&str>,
    view: &JsonCardView,
    title_lines: &[String],
    desc_lines: &[String],
    footer_line: Option<&str>,
    media: Option<&ResolvedImage>,
    tag_icon: Option<&ResolvedImage>,
    brand_icon: Option<&ResolvedImage>,
) {
    let bottom_qr = json_card_uses_bottom_qr(view);
    let card_draw_height = frame.height.saturating_sub(if bottom_qr {
        JSON_CARD_BOTTOM_QR_DRAW_HEIGHT_SHRINK_PX
    } else {
        0
    });
    let rect = Rect::from_xywh(
        frame.x as f32,
        frame.y as f32,
        frame.width as f32,
        card_draw_height.max(1) as f32,
    );
    let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
    draw_shadowed_rrect(canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);
    let mut card_bg = Paint::default();
    card_bg.set_color4f(color_white, None);
    card_bg.set_anti_alias(true);
    canvas.draw_rrect(rr, &card_bg);

    let side_y = frame.y + card_padding;
    let qr_text = qr_url.or(view.jump_url.as_deref());
    if !bottom_qr {
        if let Some(url) = qr_text.filter(|_| view.jump_url.is_some()) {
            let qr_x = frame
                .x
                .saturating_add(frame.width.saturating_sub(card_padding + JSON_CARD_QR_SIZE));
            draw_qr_code(
                canvas,
                url,
                Rect::from_xywh(
                    qr_x as f32,
                    side_y as f32,
                    JSON_CARD_QR_SIZE as f32,
                    JSON_CARD_QR_SIZE as f32,
                ),
                color_white,
                color_text,
            );
        }
    }

    match view.view_kind.as_str() {
        "miniapp" => {
            let text_x = frame.x + card_padding;
            if !desc_lines.is_empty() {
                let source_baseline =
                    frame.y + card_padding + meta_size + JSON_CARD_MINIAPP_SOURCE_BASELINE_DOWN_PX;
                let mut source_text_x = text_x;
                if brand_icon.is_some_and(|img| img.has_bytes()) {
                    let icon_top = source_baseline.saturating_sub(meta_size + 1);
                    draw_json_card_image(
                        canvas,
                        brand_icon,
                        Rect::from_xywh(
                            text_x as f32,
                            icon_top as f32,
                            JSON_CARD_SOURCE_ICON_SIZE as f32,
                            JSON_CARD_SOURCE_ICON_SIZE as f32,
                        ),
                        2.0,
                    );
                    source_text_x = source_text_x.saturating_add(JSON_CARD_SOURCE_ICON_SIZE + 4);
                }
                for (idx, line) in desc_lines.iter().enumerate() {
                    let baseline =
                        source_baseline.saturating_add(file_meta_height.saturating_mul(idx as u32));
                    draw_text_line(
                        canvas,
                        font_collection,
                        emoji_cache,
                        line,
                        source_text_x as f32,
                        baseline as f32,
                        meta_size,
                        font_weight_body,
                        color_meta,
                    );
                }
            }

            let mut title_baseline = if desc_lines.is_empty() {
                frame.y + card_padding + card_title_size
            } else {
                frame.y
                    + card_padding
                    + meta_size
                    + card_line_height
                    + 4
                    + JSON_CARD_MINIAPP_TITLE_BASELINE_DOWN_PX
            };
            for line in title_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    text_x as f32,
                    title_baseline as f32,
                    card_title_size,
                    font_weight_title,
                    color_text,
                );
                title_baseline = title_baseline.saturating_add(card_line_height);
            }

            let header_height = json_card_miniapp_header_height(
                desc_lines.len(),
                title_lines.len(),
                view.jump_url.is_some(),
                file_meta_height,
                card_line_height,
            );
            let preview_h = json_card_preview_height(view, media, frame.width, card_padding);
            let mut cursor_y = frame.y + card_padding + header_height;
            if preview_h > 0 {
                cursor_y = cursor_y.saturating_add(JSON_CARD_SIDE_GAP.saturating_sub(2));
                draw_json_card_image(
                    canvas,
                    media,
                    Rect::from_xywh(
                        (frame.x + card_padding) as f32,
                        cursor_y as f32,
                        frame.width.saturating_sub(card_padding * 2) as f32,
                        preview_h as f32,
                    ),
                    0.0,
                );
                cursor_y = cursor_y.saturating_add(preview_h);
            }
            if let Some(line) = footer_line.filter(|line| !line.is_empty()) {
                cursor_y = cursor_y.saturating_add(JSON_CARD_SIDE_GAP.saturating_sub(2));
                draw_json_card_tag_row(
                    canvas,
                    font_collection,
                    emoji_cache,
                    frame.x + card_padding,
                    cursor_y,
                    line,
                    tag_icon,
                    meta_size,
                    font_weight_body,
                    color_muted,
                );
            }
        }
        "news" => {
            let side_slot = json_card_has_side_media_slot(view, media);
            if side_slot {
                draw_json_card_image(
                    canvas,
                    media,
                    Rect::from_xywh(
                        (frame.x + card_padding) as f32,
                        side_y as f32,
                        JSON_CARD_QR_SIZE as f32,
                        JSON_CARD_QR_SIZE as f32,
                    ),
                    4.0,
                );
            }

            let title_x = frame
                .x
                .saturating_add(card_padding)
                .saturating_add(json_card_side_media_space(view, side_slot));
            let title_block_height = (title_lines.len() as u32)
                .saturating_mul(card_line_height)
                .max(card_line_height);
            let mut baseline =
                side_y + JSON_CARD_QR_SIZE.saturating_sub(title_block_height) / 2 + card_title_size;
            for line in title_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    title_x as f32,
                    baseline as f32,
                    card_title_size,
                    font_weight_title,
                    color_text,
                );
                baseline = baseline.saturating_add(card_line_height);
            }

            let body_x = frame.x + card_padding;
            let body_top = side_y
                + JSON_CARD_QR_SIZE
                + JSON_CARD_SIDE_GAP.saturating_sub(2)
                + JSON_CARD_SIDE_GAP.saturating_sub(4);
            baseline = body_top + meta_size;
            for line in desc_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    body_x as f32,
                    baseline as f32,
                    meta_size,
                    font_weight_body,
                    color_meta,
                );
                baseline = baseline.saturating_add(file_meta_height);
            }
            if let Some(line) = footer_line.filter(|line| !line.is_empty()) {
                let tag_top = if desc_lines.is_empty() {
                    body_top
                } else {
                    body_top
                        + (desc_lines.len() as u32).saturating_mul(file_meta_height)
                        + file_meta_gap
                };
                draw_json_card_tag_row(
                    canvas,
                    font_collection,
                    emoji_cache,
                    body_x,
                    tag_top,
                    line,
                    tag_icon,
                    meta_size,
                    font_weight_body,
                    color_muted,
                );
            }
        }
        "contact" => {
            let text_x = frame.x + card_padding;
            let side_slot = json_card_has_side_media_slot(view, media);
            if side_slot {
                draw_json_card_image(
                    canvas,
                    media,
                    Rect::from_xywh(
                        text_x as f32,
                        side_y as f32,
                        JSON_CARD_QR_SIZE as f32,
                        JSON_CARD_QR_SIZE as f32,
                    ),
                    4.0,
                );
            } else if view.media_source.is_none() {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    "avatar",
                    text_x as f32,
                    (side_y + 34) as f32,
                    font_size,
                    font_weight_body,
                    color_text,
                );
            }
            let text_x = text_x.saturating_add(json_card_side_media_space(view, side_slot));
            let mut baseline = frame.y + card_padding + card_title_size;
            for line in title_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    text_x as f32,
                    baseline as f32,
                    card_title_size,
                    font_weight_title,
                    color_text,
                );
                baseline = baseline.saturating_add(card_line_height);
            }
            for line in desc_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    text_x as f32,
                    baseline as f32,
                    meta_size,
                    font_weight_body,
                    color_meta,
                );
                baseline = baseline.saturating_add(file_meta_height);
            }
            if let Some(line) = footer_line.filter(|line| !line.is_empty()) {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    text_x as f32,
                    baseline as f32,
                    meta_size,
                    font_weight_body,
                    color_muted,
                );
            }
        }
        _ => {
            let text_x = frame.x + card_padding;
            let preview_h = json_card_preview_height(view, media, frame.width, card_padding);
            let mut cursor_y = frame.y + card_padding;
            if preview_h > 0 {
                draw_json_card_image(
                    canvas,
                    media,
                    Rect::from_xywh(
                        text_x as f32,
                        cursor_y as f32,
                        frame.width.saturating_sub(card_padding * 2) as f32,
                        preview_h as f32,
                    ),
                    0.0,
                );
                cursor_y = cursor_y.saturating_add(preview_h + card_padding);
            }

            let mut baseline = cursor_y + card_title_size;
            if preview_h == 0 {
                baseline = baseline.saturating_sub(JSON_CARD_VERTICAL_TEXT_BASELINE_RAISE_PX);
            }
            for line in title_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    text_x as f32,
                    baseline as f32,
                    card_title_size,
                    font_weight_title,
                    color_text,
                );
                baseline = baseline.saturating_add(card_line_height);
            }
            for line in desc_lines {
                draw_text_line(
                    canvas,
                    font_collection,
                    emoji_cache,
                    line,
                    text_x as f32,
                    baseline as f32,
                    meta_size,
                    font_weight_body,
                    color_meta,
                );
                baseline = baseline.saturating_add(file_meta_height);
            }
            if let Some(line) = footer_line.filter(|line| !line.is_empty()) {
                draw_json_card_tag_row(
                    canvas,
                    font_collection,
                    emoji_cache,
                    text_x,
                    baseline.saturating_sub(meta_size),
                    line,
                    tag_icon,
                    meta_size,
                    font_weight_body,
                    color_muted,
                );
            }
        }
    }

    if let Some(url) = qr_text.filter(|_| bottom_qr) {
        let qr_y = frame
            .y
            .saturating_add(frame.height)
            .saturating_sub(card_padding + JSON_CARD_QR_SIZE + JSON_CARD_BOTTOM_QR_RAISE_PX);
        draw_qr_code(
            canvas,
            url,
            Rect::from_xywh(
                (frame.x + card_padding) as f32,
                qr_y as f32,
                JSON_CARD_QR_SIZE as f32,
                JSON_CARD_QR_SIZE as f32,
            ),
            color_white,
            color_text,
        );
    }
}

struct ForwardDrawContext<'a> {
    canvas: &'a Canvas,
    font_collection: &'a FontCollection,
    emoji_cache: &'a mut EmojiRenderCache,
    text_measurer: &'a mut TextMeasurer,
    face_cache: &'a mut HashMap<String, ResolvedImage>,
    face_image_cache: &'a mut HashMap<String, Option<Image>>,
    file_icon_cache: &'a mut HashMap<&'static str, Option<Image>>,
    bubble_pad_left: u32,
    bubble_pad_right: u32,
    bubble_pad_top: u32,
    bubble_pad_bottom: u32,
    font_size: u32,
    line_height: u32,
    face_size: u32,
    font_weight_title: u32,
    font_weight_body: u32,
    meta_size: u32,
    card_title_size: u32,
    card_padding: u32,
    card_line_height: u32,
    file_padding: u32,
    file_line_height: u32,
    file_meta_height: u32,
    file_meta_gap: u32,
    file_icon_size: u32,
    radius_lg: u32,
    forward_line_gap: u32,
    color_white: Color4f,
    color_border: Color4f,
    color_text: Color4f,
    color_meta: Color4f,
    color_muted: Color4f,
}

impl ForwardDrawContext<'_> {
    fn draw_forward(&mut self, frame: BlockFrame, items: &[ForwardLayoutItem]) {
        let title = "合并转发聊天记录";
        let title_y = frame.y.saturating_add(self.meta_size).saturating_sub(1);
        draw_text_line(
            self.canvas,
            self.font_collection,
            self.emoji_cache,
            title,
            frame.x as f32,
            title_y as f32,
            self.meta_size,
            self.font_weight_body,
            self.color_meta,
        );

        let first_child_y = items
            .iter()
            .flat_map(|item| item.blocks.iter().map(|child| child.y))
            .min()
            .unwrap_or(frame.y);
        let top = first_child_y.saturating_sub(self.forward_line_gap);
        let bottom = frame.y.saturating_add(frame.height).saturating_add(2);
        let mut accent = Paint::default();
        accent.set_color4f(color_from_hex(0x71A1CC), None);
        accent.set_anti_alias(true);
        self.canvas.draw_rect(
            Rect::from_xywh(
                frame.x as f32,
                top as f32,
                3.0,
                bottom.saturating_sub(top) as f32,
            ),
            &accent,
        );

        for item in items {
            for child in item.blocks.iter().cloned() {
                self.draw_child(child);
            }
        }
    }

    fn draw_child(&mut self, child: BlockLayout) {
        let frame = BlockFrame::from_layout(&child);
        match child.kind {
            BlockKind::Text { lines } => self.draw_text_bubble(frame, &lines),
            BlockKind::FileCard {
                name_lines,
                meta_line,
                icon_path,
                icon_text,
            } => self.draw_file_card(
                frame,
                &name_lines,
                meta_line.as_deref(),
                icon_path,
                &icon_text,
            ),
            BlockKind::JsonCard {
                qr_url,
                view,
                title_lines,
                desc_lines,
                footer_line,
                media,
                tag_icon,
                brand_icon,
            } => self.draw_json_card(
                frame,
                qr_url.as_deref(),
                &view,
                &title_lines,
                &desc_lines,
                footer_line.as_deref(),
                media.as_ref(),
                tag_icon.as_ref(),
                brand_icon.as_ref(),
            ),
            BlockKind::Image { image } => {
                draw_image_preview_frame(
                    self.canvas,
                    image.as_ref(),
                    frame.rect(),
                    self.radius_lg as f32,
                    self.color_white,
                    self.color_border,
                );
            }
            BlockKind::VideoPreview { image } => {
                let bg = if image.as_ref().filter(|img| img.has_bytes()).is_some() {
                    self.color_white
                } else {
                    color_from_hex(0x262626)
                };
                let rect = frame.rect();
                draw_image_preview_frame(
                    self.canvas,
                    image.as_ref(),
                    rect,
                    self.radius_lg as f32,
                    bg,
                    self.color_border,
                );
                draw_video_play_triangle(self.canvas, rect);
            }
            BlockKind::MediaCard { media_kind, .. } => {
                draw_text_line(
                    self.canvas,
                    self.font_collection,
                    self.emoji_cache,
                    media_label(media_kind),
                    frame.x as f32,
                    (frame.y + self.font_size) as f32,
                    self.font_size,
                    self.font_weight_body,
                    self.color_text,
                );
            }
            BlockKind::Reply {
                meta_lines,
                body_lines,
            } => self.draw_reply(frame, &meta_lines, &body_lines),
            BlockKind::Poke { image } => self.draw_poke(frame, image.as_ref()),
            BlockKind::Forward { items } => self.draw_forward(frame, &items),
        }
    }

    fn draw_text_bubble(&mut self, frame: BlockFrame, lines: &[InlineLine]) {
        let rr = RRect::new_rect_xy(frame.rect(), self.radius_lg as f32, self.radius_lg as f32);
        draw_shadowed_rrect(self.canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);

        let mut bubble_bg = Paint::default();
        bubble_bg.set_color4f(self.color_white, None);
        bubble_bg.set_anti_alias(true);
        self.canvas.draw_rrect(rr, &bubble_bg);

        let line_x = frame.x + self.bubble_pad_left;
        let line_y = frame.y + self.bubble_pad_top + self.font_size;
        for (idx, line) in lines.iter().enumerate() {
            let baseline_y = line_y + self.line_height.saturating_mul(idx as u32);
            self.draw_inline_line(line, line_x, baseline_y);
        }
    }

    fn draw_inline_line(&mut self, line: &InlineLine, line_x: u32, baseline_y: u32) {
        let mut cursor_x = line_x;
        for run in &line.runs {
            match run {
                InlineRun::Text(text) => {
                    if !text.is_empty() {
                        draw_text_line(
                            self.canvas,
                            self.font_collection,
                            self.emoji_cache,
                            text,
                            cursor_x as f32,
                            baseline_y as f32,
                            self.font_size,
                            self.font_weight_body,
                            self.color_text,
                        );
                        cursor_x = cursor_x.saturating_add(measure_inline_text_width(
                            text,
                            self.font_size,
                            self.font_weight_body,
                            self.text_measurer,
                            self.emoji_cache,
                        ));
                    }
                }
                InlineRun::Face { id } => {
                    if let Some(face) = resolve_face_image(id, self.face_cache) {
                        let line_top = baseline_y.saturating_sub(self.font_size);
                        let face_y = line_top
                            .saturating_add(self.line_height.saturating_sub(self.face_size) / 2);
                        let face_x = cursor_x;
                        let face_image = self
                            .face_image_cache
                            .entry(id.clone())
                            .or_insert_with(|| decode_image(&face));
                        if let Some(image) = face_image.as_ref() {
                            draw_image_cover_rounded(
                                self.canvas,
                                image,
                                Rect::from_xywh(
                                    face_x as f32,
                                    face_y as f32,
                                    self.face_size as f32,
                                    self.face_size as f32,
                                ),
                                3.0,
                            );
                        }
                        cursor_x = cursor_x.saturating_add(self.face_size);
                    }
                }
                InlineRun::Emoji { glyph_id } => {
                    let line_top = baseline_y.saturating_sub(self.font_size);
                    let emoji_y = line_top
                        .saturating_add(self.line_height.saturating_sub(self.face_size) / 2);
                    if let Some(image) = self.emoji_cache.resolve_image_by_gid(*glyph_id) {
                        draw_image_stretch(
                            self.canvas,
                            &image,
                            Rect::from_xywh(
                                cursor_x as f32,
                                emoji_y as f32,
                                self.face_size as f32,
                                self.face_size as f32,
                            ),
                        );
                    }
                    cursor_x = cursor_x.saturating_add(self.face_size);
                }
            }
        }
    }

    fn draw_file_card(
        &mut self,
        frame: BlockFrame,
        name_lines: &[String],
        meta_line: Option<&str>,
        icon_path: Option<&'static str>,
        icon_text: &str,
    ) {
        let rr = RRect::new_rect_xy(frame.rect(), self.radius_lg as f32, self.radius_lg as f32);
        draw_shadowed_rrect(self.canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);

        let mut card_bg = Paint::default();
        card_bg.set_color4f(self.color_white, None);
        card_bg.set_anti_alias(true);
        self.canvas.draw_rrect(rr, &card_bg);

        let icon_x = frame.x
            + frame
                .width
                .saturating_sub(self.file_padding + self.file_icon_size);
        let icon_y = frame.y + (frame.height - self.file_icon_size) / 2;
        let mut drew_icon = false;
        if let Some(path) = icon_path {
            let icon_image = self.file_icon_cache.entry(path).or_insert_with(|| {
                resolved_image_from_path(path).and_then(|img| decode_image(&img))
            });
            if let Some(image) = icon_image.as_ref() {
                draw_image_stretch(
                    self.canvas,
                    image,
                    Rect::from_xywh(
                        icon_x as f32,
                        icon_y as f32,
                        self.file_icon_size as f32,
                        self.file_icon_size as f32,
                    ),
                );
                drew_icon = true;
            }
        }
        if !drew_icon {
            let icon_center_x = icon_x + self.file_icon_size / 2;
            let icon_center_y = icon_y + self.file_icon_size / 2;
            if let Some((paragraph, metrics)) = build_line_paragraph(
                self.font_collection,
                icon_text,
                11,
                self.font_weight_body,
                self.color_meta,
            ) {
                let icon_baseline = center_baseline(icon_center_y as f32, &metrics);
                let icon_x = icon_center_x as f32 - metrics.width * 0.5;
                paragraph.paint(self.canvas, (icon_x, icon_baseline - metrics.baseline));
            }
        }

        let text_x = frame.x + self.file_padding;
        let name_height = name_lines.len() as u32 * self.file_line_height;
        let meta_height = if meta_line.is_some() {
            self.file_meta_height + self.file_meta_gap
        } else {
            0
        };
        let text_height = name_height + meta_height;
        let content_height = self.file_icon_size.max(text_height);
        let text_top = frame.y + self.file_padding + (content_height - text_height) / 2 - 4;
        let file_name_size = self.font_size.saturating_sub(2).max(self.meta_size);
        let text_baseline = text_top + file_name_size;

        for (idx, line) in name_lines.iter().enumerate() {
            let baseline = text_baseline + self.file_line_height.saturating_mul(idx as u32);
            draw_text_line(
                self.canvas,
                self.font_collection,
                self.emoji_cache,
                line,
                text_x as f32,
                baseline as f32,
                file_name_size,
                self.font_weight_body,
                self.color_text,
            );
        }
        if let Some(meta) = meta_line {
            let meta_font_size = 11u32;
            let meta_y = text_top + name_height + self.file_meta_gap + meta_font_size + 6;
            draw_text_line(
                self.canvas,
                self.font_collection,
                self.emoji_cache,
                meta,
                text_x as f32,
                meta_y as f32,
                meta_font_size,
                self.font_weight_body,
                self.color_muted,
            );
        }
    }

    fn draw_json_card(
        &mut self,
        frame: BlockFrame,
        qr_url: Option<&str>,
        view: &JsonCardView,
        title_lines: &[String],
        desc_lines: &[String],
        footer_line: Option<&str>,
        media: Option<&ResolvedImage>,
        tag_icon: Option<&ResolvedImage>,
        brand_icon: Option<&ResolvedImage>,
    ) {
        draw_json_card_frame(
            self.canvas,
            self.font_collection,
            self.emoji_cache,
            frame,
            self.radius_lg,
            self.card_padding,
            self.font_size,
            self.meta_size,
            self.card_title_size,
            self.card_line_height,
            self.file_meta_height,
            self.file_meta_gap,
            self.font_weight_title,
            self.font_weight_body,
            self.color_white,
            self.color_text,
            self.color_meta,
            self.color_muted,
            qr_url,
            view,
            title_lines,
            desc_lines,
            footer_line,
            media,
            tag_icon,
            brand_icon,
        );
    }

    fn draw_reply(&mut self, frame: BlockFrame, meta_lines: &[String], body_lines: &[String]) {
        let rr = RRect::new_rect_xy(frame.rect(), self.radius_lg as f32, self.radius_lg as f32);
        draw_shadowed_rrect(self.canvas, rr, SHADOW_SM_BLUR_PX, SHADOW_SM_ALPHA);
        let mut bubble_bg = Paint::default();
        bubble_bg.set_color4f(self.color_white, None);
        bubble_bg.set_anti_alias(true);
        self.canvas.draw_rrect(rr, &bubble_bg);

        let reply_accent_width = 3u32;
        let reply_inner_pad_x = self.card_padding;
        let reply_inner_pad_y = 6u32;
        let reply_meta_size = self.font_size.saturating_sub(2).max(self.meta_size);
        let reply_body_size = self.font_size;
        let reply_meta_line_height = self.file_line_height;
        let reply_body_line_height = self.line_height;
        let inner_x = frame.x + self.bubble_pad_left;
        let inner_y = frame
            .y
            .saturating_add(self.bubble_pad_top)
            .saturating_sub(REPLY_INNER_Y_RAISE_PX);
        let inner_width = frame
            .width
            .saturating_sub(self.bubble_pad_left + self.bubble_pad_right);
        let inner_height = frame
            .height
            .saturating_sub(self.bubble_pad_top + self.bubble_pad_bottom);
        let inner_rect = Rect::from_xywh(
            inner_x as f32,
            inner_y as f32,
            inner_width as f32,
            inner_height as f32,
        );
        let inner_rr = RRect::new_rect_xy(inner_rect, 4.0, 4.0);
        let mut bg = Paint::default();
        bg.set_color4f(color_from_hex(0xFAFAFA), None);
        bg.set_anti_alias(true);
        self.canvas.draw_rrect(inner_rr, &bg);
        let mut accent = Paint::default();
        accent.set_color4f(self.color_border, None);
        accent.set_anti_alias(true);
        self.canvas.draw_rect(
            Rect::from_xywh(
                inner_x as f32,
                inner_y as f32,
                reply_accent_width as f32,
                inner_height as f32,
            ),
            &accent,
        );

        let text_x = inner_x + reply_accent_width + reply_inner_pad_x;
        let mut baseline = inner_y + reply_inner_pad_y + reply_meta_size;
        for line in meta_lines {
            draw_text_line(
                self.canvas,
                self.font_collection,
                self.emoji_cache,
                line,
                text_x as f32,
                baseline as f32,
                reply_meta_size,
                self.font_weight_body,
                self.color_meta,
            );
            baseline = baseline.saturating_add(reply_meta_line_height);
        }
        baseline = inner_y
            + reply_inner_pad_y
            + meta_lines.len() as u32 * reply_meta_line_height
            + self.file_meta_gap
            + reply_body_size
            + REPLY_BODY_BASELINE_OFFSET_PX;
        for line in body_lines {
            draw_text_line(
                self.canvas,
                self.font_collection,
                self.emoji_cache,
                line,
                text_x as f32,
                baseline as f32,
                reply_body_size,
                self.font_weight_body,
                color_from_hex(0x333333),
            );
            baseline = baseline.saturating_add(reply_body_line_height);
        }
    }

    fn draw_poke(&mut self, frame: BlockFrame, image: Option<&ResolvedImage>) {
        let rect = frame.rect();
        if let Some(img) = image.filter(|img| img.has_bytes()) {
            if let Some(decoded) = decode_image(img) {
                draw_image_stretch(self.canvas, &decoded, rect);
                return;
            }
        }
        draw_text_line(
            self.canvas,
            self.font_collection,
            self.emoji_cache,
            "[戳一戳]",
            frame.x as f32,
            (frame.y + self.font_size) as f32,
            self.font_size,
            self.font_weight_body,
            self.color_text,
        );
    }
}

fn draw_watermark_layer(
    canvas: &Canvas,
    font_collection: &FontCollection,
    emoji_cache: &mut EmojiRenderCache,
    header: &HeaderInfo,
    width: u32,
    height: u32,
    text: &str,
) {
    let opacity = 0.12f32;
    let angle = 24f32;
    let font_size = 40u32;
    let font_weight = 500u32;
    let tile = 480f32;
    let jitter = 10f32;
    let color = Color4f::new(0.0, 0.0, 0.0, opacity);
    let (text_width, metrics) =
        build_line_paragraph(font_collection, text, font_size, font_weight, color)
            .map(|(_, metrics)| (metrics.width, metrics))
            .unwrap_or_else(|| {
                (
                    text.len() as f32 * font_size as f32 * 0.55,
                    LineMetricsSnapshot {
                        baseline: font_size as f32,
                        ascent: font_size as f32,
                        descent: 0.0,
                        width: 0.0,
                    },
                )
            });
    let text_height = font_size as f32;
    let radians = angle.to_radians();
    let stamp_w = text_width * radians.cos().abs() + text_height * radians.sin().abs();
    let stamp_h = text_width * radians.sin().abs() + text_height * radians.cos().abs();
    let pad_x = (stamp_w * 0.5).ceil();
    let pad_y = (stamp_h * 0.5).ceil();

    let mut rng = DeterministicRng::new(watermark_seed(header, text));
    let cols = (((width as f32 - pad_x * 2.0) / tile).floor() as i32 + 1).max(1);
    let rows = (((height as f32 - pad_y * 2.0) / tile).floor() as i32 + 1).max(1);
    let center_x = width as f32 * 0.5;
    let grid_span_x = (cols - 1) as f32 * tile;
    let first_cx = center_x - grid_span_x * 0.5;
    let first_cy = pad_y + stamp_h * 0.5;
    let max_x = (width as f32 - stamp_w).max(0.0);
    let max_y = (height as f32 - stamp_h).max(0.0);

    for row in 0..rows {
        for col in 0..cols {
            let jx = rng.jitter(jitter);
            let jy = rng.jitter(jitter);
            let stagger = if row % 2 == 0 { 0.0 } else { tile * 0.5 };
            let cx = first_cx + col as f32 * tile + stagger;
            let cy = first_cy + row as f32 * tile;
            let x = (cx + jx - stamp_w * 0.5).round().clamp(0.0, max_x);
            let y = (cy + jy - stamp_h * 0.5).round().clamp(0.0, max_y);
            let draw_cx = x + text_width * 0.5;
            let draw_cy = y + text_height * 0.5;

            canvas.save();
            canvas.translate((draw_cx, draw_cy));
            canvas.rotate(-angle, None);
            draw_text_line(
                canvas,
                font_collection,
                emoji_cache,
                text,
                -text_width * 0.5,
                -text_height * 0.5 + metrics.baseline,
                font_size,
                font_weight,
                color,
            );
            canvas.restore();
        }
    }
}

fn watermark_seed(header: &HeaderInfo, watermark_text: &str) -> u64 {
    let mut out: u64 = 0xcbf29ce484222325;
    for byte in header.post_id_hex.as_bytes() {
        out ^= u64::from(*byte);
        out = out.wrapping_mul(0x100000001b3);
    }
    out ^= 0xff;
    for byte in watermark_text.as_bytes() {
        out ^= u64::from(*byte);
        out = out.wrapping_mul(0x100000001b3);
    }
    out
}

struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        let state = if seed == 0 { 0x9e3779b97f4a7c15 } else { seed };
        Self { state }
    }

    fn next_u32(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state >> 16) as u32
    }

    fn jitter(&mut self, bound: f32) -> f32 {
        let unit = self.next_u32() as f32 / u32::MAX as f32;
        unit * (bound * 2.0) - bound
    }
}

struct LineMetricsSnapshot {
    baseline: f32,
    ascent: f32,
    descent: f32,
    width: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmojiGlyphRecord {
    glyph_id: u16,
    width: u8,
    height: u8,
    bearing_x: i8,
    bearing_y: i8,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmojiPngMetadata {
    strike_ppem: u8,
    glyphs: Vec<EmojiGlyphRecord>,
    #[serde(default)]
    codepoints: HashMap<String, u16>,
    #[serde(default)]
    sequences: HashMap<String, u16>,
}

#[derive(Debug, Clone)]
struct EmojiSequence {
    seq: String,
    glyph_id: u16,
    len_bytes: usize,
}

#[derive(Debug)]
struct EmojiPngStore {
    res_prefix: &'static str,
    codepoints: HashMap<char, u16>,
    sequences: HashMap<String, u16>,
    png_cache: HashMap<u16, Arc<[u8]>>,
}

struct EmojiRenderCache {
    store: Option<&'static Mutex<EmojiPngStore>>,
    codepoints: HashMap<char, u16>,
    sequences_by_first: HashMap<char, Vec<EmojiSequence>>,
    keycap_map: HashMap<char, u16>,
    image_cache: HashMap<u16, Option<Image>>,
}

impl EmojiRenderCache {
    fn new() -> Self {
        let store = emoji_png_store();
        let mut codepoints = HashMap::new();
        let mut sequences_by_first = HashMap::new();
        let mut keycap_map = HashMap::new();
        let mut keycap_prefers_vs: HashMap<char, bool> = HashMap::new();
        if let Some(store_ref) = store {
            let guard = match store_ref.lock() {
                Ok(guard) => guard,
                Err(guard) => guard.into_inner(),
            };
            codepoints = guard.codepoints.clone();
            for (seq, glyph_id) in &guard.sequences {
                if seq.is_empty() {
                    continue;
                }
                if let Some(first) = seq.chars().next() {
                    let entry = sequences_by_first.entry(first).or_insert_with(Vec::new);
                    entry.push(EmojiSequence {
                        seq: seq.clone(),
                        glyph_id: *glyph_id,
                        len_bytes: seq.len(),
                    });
                }
                if let Some((base, has_vs)) = keycap_sequence_base(seq) {
                    let replace = match keycap_prefers_vs.get(&base) {
                        None => true,
                        Some(existing_has_vs) => !*existing_has_vs && has_vs,
                    };
                    if replace {
                        keycap_map.insert(base, *glyph_id);
                        keycap_prefers_vs.insert(base, has_vs);
                    }
                }
            }
        }
        for entries in sequences_by_first.values_mut() {
            entries.sort_by_key(|item| std::cmp::Reverse(item.len_bytes));
        }
        Self {
            store,
            codepoints,
            sequences_by_first,
            keycap_map,
            image_cache: HashMap::new(),
        }
    }

    fn is_emoji_char(&self, ch: char) -> bool {
        if ch.is_ascii() {
            return false;
        }
        self.codepoints.contains_key(&ch)
    }

    fn glyph_id_for_char(&self, ch: char) -> Option<u16> {
        self.codepoints.get(&ch).copied()
    }

    fn match_sequence(&self, text: &str, idx: usize) -> Option<(u16, usize)> {
        let rest = text.get(idx..)?;
        let mut chars = rest.chars();
        let first = chars.next()?;
        if let Some(glyph_id) = self.keycap_map.get(&first).copied() {
            let mut consumed = first.len_utf8();
            if let Some(next) = chars.next() {
                if next == '\u{FE0F}' || next == '\u{FE0E}' {
                    consumed += next.len_utf8();
                    if let Some(after) = chars.next() {
                        if after == '\u{20E3}' {
                            consumed += after.len_utf8();
                            return Some((glyph_id, consumed));
                        }
                    }
                } else if next == '\u{20E3}' {
                    consumed += next.len_utf8();
                    return Some((glyph_id, consumed));
                }
            }
        }
        let entries = self.sequences_by_first.get(&first)?;
        for entry in entries {
            if rest.starts_with(&entry.seq) {
                return Some((entry.glyph_id, entry.len_bytes));
            }
        }
        None
    }

    fn resolve_image_by_gid(&mut self, glyph_id: u16) -> Option<Image> {
        if let Some(entry) = self.image_cache.get(&glyph_id) {
            return entry.clone();
        }
        let Some(store_ref) = self.store else {
            return None;
        };
        let mut store = match store_ref.lock() {
            Ok(guard) => guard,
            Err(guard) => guard.into_inner(),
        };
        let png_bytes = match store.png_cache.get(&glyph_id).cloned() {
            Some(bytes) => bytes,
            None => {
                let path = emoji_png_resource_path(store.res_prefix, glyph_id);
                let bytes = read_res_relative_bytes(&path)?;
                store.png_cache.insert(glyph_id, bytes.clone());
                bytes
            }
        };
        let image = decode_emoji_image(png_bytes.as_ref());
        self.image_cache.insert(glyph_id, image.clone());
        image
    }
}

fn decode_emoji_image(bytes: &[u8]) -> Option<Image> {
    let data = Data::new_copy(bytes);
    Image::from_encoded(data)
}

fn keycap_sequence_base(seq: &str) -> Option<(char, bool)> {
    let mut chars = seq.chars();
    let base = chars.next()?;
    if !matches!(base, '0'..='9' | '#' | '*') {
        return None;
    }
    let second = chars.next()?;
    if second == '\u{FE0F}' || second == '\u{FE0E}' {
        let third = chars.next()?;
        if third != '\u{20E3}' || chars.next().is_some() {
            return None;
        }
        return Some((base, true));
    }
    if second == '\u{20E3}' && chars.next().is_none() {
        return Some((base, false));
    }
    None
}

fn build_text_style(font_size: u32, font_weight: u32, color: Color4f) -> TextStyle {
    debug_log!(
        "text style: size={} weight={} color={:?} families={:?}",
        font_size,
        font_weight,
        color,
        FONT_FAMILIES
    );
    let mut ts = TextStyle::new();
    ts.set_font_size(font_size as f32);
    ts.set_font_families(&FONT_FAMILIES);
    let font_style = FontStyle::new(
        Weight::from(font_weight as i32),
        Width::NORMAL,
        Slant::Upright,
    );
    ts.set_font_style(font_style);
    let mut paint = Paint::default();
    paint.set_color4f(color, None);
    paint.set_style(skia_safe::paint::Style::StrokeAndFill);
    let stroke_width = if font_size == 16 {
        0.015
    } else {
        TEXT_RASTER_STROKE_PX
    };
    paint.set_stroke_width(stroke_width);
    ts.set_foreground_paint(&paint);
    ts
}

const EMOJI_PNG_RES_PREFIX: &str = "emoji_png/apple_color_emoji";
const EMOJI_PNG_METADATA_PATH: &str = "emoji_png/apple_color_emoji/metadata.json";

fn emoji_png_store() -> Option<&'static Mutex<EmojiPngStore>> {
    static EMOJI_PNG_STORE: OnceLock<Option<Mutex<EmojiPngStore>>> = OnceLock::new();
    EMOJI_PNG_STORE
        .get_or_init(init_emoji_png_store_inner)
        .as_ref()
}

fn init_emoji_png_store_inner() -> Option<Mutex<EmojiPngStore>> {
    let metadata = match load_emoji_png_metadata() {
        Some(metadata) => metadata,
        None => {
            debug_log!("emoji png: metadata missing; run scripts/extract_apple_emoji_pngs.py");
            return None;
        }
    };
    let mut codepoints = HashMap::new();
    for (emoji, glyph_id) in metadata.codepoints {
        let mut chars = emoji.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            codepoints.insert(ch, glyph_id);
        }
    }
    if codepoints.is_empty() {
        debug_log!("emoji png: codepoint map missing; rerun extract script");
        return None;
    }
    let mut sequences = HashMap::new();
    for (sequence, glyph_id) in metadata.sequences {
        if sequence.chars().count() >= 2 {
            sequences.insert(sequence, glyph_id);
        }
    }
    Some(Mutex::new(EmojiPngStore {
        res_prefix: EMOJI_PNG_RES_PREFIX,
        codepoints,
        sequences,
        png_cache: HashMap::new(),
    }))
}

fn emoji_png_resource_path(prefix: &str, glyph_id: u16) -> String {
    format!("{}/gid_{:04x}.png", prefix, glyph_id)
}

fn load_emoji_png_metadata() -> Option<EmojiPngMetadata> {
    let bytes = read_res_relative_bytes(EMOJI_PNG_METADATA_PATH)?;
    serde_json::from_slice(bytes.as_ref()).ok()
}

fn build_line_paragraph(
    font_collection: &FontCollection,
    text: &str,
    font_size: u32,
    font_weight: u32,
    color: Color4f,
) -> Option<(Paragraph, LineMetricsSnapshot)> {
    if text.is_empty() {
        return None;
    }
    debug_log!(
        "paragraph build: text_len={} size={} weight={}",
        text.len(),
        font_size,
        font_weight
    );
    let mut ps = ParagraphStyle::new();
    let ts = build_text_style(font_size, font_weight, color);
    ps.set_text_style(&ts);
    let mut builder = ParagraphBuilder::new(&ps, font_collection.clone());
    builder.add_text(text);
    let mut paragraph = builder.build();
    paragraph.layout(MEASURE_MAX_WIDTH);
    let line_metrics = paragraph.get_line_metrics();
    let metrics = line_metrics.get(0)?;
    debug_log!(
        "paragraph metrics: text_len={} baseline={} ascent={} descent={} width={}",
        text.len(),
        metrics.baseline,
        metrics.ascent,
        metrics.descent,
        metrics.width
    );
    let snapshot = LineMetricsSnapshot {
        baseline: metrics.baseline as f32,
        ascent: metrics.ascent as f32,
        descent: metrics.descent as f32,
        width: metrics.width as f32,
    };
    Some((paragraph, snapshot))
}

fn draw_shadowed_rrect(canvas: &Canvas, rr: RRect, blur: f32, alpha: f32) {
    let shadow = image_filters::drop_shadow_only(
        (0.0, 0.0),
        (blur, blur),
        Color4f::new(0.0, 0.0, 0.0, alpha),
        None,
        None,
        image_filters::CropRect::from(None),
    );
    if let Some(filter) = shadow {
        let mut paint = Paint::default();
        paint.set_image_filter(filter);
        paint.set_anti_alias(true);
        canvas.draw_rrect(rr, &paint);
    }
}

fn draw_qr_code(canvas: &Canvas, text: &str, dst: Rect, bg: Color4f, fg: Color4f) {
    let Some((colors, width)) = legacy_qr_colors(text).or_else(|| {
        QrCode::with_error_correction_level(text.as_bytes(), EcLevel::L)
            .ok()
            .map(|code| (code.to_colors(), code.width()))
    }) else {
        return;
    };
    let mut bg_paint = Paint::default();
    bg_paint.set_color4f(bg, None);
    bg_paint.set_anti_alias(true);
    canvas.draw_rect(dst, &bg_paint);

    let module_px = 3u32;
    let width = width.max(1) as u32;
    let qr_px = width.saturating_mul(module_px).max(1);
    let Some(mut surface) = skia_safe::surfaces::raster_n32_premul((qr_px as i32, qr_px as i32))
    else {
        return;
    };
    let qr_canvas = surface.canvas();
    qr_canvas.draw_rect(
        Rect::from_xywh(0.0, 0.0, qr_px as f32, qr_px as f32),
        &bg_paint,
    );

    let mut fg_paint = Paint::default();
    fg_paint.set_color4f(fg, None);
    fg_paint.set_anti_alias(false);
    for y in 0..width as usize {
        for x in 0..width as usize {
            if colors[y * width as usize + x] == QrColor::Dark {
                qr_canvas.draw_rect(
                    Rect::from_xywh(
                        (x as u32 * module_px) as f32,
                        (y as u32 * module_px) as f32,
                        module_px as f32,
                        module_px as f32,
                    ),
                    &fg_paint,
                );
            }
        }
    }
    let image = surface.image_snapshot();
    let sampling = SamplingOptions {
        filter: skia_safe::FilterMode::Linear,
        mipmap: skia_safe::MipmapMode::None,
        ..SamplingOptions::default()
    };
    let paint = Paint::default();
    canvas.draw_image_rect_with_sampling_options(&image, None, dst, sampling, &paint);
}

fn legacy_qr_colors(text: &str) -> Option<(Vec<QrColor>, usize)> {
    let bits = bits::encode_auto(text.as_bytes(), EcLevel::L).ok()?;
    let version = bits.version();
    let data = bits.into_bytes();
    let (encoded_data, ec_data) = ec::construct_codewords(&data, version, EcLevel::L).ok()?;
    let mut canvas = QrCanvas::new(version, EcLevel::L);
    canvas.draw_all_functional_patterns();
    canvas.draw_data(&encoded_data, &ec_data);
    let width = usize::try_from(version.width()).ok()?;
    if let Some(pattern) = legacy_qr_mask_pattern_override() {
        canvas.apply_mask(pattern);
        return Some((canvas.into_colors(), width));
    }
    let (_pattern, colors) = libqrencode_best_qr_mask(&canvas, width);
    Some((colors, width))
}

fn legacy_qr_mask_pattern_override() -> Option<MaskPattern> {
    match std::env::var("OQQWALL_RENDER_QR_MASK").ok().as_deref() {
        Some("0") | Some("checkerboard") => Some(MaskPattern::Checkerboard),
        Some("1") | Some("horizontal") => Some(MaskPattern::HorizontalLines),
        Some("2") | Some("vertical") => Some(MaskPattern::VerticalLines),
        Some("3") | Some("diagonal") => Some(MaskPattern::DiagonalLines),
        Some("4") | Some("large_checkerboard") => Some(MaskPattern::LargeCheckerboard),
        Some("5") | Some("fields") => Some(MaskPattern::Fields),
        Some("6") | Some("diamonds") => Some(MaskPattern::Diamonds),
        Some("7") | Some("meadow") => Some(MaskPattern::Meadow),
        _ => None,
    }
}

fn libqrencode_best_qr_mask(canvas: &QrCanvas, width: usize) -> (MaskPattern, Vec<QrColor>) {
    const PATTERNS: [MaskPattern; 8] = [
        MaskPattern::Checkerboard,
        MaskPattern::HorizontalLines,
        MaskPattern::VerticalLines,
        MaskPattern::DiagonalLines,
        MaskPattern::LargeCheckerboard,
        MaskPattern::Fields,
        MaskPattern::Diamonds,
        MaskPattern::Meadow,
    ];

    let mut best_pattern = MaskPattern::HorizontalLines;
    let mut best_colors = Vec::new();
    let mut best_score = i32::MAX;

    for pattern in PATTERNS {
        let mut masked = canvas.clone();
        masked.apply_mask(pattern);
        let colors = masked.into_colors();
        let score = libqrencode_qr_demerit(&colors, width);
        if score < best_score {
            best_score = score;
            best_pattern = pattern;
            best_colors = colors;
        }
    }

    (best_pattern, best_colors)
}

fn libqrencode_qr_demerit(colors: &[QrColor], width: usize) -> i32 {
    const N2: i32 = 3;
    const N4: i32 = 10;

    let blacks = colors
        .iter()
        .filter(|&&color| color == QrColor::Dark)
        .count() as i32;
    let w2 = width.saturating_mul(width) as i32;
    let bratio = (200 * blacks + w2) / w2 / 2;
    let mut demerit = ((bratio - 50).abs() / 5) * N4;

    for y in 1..width {
        for x in 1..width {
            let current = qr_color_is_dark(colors, width, x, y);
            if current == qr_color_is_dark(colors, width, x - 1, y)
                && current == qr_color_is_dark(colors, width, x, y - 1)
                && current == qr_color_is_dark(colors, width, x - 1, y - 1)
            {
                demerit += N2;
            }
        }
    }

    for y in 0..width {
        let run_lengths = (0..width)
            .map(|x| qr_color_is_dark(colors, width, x, y))
            .collect::<Vec<_>>();
        demerit += libqrencode_qr_run_demerit(&run_lengths);
    }
    for x in 0..width {
        let run_lengths = (0..width)
            .map(|y| qr_color_is_dark(colors, width, x, y))
            .collect::<Vec<_>>();
        demerit += libqrencode_qr_run_demerit(&run_lengths);
    }

    demerit
}

fn libqrencode_qr_run_demerit(line: &[bool]) -> i32 {
    const N1: i32 = 3;
    const N3: i32 = 40;

    if line.is_empty() {
        return 0;
    }

    let mut runs = Vec::with_capacity(line.len() + 1);
    if line[0] {
        runs.push(-1);
    }
    let mut previous = line[0];
    let mut current_len = 1i32;
    for &current in &line[1..] {
        if current != previous {
            runs.push(current_len);
            current_len = 1;
            previous = current;
        } else {
            current_len += 1;
        }
    }
    runs.push(current_len);

    let mut demerit = 0;
    for i in 0..runs.len() {
        if runs[i] >= 5 {
            demerit += N1 + (runs[i] - 5);
        }
        if (i & 1) == 1 && i >= 3 && i < runs.len().saturating_sub(2) && runs[i] % 3 == 0 {
            let fact = runs[i] / 3;
            if runs[i - 2] == fact
                && runs[i - 1] == fact
                && runs[i + 1] == fact
                && runs[i + 2] == fact
            {
                if i == 3 || runs[i - 3] >= 4 * fact {
                    demerit += N3;
                } else if i + 4 >= runs.len() || runs[i + 3] >= 4 * fact {
                    demerit += N3;
                }
            }
        }
    }

    demerit
}

fn qr_color_is_dark(colors: &[QrColor], width: usize, x: usize, y: usize) -> bool {
    colors[y * width + x] == QrColor::Dark
}

fn draw_text_line(
    canvas: &Canvas,
    font_collection: &FontCollection,
    emoji_cache: &mut EmojiRenderCache,
    text: &str,
    x: f32,
    baseline_y: f32,
    font_size: u32,
    font_weight: u32,
    color: Color4f,
) {
    debug_log!(
        "text draw: text_len={} size={} weight={} x={} baseline_y={}",
        text.len(),
        font_size,
        font_weight,
        x,
        baseline_y
    );
    let mut cursor_x = x;
    let mut buffer = String::new();
    let emoji_size = font_size as f32;
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if let Some((glyph_id, consumed)) = emoji_cache.match_sequence(text, idx) {
            if !buffer.is_empty() {
                cursor_x += draw_text_segment(
                    canvas,
                    font_collection,
                    &buffer,
                    cursor_x,
                    baseline_y,
                    font_size,
                    font_weight,
                    color,
                );
                buffer.clear();
            }
            let top_y = baseline_y - font_size as f32;
            if let Some(image) = emoji_cache.resolve_image_by_gid(glyph_id) {
                draw_image_stretch(
                    canvas,
                    &image,
                    Rect::from_xywh(cursor_x, top_y, emoji_size, emoji_size),
                );
                cursor_x += emoji_size;
            }
            idx += consumed;
            continue;
        }
        let ch = text[idx..].chars().next().unwrap();
        if should_skip_emoji_char(ch) {
            idx += ch.len_utf8();
            continue;
        }
        if emoji_cache.is_emoji_char(ch) {
            if !buffer.is_empty() {
                cursor_x += draw_text_segment(
                    canvas,
                    font_collection,
                    &buffer,
                    cursor_x,
                    baseline_y,
                    font_size,
                    font_weight,
                    color,
                );
                buffer.clear();
            }
            let top_y = baseline_y - font_size as f32;
            if let Some(glyph_id) = emoji_cache.glyph_id_for_char(ch) {
                if let Some(image) = emoji_cache.resolve_image_by_gid(glyph_id) {
                    draw_image_stretch(
                        canvas,
                        &image,
                        Rect::from_xywh(cursor_x, top_y, emoji_size, emoji_size),
                    );
                    cursor_x += emoji_size;
                } else {
                    buffer.push(ch);
                }
            } else {
                buffer.push(ch);
            }
        } else {
            buffer.push(ch);
        }
        idx += ch.len_utf8();
    }
    if !buffer.is_empty() {
        let _ = draw_text_segment(
            canvas,
            font_collection,
            &buffer,
            cursor_x,
            baseline_y,
            font_size,
            font_weight,
            color,
        );
    }
}

fn draw_text_segment(
    canvas: &Canvas,
    font_collection: &FontCollection,
    text: &str,
    x: f32,
    baseline_y: f32,
    font_size: u32,
    font_weight: u32,
    color: Color4f,
) -> f32 {
    if let Some((paragraph, metrics)) =
        build_line_paragraph(font_collection, text, font_size, font_weight, color)
    {
        let top_y = baseline_y - metrics.baseline;
        paragraph.paint(canvas, (x, top_y));
        metrics.width
    } else {
        0.0
    }
}

fn center_baseline(center_y: f32, metrics: &LineMetricsSnapshot) -> f32 {
    center_y + (metrics.ascent - metrics.descent) * 0.5
}

fn decode_image(image: &ResolvedImage) -> Option<Image> {
    let bytes = image.bytes.as_ref()?;
    debug_log!("image decode: bytes={}", bytes.len());
    let data = Data::new_copy(bytes.as_ref());
    if let Some(image) = Image::from_encoded(data) {
        return Some(image);
    }
    let bytes = transcode_image_to_png(bytes.as_ref())?;
    debug_log!("image decode fallback png: bytes={}", bytes.len());
    let data = Data::new_copy(&bytes);
    Image::from_encoded(data)
}

fn transcode_image_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, image::ImageFormat::Png).ok()?;
    Some(encoded.into_inner())
}

fn draw_image_preview_frame(
    canvas: &Canvas,
    image: Option<&ResolvedImage>,
    rect: Rect,
    radius: f32,
    bg_color: Color4f,
    border_color: Color4f,
) {
    let rr = RRect::new_rect_xy(rect, radius, radius);
    draw_shadowed_rrect(canvas, rr, 3.0, 0.20);

    let mut bg = Paint::default();
    bg.set_color4f(bg_color, None);
    bg.set_anti_alias(true);
    canvas.draw_rrect(rr, &bg);

    let mut border = Paint::default();
    border.set_color4f(border_color, None);
    border.set_style(skia_safe::paint::Style::Stroke);
    border.set_stroke_width(1.0);
    border.set_anti_alias(true);
    canvas.draw_rrect(rr, &border);

    if let Some(decoded) = image.filter(|img| img.has_bytes()).and_then(decode_image) {
        draw_image_cover_rounded(canvas, &decoded, rect, radius);
    }
}

fn draw_video_play_triangle(canvas: &Canvas, rect: Rect) {
    let size = rect.width().min(rect.height()).min(42.0).max(18.0);
    let tri_w = size * 0.42;
    let tri_h = size * 0.52;
    let cx = rect.x() + rect.width() * 0.5 + tri_w * 0.08;
    let cy = rect.y() + rect.height() * 0.5;
    let left = cx - tri_w * 0.45;
    let top = cy - tri_h * 0.5;

    let mut shadow = PathBuilder::new();
    shadow.move_to((left + 1.5, top + 1.5));
    shadow.line_to((left + 1.5, top + tri_h + 1.5));
    shadow.line_to((left + tri_w + 1.5, cy + 1.5));
    shadow.close();
    let shadow = shadow.detach();
    let mut shadow_paint = Paint::default();
    shadow_paint.set_color4f(Color4f::new(0.0, 0.0, 0.0, 0.40), None);
    shadow_paint.set_anti_alias(true);
    canvas.draw_path(&shadow, &shadow_paint);

    let mut play = PathBuilder::new();
    play.move_to((left, top));
    play.line_to((left, top + tri_h));
    play.line_to((left + tri_w, cy));
    play.close();
    let play = play.detach();
    let mut play_paint = Paint::default();
    play_paint.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.88), None);
    play_paint.set_anti_alias(true);
    canvas.draw_path(&play, &play_paint);
}

fn draw_image_cover_rounded(canvas: &Canvas, img: &Image, dst: Rect, radius: f32) {
    let sw = img.width() as f32;
    let sh = img.height() as f32;
    debug_log!(
        "image draw: src=({}x{}) dst=({},{} {}x{}) radius={}",
        sw,
        sh,
        dst.x(),
        dst.y(),
        dst.width(),
        dst.height(),
        radius
    );
    if sw <= 0.0 || sh <= 0.0 || dst.width() <= 0.0 || dst.height() <= 0.0 {
        return;
    }
    let scale = (dst.width() / sw).max(dst.height() / sh);
    let crop_w = dst.width() / scale;
    let crop_h = dst.height() / scale;
    let crop_x = (sw - crop_w) * 0.5;
    let crop_y = (sh - crop_h) * 0.5;
    let src = Rect::from_xywh(
        crop_x.max(0.0),
        crop_y.max(0.0),
        crop_w.max(1.0),
        crop_h.max(1.0),
    );
    let rr = RRect::new_rect_xy(dst, radius, radius);

    canvas.save();
    canvas.clip_rrect(rr, Some(ClipOp::Intersect), Some(true));

    let sampling = SamplingOptions {
        filter: skia_safe::FilterMode::Linear,
        mipmap: skia_safe::MipmapMode::None,
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

fn draw_image_stretch(canvas: &Canvas, img: &Image, dst: Rect) {
    if dst.width() <= 0.0 || dst.height() <= 0.0 {
        return;
    }
    let sampling = SamplingOptions {
        filter: skia_safe::FilterMode::Linear,
        mipmap: skia_safe::MipmapMode::None,
        ..SamplingOptions::default()
    };
    let paint = Paint::default();
    canvas.draw_image_rect_with_sampling_options(img, None, dst, sampling, &paint);
}

async fn resolve_image_sources(
    state: &StateView,
    draft: &Draft,
    header: &HeaderInfo,
    cmd_tx: &mpsc::Sender<Command>,
    image_cache: &mut ImageMemoryCache,
) -> (RenderImageSources, HashSet<ImageCacheKey>) {
    let mut block_images = vec![None; draft.blocks.len()];
    let mut block_tag_icons = vec![None; draft.blocks.len()];
    let mut block_brand_icons = vec![None; draft.blocks.len()];
    let mut block_labels = vec![None; draft.blocks.len()];
    let mut used_keys = HashSet::new();
    for (idx, block) in draft.blocks.iter().enumerate() {
        match block {
            DraftBlock::Attachment {
                kind, reference, ..
            } => {
                if is_renderable_image(*kind) {
                    block_images[idx] = resolve_media_reference_for_image(
                        reference,
                        state,
                        image_cache,
                        &mut used_keys,
                    );
                } else if is_video_media(*kind) {
                    block_images[idx] = resolve_media_reference_for_video_preview(
                        reference,
                        state,
                        image_cache,
                        &mut used_keys,
                    );
                } else {
                    block_labels[idx] = resolve_media_reference_for_label(reference, state);
                }
            }
            DraftBlock::JsonCard { raw } => {
                let view = parse_json_card_view(raw);
                if let Some(source) = view.media_source {
                    block_images[idx] =
                        resolve_json_card_media_source(&source, image_cache, &mut used_keys).await;
                }
                if let Some(source) = view.tag_icon_source {
                    block_tag_icons[idx] =
                        resolve_json_card_media_source(&source, image_cache, &mut used_keys).await;
                }
                if let Some(source) = view.brand_icon_source {
                    block_brand_icons[idx] =
                        resolve_json_card_media_source(&source, image_cache, &mut used_keys).await;
                }
            }
            _ => {}
        }
    }
    let avatar = resolve_avatar_image(header, cmd_tx, image_cache, &mut used_keys).await;
    (
        RenderImageSources {
            avatar,
            block_images,
            block_tag_icons,
            block_brand_icons,
            block_labels,
        },
        used_keys,
    )
}

fn render_preview_image_sources(draft: &Draft) -> RenderImageSources {
    let mut image_cache = ImageMemoryCache::default();
    let mut block_images = vec![None; draft.blocks.len()];
    let mut block_tag_icons = vec![None; draft.blocks.len()];
    let mut block_brand_icons = vec![None; draft.blocks.len()];
    let mut block_labels = vec![None; draft.blocks.len()];

    for (idx, block) in draft.blocks.iter().enumerate() {
        let DraftBlock::Attachment {
            kind, reference, ..
        } = block
        else {
            continue;
        };
        match reference {
            MediaReference::RemoteUrl { url } if is_renderable_image(*kind) => {
                if !is_remote_http(url) {
                    block_images[idx] = image_cache.get_or_load_source(url);
                }
            }
            MediaReference::RemoteUrl { url } if is_video_media(*kind) => {
                if !is_remote_http(url) {
                    block_images[idx] = image_cache.get_or_load_video_preview_source(url);
                }
            }
            MediaReference::RemoteUrl { url } => {
                block_labels[idx] = Some(url.clone());
            }
            MediaReference::Blob { .. } => {}
        }
    }

    for (idx, block) in draft.blocks.iter().enumerate() {
        let DraftBlock::JsonCard { raw } = block else {
            continue;
        };
        let view = parse_json_card_view(raw);
        if let Some(source) = view.media_source {
            if !is_remote_http(&source) {
                block_images[idx] = image_cache.get_or_load_source(&source);
            }
        }
        if let Some(source) = view.tag_icon_source {
            if !is_remote_http(&source) {
                block_tag_icons[idx] = image_cache.get_or_load_source(&source);
            }
        }
        if let Some(source) = view.brand_icon_source {
            if !is_remote_http(&source) {
                block_brand_icons[idx] = image_cache.get_or_load_source(&source);
            }
        }
    }

    RenderImageSources {
        avatar: image_cache.get_or_load_source(DEFAULT_AVATAR_PATH),
        block_images,
        block_tag_icons,
        block_brand_icons,
        block_labels,
    }
}

async fn resolve_json_card_media_source(
    source: &str,
    image_cache: &mut ImageMemoryCache,
    used_keys: &mut HashSet<ImageCacheKey>,
) -> Option<ResolvedImage> {
    if is_remote_http(source) {
        let key = ImageCacheKey::Source(source.to_string());
        used_keys.insert(key);
        image_cache.get_or_fetch_remote_source(source).await
    } else {
        let key = ImageCacheKey::Source(source.to_string());
        used_keys.insert(key);
        image_cache.get_or_load_source(source)
    }
}

fn resolve_media_reference_for_image(
    reference: &MediaReference,
    state: &StateView,
    image_cache: &mut ImageMemoryCache,
    used_keys: &mut HashSet<ImageCacheKey>,
) -> Option<ResolvedImage> {
    match reference {
        MediaReference::Blob { blob_id } => {
            debug_log!("media image: blob_id={:?}", blob_id);
            let key = ImageCacheKey::Blob(*blob_id);
            used_keys.insert(key);
            image_cache.get_or_load_blob(state, *blob_id)
        }
        MediaReference::RemoteUrl { url } => {
            debug_log!("media image: remote url={}", url);
            if is_remote_http(url) {
                debug_log!("image load blocked remote url: {}", url);
                return None;
            }
            let key = ImageCacheKey::Source(url.clone());
            used_keys.insert(key);
            image_cache.get_or_load_source(url)
        }
    }
}

fn resolve_media_reference_for_video_preview(
    reference: &MediaReference,
    state: &StateView,
    image_cache: &mut ImageMemoryCache,
    used_keys: &mut HashSet<ImageCacheKey>,
) -> Option<ResolvedImage> {
    match reference {
        MediaReference::Blob { blob_id } => {
            let key = ImageCacheKey::Source(format!("video-preview:blob:{}", id128_hex(blob_id.0)));
            used_keys.insert(key);
            image_cache.get_or_load_video_preview_blob(state, *blob_id)
        }
        MediaReference::RemoteUrl { url } => {
            if is_remote_http(url) {
                debug_log!("video preview load blocked remote url: {}", url);
                return None;
            }
            let key = ImageCacheKey::Source(format!("video-preview:{}", url));
            used_keys.insert(key);
            image_cache.get_or_load_video_preview_source(url)
        }
    }
}

fn resolve_media_reference_for_label(
    reference: &MediaReference,
    _state: &StateView,
) -> Option<String> {
    match reference {
        MediaReference::Blob { .. } => None,
        MediaReference::RemoteUrl { url } => Some(url.clone()),
    }
}

fn format_size_line(size_bytes: u64) -> Option<String> {
    if size_bytes == 0 {
        return None;
    }
    Some(format_bytes(size_bytes))
}

fn format_bytes(size_bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = size_bytes as f64;
    let mut unit_idx = 0usize;
    while size >= 1024.0 && unit_idx + 1 < UNITS.len() {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", size_bytes, UNITS[unit_idx])
    } else if (size - size.round()).abs() < f64::EPSILON {
        format!("{:.0} {}", size, UNITS[unit_idx])
    } else if size >= 10.0 {
        format!("{:.0} {}", size.round(), UNITS[unit_idx])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

fn resolve_face_image(
    id: &str,
    cache: &mut HashMap<String, ResolvedImage>,
) -> Option<ResolvedImage> {
    if let Some(found) = cache.get(id) {
        debug_log!("face cache hit: id={}", id);
        return Some(found.clone());
    }
    let path = Path::new("res").join("face").join(format!("{}.png", id));
    debug_log!("face load: id={} path={}", id, path.display());
    let path_str = path.to_string_lossy();
    let resolved = resolved_image_from_path(&path_str)?;
    cache.insert(id.to_string(), resolved.clone());
    Some(resolved)
}

fn resolve_blob_image(state: &StateView, blob_id: BlobId) -> Option<ResolvedImage> {
    if let Some(entry) = blob_cache::get_entry(blob_id) {
        return Some(ResolvedImage::from_arc(entry.bytes));
    }
    let path = state
        .blobs
        .get(&blob_id)
        .and_then(|meta| meta.persisted_path.clone())?;
    debug_log!("blob image path: blob_id={:?} path={}", blob_id, path);
    let bytes = fs::read(path).ok()?;
    let bytes: Arc<[u8]> = Arc::from(bytes);
    blob_cache::store_arc(
        blob_id,
        bytes.clone(),
        CacheKind::Image,
        CacheRetention::UntilSend,
        None,
    );
    Some(ResolvedImage::from_arc(bytes))
}

async fn resolve_avatar_image(
    header: &HeaderInfo,
    cmd_tx: &mpsc::Sender<Command>,
    image_cache: &mut ImageMemoryCache,
    used_keys: &mut HashSet<ImageCacheKey>,
) -> Option<ResolvedImage> {
    if header.is_anonymous {
        return resolve_default_avatar(image_cache, used_keys);
    }
    if let Some(bytes) = avatar_cache::get_avatar_bytes(&header.user_id) {
        return Some(ResolvedImage::from_arc(bytes));
    }
    if let Some((notify, created)) = avatar_cache::ensure_in_flight(&header.user_id) {
        if created {
            let _ = send_event(
                cmd_tx,
                Event::Media(MediaEvent::AvatarFetchRequested {
                    user_id: header.user_id.clone(),
                }),
            )
            .await;
        }
        if let Some(bytes) =
            avatar_cache::wait_for_avatar(&header.user_id, notify, AVATAR_FETCH_TIMEOUT).await
        {
            return Some(ResolvedImage::from_arc(bytes));
        }
    }
    resolve_default_avatar(image_cache, used_keys)
}

fn resolve_default_avatar(
    image_cache: &mut ImageMemoryCache,
    used_keys: &mut HashSet<ImageCacheKey>,
) -> Option<ResolvedImage> {
    let key = ImageCacheKey::Source(DEFAULT_AVATAR_PATH.to_string());
    used_keys.insert(key);
    image_cache.get_or_load_source(DEFAULT_AVATAR_PATH)
}

fn resolve_source_to_image(source: &str) -> Option<ResolvedImage> {
    if is_remote_http(source) {
        debug_log!("image load blocked remote url: {}", source);
        return None;
    }
    if source.starts_with("data:") {
        debug_log!("image load data url");
        return resolved_image_from_data_url(source);
    }
    if source.starts_with("base64://") {
        debug_log!("image load base64 url");
        return resolved_image_from_base64_url(source);
    }
    if let Some(path) = source.strip_prefix("file://") {
        debug_log!("image load file url: {}", path);
        return resolved_image_from_path(path);
    }
    if source.starts_with("res/") {
        debug_log!("image load embedded path: {}", source);
        return resolved_image_from_path(source);
    }
    if Path::new(source).exists() {
        debug_log!("image load local path: {}", source);
        return resolved_image_from_path(source);
    }
    None
}

async fn fetch_remote_image(source: &str) -> Option<ResolvedImage> {
    debug_log!("image fetch remote url: {}", source);
    let client = reqwest::Client::builder()
        .timeout(CARD_REMOTE_IMAGE_TIMEOUT)
        .user_agent("Mozilla/5.0 OQQWall_RUST renderer")
        .build()
        .ok()?;
    let response = match client.get(source).send().await {
        Ok(response) => response,
        Err(err) => {
            debug_log!("image fetch failed: {} err={}", source, err);
            return None;
        }
    };
    let status = response.status();
    if !status.is_success() {
        debug_log!("image fetch http status: {} status={}", source, status);
        return None;
    }
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            debug_log!("image fetch read failed: {} err={}", source, err);
            return None;
        }
    };
    if bytes.len() > CARD_REMOTE_IMAGE_MAX_BYTES {
        debug_log!(
            "image fetch too large: {} bytes={} max={}",
            source,
            bytes.len(),
            CARD_REMOTE_IMAGE_MAX_BYTES
        );
        return None;
    }
    Some(ResolvedImage::from_bytes(bytes.to_vec()))
}

fn resolve_source_to_video_preview(source: &str) -> Option<ResolvedImage> {
    if let Some(image) = resolve_source_to_image(source).filter(|image| image.width.is_some()) {
        return Some(image);
    }
    let path = source_to_local_path(source)?;
    extract_video_frame_image(&path)
}

fn resolve_blob_video_preview(state: &StateView, blob_id: BlobId) -> Option<ResolvedImage> {
    if let Some(entry) = blob_cache::get_entry(blob_id) {
        let image = ResolvedImage::from_arc(entry.bytes);
        if image.width.is_some() {
            return Some(image);
        }
    }
    let path = state
        .blobs
        .get(&blob_id)
        .and_then(|meta| meta.persisted_path.as_deref())?;
    extract_video_frame_image(Path::new(path))
}

fn source_to_local_path(source: &str) -> Option<PathBuf> {
    if source.starts_with("data:") || source.starts_with("base64://") || is_remote_http(source) {
        return None;
    }
    if let Some(path) = source.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }
    let path = Path::new(source);
    if let Some(resolved) = resolve_res_disk_path(path) {
        return Some(resolved);
    }
    path.exists().then(|| path.to_path_buf())
}

fn extract_video_frame_image(path: &Path) -> Option<ResolvedImage> {
    if !path.exists() {
        return None;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let out_path = std::env::temp_dir().join(format!(
        "oqqwall-video-frame-{}-{}.png",
        std::process::id(),
        stamp
    ));
    let status = ProcessCommand::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(path)
        .arg("-frames:v")
        .arg("1")
        .arg(&out_path)
        .status()
        .ok()?;
    if !status.success() {
        let _ = fs::remove_file(&out_path);
        return None;
    }
    let bytes = fs::read(&out_path).ok()?;
    let _ = fs::remove_file(&out_path);
    Some(ResolvedImage::from_bytes(bytes))
}

fn resolved_image_from_path(path: &str) -> Option<ResolvedImage> {
    let path_obj = Path::new(path);
    if let Some(resolved) = resolve_res_disk_path(path_obj) {
        debug_log!("image load disk: {} -> {}", path, resolved.display());
        let bytes = match fs::read(&resolved) {
            Ok(bytes) => bytes,
            Err(err) => {
                debug_log!("image load disk failed: {} err={}", resolved.display(), err);
                return None;
            }
        };
        return Some(ResolvedImage::from_bytes(bytes));
    }
    debug_log!("image load disk: {}", path);
    let bytes = match fs::read(path_obj) {
        Ok(bytes) => bytes,
        Err(err) => {
            debug_log!("image load disk failed: {} err={}", path_obj.display(), err);
            return None;
        }
    };
    Some(ResolvedImage::from_bytes(bytes))
}

fn resolved_image_from_data_url(url: &str) -> Option<ResolvedImage> {
    let (_mime, bytes) = parse_data_url(url)?;
    Some(ResolvedImage::from_bytes(bytes))
}

fn resolved_image_from_base64_url(url: &str) -> Option<ResolvedImage> {
    let bytes = parse_base64_url(url)?;
    Some(ResolvedImage::from_bytes(bytes))
}

fn image_size_from_bytes(bytes: &[u8]) -> Option<(u32, u32)> {
    if let Ok(size) = imagesize::blob_size(bytes) {
        let width = u32::try_from(size.width).ok()?;
        let height = u32::try_from(size.height).ok()?;
        return Some((width, height));
    }
    let image = image::load_from_memory(bytes).ok()?;
    Some((image.width(), image.height()))
}

fn parse_data_url(source: &str) -> Option<(Option<String>, Vec<u8>)> {
    let payload = source.strip_prefix("data:")?;
    let (meta, data) = payload.split_once(',')?;
    let mime = meta.split(';').next().map(|value| value.to_string());
    let bytes = if meta.contains(";base64") {
        STANDARD.decode(data).ok()?
    } else {
        data.as_bytes().to_vec()
    };
    Some((mime, bytes))
}

fn parse_base64_url(source: &str) -> Option<Vec<u8>> {
    let payload = source.strip_prefix("base64://")?;
    STANDARD.decode(payload).ok()
}

fn is_remote_http(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn extract_filename(url: &str) -> Option<String> {
    let trimmed = url.split('?').next().unwrap_or(url);
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn file_icon_text(name: &str) -> String {
    let ext = name
        .rsplit('.')
        .next()
        .filter(|part| *part != name)
        .unwrap_or("");
    if ext.is_empty() {
        return "FILE".to_string();
    }
    let mut out = ext.to_ascii_uppercase();
    if out.len() > 4 {
        out.truncate(4);
    }
    out
}

fn file_icon_path(name: &str) -> Option<&'static str> {
    let ext = name
        .rsplit('.')
        .next()
        .filter(|part| *part != name)
        .unwrap_or("")
        .to_ascii_lowercase();
    let path = match ext.as_str() {
        "doc" | "docx" => "res/doc.png",
        "pdf" => "res/pdf.png",
        "xls" | "xlsx" | "csv" => "res/xls.png",
        "ppt" | "pptx" => "res/ppt.png",
        "zip" | "7z" => "res/zip.png",
        "rar" => "res/rar.png",
        "txt" | "md" | "log" | "rtf" => "res/txt.png",
        "mp3" | "wav" | "flac" | "m4a" | "ogg" | "aac" => "res/audio.png",
        "mp4" | "mov" | "mkv" | "avi" | "flv" | "wmv" => "res/video.png",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" => {
            "res/image.png"
        }
        "apk" => "res/apk.png",
        "ipa" => "res/ipa.png",
        "dmg" => "res/dmg.png",
        "pkg" => "res/pkg.png",
        "exe" | "msi" => "res/exe.png",
        "psd" | "psb" => "res/ps.png",
        "ai" => "res/ai.png",
        "sketch" => "res/sketch.png",
        "xmind" | "mindmap" => "res/mindmap.png",
        "note" => "res/note.png",
        "pages" => "res/pages.png",
        "key" | "keynote" => "res/keynote.png",
        "numbers" => "res/numbers.png",
        "rs" | "js" | "ts" | "json" | "yaml" | "yml" | "toml" | "go" | "py" | "java" | "c"
        | "cc" | "cpp" | "h" | "hpp" | "cs" | "php" | "rb" | "swift" | "kt" | "kts" | "html"
        | "css" | "scss" | "sh" | "bat" | "ps1" | "sql" => "res/code.png",
        "" => "res/unknown.png",
        _ => "res/unknown.png",
    };
    Some(path)
}

fn media_label(kind: oqqwall_rust_core::MediaKind) -> &'static str {
    match kind {
        oqqwall_rust_core::MediaKind::Image => "Image",
        oqqwall_rust_core::MediaKind::Video => "Video",
        oqqwall_rust_core::MediaKind::File => "File",
        oqqwall_rust_core::MediaKind::Audio => "Audio",
        oqqwall_rust_core::MediaKind::Other => "Attachment",
        oqqwall_rust_core::MediaKind::Sticker => "Image",
    }
}

fn media_icon_text(kind: oqqwall_rust_core::MediaKind) -> String {
    match kind {
        oqqwall_rust_core::MediaKind::Video => "VID".to_string(),
        oqqwall_rust_core::MediaKind::Audio => "AUD".to_string(),
        oqqwall_rust_core::MediaKind::Other => "ATT".to_string(),
        oqqwall_rust_core::MediaKind::Sticker => "IMG".to_string(),
        _ => "FILE".to_string(),
    }
}

fn measure_inline_text_width(
    text: &str,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let emoji_size = font_size;
    let mut width = 0u32;
    let mut segment = String::new();
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if let Some((_glyph_id, consumed)) = emoji_cache.match_sequence(text, idx) {
            if !segment.is_empty() {
                width = width.saturating_add(measurer.measure_text_width(
                    &segment,
                    font_size,
                    font_weight,
                ));
                segment.clear();
            }
            width = width.saturating_add(emoji_size);
            idx += consumed;
            continue;
        }
        let ch = text[idx..].chars().next().unwrap();
        if should_skip_emoji_char(ch) {
            idx += ch.len_utf8();
            continue;
        }
        if emoji_cache.is_emoji_char(ch) {
            if !segment.is_empty() {
                width = width.saturating_add(measurer.measure_text_width(
                    &segment,
                    font_size,
                    font_weight,
                ));
                segment.clear();
            }
            width = width.saturating_add(emoji_size);
        } else {
            segment.push(ch);
        }
        idx += ch.len_utf8();
    }
    if !segment.is_empty() {
        width = width.saturating_add(measurer.measure_text_width(&segment, font_size, font_weight));
    }
    width
}

fn truncate_text(
    text: &str,
    max_width: u32,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        let width = measure_inline_text_width(&out, font_size, font_weight, measurer, emoji_cache);
        if width > max_width {
            out.pop();
            let ellipsis_width = measurer.measure_text_width("...", font_size, font_weight);
            let base_width =
                measure_inline_text_width(&out, font_size, font_weight, measurer, emoji_cache);
            if base_width + ellipsis_width <= max_width && !out.is_empty() {
                out.push_str("...");
            }
            return out;
        }
    }
    out
}

fn limit_lines(
    mut lines: Vec<String>,
    max_lines: usize,
    max_width: u32,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> Vec<String> {
    if lines.len() <= max_lines {
        return lines;
    }
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        let padded = format!("{}...", last);
        *last = truncate_text(
            &padded,
            max_width,
            font_size,
            font_weight,
            measurer,
            emoji_cache,
        );
    }
    lines
}

fn wrap_inline_text(
    text: &str,
    max_width: u32,
    font_size: u32,
    face_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> Vec<InlineLine> {
    let mut lines = Vec::new();
    let emoji_size = face_size;
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(InlineLine {
                runs: Vec::new(),
                width: 0,
            });
            continue;
        }
        let atoms = parse_inline_atoms(raw_line, emoji_cache, true);
        let mut current: Vec<InlineAtom> = Vec::new();
        let mut segment_text = String::new();
        let mut base_width = 0u32;
        let mut last_break: Option<usize> = None;
        for atom in atoms {
            let is_break = inline_atom_is_break(&atom);
            match &atom {
                InlineAtom::Char(ch) => segment_text.push(*ch),
                InlineAtom::Face(_) => {
                    if !segment_text.is_empty() {
                        base_width = base_width.saturating_add(measurer.measure_text_width(
                            &segment_text,
                            font_size,
                            font_weight,
                        ));
                        segment_text.clear();
                    }
                    base_width = base_width.saturating_add(face_size);
                }
                InlineAtom::Emoji(_) => {
                    if !segment_text.is_empty() {
                        base_width = base_width.saturating_add(measurer.measure_text_width(
                            &segment_text,
                            font_size,
                            font_weight,
                        ));
                        segment_text.clear();
                    }
                    base_width = base_width.saturating_add(emoji_size);
                }
            }
            current.push(atom);
            let segment_width = measurer.measure_text_width(&segment_text, font_size, font_weight);
            let current_width = base_width.saturating_add(segment_width);
            if current_width > max_width && current.len() > 1 {
                if let Some(break_idx) = last_break {
                    let mut line_atoms = current[..break_idx].to_vec();
                    trim_inline_trailing_spaces(&mut line_atoms);
                    lines.push(build_inline_line(
                        &line_atoms,
                        font_size,
                        face_size,
                        emoji_size,
                        font_weight,
                        measurer,
                    ));
                    let mut remainder = current[break_idx..].to_vec();
                    trim_inline_leading_spaces(&mut remainder);
                    current = remainder;
                } else {
                    let last_atom = current.pop().unwrap();
                    let line_atoms = current;
                    lines.push(build_inline_line(
                        &line_atoms,
                        font_size,
                        face_size,
                        emoji_size,
                        font_weight,
                        measurer,
                    ));
                    current = vec![last_atom];
                }
                let (next_segment_text, next_base_width) = rebuild_inline_measure_state(
                    &current,
                    font_size,
                    face_size,
                    emoji_size,
                    font_weight,
                    measurer,
                );
                segment_text = next_segment_text;
                base_width = next_base_width;
                last_break = None;
                if let Some(last_atom) = current.last() {
                    if inline_atom_is_break(last_atom) {
                        last_break = Some(current.len());
                    }
                }
            }
            if is_break {
                last_break = Some(current.len());
            }
        }
        if !current.is_empty() {
            lines.push(build_inline_line(
                &current,
                font_size,
                face_size,
                emoji_size,
                font_weight,
                measurer,
            ));
        }
    }
    if lines.is_empty() {
        lines.push(InlineLine {
            runs: Vec::new(),
            width: 0,
        });
    }
    lines
}

fn rebuild_inline_measure_state(
    atoms: &[InlineAtom],
    font_size: u32,
    face_size: u32,
    emoji_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> (String, u32) {
    let mut segment_text = String::new();
    let mut base_width = 0u32;
    for atom in atoms {
        match atom {
            InlineAtom::Char(ch) => segment_text.push(*ch),
            InlineAtom::Face(_) => {
                if !segment_text.is_empty() {
                    base_width = base_width.saturating_add(measurer.measure_text_width(
                        &segment_text,
                        font_size,
                        font_weight,
                    ));
                    segment_text.clear();
                }
                base_width = base_width.saturating_add(face_size);
            }
            InlineAtom::Emoji(_) => {
                if !segment_text.is_empty() {
                    base_width = base_width.saturating_add(measurer.measure_text_width(
                        &segment_text,
                        font_size,
                        font_weight,
                    ));
                    segment_text.clear();
                }
                base_width = base_width.saturating_add(emoji_size);
            }
        }
    }
    (segment_text, base_width)
}

fn wrap_text(
    text: &str,
    max_width: u32,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
    emoji_cache: &EmojiRenderCache,
) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current: Vec<char> = Vec::new();
        let mut current_text = String::new();
        let mut last_break: Option<usize> = None;
        for ch in raw_line.chars() {
            current.push(ch);
            current_text.push(ch);
            let current_width = measure_inline_text_width(
                &current_text,
                font_size,
                font_weight,
                measurer,
                emoji_cache,
            );
            if current_width > max_width && current.len() > 1 {
                if let Some(break_idx) = last_break {
                    let line: String = current[..break_idx].iter().collect();
                    lines.push(line.trim_end().to_string());
                    let mut remainder: Vec<char> = current[break_idx..].iter().copied().collect();
                    while remainder
                        .first()
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
                    {
                        remainder.remove(0);
                    }
                    current = remainder;
                } else {
                    let last = current.pop().unwrap();
                    let line: String = current.iter().collect();
                    lines.push(line);
                    current.clear();
                    current.push(last);
                }
                current_text = current.iter().collect();
                last_break = None;
                if let Some(last_ch) = current.last() {
                    if is_break_char(*last_ch) {
                        last_break = Some(current.len());
                    }
                }
            }
            if is_break_char(ch) {
                last_break = Some(current.len());
            }
        }
        if !current.is_empty() {
            lines.push(current.iter().collect());
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn parse_inline_atoms(
    line: &str,
    emoji_cache: &EmojiRenderCache,
    parse_faces: bool,
) -> Vec<InlineAtom> {
    let mut atoms = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if parse_faces && bytes[idx] == b'[' {
            let rest = &line[idx..];
            if rest.starts_with("[[face:") {
                let after_prefix = idx + "[[face:".len();
                if after_prefix <= line.len() {
                    if let Some(close) = line[after_prefix..].find("]]") {
                        let face_id = &line[after_prefix..after_prefix + close];
                        if !face_id.is_empty() && face_id.chars().all(|c| c.is_ascii_digit()) {
                            atoms.push(InlineAtom::Face(face_id.to_string()));
                            idx = after_prefix + close + 2;
                            continue;
                        }
                    }
                }
            } else if rest.starts_with("[face:") {
                let after_prefix = idx + "[face:".len();
                if after_prefix <= line.len() {
                    if let Some(close) = line[after_prefix..].find(']') {
                        let face_id = &line[after_prefix..after_prefix + close];
                        if !face_id.is_empty() && face_id.chars().all(|c| c.is_ascii_digit()) {
                            atoms.push(InlineAtom::Face(face_id.to_string()));
                            idx = after_prefix + close + 1;
                            continue;
                        }
                    }
                }
            }
        }
        if let Some((glyph_id, consumed)) = emoji_cache.match_sequence(line, idx) {
            atoms.push(InlineAtom::Emoji(glyph_id));
            idx += consumed;
            continue;
        }
        let ch = line[idx..].chars().next().unwrap();
        if should_skip_emoji_char(ch) {
            idx += ch.len_utf8();
            continue;
        }
        if emoji_cache.is_emoji_char(ch) {
            if let Some(glyph_id) = emoji_cache.glyph_id_for_char(ch) {
                atoms.push(InlineAtom::Emoji(glyph_id));
            } else {
                atoms.push(InlineAtom::Char(ch));
            }
            idx += ch.len_utf8();
            continue;
        }
        atoms.push(InlineAtom::Char(ch));
        idx += ch.len_utf8();
    }
    atoms
}

fn inline_atom_is_break(atom: &InlineAtom) -> bool {
    match atom {
        InlineAtom::Char(ch) => is_break_char(*ch),
        InlineAtom::Face(_) | InlineAtom::Emoji(_) => false,
    }
}

fn inline_atom_is_whitespace(atom: &InlineAtom) -> bool {
    matches!(atom, InlineAtom::Char(ch) if ch.is_whitespace())
}

fn trim_inline_leading_spaces(atoms: &mut Vec<InlineAtom>) {
    while atoms
        .first()
        .map(inline_atom_is_whitespace)
        .unwrap_or(false)
    {
        atoms.remove(0);
    }
}

fn trim_inline_trailing_spaces(atoms: &mut Vec<InlineAtom>) {
    while atoms.last().map(inline_atom_is_whitespace).unwrap_or(false) {
        atoms.pop();
    }
}

fn build_inline_line(
    atoms: &[InlineAtom],
    font_size: u32,
    face_size: u32,
    emoji_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> InlineLine {
    let mut runs = Vec::new();
    let mut current = String::new();
    let mut width = 0u32;
    for atom in atoms {
        match atom {
            InlineAtom::Char(ch) => {
                current.push(*ch);
            }
            InlineAtom::Face(id) => {
                if !current.is_empty() {
                    width = width.saturating_add(measurer.measure_text_width(
                        &current,
                        font_size,
                        font_weight,
                    ));
                    runs.push(InlineRun::Text(current.clone()));
                    current.clear();
                }
                runs.push(InlineRun::Face { id: id.clone() });
                width = width.saturating_add(face_size);
            }
            InlineAtom::Emoji(glyph_id) => {
                if !current.is_empty() {
                    width = width.saturating_add(measurer.measure_text_width(
                        &current,
                        font_size,
                        font_weight,
                    ));
                    runs.push(InlineRun::Text(current.clone()));
                    current.clear();
                }
                runs.push(InlineRun::Emoji {
                    glyph_id: *glyph_id,
                });
                width = width.saturating_add(emoji_size);
            }
        }
    }
    if !current.is_empty() {
        width = width.saturating_add(measurer.measure_text_width(&current, font_size, font_weight));
        runs.push(InlineRun::Text(current));
    }
    InlineLine { runs, width }
}

fn is_break_char(ch: char) -> bool {
    if ch.is_whitespace() {
        return true;
    }
    matches!(
        ch,
        '-' | '/' | '_' | '.' | ',' | ';' | ':' | '?' | '!' | '，' | '。' | '；' | '、' | '：'
    )
}

fn should_skip_emoji_char(ch: char) -> bool {
    matches!(ch, '\u{FE0F}' | '\u{FE0E}' | '\u{200D}' | '\u{20E3}')
        || (0xE0020..=0xE007F).contains(&(ch as u32))
}

fn persist_blob(
    root: &Path,
    kind_dir: &str,
    ext: &str,
    blob_id: oqqwall_rust_core::BlobId,
    bytes: &[u8],
) -> Result<(String, u64), String> {
    let dir = root.join(kind_dir);
    fs::create_dir_all(&dir).map_err(|err| format!("create blob dir failed: {}", err))?;
    let filename = format!("{}.{}", id128_hex(blob_id.0), ext);
    let path = dir.join(filename);
    let tmp_path = dir.join(format!("{}.{}.tmp", id128_hex(blob_id.0), ext));
    fs::write(&tmp_path, bytes).map_err(|err| format!("write blob failed: {}", err))?;
    if let Err(err) = fs::rename(&tmp_path, &path) {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            fs::remove_file(&path).map_err(|err| format!("cleanup blob failed: {}", err))?;
            fs::rename(&tmp_path, &path).map_err(|err| format!("rename blob failed: {}", err))?;
        } else {
            return Err(format!("rename blob failed: {}", err));
        }
    }
    let size_bytes = bytes.len() as u64;
    Ok((path.to_string_lossy().to_string(), size_bytes))
}

fn render_blob_id(post_id: PostId, page_index: usize) -> oqqwall_rust_core::BlobId {
    if page_index == 0 {
        return derive_blob_id(&[&post_id.to_be_bytes(), b"png"]);
    }
    let page_index = page_index.to_string();
    derive_blob_id(&[&post_id.to_be_bytes(), b"png-page", page_index.as_bytes()])
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

async fn send_event(cmd_tx: &mpsc::Sender<Command>, event: Event) -> Result<(), String> {
    cmd_tx
        .send(Command::DriverEvent(event))
        .await
        .map_err(|_| "driver event send failed".to_string())
}

async fn send_render_failed(
    cmd_tx: &mpsc::Sender<Command>,
    post_id: PostId,
    attempt: u32,
    error: String,
) -> Result<(), String> {
    let retry_at_ms = now_ms().saturating_add(render_retry_delay_ms(attempt));
    let event = RenderEvent::RenderFailed {
        post_id,
        attempt,
        retry_at_ms,
        error,
    };
    send_event(cmd_tx, Event::Render(event)).await
}

fn render_retry_delay_ms(attempt: u32) -> i64 {
    let base = 10_000i64;
    let max = 300_000i64;
    let shift = attempt.saturating_sub(1).min(10);
    let delay = base.saturating_mul(1_i64 << shift);
    delay.min(max)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone)]
struct FontBytes {
    path: PathBuf,
    bytes: Vec<u8>,
}

fn init_font_bytes_cache(font_dir: &Path) {
    if FONT_BYTES_CACHE.get().is_some() {
        return;
    }
    let mut fonts = Vec::new();
    if font_dir.exists() {
        if let Ok(entries) = fs::read_dir(font_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
                    continue;
                }
                match fs::read(&path) {
                    Ok(bytes) => fonts.push(FontBytes { path, bytes }),
                    Err(_err) => {
                        debug_log!("font cache read failed: {} err={}", path.display(), _err);
                    }
                }
            }
        }
    }
    debug_log!("font cache: disk_fonts={}", fonts.len());
    let _ = FONT_BYTES_CACHE.set(fonts);
}

fn font_bytes_cache() -> Option<&'static [FontBytes]> {
    FONT_BYTES_CACHE.get().map(|fonts| fonts.as_slice())
}

fn build_font_collection(font_dir: &Path) -> FontCollection {
    let mut asset_mgr = TypefaceFontProvider::new();
    let sys_mgr = skia_safe::FontMgr::new();
    debug_log!("font init: font_dir={}", font_dir.display());
    let _embedded_count = register_embedded_fonts(&mut asset_mgr, &sys_mgr);
    debug_log!("font init: embedded_fonts={}", _embedded_count);
    if let Some(fonts) = font_bytes_cache() {
        let mut _disk_count = 0usize;
        for font in fonts {
            let ext = font
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
                debug_log!("font skip non-ttf/otf: {}", font.path.display());
                continue;
            }
            if let Some(tf) = sys_mgr.new_from_data(&font.bytes, 0) {
                let alias = font_alias_for_path(&font.path);
                let _family = tf.family_name();
                debug_log!(
                    "font disk load: path={} family={} alias={:?}",
                    font.path.display(),
                    _family,
                    alias
                );
                register_typeface_with_alias(&mut asset_mgr, tf, alias);
                _disk_count += 1;
            } else {
                debug_log!("font disk load failed: {}", font.path.display());
            }
        }
        debug_log!("font init: disk_fonts={}", _disk_count);
    } else if font_dir.exists() {
        if let Ok(entries) = fs::read_dir(font_dir) {
            let mut _disk_count = 0usize;
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
                    debug_log!("font skip non-ttf/otf: {}", path.display());
                    continue;
                }
                if let Ok(bytes) = fs::read(&path) {
                    if let Some(tf) = sys_mgr.new_from_data(&bytes, 0) {
                        let alias = font_alias_for_path(&path);
                        let _family = tf.family_name();
                        debug_log!(
                            "font disk load: path={} family={} alias={:?}",
                            path.display(),
                            _family,
                            alias
                        );
                        register_typeface_with_alias(&mut asset_mgr, tf, alias);
                        _disk_count += 1;
                    } else {
                        debug_log!("font disk load failed: {}", path.display());
                    }
                } else {
                    debug_log!("font disk read failed: {}", path.display());
                }
            }
            debug_log!("font init: disk_fonts={}", _disk_count);
        }
    } else {
        debug_log!("font dir not found: {}", font_dir.display());
    }
    let mut ordered_mgr = OrderedFontMgr::new();
    ordered_mgr.append(asset_mgr.clone());
    let mut fc = FontCollection::new();
    fc.set_asset_font_manager(Some(asset_mgr.into()));
    fc.set_default_font_manager_and_family_names(Some(ordered_mgr.into()), &FONT_FAMILIES);
    fc.disable_font_fallback();
    debug_log!(
        "font collection: managers={} fallback_enabled={}",
        fc.font_managers_count(),
        fc.font_fallback_enabled()
    );
    fc
}

fn register_embedded_fonts(
    asset_mgr: &mut TypefaceFontProvider,
    sys_mgr: &skia_safe::FontMgr,
) -> usize {
    let mut count = 0usize;
    for entry in embedded_resources::RESOURCES {
        if !entry.path.starts_with("fonts/") {
            continue;
        }
        let path = Path::new(entry.path);
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
            debug_log!("embedded font skip non-ttf/otf: {}", entry.path);
            continue;
        }
        if let Some(tf) = sys_mgr.new_from_data(entry.bytes, 0usize) {
            let alias = font_alias_for_path(path);
            let _family = tf.family_name();
            debug_log!(
                "embedded font load: path={} family={} alias={:?} bytes={}",
                entry.path,
                _family,
                alias,
                entry.bytes.len()
            );
            register_typeface_with_alias(asset_mgr, tf, alias);
            count += 1;
        } else {
            debug_log!("embedded font load failed: {}", entry.path);
        }
    }
    count
}

fn register_typeface_with_alias(
    asset_mgr: &mut TypefaceFontProvider,
    typeface: Typeface,
    alias: Option<&'static str>,
) {
    if let Some(alias) = alias {
        asset_mgr.register_typeface(typeface.clone(), Some(alias));
    }
    asset_mgr.register_typeface(typeface, None);
}

fn font_alias_for_path(_path: &Path) -> Option<&'static str> {
    None
}

fn resolve_res_disk_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    if let Ok(rel) = path.strip_prefix("res") {
        return Some(resolve_res_dir().join(rel));
    }
    None
}

fn read_res_relative_bytes(relative: &str) -> Option<Arc<[u8]>> {
    let path = resolve_res_dir().join(relative);
    let bytes = fs::read(&path).ok()?;
    Some(Arc::from(bytes))
}

fn validate_renderer_resources() -> Result<(), String> {
    let res_dir = resolve_res_dir();
    if !res_dir.is_dir() {
        return Err(format!(
            "资源目录不存在: {}。请将资源包解压到与程序同级的 `res/`，或设置 OQQWALL_RES_DIR；也可直接把 `OQQWall_RUST-res*.tar.gz` / `res*.tar.gz`（含 .tgz/.tar）放在程序同级目录，程序会先做 SHA256 校验再自动解压。",
            res_dir.display()
        ));
    }

    let mut missing = Vec::new();
    for rel in REQUIRED_RES_FILES {
        let path = res_dir.join(rel);
        if !path.is_file() {
            missing.push(path.display().to_string());
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "资源文件缺失，请补齐资源包: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

fn resolve_res_dir() -> PathBuf {
    if let Ok(res_dir) = std::env::var("OQQWALL_RES_DIR") {
        let resolved = PathBuf::from(res_dir);
        debug_log!("res dir from env: {}", resolved.display());
        return resolved;
    }
    if let Some(exe_res_dir) = resolve_res_dir_from_exe() {
        debug_log!("res dir from exe: {}", exe_res_dir.display());
        return exe_res_dir;
    }
    let cwd_candidate = PathBuf::from("res");
    if cwd_candidate.exists() {
        debug_log!("res dir from cwd: {}", cwd_candidate.display());
        return cwd_candidate;
    }
    debug_log!("res dir fallback: {}", cwd_candidate.display());
    cwd_candidate
}

fn resolve_res_dir_from_exe() -> Option<PathBuf> {
    static RES_FROM_EXE: OnceLock<Option<PathBuf>> = OnceLock::new();
    RES_FROM_EXE
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let exe_dir = exe.parent()?;
            let exe_res_dir = exe_dir.join("res");
            if exe_res_dir.exists() {
                return Some(exe_res_dir);
            }

            if let Some(archive) = find_res_archive_in_dir(exe_dir) {
                debug_log!(
                    "res dir missing near executable, try extract archive: {}",
                    archive.display()
                );
                if let Err(err) = verify_res_archive_sha256(&archive) {
                    debug_log!("res archive sha256 verify failed: {}", err);
                    return None;
                }
                match extract_res_archive(exe_dir, &archive) {
                    Ok(()) => {
                        if !exe_res_dir.exists() {
                            if let Err(err) = fs::create_dir_all(&exe_res_dir) {
                                debug_log!(
                                    "res auto-create failed: path={} err={}",
                                    exe_res_dir.display(),
                                    err
                                );
                            }
                        }
                        if exe_res_dir.exists() {
                            debug_log!(
                                "res dir prepared by archive extraction: {}",
                                exe_res_dir.display()
                            );
                            return Some(exe_res_dir);
                        }
                        debug_log!(
                            "archive extracted but res dir still missing: {}",
                            exe_res_dir.display()
                        );
                    }
                    Err(err) => {
                        debug_log!("res archive extract failed: {}", err);
                    }
                }
            }

            let fallback_candidates = [
                exe_dir.join("..").join("res"),
                exe_dir.join("..").join("..").join("res"),
            ];
            for candidate in fallback_candidates {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            None
        })
        .clone()
}

fn find_res_archive_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut named_candidates: Vec<PathBuf> = Vec::new();
    let mut generic_candidates: Vec<PathBuf> = Vec::new();
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !(name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".tar")) {
            continue;
        }
        if name == "OQQWall_RUST-res.tar.gz"
            || name == "OQQWall_RUST-res.tgz"
            || name == "OQQWall_RUST-res.tar"
            || name.starts_with("OQQWall_RUST-res-")
        {
            named_candidates.push(path);
            continue;
        }
        if name == "res.tar.gz" || name == "res.tgz" || name == "res.tar" || name.contains("res") {
            generic_candidates.push(path);
        }
    }

    named_candidates.sort();
    generic_candidates.sort();
    named_candidates
        .into_iter()
        .rev()
        .next()
        .or_else(|| generic_candidates.into_iter().rev().next())
}

fn extract_res_archive(exe_dir: &Path, archive: &Path) -> Result<(), String> {
    let archive_name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid archive filename: {}", archive.display()))?;
    let mut cmd = ProcessCommand::new("tar");
    if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        cmd.arg("-xzf");
    } else if archive_name.ends_with(".tar") {
        cmd.arg("-xf");
    } else {
        return Err(format!(
            "unsupported archive format for auto extract: {}",
            archive.display()
        ));
    }
    let status = cmd
        .arg(archive)
        .arg("-C")
        .arg(exe_dir)
        .status()
        .map_err(|err| format!("run tar failed: archive={} err={}", archive.display(), err))?;
    if !status.success() {
        return Err(format!(
            "tar exited with non-zero status: archive={} status={}",
            archive.display(),
            status
        ));
    }
    Ok(())
}

fn verify_res_archive_sha256(archive: &Path) -> Result<(), String> {
    let archive_name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid archive filename: {}", archive.display()))?;
    let expected = if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        embedded_resources::RES_ARCHIVE_SHA256_TAR_GZ
    } else if archive_name.ends_with(".tar") {
        embedded_resources::RES_ARCHIVE_SHA256_TAR
    } else {
        return Err(format!(
            "unsupported archive format for sha256 verify: {}",
            archive.display()
        ));
    };
    if expected.is_empty() {
        return Err(
            "compiled-in resource sha256 is empty; rebuild with a complete res/ directory"
                .to_string(),
        );
    }
    let actual = sha256_file_hex(archive)?;
    debug_log!(
        "res archive sha256 check: archive={} expected={} actual={}",
        archive.display(),
        expected,
        actual
    );
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "sha256 mismatch: archive={} expected={} actual={}",
            archive.display(),
            expected,
            actual
        ));
    }
    debug_log!("res archive sha256 verified: {}", archive.display());
    Ok(())
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("open {} failed: {}", path.display(), err))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|err| format!("read {} failed: {}", path.display(), err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(nibble_to_hex(b >> 4));
        out.push(nibble_to_hex(b & 0x0f));
    }
    out
}

fn nibble_to_hex(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '0',
    }
}

fn resolve_font_dir() -> PathBuf {
    resolve_res_dir().join("fonts")
}

async fn render_png_async(
    draft: &Draft,
    header: &HeaderInfo,
    image_sources: &RenderImageSources,
    config: &RendererRuntimeConfig,
) -> Result<Vec<Vec<u8>>, String> {
    let draft = draft.clone();
    let header = header.clone();
    let image_sources = image_sources.clone();
    let config = config.clone();
    tokio::task::spawn_blocking(move || render_png_pages(&draft, &header, &image_sources, &config))
        .await
        .map_err(|err| format!("png task failed: {}", err))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_png_pages_splits_long_text() {
        let draft = Draft {
            blocks: (0..80)
                .map(|idx| DraftBlock::Paragraph {
                    text: format!("forward line {idx}: content must stay rendered"),
                })
                .collect(),
        };
        let header = HeaderInfo::from(RenderPreviewHeader::default());
        let image_sources = RenderImageSources {
            avatar: None,
            block_images: Vec::new(),
            block_tag_icons: Vec::new(),
            block_brand_icons: Vec::new(),
            block_labels: Vec::new(),
        };
        let config = RendererRuntimeConfig {
            max_height_px: 420,
            ..RendererRuntimeConfig::default()
        };

        let pages = render_png_pages(&draft, &header, &image_sources, &config).unwrap();

        assert!(
            pages.len() > 1,
            "expected multiple pages, got {}",
            pages.len()
        );
        assert!(
            pages
                .iter()
                .all(|page| page.starts_with(b"\x89PNG\r\n\x1a\n"))
        );
    }

    #[test]
    fn render_preview_video_uses_image_like_preview_source() {
        let draft = Draft {
            blocks: vec![DraftBlock::Attachment {
                kind: MediaKind::Video,
                name: Some("clip.mp4".to_string()),
                reference: MediaReference::RemoteUrl {
                    url: DEFAULT_AVATAR_PATH.to_string(),
                },
                size_bytes: None,
            }],
        };

        let image_sources = render_preview_image_sources(&draft);

        let image = image_sources.block_images[0]
            .as_ref()
            .expect("video should resolve an image-like preview source");
        assert!(image.has_bytes());
        assert!(image.width.zip(image.height).is_some());
        assert!(image_sources.block_labels[0].is_none());

        let pages = render_png_pages(
            &draft,
            &HeaderInfo::from(RenderPreviewHeader::default()),
            &image_sources,
            &RendererRuntimeConfig::default(),
        )
        .unwrap();
        assert!(pages[0].starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn local_progressive_jpeg_fixture_resolves_size() {
        let path = Path::new("data/render-fixtures/latest-post/assets/asset_001.jpg");
        if !path.exists() {
            return;
        }
        let bytes = fs::read(path).expect("read fixture image");

        assert_eq!(image_size_from_bytes(&bytes), Some((1920, 1080)));
        assert!(transcode_image_to_png(&bytes).is_some());
    }

    #[test]
    fn video_preview_extracts_first_frame_from_file_url_when_ffmpeg_is_available() {
        let ffmpeg_available = ProcessCommand::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !ffmpeg_available {
            return;
        }

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let temp_dir = std::env::temp_dir().join(format!(
            "oqqwall-video-preview-test-{}-{}",
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let video_path = temp_dir.join("clip.avi");

        let created = ProcessCommand::new("ffmpeg")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("color=c=red:s=16x12:d=0.2")
            .arg("-frames:v")
            .arg("1")
            .arg("-c:v")
            .arg("mjpeg")
            .arg(&video_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !created {
            let _ = fs::remove_dir_all(&temp_dir);
            return;
        }

        let file_url = format!("file://{}", video_path.to_string_lossy());
        let image = resolve_source_to_video_preview(&file_url);
        let _ = fs::remove_dir_all(&temp_dir);

        let image = image.expect("video preview should extract first frame");
        assert_eq!(image.width, Some(16));
        assert_eq!(image.height, Some(12));
    }

    #[test]
    fn embedded_forward_blocks_expand_recursively_inside_forward_items() {
        let draft = Draft {
            blocks: vec![DraftBlock::Forward {
                items: vec![ForwardItem {
                    sender_name: Some("Outer".to_string()),
                    blocks: vec![
                        DraftBlock::Paragraph {
                            text: "before".to_string(),
                        },
                        DraftBlock::Forward {
                            items: vec![ForwardItem {
                                sender_name: Some("Inner".to_string()),
                                blocks: vec![DraftBlock::Paragraph {
                                    text: "deep content".to_string(),
                                }],
                            }],
                        },
                        DraftBlock::Paragraph {
                            text: "after".to_string(),
                        },
                    ],
                }],
            }],
        };

        let normalized = normalize_embedded_forward_draft(&draft);

        let DraftBlock::Forward { items } = &normalized.blocks[0] else {
            panic!("top-level forward should stay a forward block");
        };
        let blocks = &items[0].blocks;
        let text = blocks
            .iter()
            .filter_map(|block| match block {
                DraftBlock::Paragraph { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text, vec!["before", "after"]);

        let nested_items = blocks
            .iter()
            .find_map(|block| match block {
                DraftBlock::Forward { items } => Some(items),
                _ => None,
            })
            .expect("nested forward should keep its container layout");
        let nested_text = nested_items[0]
            .blocks
            .iter()
            .filter_map(|block| match block {
                DraftBlock::Paragraph { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(nested_text, vec!["deep content"]);
    }
}
