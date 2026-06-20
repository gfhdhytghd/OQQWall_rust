use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::{Map, Value};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use oqqwall_rust_drivers::shortcut::{
    is_builtin_review_command_name, validate_global_shortcut_definition,
    validate_review_shortcut_definition, validate_shortcut_name,
};

#[derive(Clone, Copy)]
enum FieldKind {
    Text,
    Bool {
        default: bool,
    },
    StringList,
    #[allow(dead_code)]
    PairList {
        left: &'static str,
        right: &'static str,
    },
    MapList,
    AdminList,
}

#[derive(Clone, Copy)]
struct FieldSpec {
    key: &'static str,
    kind: FieldKind,
    hint: &'static str,
    aliases: &'static [&'static str],
}

const COMMON_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "web_api.enabled",
        kind: FieldKind::Bool { default: false },
        hint: "Enable root-token API service",
        aliases: &["use_web_review"],
    },
    FieldSpec {
        key: "web_api.port",
        kind: FieldKind::Text,
        hint: "Web API port",
        aliases: &["web_review_port"],
    },
    FieldSpec {
        key: "web_api.root_token",
        kind: FieldKind::Text,
        hint: "Web API root token",
        aliases: &["api_token", "token"],
    },
    FieldSpec {
        key: "webview.enabled",
        kind: FieldKind::Bool { default: false },
        hint: "Enable webview review frontend",
        aliases: &[],
    },
    FieldSpec {
        key: "webview.host",
        kind: FieldKind::Text,
        hint: "Webview bind host (e.g. 127.0.0.1 or 0.0.0.0)",
        aliases: &[],
    },
    FieldSpec {
        key: "webview.port",
        kind: FieldKind::Text,
        hint: "Webview port",
        aliases: &[],
    },
    FieldSpec {
        key: "webview.session_ttl_sec",
        kind: FieldKind::Text,
        hint: "Webview session TTL (seconds)",
        aliases: &[],
    },
    FieldSpec {
        key: "telemetry.enabled",
        kind: FieldKind::Bool { default: true },
        hint: "Enable submission telemetry collection",
        aliases: &[],
    },
    FieldSpec {
        key: "telemetry.local_dir",
        kind: FieldKind::Text,
        hint: "Telemetry local storage directory",
        aliases: &[],
    },
    FieldSpec {
        key: "telemetry.upload_enabled",
        kind: FieldKind::Bool { default: true },
        hint: "Enable telemetry batch upload",
        aliases: &[],
    },
    FieldSpec {
        key: "telemetry.upload_interval_sec",
        kind: FieldKind::Text,
        hint: "Telemetry upload interval in seconds",
        aliases: &[],
    },
    FieldSpec {
        key: "telemetry.upload_batch_size",
        kind: FieldKind::Text,
        hint: "Telemetry upload batch size (currently fixed at 20)",
        aliases: &[],
    },
    FieldSpec {
        key: "telemetry.max_append_messages",
        kind: FieldKind::Text,
        hint: "Max appended off-topic messages for telemetry",
        aliases: &[],
    },
    FieldSpec {
        key: "process_waittime_sec",
        kind: FieldKind::Text,
        hint: "Processing wait timeout in seconds",
        aliases: &["process_waittime", "process_waittime_ms"],
    },
    FieldSpec {
        key: "min_interval_ms",
        kind: FieldKind::Text,
        hint: "Minimum send interval in milliseconds",
        aliases: &["min_interval_sec"],
    },
    FieldSpec {
        key: "max_image_number_one_post",
        kind: FieldKind::Text,
        hint: "Max images per post",
        aliases: &["max_images_per_post"],
    },
    FieldSpec {
        key: "send_timeout_ms",
        kind: FieldKind::Text,
        hint: "Send timeout in milliseconds",
        aliases: &["send_timeout", "send_timeout_sec"],
    },
    FieldSpec {
        key: "send_max_attempts",
        kind: FieldKind::Text,
        hint: "Max send attempts",
        aliases: &["max_send_attempts", "max_send_attempt"],
    },
    FieldSpec {
        key: "tz_offset_minutes",
        kind: FieldKind::Text,
        hint: "Timezone offset in minutes",
        aliases: &[],
    },
    FieldSpec {
        key: "max_cache_mb",
        kind: FieldKind::Text,
        hint: "Blob cache size in MB",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_base_url",
        kind: FieldKind::Text,
        hint: "NapCat reverse WS base URL",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_access_token",
        kind: FieldKind::Text,
        hint: "NapCat access token",
        aliases: &[],
    },
    FieldSpec {
        key: "at_unprived_sender",
        kind: FieldKind::Bool { default: false },
        hint: "Mention sender when their space is private",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_request_window_sec",
        kind: FieldKind::Text,
        hint: "Friend request window in seconds",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_add_message",
        kind: FieldKind::Text,
        hint: "Friend add message override",
        aliases: &[],
    },
];

const GROUP_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "mangroupid",
        kind: FieldKind::Text,
        hint: "Audit group id (mangroupid)",
        aliases: &[],
    },
    FieldSpec {
        key: "accounts",
        kind: FieldKind::StringList,
        hint: "Account ids (first is primary)",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_base_url",
        kind: FieldKind::Text,
        hint: "NapCat reverse WS base URL override",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_access_token",
        kind: FieldKind::Text,
        hint: "NapCat access token override",
        aliases: &[],
    },
    FieldSpec {
        key: "process_waittime_sec",
        kind: FieldKind::Text,
        hint: "Processing wait timeout in seconds",
        aliases: &["process_waittime", "process_waittime_ms"],
    },
    FieldSpec {
        key: "min_interval_ms",
        kind: FieldKind::Text,
        hint: "Minimum send interval in milliseconds",
        aliases: &["min_interval_sec"],
    },
    FieldSpec {
        key: "max_post_stack",
        kind: FieldKind::Text,
        hint: "Maximum queued posts before flush",
        aliases: &[],
    },
    FieldSpec {
        key: "max_image_number_one_post",
        kind: FieldKind::Text,
        hint: "Max images per post",
        aliases: &["max_images_per_post"],
    },
    FieldSpec {
        key: "send_timeout_ms",
        kind: FieldKind::Text,
        hint: "Send timeout in milliseconds",
        aliases: &["send_timeout", "send_timeout_sec"],
    },
    FieldSpec {
        key: "send_max_attempts",
        kind: FieldKind::Text,
        hint: "Max send attempts",
        aliases: &["max_send_attempts", "max_send_attempt"],
    },
    FieldSpec {
        key: "send_schedule",
        kind: FieldKind::StringList,
        hint: "Send schedule list (HH:MM)",
        aliases: &[],
    },
    FieldSpec {
        key: "individual_image_in_posts",
        kind: FieldKind::Bool { default: true },
        hint: "Send original images alongside rendered post",
        aliases: &[],
    },
    FieldSpec {
        key: "watermark_text",
        kind: FieldKind::Text,
        hint: "Watermark text",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_add_message",
        kind: FieldKind::Text,
        hint: "Friend add message override",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_request_window_sec",
        kind: FieldKind::Text,
        hint: "Friend request window in seconds",
        aliases: &[],
    },
    FieldSpec {
        key: "quick_replies",
        kind: FieldKind::MapList,
        hint: "Quick replies map (command -> reply)",
        aliases: &[],
    },
    FieldSpec {
        key: "review_shortcuts",
        kind: FieldKind::MapList,
        hint: "Review shortcuts map (command -> steps)",
        aliases: &[],
    },
    FieldSpec {
        key: "global_shortcuts",
        kind: FieldKind::MapList,
        hint: "Global shortcuts map (command -> steps)",
        aliases: &[],
    },
    FieldSpec {
        key: "webview_admins",
        kind: FieldKind::AdminList,
        hint: "Webview admins (username/password, support sha256: prefix)",
        aliases: &["admins"],
    },
];

const ROOT_FIELDS: &[FieldSpec] = &[FieldSpec {
    key: "webview_global_admins",
    kind: FieldKind::AdminList,
    hint: "Global webview admins (username/password, support sha256: prefix)",
    aliases: &[],
}];

#[derive(Clone)]
struct FieldEntry {
    key: String,
    spec: Option<FieldSpec>,
}

#[derive(Clone, Copy)]
struct TabBounds {
    start: u16,
    end: u16,
}

impl TabBounds {
    fn contains(&self, x: u16) -> bool {
        x >= self.start && x < self.end
    }
}

struct GroupTab {
    name: String,
    bounds: TabBounds,
}

struct GroupActionTab {
    action: GroupAction,
    bounds: TabBounds,
}

#[derive(Clone, Copy)]
enum GroupAction {
    Add,
}

