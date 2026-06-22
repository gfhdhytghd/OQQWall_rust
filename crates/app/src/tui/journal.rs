use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::io::{self, Stdout, Write};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyEvent,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use oqqwall_rust_core::draft::{Draft, DraftBlock, IngressMessage};
use oqqwall_rust_core::event::{
    AccountEvent, BlobEvent, ConfigEvent, DraftEvent, Event, EventEnvelope, IngressEvent,
    ManualEvent, MediaEvent, RenderEvent, ReviewEvent, ScheduleEvent, SendEvent, SessionEvent,
    SystemEvent,
};
use oqqwall_rust_core::ids::Id128;
use oqqwall_rust_infra::{JournalCorruption, LocalJournal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use unicode_width::UnicodeWidthChar;

struct EventEntry {
    summary: String,
    detail: String,
}

struct UserEntry {
    user_id: String,
    nickname: Option<String>,
    events: Vec<EventEntry>,
    last_ts_ms: i64,
}

struct LoadResult {
    events: Vec<EventEntry>,
    users: Vec<UserEntry>,
    corruption: Option<JournalCorruption>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    All,
    Users,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UserFocus {
    Users,
    Events,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailKey {
    All(usize),
    Users { user: usize, event: usize },
}

#[derive(Clone, Copy, Default)]
struct TabBounds {
    start: u16,
    end: u16,
}

impl TabBounds {
    fn contains(self, x: u16) -> bool {
        x >= self.start && x < self.end
    }

    fn offset(self, dx: u16) -> Self {
        Self {
            start: self.start.saturating_add(dx),
            end: self.end.saturating_add(dx),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct AppLayout {
    tabs: Rect,
    tab_all: TabBounds,
    tab_users: TabBounds,
    all_list: Rect,
    all_detail: Rect,
    users_list: Rect,
    user_events: Rect,
    detail: Rect,
}

struct App {
    data_dir: String,
    events: Vec<EventEntry>,
    selected: Option<usize>,
    list_offset: usize,
    list_height: usize,
    users: Vec<UserEntry>,
    user_selected: Option<usize>,
    user_list_offset: usize,
    user_list_height: usize,
    user_event_selected: Option<usize>,
    user_event_offset: usize,
    user_event_height: usize,
    view: ViewMode,
    user_focus: UserFocus,
    detail_scroll: u16,
    detail_height: usize,
    detail_select_anchor: Option<usize>,
    detail_select_end: Option<usize>,
    detail_wrapped: Vec<String>,
    detail_wrap_width: u16,
    detail_wrap_key: Option<DetailKey>,
    layout: AppLayout,
    load_error: Option<String>,
    corruption: Option<JournalCorruption>,
}

impl App {
    fn new(data_dir: String) -> Self {
        Self {
            data_dir,
            events: Vec::new(),
            selected: None,
            list_offset: 0,
            list_height: 0,
            users: Vec::new(),
            user_selected: None,
            user_list_offset: 0,
            user_list_height: 0,
            user_event_selected: None,
            user_event_offset: 0,
            user_event_height: 0,
            view: ViewMode::All,
            user_focus: UserFocus::Users,
            detail_scroll: 0,
            detail_height: 0,
            detail_select_anchor: None,
            detail_select_end: None,
            detail_wrapped: Vec::new(),
            detail_wrap_width: 0,
            detail_wrap_key: None,
            layout: AppLayout::default(),
            load_error: None,
            corruption: None,
        }
    }

    fn set_events(&mut self, result: LoadResult) {
        self.events = result.events;
        self.users = result.users;
        self.corruption = result.corruption;
        self.load_error = None;
        self.detail_wrapped.clear();
        self.detail_wrap_key = None;

        if self.events.is_empty() {
            self.selected = None;
            self.list_offset = 0;
        } else {
            self.selected = Some(self.events.len() - 1);
        }

        if self.users.is_empty() {
            self.user_selected = None;
            self.user_event_selected = None;
            self.user_list_offset = 0;
            self.user_event_offset = 0;
        } else {
            self.user_selected = Some(0);
            self.user_list_offset = 0;
            self.reset_user_event_selection();
        }

        self.clear_detail_selection();
        self.detail_scroll = 0;
        self.ensure_visible();
        self.clamp_detail_scroll();
    }

    fn reload(&mut self) {
        match load_events(&self.data_dir) {
            Ok(result) => self.set_events(result),
            Err(err) => self.load_error = Some(err),
        }
    }

    fn switch_view(&mut self, view: ViewMode) {
        self.view = view;
        self.clear_detail_selection();
        self.detail_scroll = 0;
        if self.view == ViewMode::Users {
            self.user_focus = UserFocus::Users;
            if self.user_selected.is_none() && !self.users.is_empty() {
                self.user_selected = Some(0);
            }
            if self.user_event_selected.is_none() && self.selected_user_event_len() > 0 {
                self.user_event_selected = Some(self.selected_user_event_len() - 1);
            }
        }
        self.ensure_visible();
        self.clamp_detail_scroll();
    }

    fn toggle_view(&mut self) {
        let next = match self.view {
            ViewMode::All => ViewMode::Users,
            ViewMode::Users => ViewMode::All,
        };
        self.switch_view(next);
    }

    fn focus_users(&mut self) {
        if self.view == ViewMode::Users {
            self.user_focus = UserFocus::Users;
        }
    }

    fn focus_events(&mut self) {
        if self.view == ViewMode::Users {
            self.user_focus = UserFocus::Events;
        }
    }

    fn toggle_focus(&mut self) {
        if self.view != ViewMode::Users {
            return;
        }
        self.user_focus = match self.user_focus {
            UserFocus::Users => UserFocus::Events,
            UserFocus::Events => UserFocus::Users,
        };
    }

    fn selected_entry(&self) -> Option<&EventEntry> {
        self.selected.and_then(|idx| self.events.get(idx))
    }

    fn selected_user_entry(&self) -> Option<&UserEntry> {
        self.user_selected.and_then(|idx| self.users.get(idx))
    }

    fn selected_user_event_entry(&self) -> Option<&EventEntry> {
        let user = self.selected_user_entry()?;
        self.user_event_selected
            .and_then(|idx| user.events.get(idx))
    }

    fn selected_detail_entry(&self) -> Option<&EventEntry> {
        match self.view {
            ViewMode::All => self.selected_entry(),
            ViewMode::Users => self.selected_user_event_entry(),
        }
    }

    fn current_detail_key(&self) -> Option<DetailKey> {
        match self.view {
            ViewMode::All => self.selected.map(DetailKey::All),
            ViewMode::Users => {
                let user = self.user_selected?;
                let event = self.user_event_selected?;
                Some(DetailKey::Users { user, event })
            }
        }
    }

    fn ensure_detail_wrapped(&mut self) {
        let width = self.layout.detail.width.saturating_sub(2).max(1);
        let key = self.current_detail_key();
        if self.detail_wrap_width == width && self.detail_wrap_key == key {
            return;
        }
        self.detail_wrap_width = width;
        self.detail_wrap_key = key;
        self.detail_wrapped = if let Some(entry) = self.selected_detail_entry() {
            wrap_detail_text(&entry.detail, width as usize)
        } else {
            Vec::new()
        };
        self.clamp_detail_selection();
    }

    fn clear_detail_selection(&mut self) {
        self.detail_select_anchor = None;
        self.detail_select_end = None;
    }

    fn start_detail_selection(&mut self, line: usize) {
        self.detail_select_anchor = Some(line);
        self.detail_select_end = Some(line);
    }

    fn update_detail_selection(&mut self, line: usize) {
        if self.detail_select_anchor.is_some() {
            self.detail_select_end = Some(line);
        }
    }

    fn detail_selection_range(&self) -> Option<(usize, usize)> {
        let start = self.detail_select_anchor?;
        let end = self.detail_select_end.unwrap_or(start);
        Some((start.min(end), start.max(end)))
    }

    fn clamp_detail_selection(&mut self) {
        let len = self.detail_wrapped.len();
        if len == 0 {
            self.clear_detail_selection();
            return;
        }
        if let Some(anchor) = self.detail_select_anchor {
            self.detail_select_anchor = Some(anchor.min(len - 1));
        }
        if let Some(end) = self.detail_select_end {
            self.detail_select_end = Some(end.min(len - 1));
        }
    }

    fn ensure_visible(&mut self) {
        match self.view {
            ViewMode::All => self.ensure_all_visible(),
            ViewMode::Users => {
                self.ensure_user_list_visible();
                self.ensure_user_event_visible();
            }
        }
    }

    fn ensure_all_visible(&mut self) {
        let Some(selected) = self.selected else {
            self.list_offset = 0;
            return;
        };
        if self.list_height == 0 {
            return;
        }
        let height = self.list_height as usize;
        if selected < self.list_offset {
            self.list_offset = selected;
        } else if selected >= self.list_offset + height {
            self.list_offset = selected + 1 - height;
        }
        let max_offset = self.events.len().saturating_sub(height);
        if self.list_offset > max_offset {
            self.list_offset = max_offset;
        }
    }

    fn ensure_user_list_visible(&mut self) {
        let Some(selected) = self.user_selected else {
            self.user_list_offset = 0;
            return;
        };
        if self.user_list_height == 0 {
            return;
        }
        let height = self.user_list_height as usize;
        if selected < self.user_list_offset {
            self.user_list_offset = selected;
        } else if selected >= self.user_list_offset + height {
            self.user_list_offset = selected + 1 - height;
        }
        let max_offset = self.users.len().saturating_sub(height);
        if self.user_list_offset > max_offset {
            self.user_list_offset = max_offset;
        }
    }

    fn ensure_user_event_visible(&mut self) {
        let Some(selected) = self.user_event_selected else {
            self.user_event_offset = 0;
            return;
        };
        if self.user_event_height == 0 {
            return;
        }
        let height = self.user_event_height as usize;
        if selected < self.user_event_offset {
            self.user_event_offset = selected;
        } else if selected >= self.user_event_offset + height {
            self.user_event_offset = selected + 1 - height;
        }
        let max_offset = self.selected_user_event_len().saturating_sub(height);
        if self.user_event_offset > max_offset {
            self.user_event_offset = max_offset;
        }
    }

    fn clamp_detail_scroll(&mut self) {
        let max_scroll = self.max_detail_scroll();
        if self.detail_scroll > max_scroll {
            self.detail_scroll = max_scroll;
        }
    }

    fn max_detail_scroll(&mut self) -> u16 {
        self.ensure_detail_wrapped();
        if self.detail_height == 0 {
            return 0;
        }
        let height = self.detail_height as usize;
        let total = self.detail_wrapped.len();
        if total > height {
            (total - height) as u16
        } else {
            0
        }
    }

    fn select_index(&mut self, index: usize) {
        if self.events.is_empty() {
            self.selected = None;
            self.list_offset = 0;
            self.clear_detail_selection();
            return;
        }
        let max_idx = self.events.len() - 1;
        let index = index.min(max_idx);
        self.selected = Some(index);
        self.clear_detail_selection();
        self.detail_scroll = 0;
        self.ensure_all_visible();
        self.clamp_detail_scroll();
    }

    fn move_selection(&mut self, delta: isize) {
        if self.events.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let max_idx = (self.events.len() - 1) as isize;
        let next = (current + delta).clamp(0, max_idx) as usize;
        self.select_index(next);
    }

    fn select_user_index(&mut self, index: usize) {
        if self.users.is_empty() {
            self.user_selected = None;
            self.user_event_selected = None;
            self.user_list_offset = 0;
            self.user_event_offset = 0;
            self.clear_detail_selection();
            return;
        }
        let max_idx = self.users.len() - 1;
        let index = index.min(max_idx);
        self.user_selected = Some(index);
        self.clear_detail_selection();
        self.detail_scroll = 0;
        self.ensure_user_list_visible();
        self.reset_user_event_selection();
        self.ensure_user_event_visible();
        self.clamp_detail_scroll();
    }

    fn move_user_selection(&mut self, delta: isize) {
        if self.users.is_empty() {
            return;
        }
        let current = self.user_selected.unwrap_or(0) as isize;
        let max_idx = (self.users.len() - 1) as isize;
        let next = (current + delta).clamp(0, max_idx) as usize;
        self.select_user_index(next);
    }

    fn select_user_event_index(&mut self, index: usize) {
        let len = self.selected_user_event_len();
        if len == 0 {
            self.user_event_selected = None;
            self.user_event_offset = 0;
            self.clear_detail_selection();
            return;
        }
        let max_idx = len - 1;
        let index = index.min(max_idx);
        self.user_event_selected = Some(index);
        self.clear_detail_selection();
        self.detail_scroll = 0;
        self.ensure_user_event_visible();
        self.clamp_detail_scroll();
    }

    fn move_user_event_selection(&mut self, delta: isize) {
        let len = self.selected_user_event_len();
        if len == 0 {
            return;
        }
        let current = self.user_event_selected.unwrap_or(0) as isize;
        let max_idx = (len - 1) as isize;
        let next = (current + delta).clamp(0, max_idx) as usize;
        self.select_user_event_index(next);
    }

    fn reset_user_event_selection(&mut self) {
        let len = self.selected_user_event_len();
        if len == 0 {
            self.user_event_selected = None;
            self.user_event_offset = 0;
        } else {
            self.user_event_selected = Some(len - 1);
        }
        self.clear_detail_selection();
        self.detail_scroll = 0;
    }

    fn selected_user_event_len(&self) -> usize {
        self.selected_user_entry()
            .map(|user| user.events.len())
            .unwrap_or(0)
    }

    fn move_focus_selection(&mut self, delta: isize) {
        match self.view {
            ViewMode::All => self.move_selection(delta),
            ViewMode::Users => match self.user_focus {
                UserFocus::Users => self.move_user_selection(delta),
                UserFocus::Events => self.move_user_event_selection(delta),
            },
        }
    }

    fn page_focus(&mut self, direction: isize) {
        let step = match self.view {
            ViewMode::All => self.list_height.max(1) as isize,
            ViewMode::Users => match self.user_focus {
                UserFocus::Users => self.user_list_height.max(1) as isize,
                UserFocus::Events => self.user_event_height.max(1) as isize,
            },
        };
        self.move_focus_selection(step * direction);
    }

    fn jump_focus_top(&mut self) {
        match self.view {
            ViewMode::All => self.select_index(0),
            ViewMode::Users => match self.user_focus {
                UserFocus::Users => self.select_user_index(0),
                UserFocus::Events => self.select_user_event_index(0),
            },
        }
    }

    fn jump_focus_bottom(&mut self) {
        match self.view {
            ViewMode::All => {
                if !self.events.is_empty() {
                    self.select_index(self.events.len() - 1);
                }
            }
            ViewMode::Users => match self.user_focus {
                UserFocus::Users => {
                    if !self.users.is_empty() {
                        self.select_user_index(self.users.len() - 1);
                    }
                }
                UserFocus::Events => {
                    let len = self.selected_user_event_len();
                    if len > 0 {
                        self.select_user_event_index(len - 1);
                    }
                }
            },
        }
    }

    fn scroll_detail_lines(&mut self, delta: i16) {
        let max_scroll = self.max_detail_scroll();
        if delta < 0 {
            self.detail_scroll = self.detail_scroll.saturating_sub((-delta) as u16);
        } else {
            self.detail_scroll = (self.detail_scroll + delta as u16).min(max_scroll);
        }
    }

    fn detail_scroll_step(&self) -> u16 {
        let step = (self.detail_height / 2).max(1) as u16;
        step.max(3)
    }
}

pub struct JournalUi {
    app: App,
}

impl JournalUi {
    pub fn new(data_dir: impl Into<String>) -> Self {
        let data_dir = data_dir.into();
        let mut app = App::new(data_dir);
        match load_events(&app.data_dir) {
            Ok(result) => app.set_events(result),
            Err(err) => app.load_error = Some(err),
        }
        Self { app }
    }

    pub fn reload(&mut self) {
        self.app.reload();
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        ui(f, &mut self.app, area);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        handle_key(&mut self.app, key)
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        handle_mouse(&mut self.app, mouse);
    }
}

pub fn run_cli() -> io::Result<()> {
    let mut data_dir: Option<String> = None;
    for arg in env::args().skip(1) {
        if arg == "--help" || arg == "-h" {
            print_usage();
            return Ok(());
        }
        if data_dir.is_none() {
            data_dir = Some(arg);
        } else {
            eprintln!("未知参数: {arg}");
            print_usage();
            return Ok(());
        }
    }

    let data_dir = data_dir.unwrap_or_else(|| "data".to_string());
    let mut app = App::new(data_dir);
    match load_events(&app.data_dir) {
        Ok(result) => app.set_events(result),
        Err(err) => app.load_error = Some(err),
    }

    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;

    if let Err(err) = result {
        eprintln!("日志 TUI 运行异常: {err}");
    }
    Ok(())
}

fn print_usage() {
    println!("用法: journal_tui [data_dir]");
    println!("按键: q/esc 退出, r 重载, t 切换视图, u 用户视图, a 全部视图");
    println!("      方向键/j/k 导航, PgUp/PgDn 翻页, g/G 或 Home/End 跳转首尾");
    println!("      Tab 或 h/l 切焦点（用户视图）, Ctrl+u/d 滚动详情");
    println!("鼠标: 点击选择/切焦点, 在详情区域拖拽复制, 滚轮滚动");
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    let tick_rate = Duration::from_millis(200);
    loop {
        terminal.draw(|f| ui(f, app, f.area()))?;
        if event::poll(tick_rate)? {
            match event::read()? {
                CrosstermEvent::Key(key) => {
                    if handle_key(app, key) {
                        return Ok(());
                    }
                }
                CrosstermEvent::Mouse(mouse) => {
                    handle_mouse(app, mouse);
                }
                _ => {}
            }
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('r') => app.reload(),
        KeyCode::Char('t') => app.toggle_view(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let step = app.detail_scroll_step();
            app.scroll_detail_lines(-(step as i16));
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let step = app.detail_scroll_step();
            app.scroll_detail_lines(step as i16);
        }
        KeyCode::Char('u') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.switch_view(ViewMode::Users);
        }
        KeyCode::Char('a') => app.switch_view(ViewMode::All),
        KeyCode::Tab => app.toggle_focus(),
        KeyCode::Left | KeyCode::Char('h') => app.focus_users(),
        KeyCode::Right | KeyCode::Char('l') => app.focus_events(),
        KeyCode::Up | KeyCode::Char('k') => app.move_focus_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_focus_selection(1),
        KeyCode::PageUp => app.page_focus(-1),
        KeyCode::PageDown => app.page_focus(1),
        KeyCode::Home | KeyCode::Char('g') => app.jump_focus_top(),
        KeyCode::End | KeyCode::Char('G') => app.jump_focus_bottom(),
        _ => {}
    }
    false
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            handle_mouse_click(app, mouse.column, mouse.row);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            handle_mouse_drag(app, mouse.column, mouse.row);
        }
        MouseEventKind::Up(MouseButton::Left) => {
            handle_mouse_release(app, mouse.column, mouse.row);
        }
        MouseEventKind::ScrollUp => {
            handle_mouse_scroll(app, -1, mouse.column, mouse.row);
        }
        MouseEventKind::ScrollDown => {
            handle_mouse_scroll(app, 1, mouse.column, mouse.row);
        }
        _ => {}
    }
}

fn handle_mouse_click(app: &mut App, x: u16, y: u16) {
    if rect_contains(app.layout.tabs, x, y) {
        if app.layout.tab_all.contains(x) {
            app.switch_view(ViewMode::All);
        } else if app.layout.tab_users.contains(x) {
            app.switch_view(ViewMode::Users);
        }
        return;
    }

    if rect_contains(app.layout.detail, x, y) {
        if let Some(line) = detail_line_at_y(app, y, false) {
            app.start_detail_selection(line);
        } else {
            app.clear_detail_selection();
        }
        return;
    }

    match app.view {
        ViewMode::All => {
            if rect_contains(app.layout.all_list, x, y) {
                if let Some(index) =
                    list_click_index(app.layout.all_list, y, app.list_offset, app.events.len())
                {
                    app.select_index(index);
                }
            }
        }
        ViewMode::Users => {
            if rect_contains(app.layout.users_list, x, y) {
                app.focus_users();
                if let Some(index) = list_click_index(
                    app.layout.users_list,
                    y,
                    app.user_list_offset,
                    app.users.len(),
                ) {
                    app.select_user_index(index);
                }
                return;
            }
            if rect_contains(app.layout.user_events, x, y) {
                app.focus_events();
                let len = app.selected_user_event_len();
                if let Some(index) =
                    list_click_index(app.layout.user_events, y, app.user_event_offset, len)
                {
                    app.select_user_event_index(index);
                }
            }
        }
    }
}

fn handle_mouse_drag(app: &mut App, x: u16, y: u16) {
    if app.detail_select_anchor.is_none() {
        return;
    }
    let rect = app.layout.detail;
    if rect.width == 0 || rect.height < 2 {
        return;
    }
    if x < rect.x || x >= rect.x.saturating_add(rect.width) {
        return;
    }
    let top = rect.y.saturating_add(1);
    let bottom = rect.y.saturating_add(rect.height).saturating_sub(2);
    if y < top {
        if app.detail_scroll > 0 {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }
        app.update_detail_selection(app.detail_scroll as usize);
        return;
    }
    if y > bottom {
        let max_scroll = app.max_detail_scroll();
        if app.detail_scroll < max_scroll {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        }
        let line = app.detail_scroll as usize + app.detail_height.saturating_sub(1);
        app.update_detail_selection(line);
        return;
    }
    if let Some(line) = detail_line_at_y(app, y, true) {
        app.update_detail_selection(line);
    }
}

fn handle_mouse_release(app: &mut App, _x: u16, y: u16) {
    if app.detail_select_anchor.is_none() {
        return;
    }
    if let Some(line) = detail_line_at_y(app, y, true) {
        app.update_detail_selection(line);
    }
    if let Some(text) = detail_selection_text(app) {
        let _ = copy_to_clipboard_osc52(&text);
    }
}

fn handle_mouse_scroll(app: &mut App, direction: isize, x: u16, y: u16) {
    let detail_step: i16 = if direction < 0 { -3 } else { 3 };
    match app.view {
        ViewMode::All => {
            if rect_contains(app.layout.all_detail, x, y) {
                app.scroll_detail_lines(detail_step);
                if app.detail_select_anchor.is_some() {
                    if let Some(line) = detail_line_at_y(app, y, true) {
                        app.update_detail_selection(line);
                    }
                }
            } else if rect_contains(app.layout.all_list, x, y) {
                app.move_selection(direction);
            }
        }
        ViewMode::Users => {
            if rect_contains(app.layout.detail, x, y) {
                app.scroll_detail_lines(detail_step);
                if app.detail_select_anchor.is_some() {
                    if let Some(line) = detail_line_at_y(app, y, true) {
                        app.update_detail_selection(line);
                    }
                }
            } else if rect_contains(app.layout.user_events, x, y) {
                app.focus_events();
                app.move_user_event_selection(direction);
            } else if rect_contains(app.layout.users_list, x, y) {
                app.focus_users();
                app.move_user_selection(direction);
            }
        }
    }
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

fn detail_line_at_y(app: &mut App, y: u16, clamp: bool) -> Option<usize> {
    app.ensure_detail_wrapped();
    if app.detail_wrapped.is_empty() || app.detail_height == 0 {
        return None;
    }
    let rect = app.layout.detail;
    if rect.height < 2 {
        return None;
    }
    let start_y = rect.y.saturating_add(1);
    let end_y = rect.y.saturating_add(rect.height).saturating_sub(1);
    let row = if y < start_y {
        if clamp {
            0usize
        } else {
            return None;
        }
    } else if y >= end_y {
        if clamp {
            app.detail_height.saturating_sub(1)
        } else {
            return None;
        }
    } else {
        y.saturating_sub(start_y) as usize
    };
    let mut line = app.detail_scroll as usize + row;
    if line >= app.detail_wrapped.len() {
        if clamp {
            line = app.detail_wrapped.len().saturating_sub(1);
        } else {
            return None;
        }
    }
    Some(line)
}

fn detail_selection_text(app: &mut App) -> Option<String> {
    app.ensure_detail_wrapped();
    let (start, end) = app.detail_selection_range()?;
    if app.detail_wrapped.is_empty() || start >= app.detail_wrapped.len() {
        return None;
    }
    let end = end.min(app.detail_wrapped.len().saturating_sub(1));
    let mut out = String::new();
    for line in &app.detail_wrapped[start..=end] {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.is_empty() { None } else { Some(out) }
}

fn wrap_detail_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_width = 0usize;
        for ch in line.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
            if current_width + ch_width > width && !current.is_empty() {
                out.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
            if current_width >= width {
                out.push(current);
                current = String::new();
                current_width = 0;
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

fn copy_to_clipboard_osc52(text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let payload = STANDARD.encode(text.as_bytes());
    let mut stdout = io::stdout();
    write!(stdout, "\x1b]52;c;{}\x07", payload)?;
    stdout.flush()
}

fn ui(f: &mut Frame, app: &mut App, area: Rect) {
    let size = area;
    app.layout = AppLayout::default();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(size);

    let (tabs_line, tab_all, tab_users) = tabs_line(app.view);
    app.layout.tabs = chunks[0];
    app.layout.tab_all = tab_all.offset(chunks[0].x);
    app.layout.tab_users = tab_users.offset(chunks[0].x);
    let tabs = Paragraph::new(tabs_line).style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(tabs, chunks[0]);

    let status_line = status_text(app);
    let status =
        Paragraph::new(status_line).style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(status, chunks[1]);

    match app.view {
        ViewMode::All => {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(chunks[2]);
            app.layout.all_list = body[0];
            app.layout.all_detail = body[1];
            app.layout.detail = body[1];

            app.list_height = body[0].height.saturating_sub(2) as usize;
            app.detail_height = body[1].height.saturating_sub(2) as usize;
            app.ensure_visible();
            app.clamp_detail_scroll();

            let list = events_list_widget(app);
            f.render_widget(list, body[0]);

            let detail = detail_widget(app);
            f.render_widget(detail, body[1]);
        }
        ViewMode::Users => {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(25),
                    Constraint::Percentage(35),
                    Constraint::Percentage(40),
                ])
                .split(chunks[2]);
            app.layout.users_list = body[0];
            app.layout.user_events = body[1];
            app.layout.detail = body[2];

            app.user_list_height = body[0].height.saturating_sub(2) as usize;
            app.user_event_height = body[1].height.saturating_sub(2) as usize;
            app.detail_height = body[2].height.saturating_sub(2) as usize;
            app.ensure_visible();
            app.clamp_detail_scroll();

            let users = user_list_widget(app);
            f.render_widget(users, body[0]);

            let user_events = user_events_widget(app);
            f.render_widget(user_events, body[1]);

            let detail = detail_widget(app);
            f.render_widget(detail, body[2]);
        }
    }

    let footer_line = help_text(app);
    let footer =
        Paragraph::new(footer_line).style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(footer, chunks[3]);
}

fn tabs_line(view: ViewMode) -> (Line<'static>, TabBounds, TabBounds) {
    let active_style = Style::default();
    let inactive_style = Style::default().add_modifier(Modifier::DIM);
    let mut spans = Vec::new();
    let mut x: u16 = 0;

    let pad = " ";
    spans.push(Span::raw(pad));
    x = x.saturating_add(pad.len() as u16);

    let all_start = x;
    let all_style = if view == ViewMode::All {
        active_style
    } else {
        inactive_style
    };
    spans.push(Span::styled("全部", all_style));
    x = x.saturating_add("全部".len() as u16);
    let all_end = x;

    let sep = " | ";
    spans.push(Span::raw(sep));
    x = x.saturating_add(sep.len() as u16);

    let users_start = x;
    let users_style = if view == ViewMode::Users {
        active_style
    } else {
        inactive_style
    };
    spans.push(Span::styled("用户", users_style));
    x = x.saturating_add("用户".len() as u16);
    let users_end = x;

    (
        Line::from(spans),
        TabBounds {
            start: all_start,
            end: all_end,
        },
        TabBounds {
            start: users_start,
            end: users_end,
        },
    )
}

fn status_text(app: &App) -> String {
    let mut line = match app.view {
        ViewMode::All => {
            let total = app.events.len();
            let selected = app.selected.map(|idx| idx + 1).unwrap_or(0);
            format!(
                "视图=全部 数据目录={} 事件={} 选中={}/{}",
                app.data_dir, total, selected, total
            )
        }
        ViewMode::Users => {
            let total_users = app.users.len();
            let selected_user = app.user_selected.map(|idx| idx + 1).unwrap_or(0);
            let user_display = app
                .selected_user_entry()
                .map(user_display)
                .unwrap_or_else(|| "-".to_string());
            let event_total = app.selected_user_event_len();
            let event_selected = app.user_event_selected.map(|idx| idx + 1).unwrap_or(0);
            let focus = match app.user_focus {
                UserFocus::Users => "用户列表",
                UserFocus::Events => "用户事件",
            };
            format!(
                "视图=用户 数据目录={} 用户={}/{} 当前用户={} 事件={}/{} 焦点={}",
                app.data_dir,
                selected_user,
                total_users,
                user_display,
                event_selected,
                event_total,
                focus
            )
        }
    };

    if let Some(err) = &app.load_error {
        line.push_str(" | 错误=");
        line.push_str(err);
    }
    if let Some(corruption) = &app.corruption {
        let _ = write!(
            line,
            " | 日志损坏 seg={} off={} 原因={}",
            corruption.segment, corruption.offset, corruption.reason
        );
    }
    line
}

fn help_text(_app: &App) -> String {
    "q/esc 退出 | r 重载 | t 切视图 | u 用户视图 | a 全部视图 | Tab/h/l 切焦点 | 方向键/j/k 导航 | PgUp/PgDn 翻页 | g/G Home/End 跳转 | Ctrl+u/d 详情滚动 | 鼠标点击/拖拽复制/滚轮".to_string()
}

fn events_list_widget(app: &App) -> Paragraph<'_> {
    let mut lines = Vec::new();
    if app.events.is_empty() {
        lines.push(Line::raw("暂无事件数据。"));
    } else {
        let start = app.list_offset.min(app.events.len());
        let end = (start + app.list_height.max(1)).min(app.events.len());
        for (idx, entry) in app.events[start..end].iter().enumerate() {
            let absolute = start + idx;
            let prefix = format!("{:>6} ", absolute + 1);
            let mut line_text = String::new();
            line_text.push_str(&prefix);
            line_text.push_str(&entry.summary);
            let style = if Some(absolute) == app.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(line_text, style));
        }
    }
    Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("事件列表"))
}

fn user_list_widget(app: &App) -> Paragraph<'_> {
    let mut lines = Vec::new();
    if app.users.is_empty() {
        lines.push(Line::raw("暂无用户数据。"));
    } else {
        let start = app.user_list_offset.min(app.users.len());
        let end = (start + app.user_list_height.max(1)).min(app.users.len());
        for (idx, user) in app.users[start..end].iter().enumerate() {
            let absolute = start + idx;
            let prefix = format!("{:>4} ", absolute + 1);
            let mut line_text = String::new();
            line_text.push_str(&prefix);
            line_text.push_str(&user_display(user));
            let style = if Some(absolute) == app.user_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(line_text, style));
        }
    }

    let title = match app.user_focus {
        UserFocus::Users => "用户*",
        UserFocus::Events => "用户",
    };
    Paragraph::new(Text::from(lines)).block(Block::default().borders(Borders::ALL).title(title))
}

fn user_events_widget(app: &App) -> Paragraph<'_> {
    let mut lines = Vec::new();
    let Some(user) = app.selected_user_entry() else {
        lines.push(Line::raw("未选中用户。"));
        return Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("用户事件"));
    };

    if user.events.is_empty() {
        lines.push(Line::raw("该用户暂无事件。"));
    } else {
        let start = app.user_event_offset.min(user.events.len());
        let end = (start + app.user_event_height.max(1)).min(user.events.len());
        for (idx, entry) in user.events[start..end].iter().enumerate() {
            let absolute = start + idx;
            let prefix = format!("{:>6} ", absolute + 1);
            let mut line_text = String::new();
            line_text.push_str(&prefix);
            line_text.push_str(&entry.summary);
            let style = if Some(absolute) == app.user_event_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(line_text, style));
        }
    }

    let title = match app.user_focus {
        UserFocus::Events => "用户事件*",
        UserFocus::Users => "用户事件",
    };
    Paragraph::new(Text::from(lines)).block(Block::default().borders(Borders::ALL).title(title))
}

