use std::env;
use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyEvent,
    KeyModifiers, MouseButton, MouseEvent,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use unicode_width::UnicodeWidthStr;

use super::config_editor::ConfigEditor;
use super::journal::JournalUi;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MainTab {
    Config,
    Journal,
}

struct App {
    tab: MainTab,
    config: ConfigEditor,
    journal: JournalUi,
    layout: TabLayout,
}

#[derive(Clone, Copy, Default)]
struct TabBounds {
    start: u16,
    end: u16,
}

impl TabBounds {
    fn contains(&self, x: u16) -> bool {
        x >= self.start && x < self.end
    }

    fn offset(self, offset: u16) -> Self {
        Self {
            start: self.start.saturating_add(offset),
            end: self.end.saturating_add(offset),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct TabLayout {
    bar: ratatui::layout::Rect,
    config: TabBounds,
    journal: TabBounds,
}

pub fn run_cli(args: &[String]) -> io::Result<()> {
    let mut config_path = env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string());
    let mut data_dir = "data".to_string();

    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--tui" => {}
            "--config" => {
                let Some(value) = iter.next() else {
                    eprintln!("--config 缺少参数值");
                    print_usage();
                    return Ok(());
                };
                config_path = value.to_string();
            }
            "--data-dir" => {
                let Some(value) = iter.next() else {
                    eprintln!("--data-dir 缺少参数值");
                    print_usage();
                    return Ok(());
                };
                data_dir = value.to_string();
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                eprintln!("未知参数: {other}");
                print_usage();
                return Ok(());
            }
        }
    }

    let config = match ConfigEditor::load(config_path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("TUI 启动失败: {err}");
            return Ok(());
        }
    };

    let journal = JournalUi::new(data_dir);
    let mut app = App {
        tab: MainTab::Config,
        config,
        journal,
        layout: TabLayout::default(),
    };

    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;

    if let Err(err) = result {
        eprintln!("TUI 运行异常: {err}");
    }
    Ok(())
}

fn print_usage() {
    println!("用法: OQQWall_RUST --tui [--config <路径>] [--data-dir <路径>]");
    println!("全局按键: 1/2 切页, q 退出");
    println!(
        "配置页: 方向键移动, Tab 切焦点, Enter/e 编辑, 空格切布尔, a 新增字段, g 新增组, x 删除组, s 保存, r 重载"
    );
    println!("列表编辑: Enter/e 编辑, a 新增, d 删除, Tab 切列, Esc 返回");
    println!("日志页: r 重载, t 切视图, u 用户视图, a 全部视图, 方向键导航, q 退出");
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
        app.config.tick();
        terminal.draw(|f| ui(f, app))?;
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
    if app.tab == MainTab::Config && app.config.is_editing() {
        app.config.handle_key(key);
        return false;
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('1') => {
            app.tab = MainTab::Config;
            return false;
        }
        KeyCode::Char('2') => {
            app.tab = MainTab::Journal;
            return false;
        }
        _ => {}
    }

    match app.tab {
        MainTab::Config => app.config.handle_key(key),
        MainTab::Journal => {
            if app.journal.handle_key(key) {
                return true;
            }
        }
    }
    false
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    if matches!(
        mouse.kind,
        crossterm::event::MouseEventKind::Down(MouseButton::Left)
    ) && rect_contains(app.layout.bar, mouse.column, mouse.row)
    {
        if app.layout.config.contains(mouse.column) {
            app.tab = MainTab::Config;
            return;
        }
        if app.layout.journal.contains(mouse.column) {
            app.tab = MainTab::Journal;
            return;
        }
    }
    match app.tab {
        MainTab::Config => app.config.handle_mouse(mouse),
        MainTab::Journal => app.journal.handle_mouse(mouse),
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let size = f.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(size);

    let (tabs, tab_config, tab_journal) = tabs_line(app.tab, app.config.is_dirty());
    app.layout.bar = layout[0];
    app.layout.config = tab_config.offset(layout[0].x);
    app.layout.journal = tab_journal.offset(layout[0].x);
    let tabs_widget = Paragraph::new(tabs).style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(tabs_widget, layout[0]);

    match app.tab {
        MainTab::Config => app.config.render(f, layout[1]),
        MainTab::Journal => app.journal.render(f, layout[1]),
    }
}

fn tabs_line(active: MainTab, dirty: bool) -> (Line<'static>, TabBounds, TabBounds) {
    let active_style = Style::default();
    let inactive_style = Style::default().add_modifier(Modifier::DIM);
    let config_label = if dirty { "配置*" } else { "配置" };

    let mut spans = Vec::new();
    let mut col: u16 = 0;
    spans.push(Span::raw(" "));
    col = col.saturating_add(1);
    let config_text = format!("1 {config_label}");
    let config_width = UnicodeWidthStr::width(config_text.as_str()) as u16;
    let config_start = col;
    col = col.saturating_add(config_width);
    spans.push(Span::styled(
        config_text,
        if active == MainTab::Config {
            active_style
        } else {
            inactive_style
        },
    ));
    spans.push(Span::raw("  "));
    col = col.saturating_add(2);
    let journal_text = "2 日志";
    let journal_width = UnicodeWidthStr::width(journal_text) as u16;
    let journal_start = col;
    let journal_end = journal_start.saturating_add(journal_width);
    spans.push(Span::styled(
        journal_text,
        if active == MainTab::Journal {
            active_style
        } else {
            inactive_style
        },
    ));
    spans.push(Span::raw("  q 退出"));

    (
        Line::from(spans),
        TabBounds {
            start: config_start,
            end: config_start.saturating_add(config_width),
        },
        TabBounds {
            start: journal_start,
            end: journal_end,
        },
    )
}

fn rect_contains(rect: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}