#[derive(Clone)]
enum SectionKind {
    Root,
    Common,
    Group(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigFocus {
    Sections,
    Fields,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupStorage {
    Inline,
    Nested,
}

enum EditMode {
    Value {
        section: SectionKind,
        key: String,
        spec: Option<FieldSpec>,
    },
    NewKeyName {
        section: SectionKind,
    },
    NewKeyValue {
        section: SectionKind,
        key: String,
    },
    NewGroupName,
}

struct EditState {
    mode: EditMode,
    buffer: String,
}

#[derive(Clone, Copy)]
enum StatusLevel {
    Info,
    Warn,
    Error,
}

struct StatusMessage {
    text: String,
    level: StatusLevel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListFocus {
    Left,
    Middle,
    Right,
}

enum ListKind {
    StringList,
    PairList { left: String, right: String },
    MapList,
    AdminList,
}

enum ListItem {
    Single(String),
    Pair(String, String),
    MapEntry(String, String),
    Admin(String, String, String),
}

struct ListInput {
    index: usize,
    focus: ListFocus,
    buffer: String,
}

struct ListEditor {
    section: SectionKind,
    key: String,
    kind: ListKind,
    items: Vec<ListItem>,
    selected: usize,
    offset: usize,
    input: Option<ListInput>,
    focus: ListFocus,
    dirty: bool,
    aliases: Vec<&'static str>,
}

pub struct ConfigEditor {
    path: PathBuf,
    common: Map<String, Value>,
    groups: BTreeMap<String, Map<String, Value>>,
    storage: GroupStorage,
    other_root: Map<String, Value>,
    focus: ConfigFocus,
    selected_section: usize,
    selected_field: usize,
    section_offset: usize,
    field_offset: usize,
    section_height: usize,
    field_height: usize,
    layout_sections: Rect,
    layout_fields: Rect,
    layout_groupbar: Rect,
    layout_list: Rect,
    editing: Option<EditState>,
    list_editor: Option<ListEditor>,
    pending_group_delete: Option<String>,
    current_group: Option<String>,
    group_tabs: Vec<GroupTab>,
    group_actions: Vec<GroupActionTab>,
    dirty: bool,
    status: Option<StatusMessage>,
    cursor_on: bool,
    last_blink: Instant,
}

impl ConfigEditor {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        match fs::read_to_string(&path) {
            Ok(data) => {
                let mut root: Value =
                    serde_json::from_str(&data).map_err(|err| format!("JSON 格式错误: {err}"))?;
                if normalize_tui_config_in_place(&mut root)? {
                    write_tui_normalized_config(&path, &root)?;
                }
                Self::from_value(path, root)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty(
                path,
                Some("未找到配置文件，已使用空配置初始化"),
            )),
            Err(err) => Err(format!("读取 {} 失败: {err}", path.display())),
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some() || self.list_editor.is_some()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.list_editor.is_some() {
            self.handle_list_key(key);
            return;
        }
        if self.editing.is_some() {
            self.handle_edit_key(key);
            return;
        }

        if self.pending_group_delete.is_some()
            && !matches!(key.code, KeyCode::Char('x') | KeyCode::Delete)
        {
            self.pending_group_delete = None;
        }

        match key.code {
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Left => self.focus_sections(),
            KeyCode::Right => self.focus_fields(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.page_selection(-1),
            KeyCode::PageDown => self.page_selection(1),
            KeyCode::Home => self.jump_selection_top(),
            KeyCode::End => self.jump_selection_bottom(),
            KeyCode::Enter | KeyCode::Char('e') => self.begin_edit_value(),
            KeyCode::Char(' ') | KeyCode::Char('t') => self.toggle_selected_bool(),
            KeyCode::Char('a') => self.begin_new_key(),
            KeyCode::Char('g') => self.begin_new_group(),
            KeyCode::Char('x') | KeyCode::Delete => self.request_delete_group(),
            KeyCode::Char('[') => self.select_prev_group(),
            KeyCode::Char(']') => self.select_next_group(),
            KeyCode::Char('s') => self.save_current(),
            KeyCode::Char('r') => self.reload_current(),
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_mouse_click(mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => {
                self.handle_mouse_scroll(-1, mouse.column, mouse.row);
            }
            MouseEventKind::ScrollDown => {
                self.handle_mouse_scroll(1, mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        let active_cursor = self.editing.is_some()
            || self
                .list_editor
                .as_ref()
                .map(|editor| editor.input.is_some())
                .unwrap_or(false);
        if !active_cursor {
            self.cursor_on = false;
            return;
        }
        if self.last_blink.elapsed() >= Duration::from_millis(500) {
            self.cursor_on = !self.cursor_on;
            self.last_blink = Instant::now();
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        if self.list_editor.is_some() {
            self.render_list_editor(f, area);
            return;
        }

        let sections = self.section_labels();
        self.clamp_section(sections.len());
        self.ensure_current_group();
        let fields = self.current_fields();
        self.clamp_field(fields.len());

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        let header_line = self.header_line(layout[0].width as usize);
        let header =
            Paragraph::new(header_line).style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(header, layout[0]);

        let groupbar_line = self.group_bar_line(layout[1].width as usize);
        let groupbar =
            Paragraph::new(groupbar_line).style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(groupbar, layout[1]);
        self.layout_groupbar = layout[1];

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(layout[2]);

        self.layout_sections = body[0];
        self.layout_fields = body[1];
        self.section_height = body[0].height.saturating_sub(2) as usize;
        self.field_height = body[1].height.saturating_sub(2) as usize;
        self.ensure_visible();

        let section_lines = self.section_lines(&sections, body[0].width as usize);
        let sections_widget = Paragraph::new(Text::from(section_lines))
            .block(Block::default().borders(Borders::ALL).title("分区"));
        f.render_widget(sections_widget, body[0]);

        let field_lines = self.field_lines(&fields, body[1].width as usize);
        let fields_title = match self.current_section() {
            SectionKind::Root => "字段（根配置）".to_string(),
            SectionKind::Common => "字段（公共配置）".to_string(),
            SectionKind::Group(name) => {
                if name.is_empty() {
                    "字段（组配置）".to_string()
                } else {
                    format!("字段（组 {name}）")
                }
            }
        };
        let fields_widget = Paragraph::new(Text::from(field_lines))
            .block(Block::default().borders(Borders::ALL).title(fields_title));
        f.render_widget(fields_widget, body[1]);

        let hint_line = self.hint_line(layout[3].width as usize);
        let hint_widget =
            Paragraph::new(hint_line).style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(hint_widget, layout[3]);

        let footer = self.footer_line(layout[4].width as usize);
        let footer_widget =
            Paragraph::new(footer).style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(footer_widget, layout[4]);
    }

    fn empty(path: PathBuf, status: Option<&str>) -> Self {
        let mut other_root = Map::new();
        other_root.insert(
            "schema_version".to_string(),
            Value::Number(serde_json::Number::from(1)),
        );
        let mut editor = Self {
            path,
            common: Map::new(),
            groups: BTreeMap::new(),
            storage: GroupStorage::Nested,
            other_root,
            focus: ConfigFocus::Fields,
            selected_section: 0,
            selected_field: 0,
            section_offset: 0,
            field_offset: 0,
            section_height: 0,
            field_height: 0,
            layout_sections: Rect::default(),
            layout_fields: Rect::default(),
            layout_groupbar: Rect::default(),
            layout_list: Rect::default(),
            editing: None,
            list_editor: None,
            pending_group_delete: None,
            current_group: None,
            group_tabs: Vec::new(),
            group_actions: Vec::new(),
            dirty: false,
            status: None,
            cursor_on: false,
            last_blink: Instant::now(),
        };
        if let Some(message) = status {
            editor.set_status(message, StatusLevel::Warn);
        }
        editor
    }

    fn from_value(path: PathBuf, root: Value) -> Result<Self, String> {
        let obj = root
            .as_object()
            .ok_or_else(|| "配置根节点必须是 JSON 对象".to_string())?;
        let mut warnings = Vec::new();
        let (common, common_warn) = map_from_value(obj.get("common"));
        if let Some(message) = common_warn {
            warnings.push(format!("common: {message}"));
        }

        let mut groups = BTreeMap::new();
        let mut other_root = Map::new();
        let storage = if let Some(groups_obj) = obj.get("groups").and_then(|v| v.as_object()) {
            for (name, value) in groups_obj {
                let (map, warn) = map_from_value(Some(value));
                if let Some(message) = warn {
                    warnings.push(format!("组 {name}: {message}"));
                }
                groups.insert(name.clone(), map);
            }
            for (key, value) in obj {
                if key == "common" || key == "groups" {
                    continue;
                }
                other_root.insert(key.clone(), value.clone());
            }
            GroupStorage::Nested
        } else {
            for (key, value) in obj {
                if key == "common" {
                    continue;
                }
                if key == "schema_version" {
                    other_root.insert(key.clone(), value.clone());
                    continue;
                }
                if let Some(group_obj) = value.as_object() {
                    groups.insert(key.clone(), group_obj.clone());
                    continue;
                }
                other_root.insert(key.clone(), value.clone());
            }
            GroupStorage::Inline
        };

        let mut editor = Self {
            path,
            common,
            groups,
            storage,
            other_root,
            focus: ConfigFocus::Fields,
            selected_section: 0,
            selected_field: 0,
            section_offset: 0,
            field_offset: 0,
            section_height: 0,
            field_height: 0,
            layout_sections: Rect::default(),
            layout_fields: Rect::default(),
            layout_groupbar: Rect::default(),
            layout_list: Rect::default(),
            editing: None,
            list_editor: None,
            pending_group_delete: None,
            current_group: None,
            group_tabs: Vec::new(),
            group_actions: Vec::new(),
            dirty: false,
            status: None,
            cursor_on: false,
            last_blink: Instant::now(),
        };

        if let Some(warn) = warnings.first() {
            editor.set_status(warn, StatusLevel::Warn);
        }
        editor.ensure_current_group();

        Ok(editor)
    }

    fn save_current(&mut self) {
        let (errors, warnings) = self.validate();
        if !errors.is_empty() {
            let msg = summarize_messages(&errors, 1, "条错误");
            self.set_status(format!("保存失败: {msg}"), StatusLevel::Error);
            return;
        }
        match self.save() {
            Ok(()) => {
                if warnings.is_empty() {
                    self.set_status("配置已保存", StatusLevel::Info);
                } else {
                    let msg = summarize_messages(&warnings, 1, "条告警");
                    self.set_status(format!("保存成功，但有告警: {msg}"), StatusLevel::Warn);
                }
            }
            Err(err) => self.set_status(err, StatusLevel::Error),
        }
    }

    fn reload_current(&mut self) {
        let path = self.path.clone();
        match Self::load(path) {
            Ok(mut fresh) => {
                let had_status = fresh.status.take();
                *self = fresh;
                if let Some(status) = had_status {
                    self.status = Some(status);
                } else {
                    self.set_status("配置已重载", StatusLevel::Info);
                }
            }
            Err(err) => self.set_status(format!("重载失败: {err}"), StatusLevel::Error),
        }
    }

    fn save(&mut self) -> Result<(), String> {
        let mut root = Map::new();
        root.insert(
            "common".to_string(),
            Value::Object(sanitize_map(&self.common)),
        );
        match self.storage {
            GroupStorage::Nested => {
                let mut groups_map = Map::new();
                for (name, map) in &self.groups {
                    let mut group = sanitize_map(map);
                    normalize_group_accounts_map(&mut group);
                    groups_map.insert(name.clone(), Value::Object(group));
                }
                root.insert("groups".to_string(), Value::Object(groups_map));
                for (key, value) in sanitize_map(&self.other_root) {
                    if key == "common" || key == "groups" {
                        continue;
                    }
                    root.insert(key, value);
                }
            }
            GroupStorage::Inline => {
                for (key, value) in sanitize_map(&self.other_root) {
                    if key == "common" {
                        continue;
                    }
                    root.insert(key, value);
                }
                for (name, map) in &self.groups {
                    let mut group = sanitize_map(map);
                    normalize_group_accounts_map(&mut group);
                    root.insert(name.clone(), Value::Object(group));
                }
            }
        }

        let output = serde_json::to_string_pretty(&Value::Object(root))
            .map_err(|err| format!("序列化失败: {err}"))?;
        fs::write(&self.path, format!("{output}\n")).map_err(|err| format!("写入失败: {err}"))?;
        self.dirty = false;
        Ok(())
    }

    fn section_labels(&self) -> Vec<String> {
        vec![
            "根配置".to_string(),
            "公共配置".to_string(),
            "组配置".to_string(),
        ]
    }

    fn current_section(&self) -> SectionKind {
        match self.selected_section {
            0 => SectionKind::Root,
            1 => SectionKind::Common,
            _ => {
                let name = self.current_group.clone().unwrap_or_default();
                SectionKind::Group(name)
            }
        }
    }

    fn current_fields(&self) -> Vec<FieldEntry> {
        let section = self.current_section();
        if let SectionKind::Group(name) = &section {
            if name.is_empty() || !self.groups.contains_key(name) {
                return Vec::new();
            }
        }
        let specs = match &section {
            SectionKind::Root => ROOT_FIELDS,
            SectionKind::Common => COMMON_FIELDS,
            SectionKind::Group(_) => GROUP_FIELDS,
        };

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for spec in specs {
            out.push(FieldEntry {
                key: spec.key.to_string(),
                spec: Some(*spec),
            });
            seen.insert(spec.key);
            if let Some((prefix, _)) = spec.key.split_once('.') {
                seen.insert(prefix);
            }
            for alias in spec.aliases {
                seen.insert(*alias);
                if let Some((prefix, _)) = alias.split_once('.') {
                    seen.insert(prefix);
                }
            }
        }

        if let Some(map) = self.section_map(&section) {
            let mut extras: Vec<String> = map
                .keys()
                .filter(|k| !seen.contains(k.as_str()))
                .cloned()
                .collect();
            extras.sort();
            for key in extras {
                out.push(FieldEntry { key, spec: None });
            }
        }

        out
    }

    fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    fn ensure_current_group(&mut self) {
        let current_valid = self
            .current_group
            .as_ref()
            .map(|name| self.groups.contains_key(name))
            .unwrap_or(false);
        if current_valid {
            return;
        }
        self.current_group = self.groups.keys().next().cloned();
    }

    fn group_bar_line(&mut self, width: usize) -> Line<'static> {
        self.group_tabs.clear();
        self.group_actions.clear();
        let mut spans = Vec::new();
        let prefix = "组: ";
        spans.push(Span::raw(prefix.to_string()));
        let mut col = UnicodeWidthStr::width(prefix) as u16;

        let names = self.group_names();
        if names.is_empty() {
            let label = "<暂无>";
            spans.push(Span::styled(label, Style::default().fg(Color::Yellow)));
            col = col.saturating_add(UnicodeWidthStr::width(label) as u16);
        } else {
            for (idx, name) in names.iter().enumerate() {
                if idx > 0 {
                    spans.push(Span::raw(" "));
                    col = col.saturating_add(1);
                }
                let active = self.current_group.as_deref() == Some(name.as_str());
                let label = if active {
                    format!("[{name}]")
                } else {
                    name.to_string()
                };
                let width_label = UnicodeWidthStr::width(label.as_str()) as u16;
                let bounds = TabBounds {
                    start: col,
                    end: col.saturating_add(width_label),
                };
                let style = if active {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(label.clone(), style));
                self.group_tabs.push(GroupTab {
                    name: name.clone(),
                    bounds,
                });
                col = col.saturating_add(width_label);
            }
        }

        spans.push(Span::raw(" "));
        col = col.saturating_add(1);
        let add_label = "[+]";
        let add_width = UnicodeWidthStr::width(add_label) as u16;
        let add_bounds = TabBounds {
            start: col,
            end: col.saturating_add(add_width),
        };
        spans.push(Span::styled(
            add_label,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        self.group_actions.push(GroupActionTab {
            action: GroupAction::Add,
            bounds: add_bounds,
        });

        let line = Line::from(spans);
        if width == 0 { Line::raw("") } else { line }
    }

    fn select_prev_group(&mut self) {
        let names = self.group_names();
        if names.is_empty() {
            return;
        }
        let idx = self
            .current_group
            .as_ref()
            .and_then(|name| names.iter().position(|item| item == name))
            .unwrap_or(0);
        let next = if idx == 0 { names.len() - 1 } else { idx - 1 };
        self.current_group = Some(names[next].clone());
        self.selected_section = 2;
        self.selected_field = 0;
        self.field_offset = 0;
        self.pending_group_delete = None;
    }

    fn select_next_group(&mut self) {
        let names = self.group_names();
        if names.is_empty() {
            return;
        }
        let idx = self
            .current_group
            .as_ref()
            .and_then(|name| names.iter().position(|item| item == name))
            .unwrap_or(0);
        let next = (idx + 1) % names.len();
        self.current_group = Some(names[next].clone());
        self.selected_section = 2;
        self.selected_field = 0;
        self.field_offset = 0;
        self.pending_group_delete = None;
    }

    fn selected_field_entry<'a>(&self, fields: &'a [FieldEntry]) -> Option<&'a FieldEntry> {
        fields.get(self.selected_field)
    }

    fn section_map(&self, section: &SectionKind) -> Option<&Map<String, Value>> {
        match section {
            SectionKind::Root => Some(&self.other_root),
            SectionKind::Common => Some(&self.common),
            SectionKind::Group(name) => self.groups.get(name),
        }
    }

    fn section_map_mut(&mut self, section: &SectionKind) -> Option<&mut Map<String, Value>> {
        match section {
            SectionKind::Root => Some(&mut self.other_root),
            SectionKind::Common => Some(&mut self.common),
            SectionKind::Group(name) => self.groups.get_mut(name),
        }
    }

    fn clamp_section(&mut self, len: usize) {
        if len == 0 {
            self.selected_section = 0;
            return;
        }
        if self.selected_section >= len {
            self.selected_section = len - 1;
        }
    }

    fn clamp_field(&mut self, len: usize) {
        if len == 0 {
            self.selected_field = 0;
            return;
        }
        if self.selected_field >= len {
            self.selected_field = len - 1;
        }
    }

    fn ensure_visible(&mut self) {
        let section_count = self.section_labels().len();
        self.section_offset = ensure_visible_offset(
            self.section_offset,
            self.selected_section,
            self.section_height,
            section_count,
        );

        let field_count = self.current_fields().len();
        self.field_offset = ensure_visible_offset(
            self.field_offset,
            self.selected_field,
            self.field_height,
            field_count,
        );
    }

    fn selection_style(&self, selected: bool, _focused: bool) -> Style {
        let mut style = Style::default();
        if selected && _focused {
            style = style.add_modifier(Modifier::REVERSED);
        }
        style
    }

    fn section_lines(&self, sections: &[String], width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if sections.is_empty() {
            lines.push(Line::raw("没有可用分区"));
            return lines;
        }
        let start = self.section_offset.min(sections.len());
        let end = (start + self.section_height.max(1)).min(sections.len());
        for (idx, label) in sections[start..end].iter().enumerate() {
            let absolute = start + idx;
            let style = self.selection_style(
                absolute == self.selected_section,
                self.focus == ConfigFocus::Sections,
            );
            let text = truncate_to_width(label, width.saturating_sub(2));
            lines.push(Line::styled(text, style));
        }
        lines
    }

    fn field_lines(&self, fields: &[FieldEntry], width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if fields.is_empty() {
            if self.selected_section == 2 && self.current_group.is_none() {
                lines.push(Line::raw("还没有组，按 g 新建组"));
            } else {
                lines.push(Line::raw("当前无字段，按 a 新增字段"));
            }
            return lines;
        }
        let section = self.current_section();
        let mut inline_key: Option<&str> = None;
        let mut inline_value: Option<&str> = None;
        let mut inline_cursor = false;
        if let Some(edit) = &self.editing {
            if let EditMode::Value { key, .. } = &edit.mode {
                inline_key = Some(key.as_str());
                inline_value = Some(edit.buffer.as_str());
                inline_cursor = self.cursor_on;
            }
        }
        let start = self.field_offset.min(fields.len());
        let end = (start + self.field_height.max(1)).min(fields.len());
        for (idx, entry) in fields[start..end].iter().enumerate() {
            let absolute = start + idx;
            let style = self.selection_style(
                absolute == self.selected_field,
                self.focus == ConfigFocus::Fields,
            );
            let preview = if Some(entry.key.as_str()) == inline_key {
                let buffer = inline_value.unwrap_or("");
                if inline_cursor {
                    format!("{buffer}|")
                } else {
                    buffer.to_string()
                }
            } else {
                self.field_preview(&section, entry)
            };
            let summary = entry.spec.map(field_summary_text).unwrap_or("自定义字段");
            let line = format!("{} | {} | {}", entry.key, summary, preview);
            let text = truncate_to_width(&line, width.saturating_sub(2));
            lines.push(Line::styled(text, style));
        }
        lines
    }

    fn field_preview(&self, section: &SectionKind, entry: &FieldEntry) -> String {
        let map = match self.section_map(section) {
            Some(map) => map,
            None => return "<未设置>".to_string(),
        };
        let Some(spec) = entry.spec else {
            return map
                .get(&entry.key)
                .map(value_display)
                .unwrap_or_else(|| "<未设置>".to_string());
        };

        let value = resolve_value(map, &spec);
        match spec.kind {
            FieldKind::Bool { default } => {
                let (val, missing, invalid) = bool_value(value, default);
                if invalid {
                    value
                        .map(value_display)
                        .unwrap_or_else(|| "<非法值>".to_string())
                } else {
                    let display = if val { "[x]" } else { "[ ]" };
                    if missing {
                        format!("{display} (默认)")
                    } else {
                        display.to_string()
                    }
                }
            }
            FieldKind::StringList => {
                let items = extract_string_list(value);
                if is_unset_value(value) {
                    "<未设置>".to_string()
                } else {
                    format!("列表({})", items.len())
                }
            }
            FieldKind::PairList { left, right } => {
                let items = extract_pair_list(map, left, right);
                if map.get(left).is_none() && map.get(right).is_none() {
                    "<未设置>".to_string()
                } else {
                    format!("成对({})", items.len())
                }
            }
            FieldKind::MapList => {
                let items = extract_map_list(value);
                if is_unset_value(value) {
                    "<未设置>".to_string()
                } else {
                    format!("映射({})", items.len())
                }
            }
            FieldKind::AdminList => {
                let items = extract_admin_list(value);
                if is_unset_value(value) {
                    "<未设置>".to_string()
                } else {
                    format!("管理员({})", items.len())
                }
            }
            FieldKind::Text => {
                if is_unset_value(value) {
                    "<未设置>".to_string()
                } else {
                    value
                        .map(value_display)
                        .unwrap_or_else(|| "<未设置>".to_string())
                }
            }
        }
    }

    fn header_line(&self, width: usize) -> Line<'static> {
        let dirty = if self.dirty { " *" } else { "" };
        let text = format!("配置文件: {}{}", self.path.display(), dirty);
        Line::raw(truncate_to_width(&text, width))
    }

    fn hint_line(&self, width: usize) -> Line<'static> {
        let detail = self.current_detail();
        if detail.is_empty() {
            return Line::raw(truncate_to_width("详细说明: -", width));
        }
        let text = format!("详细说明: {detail}");
        Line::raw(truncate_to_width(&text, width))
    }

    fn current_detail(&self) -> String {
        let fields = self.current_fields();
        let Some(entry) = self.selected_field_entry(&fields) else {
            return String::new();
        };
        entry.spec.map(field_detail_text).unwrap_or_else(|| {
            format!(
                "自定义字段 `{}`：请确认值类型后保存；支持直接输入 JSON（如 true、123、[\"a\"]）。",
                entry.key
            )
        })
    }

    fn footer_line(&self, width: usize) -> Line<'static> {
        if let Some(edit) = &self.editing {
            let label = match &edit.mode {
                EditMode::Value { key, .. } => format!("编辑 {key}: "),
                EditMode::NewKeyName { .. } => "新字段名: ".to_string(),
                EditMode::NewKeyValue { key, .. } => format!("字段 {key} 的新值: "),
                EditMode::NewGroupName => "新组名: ".to_string(),
            };
            let text = format!("{label}{}", edit.buffer);
            return Line::raw(truncate_to_width(&text, width));
        }

        let help = "鼠标点击 | Tab 切焦点 | 方向键移动 | Enter/e 编辑 | 空格切换布尔 | a 新增字段 | g 新增组 | x 删除组 | [ ] 切组 | s 保存 | r 重载";
        self.status_with_help(help, width)
    }

    fn status_with_help(&self, help: &str, width: usize) -> Line<'static> {
        if let Some(status) = &self.status {
            let status_text = truncate_to_width(&status.text, width.saturating_sub(help.len() + 3));
            let style = match status.level {
                StatusLevel::Info => Style::default().fg(Color::Green),
                StatusLevel::Warn => Style::default().fg(Color::Yellow),
                StatusLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            };
            let mut spans = Vec::new();
            spans.push(Span::styled(status_text, style));
            spans.push(Span::raw(" | "));
            spans.push(Span::raw(help.to_string()));
            Line::from(spans)
        } else {
            Line::raw(truncate_to_width(help, width))
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            ConfigFocus::Sections => ConfigFocus::Fields,
            ConfigFocus::Fields => ConfigFocus::Sections,
        };
    }

    fn focus_sections(&mut self) {
        self.focus = ConfigFocus::Sections;
    }

    fn focus_fields(&mut self) {
        self.focus = ConfigFocus::Fields;
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            ConfigFocus::Sections => {
                let len = self.section_labels().len();
                if len == 0 {
                    return;
                }
                let next = clamp_index(self.selected_section, delta, len);
                if next != self.selected_section {
                    self.selected_section = next;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                }
            }
            ConfigFocus::Fields => {
                let len = self.current_fields().len();
                if len == 0 {
                    return;
                }
                self.selected_field = clamp_index(self.selected_field, delta, len);
            }
        }
    }

    fn page_selection(&mut self, direction: isize) {
        let step = match self.focus {
            ConfigFocus::Sections => self.section_height.max(1) as isize,
            ConfigFocus::Fields => self.field_height.max(1) as isize,
        };
        self.move_selection(step * direction);
    }

    fn jump_selection_top(&mut self) {
        match self.focus {
            ConfigFocus::Sections => {
                self.selected_section = 0;
                self.selected_field = 0;
                self.field_offset = 0;
                self.pending_group_delete = None;
            }
            ConfigFocus::Fields => {
                self.selected_field = 0;
            }
        }
    }

    fn jump_selection_bottom(&mut self) {
        match self.focus {
            ConfigFocus::Sections => {
                let len = self.section_labels().len();
                if len > 0 {
                    self.selected_section = len - 1;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                }
            }
            ConfigFocus::Fields => {
                let len = self.current_fields().len();
                if len > 0 {
                    self.selected_field = len - 1;
                }
            }
        }
    }

    fn begin_edit_value(&mut self) {
        if self.focus != ConfigFocus::Fields {
            return;
        }
        let fields = self.current_fields();
        if fields.is_empty() {
            self.set_status("未选中字段", StatusLevel::Warn);
            return;
        }
        let entry = fields[self.selected_field].clone();
        let section = self.current_section();
        if matches!(section, SectionKind::Group(ref name) if name.is_empty()) {
            self.set_status("未选中组", StatusLevel::Warn);
            return;
        }
        if let Some(spec) = entry.spec {
            match spec.kind {
                FieldKind::StringList
                | FieldKind::PairList { .. }
                | FieldKind::MapList
                | FieldKind::AdminList => {
                    self.open_list_editor(section, entry, spec);
                    return;
                }
                _ => {}
            }
        }

        let map = match self.section_map(&section) {
            Some(map) => map,
            None => return,
        };
        let value = entry
            .spec
            .and_then(|spec| resolve_value(map, &spec))
            .or_else(|| map.get(&entry.key))
            .unwrap_or(&Value::Null);
        let buffer = value_to_input(value);
        self.editing = Some(EditState {
            mode: EditMode::Value {
                section,
                key: entry.key,
                spec: entry.spec,
            },
            buffer,
        });
        self.cursor_on = true;
        self.last_blink = Instant::now();
    }

    fn begin_new_key(&mut self) {
        let section = self.current_section();
        if matches!(section, SectionKind::Group(ref name) if name.is_empty()) {
            self.set_status("未选中组", StatusLevel::Warn);
            return;
        }
        self.editing = Some(EditState {
            mode: EditMode::NewKeyName { section },
            buffer: String::new(),
        });
    }

    fn begin_new_group(&mut self) {
        self.editing = Some(EditState {
            mode: EditMode::NewGroupName,
            buffer: String::new(),
        });
    }

    fn request_delete_group(&mut self) {
        if self.focus != ConfigFocus::Sections {
            return;
        }
        let section = self.current_section();
        let SectionKind::Group(name) = section else {
            self.set_status("只能在“组配置”分区删除组", StatusLevel::Warn);
            return;
        };
        if name.is_empty() {
            self.set_status("未选中组", StatusLevel::Warn);
            return;
        }
        if self.pending_group_delete.as_deref() == Some(&name) {
            self.groups.remove(&name);
            self.pending_group_delete = None;
            if self.current_group.as_deref() == Some(&name) {
                self.current_group = self.groups.keys().next().cloned();
            }
            self.selected_section = 2;
            self.selected_field = 0;
            self.field_offset = 0;
            self.dirty = true;
            self.set_status(format!("已删除组 {name}"), StatusLevel::Info);
        } else {
            self.pending_group_delete = Some(name.clone());
            self.set_status(format!("再次按 x 确认删除组 {name}"), StatusLevel::Warn);
        }
    }

    fn toggle_selected_bool(&mut self) {
        if self.focus != ConfigFocus::Fields {
            return;
        }
        let fields = self.current_fields();
        let Some(entry) = self.selected_field_entry(&fields) else {
            return;
        };
        let section = self.current_section();
        let map = match self.section_map(&section) {
            Some(map) => map,
            None => return,
        };
        let mut default = false;
        let mut bool_field = false;
        let mut spec = None;
        if let Some(entry_spec) = entry.spec {
            if let FieldKind::Bool {
                default: spec_default,
            } = entry_spec.kind
            {
                default = spec_default;
                bool_field = true;
                spec = Some(entry_spec);
            }
        }
        let value = spec
            .and_then(|spec| resolve_value(map, &spec))
            .or_else(|| map.get(&entry.key));
        let (current, _, invalid) = bool_value(value, default);
        if !bool_field && invalid {
            self.set_status("当前字段不是布尔值", StatusLevel::Warn);
            return;
        }
        let next = !current;
        let key = entry.key.clone();
        if let Some(map) = self.section_map_mut(&section) {
            if let Some(spec) = spec {
                clear_aliases(map, &spec);
            }
            set_path_value(map, &key, Value::Bool(next));
            self.dirty = true;
            self.set_status(format!("已设置 {key} = {next}"), StatusLevel::Info);
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) {
        let Some(edit) = &mut self.editing else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.editing = None;
                self.cursor_on = false;
            }
            KeyCode::Enter => {
                let edit = self.editing.take().unwrap();
                self.commit_edit(edit);
                self.cursor_on = false;
            }
            KeyCode::Backspace => {
                edit.buffer.pop();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                edit.buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) {
        if let Some(editor) = self.list_editor.as_mut() {
            if rect_contains(self.layout_list, x, y) {
                if let Some(index) =
                    list_click_index(self.layout_list, y, editor.offset, editor.items.len())
                {
                    let was_selected = index == editor.selected;
                    editor.selected = index;
                    if was_selected {
                        editor.start_edit();
                        self.cursor_on = true;
                        self.last_blink = Instant::now();
                    }
                }
            }
            return;
        }

        if rect_contains(self.layout_groupbar, x, y) {
            let local_x = x.saturating_sub(self.layout_groupbar.x);
            for tab in &self.group_tabs {
                if tab.bounds.contains(local_x) {
                    self.current_group = Some(tab.name.clone());
                    self.selected_section = 2;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                    return;
                }
            }
            for action in &self.group_actions {
                if action.bounds.contains(local_x) {
                    match action.action {
                        GroupAction::Add => self.begin_new_group(),
                    }
                    return;
                }
            }
        }

        if rect_contains(self.layout_sections, x, y) {
            if let Some(index) = list_click_index(
                self.layout_sections,
                y,
                self.section_offset,
                self.section_labels().len(),
            ) {
                self.focus_sections();
                if index != self.selected_section {
                    self.selected_section = index;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                }
            }
            return;
        }

        if rect_contains(self.layout_fields, x, y) {
            let fields = self.current_fields();
            if let Some(index) =
                list_click_index(self.layout_fields, y, self.field_offset, fields.len())
            {
                let was_selected = index == self.selected_field;
                self.focus_fields();
                self.selected_field = index;
                if was_selected {
                    if let Some(spec) = fields[index].spec {
                        if matches!(spec.kind, FieldKind::Bool { .. }) {
                            self.toggle_selected_bool();
                            return;
                        }
                    }
                    self.begin_edit_value();
                }
            }
        }
    }

    fn handle_mouse_scroll(&mut self, direction: isize, x: u16, y: u16) {
        if let Some(editor) = self.list_editor.as_mut() {
            if rect_contains(self.layout_list, x, y) {
                editor.move_selection(direction);
            }
            return;
        }

        if rect_contains(self.layout_groupbar, x, y) {
            if direction < 0 {
                self.select_prev_group();
            } else {
                self.select_next_group();
            }
            return;
        }

        if rect_contains(self.layout_sections, x, y) {
            self.focus_sections();
            self.move_selection(direction);
        } else if rect_contains(self.layout_fields, x, y) {
            self.focus_fields();
            self.move_selection(direction);
        }
    }

    fn commit_edit(&mut self, edit: EditState) {
        match edit.mode {
            EditMode::Value { section, key, spec } => {
                if let Some(map) = self.section_map_mut(&section) {
                    if let Some(spec) = spec {
                        clear_aliases(map, &spec);
                    }
                    if edit.buffer.trim().is_empty() {
                        remove_path_value(map, &key);
                        self.dirty = true;
                        self.set_status(format!("已清空 {key}"), StatusLevel::Info);
                        return;
                    }
                    let value = parse_input_value(&edit.buffer);
                    set_path_value(map, &key, value);
                    self.dirty = true;
                    self.set_status(format!("已更新 {key}"), StatusLevel::Info);
                }
            }
            EditMode::NewKeyName { section } => {
                let key = edit.buffer.trim();
                if key.is_empty() {
                    self.set_status("字段名不能为空", StatusLevel::Warn);
                    return;
                }
                let exists = self
                    .section_map(&section)
                    .map(|map| map.contains_key(key))
                    .unwrap_or(false);
                if exists {
                    self.set_status("字段名已存在", StatusLevel::Warn);
                    self.editing = Some(EditState {
                        mode: EditMode::NewKeyName { section },
                        buffer: key.to_string(),
                    });
                    return;
                }
                self.editing = Some(EditState {
                    mode: EditMode::NewKeyValue {
                        section,
                        key: key.to_string(),
                    },
                    buffer: String::new(),
                });
            }
            EditMode::NewKeyValue { section, key } => {
                if let Some(map) = self.section_map_mut(&section) {
                    if edit.buffer.trim().is_empty() {
                        self.dirty = true;
                        self.select_key(&key);
                        self.set_status(format!("已清空 {key}"), StatusLevel::Info);
                        return;
                    }
                    let value = parse_input_value(&edit.buffer);
                    set_path_value(map, &key, value);
                    self.dirty = true;
                    self.select_key(&key);
                    self.set_status(format!("已新增字段 {key}"), StatusLevel::Info);
                }
            }
            EditMode::NewGroupName => {
                let name = edit.buffer.trim();
                if name.is_empty() {
                    self.set_status("组名不能为空", StatusLevel::Warn);
                    return;
                }
                if name.eq_ignore_ascii_case("common") || name.eq_ignore_ascii_case("groups") {
                    self.set_status("组名为保留字", StatusLevel::Warn);
                    return;
                }
                if !is_valid_group_name(name) {
                    self.set_status("组名只能包含字母、数字和下划线", StatusLevel::Warn);
                    return;
                }
                if self.groups.contains_key(name) {
                    self.set_status("组名已存在", StatusLevel::Warn);
                    return;
                }
                self.groups
                    .insert(name.to_string(), default_group_config_template());
                self.current_group = Some(name.to_string());
                self.selected_section = 2;
                self.selected_field = 0;
                self.field_offset = 0;
                self.dirty = true;
                self.set_status(format!("已新增组 {name}"), StatusLevel::Info);
            }
        }
    }

    fn select_key(&mut self, key: &str) {
        let fields = self.current_fields();
        if let Some(idx) = fields.iter().position(|entry| entry.key == key) {
            self.selected_field = idx;
        }
    }

    fn open_list_editor(&mut self, section: SectionKind, entry: FieldEntry, spec: FieldSpec) {
        let kind = match spec.kind {
            FieldKind::StringList => ListKind::StringList,
            FieldKind::PairList { left, right } => ListKind::PairList {
                left: left.to_string(),
                right: right.to_string(),
            },
            FieldKind::MapList => ListKind::MapList,
            FieldKind::AdminList => ListKind::AdminList,
            _ => return,
        };
        let items = self.load_list_items(&section, &entry.key, &kind, Some(spec));
        self.list_editor = Some(ListEditor {
            section,
            key: entry.key,
            kind,
            items,
            selected: 0,
            offset: 0,
            input: None,
            focus: ListFocus::Left,
            dirty: false,
            aliases: spec.aliases.to_vec(),
        });
        self.cursor_on = true;
        self.last_blink = Instant::now();
    }

    fn load_list_items(
        &self,
        section: &SectionKind,
        key: &str,
        kind: &ListKind,
        spec: Option<FieldSpec>,
    ) -> Vec<ListItem> {
        let map = match self.section_map(section) {
            Some(map) => map,
            None => return Vec::new(),
        };
        match kind {
            ListKind::StringList => {
                let value = spec.and_then(|spec| resolve_value(map, &spec));
                let value = value.or_else(|| map.get(key));
                extract_string_list(value)
                    .into_iter()
                    .map(ListItem::Single)
                    .collect()
            }
            ListKind::PairList { left, right } => {
                let items = extract_pair_list(map, left, right);
                items
                    .into_iter()
                    .map(|(l, r)| ListItem::Pair(l, r))
                    .collect()
            }
            ListKind::MapList => {
                let value = spec.and_then(|spec| resolve_value(map, &spec));
                let value = value.or_else(|| map.get(key));
                extract_map_list(value)
                    .into_iter()
                    .map(|(k, v)| ListItem::MapEntry(k, v))
                    .collect()
            }
            ListKind::AdminList => {
                let value = spec.and_then(|spec| resolve_value(map, &spec));
                let value = value.or_else(|| map.get(key));
                extract_admin_list(value)
            }
            .into_iter()
            .map(|(u, p, r)| ListItem::Admin(u, p, r))
            .collect(),
        }
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        let Some(editor) = self.list_editor.as_mut() else {
            return;
        };

        if let Some(input) = editor.input.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    editor.input = None;
                    self.cursor_on = false;
                }
                KeyCode::Enter => {
                    let input = editor.input.take().unwrap();
                    editor.apply_input(input);
                    self.cursor_on = false;
                }
                KeyCode::Backspace => {
                    input.buffer.pop();
                }
                KeyCode::Char(ch)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    input.buffer.push(ch);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.close_list_editor();
            }
            KeyCode::Up => editor.move_selection(-1),
            KeyCode::Down => editor.move_selection(1),
            KeyCode::PageUp => editor.page_selection(-1),
            KeyCode::PageDown => editor.page_selection(1),
            KeyCode::Home => editor.jump_top(),
            KeyCode::End => editor.jump_bottom(),
            KeyCode::Tab => editor.toggle_focus(),
            KeyCode::Enter | KeyCode::Char('e') => {
                editor.start_edit();
                self.cursor_on = true;
                self.last_blink = Instant::now();
            }
            KeyCode::Char('a') => {
                editor.add_item();
                self.cursor_on = true;
                self.last_blink = Instant::now();
            }
            KeyCode::Char('d') | KeyCode::Delete => editor.delete_item(),
            _ => {}
        }
    }

    fn close_list_editor(&mut self) {
        let Some(editor) = self.list_editor.take() else {
            return;
        };
        if editor.dirty {
            if let Some(map) = self.section_map_mut(&editor.section) {
                for alias in &editor.aliases {
                    remove_path_value(map, alias);
                }
                editor.apply_to_map(map);
                self.dirty = true;
            }
        }
        self.cursor_on = false;
    }

    fn render_list_editor(&mut self, f: &mut Frame, area: Rect) {
        let Some(editor) = self.list_editor.as_mut() else {
            return;
        };
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        self.layout_list = layout[1];
        let header_line = editor.header_line(layout[0].width as usize);
        let header =
            Paragraph::new(header_line).style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(header, layout[0]);

        let list_height = layout[1].height.saturating_sub(2) as usize;
        editor.ensure_visible(list_height);
        let list_lines = editor.list_lines(layout[1].width as usize, list_height, self.cursor_on);
        let list_widget = Paragraph::new(Text::from(list_lines))
            .block(Block::default().borders(Borders::ALL).title("列表编辑"));
        f.render_widget(list_widget, layout[1]);

        let footer = editor.footer_line(self.status.as_ref(), layout[2].width as usize);
        let footer_widget =
            Paragraph::new(footer).style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(footer_widget, layout[2]);
    }

    fn validate(&self) -> (Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let mut account_ids = HashSet::new();

        let (web_api_enabled, _, web_api_invalid) =
            bool_value(get_path_value(&self.common, "web_api.enabled"), false);
        if web_api_invalid {
            errors.push("common: web_api.enabled 必须是布尔值".to_string());
        }
        if web_api_enabled
            && value_to_string(get_path_value(&self.common, "web_api.root_token"))
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            warnings.push("common: web_api.enabled=true 但 web_api.root_token 为空".to_string());
        }

        let (webview_enabled, _, webview_invalid) =
            bool_value(get_path_value(&self.common, "webview.enabled"), false);
        if webview_invalid {
            errors.push("common: webview.enabled 必须是布尔值".to_string());
        }
        if webview_enabled
            && value_to_string(get_path_value(&self.common, "webview.host"))
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            errors.push("common: 启用 webview 时 webview.host 不能为空".to_string());
        }

        let global_admins = extract_admin_list(self.other_root.get("webview_global_admins"));
        for (username, password, role) in &global_admins {
            if username.trim().is_empty() {
                errors.push("webview_global_admins 存在空用户名".to_string());
                continue;
            }
            if !password.trim().is_empty() && !password.starts_with("sha256:") {
                warnings.push(format!(
                    "webview_global_admins[{username}] 使用明文密码，建议使用 sha256:..."
                ));
            }
            if !role.trim().is_empty() && !is_valid_admin_role(role) {
                errors.push(format!(
                    "webview_global_admins[{username}] role 非法: {role}"
                ));
            }
        }

        for (group, obj) in &self.groups {
            if !is_valid_group_name(group) {
                errors.push(format!(
                    "group '{group}' 名称非法（仅允许字母/数字/下划线）"
                ));
                continue;
            }

            let mangroupid = value_to_string(obj.get("mangroupid")).unwrap_or_default();
            if mangroupid.is_empty() || !is_numeric(&mangroupid) {
                errors.push(format!("{group}: mangroupid 必须是数字"));
            }
            let accounts = extract_string_list(obj.get("accounts"));
            if accounts.is_empty() {
                errors.push(format!("{group}: accounts 缺失或为空"));
            }
            for account in &accounts {
                if account.is_empty() {
                    continue;
                }
                if !is_numeric(account) {
                    errors.push(format!("{group}: accounts 包含非数字账号 {account}"));
                } else if !account_ids.insert(account.clone()) {
                    errors.push(format!("accounts 账号 {account} 重复"));
                }
            }

            for key in ["max_post_stack", "max_image_number_one_post"] {
                let val = value_to_string(obj.get(key)).unwrap_or_default();
                if !val.is_empty() && !is_numeric(&val) {
                    errors.push(format!("{group}: {key} 必须是数字"));
                }
            }

            for key in ["friend_add_message", "watermark_text"] {
                if let Some(value) = obj.get(key) {
                    if !matches!(value, Value::String(_)) {
                        errors.push(format!("{group}: {key} 必须是字符串"));
                    }
                }
            }

            if let Some(value) = obj.get("individual_image_in_posts") {
                if !matches!(value, Value::Bool(_)) {
                    errors.push(format!("{group}: individual_image_in_posts 必须是布尔值"));
                }
            }

            if let Some(value) = obj.get("send_schedule") {
                match value {
                    Value::Array(items) => {
                        for item in items {
                            if let Some(s) = value_to_string(Some(item)) {
                                if !s.is_empty() && parse_schedule_str(&s).is_none() {
                                    errors.push(format!("{group}: send_schedule 时间格式错误 {s}"));
                                }
                            }
                        }
                    }
                    Value::Null => {}
                    _ => {
                        errors.push(format!("{group}: send_schedule 必须是数组"));
                    }
                }
            }

            if let Some(value) = obj.get("quick_replies") {
                match value {
                    Value::Object(entries) => {
                        for (cmd, content) in entries {
                            if cmd.trim().is_empty() {
                                errors.push(format!("{group}: quick_replies 存在空指令名"));
                                continue;
                            }
                            if quick_reply_conflicts_with_review_command(cmd) {
                                errors.push(format!(
                                    "{group}: quick_replies 指令 {cmd} 与审核指令冲突"
                                ));
                            }
                            match content {
                                Value::String(text) if !text.trim().is_empty() => {}
                                Value::String(_) => {
                                    errors.push(format!(
                                        "{group}: quick_replies[{cmd}] 内容不能为空"
                                    ));
                                }
                                _ => {
                                    errors.push(format!(
                                        "{group}: quick_replies[{cmd}] 必须是字符串"
                                    ));
                                }
                            }
                        }
                    }
                    Value::Null => {}
                    _ => {
                        errors.push(format!("{group}: quick_replies 必须是对象"));
                    }
                }
            }

            if let Some(value) = obj.get("review_shortcuts") {
                validate_shortcut_entries(
                    group,
                    "review_shortcuts",
                    value,
                    &mut errors,
                    validate_review_shortcut_definition,
                    obj.get("quick_replies").and_then(|v| v.as_object()),
                );
            }

            if let Some(value) = obj.get("global_shortcuts") {
                validate_shortcut_entries(
                    group,
                    "global_shortcuts",
                    value,
                    &mut errors,
                    validate_global_shortcut_definition,
                    None,
                );
            }

            let admins = extract_admin_list(resolve_value(
                obj,
                &FieldSpec {
                    key: "webview_admins",
                    kind: FieldKind::AdminList,
                    hint: "",
                    aliases: &["admins"],
                },
            ));
            for (username, password, role) in admins {
                if username.trim().is_empty() {
                    errors.push(format!("{group}: webview_admins 存在空用户名"));
                    continue;
                }
                if !password.trim().is_empty() && !password.starts_with("sha256:") {
                    warnings.push(format!(
                        "{group}: 管理员 {username} 使用明文密码，建议使用 sha256:..."
                    ));
                }
                if !role.trim().is_empty() && !is_valid_admin_role(&role) {
                    errors.push(format!("{group}: 管理员 {username} role 非法: {role}"));
                }
            }
        }

        if webview_enabled {
            let mut group_admin_count = 0usize;
            for obj in self.groups.values() {
                group_admin_count += extract_admin_list(resolve_value(
                    obj,
                    &FieldSpec {
                        key: "webview_admins",
                        kind: FieldKind::AdminList,
                        hint: "",
                        aliases: &["admins"],
                    },
                ))
                .len();
            }
            if global_admins.is_empty() && group_admin_count == 0 {
                warnings.push(
                    "common: webview.enabled=true 但未配置 webview_global_admins/webview_admins"
                        .to_string(),
                );
            }
        }

        let (_, _, telemetry_enabled_invalid) =
            bool_value(get_path_value(&self.common, "telemetry.enabled"), true);
        if telemetry_enabled_invalid {
            errors.push("common: telemetry.enabled 必须是布尔值".to_string());
        }
        let (_, _, telemetry_upload_enabled_invalid) = bool_value(
            get_path_value(&self.common, "telemetry.upload_enabled"),
            true,
        );
        if telemetry_upload_enabled_invalid {
            errors.push("common: telemetry.upload_enabled 必须是布尔值".to_string());
        }
        if let Some(value) = get_path_value(&self.common, "telemetry.local_dir") {
            if !matches!(value, Value::String(_) | Value::Null) {
                errors.push("common: telemetry.local_dir 必须是字符串".to_string());
            }
        }
        for key in [
            "telemetry.upload_interval_sec",
            "telemetry.upload_batch_size",
            "telemetry.max_append_messages",
        ] {
            if let Some(value) = get_path_value(&self.common, key) {
                if matches!(value, Value::Null) {
                    continue;
                }
                let valid = match value {
                    Value::Number(n) => n.as_u64().is_some(),
                    Value::String(s) => is_numeric(s.trim()),
                    _ => false,
                };
                if !valid {
                    errors.push(format!("common: {key} 必须是非负整数"));
                }
            }
        }

        (errors, warnings)
    }