fn detail_widget(app: &mut App) -> Paragraph<'_> {
    app.ensure_detail_wrapped();
    if app.detail_wrapped.is_empty() {
        return Paragraph::new("未选中事件。")
            .block(Block::default().borders(Borders::ALL).title("事件详情"));
    }
    let selection = app.detail_selection_range();
    let highlight = Style::default().add_modifier(Modifier::REVERSED);
    let mut lines = Vec::new();
    for (idx, line) in app.detail_wrapped.iter().enumerate() {
        let style = if selection
            .map(|(start, end)| idx >= start && idx <= end)
            .unwrap_or(false)
        {
            highlight
        } else {
            Style::default()
        };
        lines.push(Line::styled(line.clone(), style));
    }
    Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("事件详情"))
        .scroll((app.detail_scroll, 0))
}

fn load_events(data_dir: &str) -> Result<LoadResult, String> {
    let journal = LocalJournal::open(data_dir).map_err(|err| format!("打开日志失败: {err}"))?;
    let mut events = Vec::new();
    let mut users: HashMap<String, UserEntry> = HashMap::new();
    let mut ingress_user: HashMap<Id128, String> = HashMap::new();
    let replay = journal.replay(None, |env| {
        let summary = summarize_event(env);
        let detail = detail_event(env);
        events.push(EventEntry { summary, detail });

        match &env.event {
            Event::Ingress(IngressEvent::MessageAccepted {
                ingress_id,
                user_id,
                sender_name,
                message,
                ..
            }) => {
                ingest_user_event(
                    env,
                    ingress_id,
                    user_id,
                    sender_name,
                    message,
                    "已接收",
                    &mut users,
                    &mut ingress_user,
                );
            }
            Event::Ingress(IngressEvent::MessageSynced {
                ingress_id,
                user_id,
                sender_name,
                message,
                ..
            }) => {
                ingest_user_event(
                    env,
                    ingress_id,
                    user_id,
                    sender_name,
                    message,
                    "已同步",
                    &mut users,
                    &mut ingress_user,
                );
            }
            Event::Draft(DraftEvent::PostDraftCreated {
                ingress_ids, draft, ..
            }) => {
                let summary = summarize_user_draft(env.ts_ms, draft);
                let detail = detail_event(env);
                let mut seen_users = HashSet::new();
                for ingress_id in ingress_ids {
                    let Some(user_id) = ingress_user.get(ingress_id) else {
                        continue;
                    };
                    if !seen_users.insert(user_id.clone()) {
                        continue;
                    }
                    let entry = users.entry(user_id.clone()).or_insert_with(|| UserEntry {
                        user_id: user_id.clone(),
                        nickname: None,
                        events: Vec::new(),
                        last_ts_ms: env.ts_ms,
                    });
                    entry.events.push(EventEntry {
                        summary: summary.clone(),
                        detail: detail.clone(),
                    });
                    entry.last_ts_ms = env.ts_ms;
                }
            }
            _ => {}
        }
    });

    match replay {
        Ok(outcome) => {
            let mut users_vec: Vec<UserEntry> = users.into_values().collect();
            users_vec.sort_by(|a, b| {
                b.last_ts_ms
                    .cmp(&a.last_ts_ms)
                    .then_with(|| a.user_id.cmp(&b.user_id))
            });
            Ok(LoadResult {
                events,
                users: users_vec,
                corruption: outcome.corruption,
            })
        }
        Err(err) => Err(format!("日志回放失败: {err}")),
    }
}