    fn set_status(&mut self, text: impl Into<String>, level: StatusLevel) {
        self.status = Some(StatusMessage {
            text: text.into(),
            level,
        });
    }
}

impl ListEditor {
    fn header_line(&self, width: usize) -> Line<'static> {
        let section = match &self.section {
            SectionKind::Root => "根配置".to_string(),
            SectionKind::Common => "公共配置".to_string(),
            SectionKind::Group(name) => format!("组 {name}"),
        };
        let key_label = match &self.kind {
            ListKind::PairList { left, right } => format!("{left}/{right}"),
            ListKind::MapList => format!("{}(key/value)", self.key),
            _ => self.key.clone(),
        };
        let focus = match self.focus {
            ListFocus::Left => "左列",
            ListFocus::Middle => "中列",
            ListFocus::Right => "右列",
        };
        let text = format!("编辑 {section}::{key_label}（当前列: {focus}）");
        Line::raw(truncate_to_width(&text, width))
    }

    fn footer_line(&self, status: Option<&StatusMessage>, width: usize) -> Line<'static> {
        if let Some(input) = &self.input {
            let label = match self.kind {
                ListKind::StringList => "编辑项: ".to_string(),
                ListKind::PairList { .. } => match input.focus {
                    ListFocus::Left => "编辑左值: ".to_string(),
                    ListFocus::Right => "编辑右值: ".to_string(),
                    ListFocus::Middle => "编辑右值: ".to_string(),
                },
                ListKind::MapList => match input.focus {
                    ListFocus::Left => "编辑指令: ".to_string(),
                    ListFocus::Right => "编辑内容: ".to_string(),
                    ListFocus::Middle => "编辑内容: ".to_string(),
                },
                ListKind::AdminList => match input.focus {
                    ListFocus::Left => "编辑用户名: ".to_string(),
                    ListFocus::Middle => "编辑密码: ".to_string(),
                    ListFocus::Right => "编辑角色: ".to_string(),
                },
            };
            let text = format!("{label}{}", input.buffer);
            return Line::raw(truncate_to_width(&text, width));
        }

        let help = "鼠标点击 | 方向键移动 | Enter/e 编辑 | a 新增 | d 删除 | Tab 切列 | Esc 返回";
        if let Some(status) = status {
            let status_text = truncate_to_width(&status.text, width.saturating_sub(help.len() + 3));
            let style = match status.level {
                StatusLevel::Info => Style::default().fg(Color::Green),
                StatusLevel::Warn => Style::default().fg(Color::Yellow),
                StatusLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            };
            let mut spans = Vec::new();
            spans.push(Span::styled(status_text, style));
            spans.push(Span::raw(" | "));
            spans.push(Span::raw(help));
            Line::from(spans)
        } else {
            Line::raw(truncate_to_width(help, width))
        }
    }

    fn list_lines(&self, width: usize, height: usize, cursor_on: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if self.items.is_empty() {
            lines.push(Line::raw("暂无条目，按 a 新增"));
            return lines;
        }
        let start = self.offset.min(self.items.len());
        let end = (start + height.max(1)).min(self.items.len());
        for (idx, item) in self.items[start..end].iter().enumerate() {
            let absolute = start + idx;
            let prefix = format!("{:>3} ", absolute + 1);
            let mut body = match item {
                ListItem::Single(val) => val.clone(),
                ListItem::Pair(left, right) => format!("{left} | {right}"),
                ListItem::MapEntry(key, value) => format!("{key} | {value}"),
                ListItem::Admin(user, pass, role) => format!("{user} | {pass} | {role}"),
            };
            if let Some(input) = &self.input {
                if input.index == absolute {
                    let cursor = if cursor_on { "|" } else { "" };
                    body = match item {
                        ListItem::Single(_) => format!("{}{}", input.buffer, cursor),
                        ListItem::Pair(left, right) => match input.focus {
                            ListFocus::Left => format!("{}{} | {right}", input.buffer, cursor),
                            ListFocus::Right => format!("{left} | {}{}", input.buffer, cursor),
                            ListFocus::Middle => format!("{left} | {}{}", input.buffer, cursor),
                        },
                        ListItem::MapEntry(key, value) => match input.focus {
                            ListFocus::Left => format!("{}{} | {value}", input.buffer, cursor),
                            ListFocus::Right => format!("{key} | {}{}", input.buffer, cursor),
                            ListFocus::Middle => format!("{key} | {}{}", input.buffer, cursor),
                        },
                        ListItem::Admin(user, pass, role) => match input.focus {
                            ListFocus::Left => {
                                format!("{}{} | {pass} | {role}", input.buffer, cursor)
                            }
                            ListFocus::Middle => {
                                format!("{user} | {}{} | {role}", input.buffer, cursor)
                            }
                            ListFocus::Right => {
                                format!("{user} | {pass} | {}{}", input.buffer, cursor)
                            }
                        },
                    };
                }
            }
            let line = format!("{prefix}{body}");
            let style = if absolute == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(
                truncate_to_width(&line, width.saturating_sub(2)),
                style,
            ));
        }
        lines
    }

    fn ensure_visible(&mut self, height: usize) {
        self.offset = ensure_visible_offset(self.offset, self.selected, height, self.items.len());
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        self.selected = clamp_index(self.selected, delta, self.items.len());
    }

    fn page_selection(&mut self, direction: isize) {
        let step = 5isize;
        self.move_selection(step * direction);
    }

    fn jump_top(&mut self) {
        if !self.items.is_empty() {
            self.selected = 0;
        }
    }

    fn jump_bottom(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        }
    }

    fn toggle_focus(&mut self) {
        if matches!(self.kind, ListKind::StringList) {
            return;
        }
        self.focus = match self.focus {
            ListFocus::Left => ListFocus::Middle,
            ListFocus::Middle => ListFocus::Right,
            ListFocus::Right => ListFocus::Left,
        };
    }

    fn start_edit(&mut self) {
        if self.items.is_empty() {
            self.add_item();
            return;
        }
        let idx = self.selected;
        let buffer = match self.items.get(idx) {
            Some(ListItem::Single(val)) => val.clone(),
            Some(ListItem::Pair(left, right)) => match self.focus {
                ListFocus::Left => left.clone(),
                ListFocus::Right => right.clone(),
                ListFocus::Middle => right.clone(),
            },
            Some(ListItem::MapEntry(key, value)) => match self.focus {
                ListFocus::Left => key.clone(),
                ListFocus::Right => value.clone(),
                ListFocus::Middle => value.clone(),
            },
            Some(ListItem::Admin(user, pass, role)) => match self.focus {
                ListFocus::Left => user.clone(),
                ListFocus::Middle => pass.clone(),
                ListFocus::Right => role.clone(),
            },
            None => String::new(),
        };
        self.input = Some(ListInput {
            index: idx,
            focus: self.focus,
            buffer,
        });
    }

    fn add_item(&mut self) {
        match self.kind {
            ListKind::StringList => self.items.push(ListItem::Single(String::new())),
            ListKind::PairList { .. } => {
                self.items
                    .push(ListItem::Pair(String::new(), String::new()));
            }
            ListKind::MapList => self
                .items
                .push(ListItem::MapEntry(String::new(), String::new())),
            ListKind::AdminList => self.items.push(ListItem::Admin(
                String::new(),
                String::new(),
                "group_admin".to_string(),
            )),
        }
        self.selected = self.items.len().saturating_sub(1);
        self.dirty = true;
        self.start_edit();
    }

    fn delete_item(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.items.remove(self.selected);
        if self.selected >= self.items.len() && !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        }
        self.dirty = true;
    }

    fn apply_input(&mut self, input: ListInput) {
        if let Some(item) = self.items.get_mut(input.index) {
            match item {
                ListItem::Single(val) => {
                    *val = input.buffer;
                }
                ListItem::Pair(left, right) => match input.focus {
                    ListFocus::Left => *left = input.buffer,
                    ListFocus::Right => *right = input.buffer,
                    ListFocus::Middle => *right = input.buffer,
                },
                ListItem::MapEntry(key, value) => match input.focus {
                    ListFocus::Left => *key = input.buffer,
                    ListFocus::Right => *value = input.buffer,
                    ListFocus::Middle => *value = input.buffer,
                },
                ListItem::Admin(user, pass, role) => match input.focus {
                    ListFocus::Left => *user = input.buffer,
                    ListFocus::Middle => *pass = input.buffer,
                    ListFocus::Right => *role = input.buffer,
                },
            }
            self.dirty = true;
        }
    }

    fn apply_to_map(self, map: &mut Map<String, Value>) {
        match self.kind {
            ListKind::StringList => {
                let mut values: Vec<Value> = Vec::new();
                for item in self.items {
                    if let ListItem::Single(val) = item {
                        if val.trim().is_empty() {
                            continue;
                        }
                        values.push(Value::String(val));
                    }
                }
                if values.is_empty() {
                    remove_path_value(map, &self.key);
                } else {
                    set_path_value(map, &self.key, Value::Array(values));
                }
            }
            ListKind::PairList { left, right } => {
                let mut left_values = Vec::new();
                let mut right_values = Vec::new();
                for item in self.items {
                    if let ListItem::Pair(l, r) = item {
                        if l.trim().is_empty() && r.trim().is_empty() {
                            continue;
                        }
                        left_values.push(Value::String(l));
                        right_values.push(Value::String(r));
                    }
                }
                if left_values.is_empty() && right_values.is_empty() {
                    map.remove(&left);
                    map.remove(&right);
                } else {
                    map.insert(left, Value::Array(left_values));
                    map.insert(right, Value::Array(right_values));
                }
            }
            ListKind::AdminList => {
                let mut list = Vec::new();
                for item in self.items {
                    if let ListItem::Admin(user, pass, role) = item {
                        if user.trim().is_empty() {
                            continue;
                        }
                        let mut obj = Map::new();
                        obj.insert("username".to_string(), Value::String(user));
                        obj.insert("password".to_string(), Value::String(pass));
                        if !role.trim().is_empty() {
                            obj.insert("role".to_string(), Value::String(role));
                        }
                        list.push(Value::Object(obj));
                    }
                }
                if list.is_empty() {
                    remove_path_value(map, &self.key);
                } else {
                    set_path_value(map, &self.key, Value::Array(list));
                }
            }
            ListKind::MapList => {
                let mut map_val = Map::new();
                for item in self.items {
                    if let ListItem::MapEntry(key, value) = item {
                        let key = key.trim();
                        let value = value.trim();
                        if key.is_empty() || value.is_empty() {
                            continue;
                        }
                        map_val.insert(key.to_string(), Value::String(value.to_string()));
                    }
                }
                if map_val.is_empty() {
                    remove_path_value(map, &self.key);
                } else {
                    set_path_value(map, &self.key, Value::Object(map_val));
                }
            }
        }
    }
}