fn ingest_user_event(
    env: &EventEnvelope,
    ingress_id: &Id128,
    user_id: &str,
    sender_name: &Option<String>,
    message: &IngressMessage,
    label: &str,
    users: &mut HashMap<String, UserEntry>,
    ingress_user: &mut HashMap<Id128, String>,
) {
    ingress_user.insert(*ingress_id, user_id.to_string());
    let entry = users
        .entry(user_id.to_string())
        .or_insert_with(|| UserEntry {
            user_id: user_id.to_string(),
            nickname: None,
            events: Vec::new(),
            last_ts_ms: env.ts_ms,
        });
    if let Some(name) = sender_name.as_ref().map(|name| name.trim()) {
        if !name.is_empty() {
            entry.nickname = Some(name.to_string());
        }
    }
    let summary = summarize_user_message(env.ts_ms, label, message);
    let detail = detail_event(env);
    entry.events.push(EventEntry { summary, detail });
    entry.last_ts_ms = env.ts_ms;
}

fn summarize_event(env: &EventEnvelope) -> String {
    let (kind, hint) = summary_parts(&env.event);
    if hint.is_empty() {
        format!("{} {} {}", env.ts_ms, short_id(env.id), kind)
    } else {
        format!("{} {} {} {}", env.ts_ms, short_id(env.id), kind, hint)
    }
}

fn summarize_user_message(ts_ms: i64, label: &str, message: &IngressMessage) -> String {
    let preview = message_preview(message);
    let mut summary = format!("{} {}: {}", ts_ms, label, preview);
    let attachments = message.attachments.len();
    if attachments > 0 {
        let _ = write!(summary, " 附件={}", attachments);
    }
    summary
}

fn summarize_user_draft(ts_ms: i64, draft: &Draft) -> String {
    let text = draft_preview_text(draft);
    let preview = if text.is_empty() {
        "无文本".to_string()
    } else {
        compact_text(&text, 72)
    };
    let mut summary = format!("{} 草稿: {}", ts_ms, preview);
    let attachments = draft_attachment_count(draft);
    if attachments > 0 {
        let _ = write!(summary, " 附件={}", attachments);
    }
    summary
}

fn message_preview(message: &IngressMessage) -> String {
    let preview = compact_text(&message.text, 72);
    if preview.is_empty() {
        "无文本".to_string()
    } else {
        preview
    }
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    let mut last_space = false;
    let mut truncated = false;

    for ch in text.chars() {
        let is_space = ch.is_whitespace();
        if is_space {
            if !last_space && !out.is_empty() {
                if count >= max_chars {
                    truncated = true;
                    break;
                }
                out.push(' ');
                count += 1;
                last_space = true;
            }
            continue;
        }
        if count >= max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
        count += 1;
        last_space = false;
    }

    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        return String::new();
    }
    if truncated {
        format!("{trimmed}...")
    } else {
        trimmed
    }
}