fn sanitize_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        if let Some(value) = sanitize_value(value) {
            out.insert(key.clone(), value);
        }
    }
    out
}

fn sanitize_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if let Some(value) = sanitize_value(item) {
                    out.push(value);
                }
            }
            Some(Value::Array(out))
        }
        Value::Object(map) => Some(Value::Object(sanitize_map(map))),
        _ => Some(value.clone()),
    }
}

fn normalize_group_accounts_map(map: &mut Map<String, Value>) {
    if !map.contains_key("accounts") {
        if let Some(alias) = map.remove("acount") {
            map.insert("accounts".to_string(), alias);
        }
    } else {
        map.remove("acount");
    }

    let mut accounts = extract_string_list(map.get("accounts"));
    if accounts.is_empty() {
        if let Some(main) = value_to_string(map.get("mainqqid")) {
            accounts.push(main);
        }
        accounts.extend(extract_string_list(map.get("minorqqid")));
    }

    let mut normalized = Vec::new();
    for account in accounts {
        let trimmed = account.trim();
        if trimmed.is_empty() {
            continue;
        }
        let account = trimmed.to_string();
        if !normalized.contains(&account) {
            normalized.push(account);
        }
    }
    if !normalized.is_empty() {
        map.insert(
            "accounts".to_string(),
            Value::Array(normalized.into_iter().map(Value::String).collect()),
        );
    }

    for key in [
        "mainqqid",
        "minorqqid",
        "mainqq_http_port",
        "minorqq_http_port",
    ] {
        map.remove(key);
    }
}

fn normalize_tui_config_in_place(root: &mut Value) -> Result<bool, String> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "配置根节点必须是 JSON 对象".to_string())?;
    let mut changed = false;

    if let Some(common_obj) = obj.get_mut("common").and_then(|v| v.as_object_mut()) {
        if normalize_tui_common(common_obj) {
            changed = true;
        }
    }

    if let Some(groups_obj) = obj.get_mut("groups").and_then(|v| v.as_object_mut()) {
        for group in groups_obj.values_mut() {
            let Some(group_obj) = group.as_object_mut() else {
                continue;
            };
            if normalize_tui_group(group_obj) {
                changed = true;
            }
        }
        return Ok(changed);
    }

    for (key, value) in obj.iter_mut() {
        if key == "common" || key == "schema_version" || key == "webview_global_admins" {
            continue;
        }
        let Some(group_obj) = value.as_object_mut() else {
            continue;
        };
        if normalize_tui_group(group_obj) {
            changed = true;
        }
    }

    Ok(changed)
}

fn normalize_tui_common(common_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    let mut web_api_obj = common_obj
        .get("web_api")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();

    if let Some(value) = common_obj.remove("use_web_review") {
        if !web_api_obj.contains_key("enabled") {
            web_api_obj.insert("enabled".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("web_review_port") {
        if !web_api_obj.contains_key("port") {
            web_api_obj.insert("port".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("api_token") {
        if !web_api_obj.contains_key("root_token") {
            web_api_obj.insert("root_token".to_string(), value);
        }
        changed = true;
    }
    if let Some(value) = common_obj.remove("token") {
        if !web_api_obj.contains_key("root_token") {
            web_api_obj.insert("root_token".to_string(), value);
        }
        changed = true;
    }
    if !web_api_obj.is_empty() {
        if common_obj
            .get("web_api")
            .and_then(|value| value.as_object())
            != Some(&web_api_obj)
        {
            common_obj.insert("web_api".to_string(), Value::Object(web_api_obj));
            changed = true;
        }
    }

    if normalize_tui_telemetry(common_obj) {
        changed = true;
    }

    for key in [
        "manage_napcat_internal",
        "renewcookies_use_napcat",
        "max_attempts_qzone_autologin",
        "force_chromium_no_sandbox",
        "http-serv-port",
        "max_queue",
    ] {
        if common_obj.remove(key).is_some() {
            changed = true;
        }
    }

    changed
}

fn normalize_tui_telemetry(common_obj: &mut Map<String, Value>) -> bool {
    let Some(telemetry_obj) = common_obj
        .get_mut("telemetry")
        .and_then(|value| value.as_object_mut())
    else {
        return false;
    };
    let mut changed = false;
    for key in ["upload_endpoint", "upload_token"] {
        if telemetry_obj.remove(key).is_some() {
            changed = true;
        }
    }
    if telemetry_obj.is_empty() {
        common_obj.remove("telemetry");
        changed = true;
    }
    changed
}

fn normalize_tui_group(group_obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    let before = group_obj.clone();
    normalize_group_accounts_map(group_obj);
    if *group_obj != before {
        changed = true;
    }

    if !group_obj.contains_key("webview_admins") {
        if let Some(admins) = group_obj.remove("admins") {
            group_obj.insert("webview_admins".to_string(), admins);
            changed = true;
        }
    } else if group_obj.contains_key("admins") {
        group_obj.remove("admins");
        changed = true;
    }

    changed
}

fn write_tui_normalized_config(path: &PathBuf, root: &Value) -> Result<(), String> {
    let mut output =
        serde_json::to_string_pretty(root).map_err(|err| format!("序列化迁移配置失败: {err}"))?;
    output.push('\n');
    fs::write(path, output).map_err(|err| format!("写入迁移后的配置失败: {err}"))
}

fn default_group_config_template() -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("mangroupid".to_string(), Value::String(String::new()));
    map.insert("accounts".to_string(), Value::Array(Vec::new()));
    map.insert("webview_admins".to_string(), Value::Array(Vec::new()));
    map.insert("max_post_stack".to_string(), Value::String("1".to_string()));
    map.insert(
        "max_image_number_one_post".to_string(),
        Value::String("18".to_string()),
    );
    map.insert(
        "friend_add_message".to_string(),
        Value::String(String::new()),
    );
    map.insert("individual_image_in_posts".to_string(), Value::Bool(true));
    map.insert("watermark_text".to_string(), Value::String(String::new()));
    map.insert("quick_replies".to_string(), Value::Object(Map::new()));
    map.insert("review_shortcuts".to_string(), Value::Object(Map::new()));
    map.insert("global_shortcuts".to_string(), Value::Object(Map::new()));
    map.insert("send_schedule".to_string(), Value::Array(Vec::new()));
    map
}

fn resolve_value<'a>(map: &'a Map<String, Value>, spec: &FieldSpec) -> Option<&'a Value> {
    if let Some(value) = get_path_value(map, spec.key) {
        return Some(value);
    }
    for alias in spec.aliases {
        if let Some(value) = get_path_value(map, alias) {
            return Some(value);
        }
    }
    None
}

fn clear_aliases(map: &mut Map<String, Value>, spec: &FieldSpec) {
    for alias in spec.aliases {
        remove_path_value(map, alias);
    }
}

fn get_path_value<'a>(map: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    if !path.contains('.') {
        return map.get(path);
    }
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = map.get(first)?;
    for part in parts {
        current = match current {
            Value::Object(obj) => obj.get(part)?,
            _ => return None,
        };
    }
    Some(current)
}

fn set_path_value(map: &mut Map<String, Value>, path: &str, value: Value) {
    if !path.contains('.') {
        map.insert(path.to_string(), value);
        return;
    }

    let mut parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let leaf = parts.pop().unwrap_or_default();
    if leaf.is_empty() {
        return;
    }

    let mut current = map;
    for part in parts {
        if part.is_empty() {
            return;
        }
        let entry = current
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let Value::Object(obj) = entry else {
            return;
        };
        current = obj;
    }
    current.insert(leaf.to_string(), value);
}

fn remove_path_value(map: &mut Map<String, Value>, path: &str) -> bool {
    if !path.contains('.') {
        return map.remove(path).is_some();
    }

    let mut parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return false;
    }
    let leaf = parts.pop().unwrap_or_default();
    if leaf.is_empty() {
        return false;
    }

    remove_nested_path(map, &parts, leaf)
}