fn draft_preview_text(draft: &Draft) -> String {
    let mut parts = Vec::new();
    for block in &draft.blocks {
        let text = match block {
            DraftBlock::Paragraph { text } => text.as_str(),
            DraftBlock::Reply { preview } => preview.body.as_str(),
            DraftBlock::Poke => "[戳一戳]",
            DraftBlock::JsonCard { .. } => "[卡片]",
            DraftBlock::Forward { .. } => "[合并转发聊天记录]",
            DraftBlock::Attachment { .. } => "",
        };
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
    }
    parts.join(" ")
}

fn draft_attachment_count(draft: &Draft) -> usize {
    draft
        .blocks
        .iter()
        .filter(|block| matches!(block, DraftBlock::Attachment { .. }))
        .count()
}

fn user_display(user: &UserEntry) -> String {
    let nickname = user
        .nickname
        .as_ref()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or("-");
    format!("{}({})", nickname, user.user_id)
}

fn detail_event(env: &EventEnvelope) -> String {
    let (kind, hint) = summary_parts(&env.event);
    let mut out = String::new();
    let _ = writeln!(out, "ts_ms: {}", env.ts_ms);
    let _ = writeln!(out, "id: {}", full_id(env.id));
    let _ = writeln!(out, "actor: {}", full_id(env.actor));
    match env.correlation_id {
        Some(id) => {
            let _ = writeln!(out, "correlation_id: {}", full_id(id));
        }
        None => {
            let _ = writeln!(out, "correlation_id: -");
        }
    }
    let _ = writeln!(out, "kind: {kind}");
    if !hint.is_empty() {
        let _ = writeln!(out, "hint: {hint}");
    }
    let _ = writeln!(out, "event:");
    let _ = writeln!(out, "{:#?}", env.event);
    out
}