fn remove_nested_path(map: &mut Map<String, Value>, parts: &[&str], leaf: &str) -> bool {
    if parts.is_empty() {
        return map.remove(leaf).is_some();
    }
    let (removed, should_prune) = {
        let Some(Value::Object(child)) = map.get_mut(parts[0]) else {
            return false;
        };
        let removed = remove_nested_path(child, &parts[1..], leaf);
        (removed, removed && child.is_empty())
    };
    if should_prune {
        map.remove(parts[0]);
    }
    removed
}

fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn list_click_index(rect: Rect, y: u16, offset: usize, len: usize) -> Option<usize> {
    if rect.height < 2 {
        return None;
    }
    let start_y = rect.y.saturating_add(1);
    let end_y = rect.y.saturating_add(rect.height).saturating_sub(1);
    if y < start_y || y >= end_y {
        return None;
    }
    let row = y.saturating_sub(start_y) as usize;
    let index = offset.saturating_add(row);
    if index < len { Some(index) } else { None }
}

fn map_from_value(value: Option<&Value>) -> (Map<String, Value>, Option<String>) {
    match value {
        Some(Value::Object(map)) => (map.clone(), None),
        Some(_) => (Map::new(), Some("应为对象类型".to_string())),
        None => (Map::new(), None),
    }
}