fn summary_parts(event: &Event) -> (&'static str, String) {
    match event {
        Event::System(SystemEvent::Booted) => ("System.Booted", String::new()),
        Event::System(SystemEvent::SnapshotLoaded) => ("System.SnapshotLoaded", String::new()),
        Event::System(SystemEvent::SnapshotTaken) => ("System.SnapshotTaken", String::new()),
        Event::Config(ConfigEvent::Applied {
            version,
            config_blob,
        }) => {
            let blob = config_blob.map(short_id).unwrap_or_else(|| "-".to_string());
            ("Config.Applied", format!("version={version} blob={blob}"))
        }
        Event::Ingress(ingress) => match ingress {
            IngressEvent::MessageAccepted {
                ingress_id,
                group_id,
                chat_id,
                user_id,
                ..
            } => (
                "Ingress.MessageAccepted",
                format!(
                    "ingress={} group={} chat={} user={}",
                    short_id(*ingress_id),
                    group_id,
                    chat_id,
                    user_id
                ),
            ),
            IngressEvent::MessageSynced {
                ingress_id,
                group_id,
                chat_id,
                user_id,
                ..
            } => (
                "Ingress.MessageSynced",
                format!(
                    "ingress={} group={} chat={} user={}",
                    short_id(*ingress_id),
                    group_id,
                    chat_id,
                    user_id
                ),
            ),
            IngressEvent::MessageIgnored { ingress_id, reason } => (
                "Ingress.MessageIgnored",
                format!("ingress={} reason={:?}", short_id(*ingress_id), reason),
            ),
            IngressEvent::MessageRecalled {
                ingress_id,
                recalled_at_ms,
            } => (
                "Ingress.MessageRecalled",
                format!(
                    "ingress={} recalled_at={}",
                    short_id(*ingress_id),
                    recalled_at_ms
                ),
            ),
            IngressEvent::InputStatusUpdated {
                group_id,
                chat_id,
                user_id,
                status,
                ..
            } => (
                "Ingress.InputStatusUpdated",
                format!(
                    "group={} chat={} user={} status={:?}",
                    group_id, chat_id, user_id, status
                ),
            ),
        },
        Event::Session(session) => match session {
            SessionEvent::Opened {
                session_id,
                group_id,
                chat_id,
                user_id,
                close_at_ms,
                ..
            } => (
                "Session.Opened",
                format!(
                    "session={} group={} chat={} user={} close_at={}",
                    short_id(*session_id),
                    group_id,
                    chat_id,
                    user_id,
                    close_at_ms
                ),
            ),
            SessionEvent::Appended {
                session_id,
                ingress_id,
                close_at_ms,
                ..
            } => (
                "Session.Appended",
                format!(
                    "session={} ingress={} close_at={}",
                    short_id(*session_id),
                    short_id(*ingress_id),
                    close_at_ms
                ),
            ),
            SessionEvent::Closed {
                session_id,
                closed_at_ms,
            } => (
                "Session.Closed",
                format!(
                    "session={} closed_at={}",
                    short_id(*session_id),
                    closed_at_ms
                ),
            ),
        },
        Event::Draft(draft) => match draft {
            DraftEvent::PostDraftCreated {
                post_id,
                session_id,
                group_id,
                ingress_ids,
                ..
            } => (
                "Draft.PostDraftCreated",
                format!(
                    "post={} session={} group={} ingress_count={}",
                    short_id(*post_id),
                    short_id(*session_id),
                    group_id,
                    ingress_ids.len()
                ),
            ),
        },
        Event::Media(media) => match media {
            MediaEvent::AvatarFetchRequested { user_id } => {
                ("Media.AvatarFetchRequested", format!("user_id={}", user_id))
            }
            MediaEvent::MediaFetchRequested {
                ingress_id,
                attachment_index,
                attempt,
            } => (
                "Media.FetchRequested",
                format!(
                    "ingress={} idx={} attempt={}",
                    short_id(*ingress_id),
                    attachment_index,
                    attempt
                ),
            ),
            MediaEvent::MediaFetchSucceeded {
                ingress_id,
                attachment_index,
                blob_id,
            } => (
                "Media.FetchSucceeded",
                format!(
                    "ingress={} idx={} blob={}",
                    short_id(*ingress_id),
                    attachment_index,
                    short_id(*blob_id)
                ),
            ),
            MediaEvent::MediaFetchFailed {
                ingress_id,
                attachment_index,
                attempt,
                retry_at_ms,
                ..
            } => (
                "Media.FetchFailed",
                format!(
                    "ingress={} idx={} attempt={} retry_at={}",
                    short_id(*ingress_id),
                    attachment_index,
                    attempt,
                    retry_at_ms
                ),
            ),
        },
        Event::Render(render) => match render {
            RenderEvent::RenderRequested {
                post_id,
                attempt,
                requested_at_ms,
            } => (
                "Render.Requested",
                format!(
                    "post={} attempt={} requested_at={}",
                    short_id(*post_id),
                    attempt,
                    requested_at_ms
                ),
            ),
            RenderEvent::PngReady { post_id, blob_id } => (
                "Render.PngReady",
                format!("post={} blob={}", short_id(*post_id), short_id(*blob_id)),
            ),
            RenderEvent::PngBatchReady { post_id, blob_ids } => (
                "Render.PngBatchReady",
                format!("post={} blobs={}", short_id(*post_id), blob_ids.len()),
            ),
            RenderEvent::RenderFailed {
                post_id,
                attempt,
                retry_at_ms,
                ..
            } => (
                "Render.Failed",
                format!(
                    "post={} attempt={} retry_at={}",
                    short_id(*post_id),
                    attempt,
                    retry_at_ms
                ),
            ),
        },
        Event::Review(review) => match review {
            ReviewEvent::ReviewItemCreated {
                review_id,
                post_id,
                review_code,
            } => (
                "Review.ItemCreated",
                format!(
                    "review={} post={} code={}",
                    short_id(*review_id),
                    short_id(*post_id),
                    review_code
                ),
            ),
            ReviewEvent::ReviewPublishRequested { review_id } => (
                "Review.PublishRequested",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewPublished {
                review_id,
                audit_msg_id,
            } => (
                "Review.Published",
                format!("review={} audit={}", short_id(*review_id), audit_msg_id),
            ),
            ReviewEvent::ReviewPublishFailed {
                review_id,
                attempt,
                retry_at_ms,
                error,
            } => (
                "Review.PublishFailed",
                format!(
                    "review={} attempt={} retry_at={} err={}",
                    short_id(*review_id),
                    attempt,
                    retry_at_ms,
                    compact_text(error, 48)
                ),
            ),
            ReviewEvent::ReviewDelayed {
                review_id,
                not_before_ms,
            } => (
                "Review.Delayed",
                format!(
                    "review={} not_before={}",
                    short_id(*review_id),
                    not_before_ms
                ),
            ),
            ReviewEvent::ReviewDecisionRecorded {
                review_id,
                decision,
                decided_by,
                decided_at_ms,
            } => (
                "Review.DecisionRecorded",
                format!(
                    "review={} decision={:?} by={} at={}",
                    short_id(*review_id),
                    decision,
                    decided_by,
                    decided_at_ms
                ),
            ),
            ReviewEvent::ReviewDecisionReasonRecorded {
                review_id,
                decision,
                reason,
            } => (
                "Review.DecisionReasonRecorded",
                format!(
                    "review={} decision={:?} reason_len={}",
                    short_id(*review_id),
                    decision,
                    reason.as_ref().map(|value| value.len()).unwrap_or(0)
                ),
            ),
            ReviewEvent::ReviewCommentAdded { review_id, text } => (
                "Review.CommentAdded",
                format!("review={} text_len={}", short_id(*review_id), text.len()),
            ),
            ReviewEvent::ReviewSubmitterNoticeRequested {
                review_id,
                kind,
                reason,
            } => (
                "Review.SubmitterNoticeRequested",
                format!(
                    "review={} kind={:?} reason_len={}",
                    short_id(*review_id),
                    kind,
                    reason.as_ref().map(|value| value.len()).unwrap_or(0)
                ),
            ),
            ReviewEvent::ReviewReplyRequested { review_id, text } => (
                "Review.ReplyRequested",
                format!("review={} text_len={}", short_id(*review_id), text.len()),
            ),
            ReviewEvent::ReviewRefreshRequested { review_id } => (
                "Review.RefreshRequested",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewRerenderRequested { review_id } => (
                "Review.RerenderRequested",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewSelectAllRequested { review_id } => (
                "Review.SelectAllRequested",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewAnonToggled { review_id } => (
                "Review.AnonToggled",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewExpandRequested { review_id } => (
                "Review.ExpandRequested",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewDisplayRequested { review_id } => (
                "Review.DisplayRequested",
                format!("review={}", short_id(*review_id)),
            ),
            ReviewEvent::ReviewBlacklistRequested { review_id, reason } => {
                let reason_len = reason.as_ref().map(|v| v.len()).unwrap_or(0);
                (
                    "Review.BlacklistRequested",
                    format!("review={} reason_len={}", short_id(*review_id), reason_len),
                )
            }
            ReviewEvent::ReviewBlacklistRemoved {
                group_id,
                sender_id,
            } => (
                "Review.BlacklistRemoved",
                format!("group={} sender={}", group_id, sender_id),
            ),
            ReviewEvent::ReviewBlacklistAdded {
                group_id,
                sender_id,
                reason,
            } => {
                let reason_len = reason.as_ref().map(|v| v.len()).unwrap_or(0);
                (
                    "Review.BlacklistAdded",
                    format!(
                        "group={} sender={} reason_len={}",
                        group_id, sender_id, reason_len
                    ),
                )
            }
            ReviewEvent::ReviewQuickReplyRequested { review_id, key } => (
                "Review.QuickReplyRequested",
                format!("review={} key={}", short_id(*review_id), key),
            ),
            ReviewEvent::ReviewExternalNumberSet {
                group_id,
                next_number,
            } => (
                "Review.ExternalNumberSet",
                format!("group={} next={}", group_id, next_number),
            ),
            ReviewEvent::ReviewExternalCodeCleared { post_id } => (
                "Review.ExternalCodeCleared",
                format!("post={}", short_id(*post_id)),
            ),
            ReviewEvent::ReviewExternalCodeAssigned {
                post_id,
                group_id,
                external_code,
            } => (
                "Review.ExternalCodeAssigned",
                format!(
                    "post={} group={} code={}",
                    short_id(*post_id),
                    group_id,
                    external_code
                ),
            ),
            ReviewEvent::ReviewInfoSynced {
                review_id,
                post_id,
                review_code,
            } => (
                "Review.InfoSynced",
                format!(
                    "review={} post={} code={}",
                    short_id(*review_id),
                    short_id(*post_id),
                    review_code
                ),
            ),
        },
        Event::Schedule(schedule) => match schedule {
            ScheduleEvent::SendPlanCreated {
                post_id,
                group_id,
                not_before_ms,
                priority,
                seq,
            } => (
                "Schedule.SendPlanCreated",
                format!(
                    "post={} group={} not_before={} prio={:?} seq={}",
                    short_id(*post_id),
                    group_id,
                    not_before_ms,
                    priority,
                    seq
                ),
            ),
            ScheduleEvent::SendPlanRescheduled {
                post_id,
                group_id,
                not_before_ms,
                priority,
                seq,
            } => (
                "Schedule.SendPlanRescheduled",
                format!(
                    "post={} group={} not_before={} prio={:?} seq={}",
                    short_id(*post_id),
                    group_id,
                    not_before_ms,
                    priority,
                    seq
                ),
            ),
            ScheduleEvent::SendPlanCanceled { post_id } => (
                "Schedule.SendPlanCanceled",
                format!("post={}", short_id(*post_id)),
            ),
            ScheduleEvent::GroupFlushRequested {
                group_id,
                minute_of_day,
                day_index,
                reason,
            } => (
                "Schedule.GroupFlushRequested",
                format!(
                    "group={} minute={} day={} reason={:?}",
                    group_id, minute_of_day, day_index, reason
                ),
            ),
        },
        Event::Send(send) => match send {
            SendEvent::SendStarted {
                post_id,
                group_id,
                account_id,
                started_at_ms,
            } => (
                "Send.Started",
                format!(
                    "post={} group={} account={} started_at={}",
                    short_id(*post_id),
                    group_id,
                    account_id,
                    started_at_ms
                ),
            ),
            SendEvent::SendAccountSucceeded {
                post_id,
                account_id,
                finished_at_ms,
                remote_id,
            } => (
                "Send.AccountSucceeded",
                format!(
                    "post={} account={} finished_at={} remote={}",
                    short_id(*post_id),
                    account_id,
                    finished_at_ms,
                    remote_id.as_deref().unwrap_or("-")
                ),
            ),
            SendEvent::SendAccountFailed {
                post_id,
                account_id,
                attempt,
                ..
            } => (
                "Send.AccountFailed",
                format!(
                    "post={} account={} attempt={}",
                    short_id(*post_id),
                    account_id,
                    attempt
                ),
            ),
            SendEvent::SendSucceeded {
                post_id,
                account_id,
                finished_at_ms,
                remote_id,
            } => (
                "Send.Succeeded",
                format!(
                    "post={} account={} finished_at={} remote={}",
                    short_id(*post_id),
                    account_id,
                    finished_at_ms,
                    remote_id.as_deref().unwrap_or("-")
                ),
            ),
            SendEvent::SendFailed {
                post_id,
                account_id,
                attempt,
                retry_at_ms,
                ..
            } => (
                "Send.Failed",
                format!(
                    "post={} account={} attempt={} retry_at={}",
                    short_id(*post_id),
                    account_id,
                    attempt,
                    retry_at_ms
                ),
            ),
            SendEvent::SendGaveUp { post_id, reason } => (
                "Send.GaveUp",
                format!("post={} reason_len={}", short_id(*post_id), reason.len()),
            ),
            SendEvent::QzonePostPublished {
                group_id,
                account_id,
                remote_id,
                items,
                ..
            } => (
                "Send.QzonePostPublished",
                format!(
                    "group={} account={} remote={} items={}",
                    group_id,
                    account_id,
                    remote_id,
                    items.len()
                ),
            ),
            SendEvent::QzonePostWithdrawRequested {
                post_id,
                account_id,
                remote_id,
                withdrawn_post_ids,
                requested_at_ms,
                ..
            } => (
                "Send.QzonePostWithdrawRequested",
                format!(
                    "post={} account={} remote={} withdrawn={} requested_at={}",
                    short_id(*post_id),
                    account_id,
                    remote_id,
                    withdrawn_post_ids.len(),
                    requested_at_ms
                ),
            ),
            SendEvent::QzonePostWithdrawSucceeded {
                post_id,
                account_id,
                remote_id,
                withdrawn_post_ids,
                withdrawn_at_ms,
                ..
            } => (
                "Send.QzonePostWithdrawSucceeded",
                format!(
                    "post={} account={} remote={} withdrawn={} at={}",
                    short_id(*post_id),
                    account_id,
                    remote_id,
                    withdrawn_post_ids.len(),
                    withdrawn_at_ms
                ),
            ),
            SendEvent::QzonePostWithdrawFailed {
                post_id,
                account_id,
                remote_id,
                withdrawn_post_ids,
                error,
            } => (
                "Send.QzonePostWithdrawFailed",
                format!(
                    "post={} account={} remote={} withdrawn={} err={}",
                    short_id(*post_id),
                    account_id,
                    remote_id,
                    withdrawn_post_ids.len(),
                    compact_text(error, 48)
                ),
            ),
        },
        Event::Blob(blob) => match blob {
            BlobEvent::BlobRegistered {
                blob_id,
                size_bytes,
            } => (
                "Blob.Registered",
                format!("blob={} size={}", short_id(*blob_id), size_bytes),
            ),
            BlobEvent::BlobPersisted { blob_id, path } => (
                "Blob.Persisted",
                format!("blob={} path_len={}", short_id(*blob_id), path.len()),
            ),
            BlobEvent::BlobReleased { blob_id } => {
                ("Blob.Released", format!("blob={}", short_id(*blob_id)))
            }
            BlobEvent::BlobGcRequested { blob_id } => {
                ("Blob.GcRequested", format!("blob={}", short_id(*blob_id)))
            }
        },
        Event::Account(account) => match account {
            AccountEvent::AccountEnabled { account_id } => {
                ("Account.Enabled", format!("account={account_id}"))
            }
            AccountEvent::AccountDisabled { account_id } => {
                ("Account.Disabled", format!("account={account_id}"))
            }
            AccountEvent::AccountCooldownSet {
                account_id,
                cooldown_until_ms,
            } => (
                "Account.CooldownSet",
                format!("account={account_id} until={cooldown_until_ms}"),
            ),
            AccountEvent::AccountLastSendUpdated {
                account_id,
                last_send_ms,
            } => (
                "Account.LastSendUpdated",
                format!("account={account_id} last_send={last_send_ms}"),
            ),
        },
        Event::Manual(manual) => match manual {
            ManualEvent::ManualInterventionRequired { post_id, reason } => (
                "Manual.InterventionRequired",
                format!("post={} reason_len={}", short_id(*post_id), reason.len()),
            ),
            ManualEvent::ManualInterventionResolved { post_id } => (
                "Manual.InterventionResolved",
                format!("post={}", short_id(*post_id)),
            ),
        },
    }
}

fn short_id(id: Id128) -> String {
    let hex = format!("{:032x}", id.0);
    hex[..8].to_string()
}

fn full_id(id: Id128) -> String {
    format!("{:032x}", id.0)
}