fn summarize_messages(items: &[String], max: usize, label: &str) -> String {
    let mut out = String::new();
    if let Some(first) = items.first() {
        out.push_str(first);
    }
    if items.len() > max {
        out.push_str(&format!("（另有 {} {label}）", items.len() - max));
    }
    out
}

fn clamp_index(current: usize, delta: isize, len: usize) -> usize {
    let next = current.saturating_add_signed(delta);
    next.min(len.saturating_sub(1))
}

fn ensure_visible_offset(offset: usize, selected: usize, height: usize, len: usize) -> usize {
    if len == 0 || height == 0 {
        return 0;
    }
    let mut out = offset.min(len.saturating_sub(1));
    if selected < out {
        out = selected;
    } else if selected >= out.saturating_add(height) {
        out = selected.saturating_add(1).saturating_sub(height);
    }
    if out + height > len {
        out = len.saturating_sub(height);
    }
    out
}

fn parse_input_value(input: &str) -> Value {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if matches!(value, Value::Null) {
            return Value::String(String::new());
        }
        return value;
    }
    Value::String(input.to_string())
}

fn field_summary_text(spec: FieldSpec) -> &'static str {
    match spec.key {
        "web_api.enabled" => "启用 HTTP 审核 API",
        "web_api.port" => "HTTP API 监听端口",
        "web_api.root_token" => "HTTP API 根令牌",
        "webview.enabled" => "启用 WebView 前端",
        "webview.host" => "WebView 监听地址",
        "webview.port" => "WebView 监听端口",
        "webview.session_ttl_sec" => "WebView 会话有效期",
        "telemetry.enabled" => "启用投稿遥测",
        "telemetry.local_dir" => "遥测本地目录",
        "telemetry.upload_enabled" => "启用遥测上传",
        "telemetry.upload_interval_sec" => "遥测上传间隔（秒）",
        "telemetry.upload_batch_size" => "遥测批量大小",
        "telemetry.max_append_messages" => "负样本追加消息上限",
        "webview_global_admins" => "全局 WebView 管理员",
        "webview_admins" => "本组 WebView 管理员",
        "http-serv-port" => "旧版 HTTP 端口（兼容）",
        "process_waittime_sec" => "处理等待时长（秒）",
        "min_interval_ms" => "最小发送间隔（毫秒）",
        "max_post_stack" => "暂存堆栈上限",
        "max_image_number_one_post" => "单条最大图片数",
        "send_timeout_ms" => "发送超时（毫秒）",
        "send_max_attempts" => "发送重试次数",
        "tz_offset_minutes" => "时区偏移（分钟）",
        "max_cache_mb" => "内存图片缓存（MB）",
        "napcat_base_url" => "NapCat 反向 WS 地址",
        "napcat_access_token" => "NapCat 访问令牌",
        "manage_napcat_internal" => "内部管理 NapCat",
        "renewcookies_use_napcat" => "NapCat 续 Cookie",
        "max_attempts_qzone_autologin" => "QZone 自动登录重试",
        "force_chromium_no_sandbox" => "Chromium 关闭沙箱",
        "at_unprived_sender" => "私密空间时 @ 投稿人",
        "friend_request_window_sec" => "好友请求窗口（秒）",
        "friend_add_message" => "好友通过自动私信",
        "mangroupid" => "审核群号",
        "accounts" => "账号列表（首项主号）",
        "send_schedule" => "定时发送时间（HH:MM）",
        "individual_image_in_posts" => "发件时同时发原图",
        "watermark_text" => "渲染水印文本",
        "quick_replies" => "快捷回复映射",
        "review_shortcuts" => "审核快捷指令映射",
        "global_shortcuts" => "全局快捷指令映射",
        _ => spec.hint,
    }
}

fn field_detail_text(spec: FieldSpec) -> String {
    match spec.key {
        "web_api.enabled" => "是否启用根令牌 API。仅当你需要脚本/外部系统远程审核时开启。".to_string(),
        "web_api.port" => "Web API 监听端口。建议与 WebView 端口不同，默认 10923。".to_string(),
        "web_api.root_token" => "Web API 认证令牌。建议使用 32 位以上随机串，可通过 OQQWALL_API_TOKEN 覆盖。".to_string(),
        "webview.enabled" => "是否启用内置 Web 审核前端。开启后需要至少配置一个 webview 管理员账号。".to_string(),
        "webview.host" => "WebView 绑定地址。127.0.0.1 仅本机可访问，0.0.0.0 允许局域网/外网访问（需自行做好安全控制）。".to_string(),
        "webview.port" => "WebView 监听端口，默认 10924。避免与其他服务冲突。".to_string(),
        "webview.session_ttl_sec" => "登录会话有效期（秒）。太短会频繁掉线，太长会增加会话泄露风险。".to_string(),
        "telemetry.enabled" => "是否记录投稿遥测并在审核完成后生成训练样本。默认开启。".to_string(),
        "telemetry.local_dir" => "遥测本地目录，相对 OQQWALL_DATA_DIR 解析；默认 data/telemetry。".to_string(),
        "telemetry.upload_enabled" => "是否启用批量上传。默认开启；上传目标和鉴权为内置固定值，不在配置中暴露。".to_string(),
        "telemetry.upload_interval_sec" => "上传轮询间隔（秒），范围 1..86400，默认 30。".to_string(),
        "telemetry.upload_batch_size" => "每批上传样本数。当前实现固定钳制为 20，修改其他值不会生效。".to_string(),
        "telemetry.max_append_messages" => "append_offtopic 负样本最多追加的后续消息数，范围 1..10，默认 2。".to_string(),
        "webview_global_admins" => "全局管理员可访问所有组。支持编辑 username/password/role，role 建议为 global_admin。".to_string(),
        "webview_admins" => "组管理员仅可操作当前组。支持编辑 username/password/role，role 建议为 group_admin。".to_string(),
        "accounts" => "账号列表，首项为主账号。系统会按顺序选可用账号发送。".to_string(),
        "mangroupid" => "审核群号（数字）。审核指令通常只在该群内生效。".to_string(),
        "send_schedule" => "每天定时触发发送，格式 HH:MM，例如 [\"08:30\",\"22:10\"]。".to_string(),
        "quick_replies" => "快捷回复映射（指令 -> 文本）。避免与审核指令（是/否/删等）重名。".to_string(),
        "review_shortcuts" => "审核快捷指令映射（指令 -> 步骤 DSL）。支持覆盖内置审核指令，步骤用 | 分隔，例如“匿 | 是”；不能与 quick_replies 重名。".to_string(),
        "global_shortcuts" => "全局快捷指令映射（指令 -> 步骤 DSL）。支持覆盖内置全局指令，步骤用 | 分隔；首版仅支持会生成后端事件的全局动作。".to_string(),
        "individual_image_in_posts" => "开启后发送时会附带原图；关闭则更偏向仅发送渲染结果。".to_string(),
        "napcat_base_url" => "NapCat 反向 WS 基础地址，支持组覆盖。示例：0.0.0.0:3001/oqqwall/ws".to_string(),
        "napcat_access_token" => "NapCat access_token。建议使用环境变量 OQQWALL_NAPCAT_TOKEN 统一覆盖。".to_string(),
        "process_waittime_sec" => "聚合等待窗口（秒）。越大越可能合并更多内容，但发送延迟更高。".to_string(),
        "min_interval_ms" => "最小发送间隔（毫秒），用于限速防刷。".to_string(),
        "send_timeout_ms" => "发送超时时间（毫秒）。网络慢时可适度调大。".to_string(),
        "send_max_attempts" => "单次发送失败后的最大重试次数。".to_string(),
        "max_post_stack" => "暂存区上限。达到上限时通常会触发刷新/发送策略。".to_string(),
        "max_image_number_one_post" => "单条内容允许的最大图片数，超限可能拆分发送。".to_string(),
        "friend_add_message" => "通过好友后自动发送的提示语。留空则不主动发送。".to_string(),
        "friend_request_window_sec" => "好友请求窗口（秒），用于限频/防重复处理。".to_string(),
        "tz_offset_minutes" => "时区偏移（分钟）。中国大陆通常为 480。".to_string(),
        "max_cache_mb" => "内存图片缓存上限（MB）。过小会频繁回源，过大会占用更多内存。".to_string(),
        _ => match spec.kind {
            FieldKind::Bool { .. } => "布尔配置：可填 true/false，或用空格键快速切换。".to_string(),
            FieldKind::Text => "文本配置：支持直接输入字符串；留空后保存会清除该键。".to_string(),
            FieldKind::StringList => "字符串列表：按 Enter 进入列表编辑器维护多项。".to_string(),
            FieldKind::PairList { .. } => "成对列表：左列/右列一一对应。".to_string(),
            FieldKind::MapList => "映射列表：按 Enter 进入列表编辑器维护 key/value。".to_string(),
            FieldKind::AdminList => "管理员列表：每项包含用户名、密码和角色（group_admin/global_admin）。".to_string(),
        },
    }
}

fn value_display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "<未设置>".to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<未设置>".to_string()),
    }
}

fn value_to_input(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "".to_string()),
    }
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn is_unset_value(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

fn extract_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| value_to_string(Some(v)))
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect(),
        Value::Number(n) => vec![n.to_string()],
        Value::Bool(b) => vec![b.to_string()],
        _ => Vec::new(),
    }
}

fn extract_pair_list(map: &Map<String, Value>, left: &str, right: &str) -> Vec<(String, String)> {
    let left_list = extract_string_list(map.get(left));
    let right_list = extract_string_list(map.get(right));
    let len = left_list.len().max(right_list.len());
    let mut out = Vec::new();
    for idx in 0..len {
        let l = left_list.get(idx).cloned().unwrap_or_default();
        let r = right_list.get(idx).cloned().unwrap_or_default();
        out.push((l, r));
    }
    out
}

fn extract_map_list(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    let mut items: Vec<(String, String)> = map
        .iter()
        .map(|(k, v)| (k.clone(), value_display(v)))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items
}

fn quick_reply_conflicts_with_review_command(key: &str) -> bool {
    is_builtin_review_command_name(key)
}

fn validate_shortcut_entries(
    group: &str,
    field: &str,
    value: &Value,
    errors: &mut Vec<String>,
    validate_definition: fn(&str) -> Result<(), String>,
    quick_replies: Option<&Map<String, Value>>,
) {
    match value {
        Value::Object(entries) => {
            for (cmd, content) in entries {
                let key = match validate_shortcut_name(cmd) {
                    Ok(value) => value,
                    Err(err) => {
                        errors.push(format!("{group}: {field}[{cmd}] 指令名无效: {err}"));
                        continue;
                    }
                };
                if quick_replies
                    .map(|entries| entries.contains_key(&key))
                    .unwrap_or(false)
                {
                    errors.push(format!("{group}: {field} 指令 {key} 与 quick_replies 冲突"));
                }
                match content {
                    Value::String(text) if !text.trim().is_empty() => {
                        if let Err(err) = validate_definition(text.trim()) {
                            errors.push(format!("{group}: {field}[{key}] 定义无效: {err}"));
                        }
                    }
                    Value::String(_) => {
                        errors.push(format!("{group}: {field}[{key}] 内容不能为空"));
                    }
                    _ => {
                        errors.push(format!("{group}: {field}[{key}] 必须是字符串"));
                    }
                }
            }
        }
        Value::Null => {}
        _ => {
            errors.push(format!("{group}: {field} 必须是对象"));
        }
    }
}

fn extract_admin_list(value: Option<&Value>) -> Vec<(String, String, String)> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::Object(obj) => {
                let user = value_to_string(obj.get("username")).unwrap_or_default();
                let pass = value_to_string(obj.get("password")).unwrap_or_default();
                let role = value_to_string(obj.get("role")).unwrap_or_default();
                out.push((user, pass, role));
            }
            Value::String(s) => {
                out.push((s.clone(), String::new(), String::new()));
            }
            _ => {}
        }
    }
    out
}

fn is_valid_admin_role(role: &str) -> bool {
    matches!(role.trim(), "group_admin" | "global_admin")
}

fn bool_value(value: Option<&Value>, default: bool) -> (bool, bool, bool) {
    match value {
        None => (default, true, false),
        Some(Value::Null) => (default, true, false),
        Some(Value::Bool(b)) => (*b, false, false),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => (true, false, false),
            "false" | "0" | "no" => (false, false, false),
            _ => (default, false, true),
        },
        Some(Value::Number(n)) => (n.as_i64().unwrap_or(0) != 0, false, false),
        _ => (default, false, true),
    }
}

fn is_valid_group_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_numeric(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn parse_schedule_str(value: &str) -> Option<u16> {
    let mut parts = value.split(':');
    let hour = parts.next()?.trim().parse::<u16>().ok()?;
    let minute = parts.next()?.trim().parse::<u16>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour.saturating_mul(60).saturating_add(minute))
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return text.chars().take(max_width).collect();
    }
    let limit = max_width.saturating_sub(3);
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + w > limit {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_tui_common_removes_telemetry_endpoint_and_token() {
        let mut common = serde_json::from_value::<Map<String, Value>>(json!({
            "telemetry": {
                "upload_enabled": true,
                "upload_endpoint": "http://127.0.0.1:10925/telemetry/v1/submission/batch",
                "upload_token": "custom-token"
            }
        }))
        .expect("common");
        assert!(normalize_tui_common(&mut common));
        assert_eq!(get_path_value(&common, "telemetry.upload_endpoint"), None);
        assert_eq!(get_path_value(&common, "telemetry.upload_token"), None);
        assert_eq!(
            get_path_value(&common, "telemetry.upload_enabled").and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn common_fields_include_telemetry_entries() {
        let keys: Vec<&str> = COMMON_FIELDS.iter().map(|spec| spec.key).collect();
        assert!(keys.contains(&"telemetry.enabled"));
        assert!(keys.contains(&"telemetry.local_dir"));
        assert!(keys.contains(&"telemetry.upload_enabled"));
        assert!(keys.contains(&"telemetry.upload_interval_sec"));
        assert!(keys.contains(&"telemetry.upload_batch_size"));
        assert!(keys.contains(&"telemetry.max_append_messages"));
    }
}
