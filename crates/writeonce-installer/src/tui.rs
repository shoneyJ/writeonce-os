// tui.rs — ratatui-based form flow for the installer.
//
// Activated by default when both stdin and stdout are TTYs and
// --no-tui isn't passed. Replaces the line-by-line prompts in
// prompt.rs with a five-screen modal form:
//
//   1. Device select  — list of removable drives, ↑↓ + Enter
//   2. Partition      — ESP MiB + root GiB input fields
//   3. User           — username + real-name + password ×2
//   4. Keyboard       — layout + variant
//   5. Summary        — review screen + Enter to commit / Esc to cancel
//
// The TUI ONLY gathers form data. After Enter on Summary, ratatui is
// torn down and the standard install pipeline runs (partition, mkfs,
// extract, customize, bootloader, verify) with the normal scrolling
// progress output. Keeps the TUI's responsibility narrow.

use anyhow::{anyhow, Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io::{stdout, Stdout};

use crate::detect::UsbDevice;
use crate::prompt::{hash_password_sha512, validate_hash};
use crate::spec::{
    InstallationPlan, PartitionPlan, ResolvedKeyboard, ResolvedUser, TargetOsSpec,
};

const DEFAULT_ESP_MIB: u32 = 512;
const DEFAULT_KEYBOARD: &str = "us";
const DEFAULT_SHELL: &str = "/bin/bash";
const DEFAULT_GROUPS: &[&str] = &["wheel", "video", "audio", "input", "plugdev"];

// ============================================================================
// Public entry
// ============================================================================

/// Returns `Ok(Some(...))` if the operator completed the form;
/// `Ok(None)` if they cancelled (Esc or Ctrl-C). Errors if the TUI
/// couldn't be set up (no TTY, terminal too small, etc.).
pub fn run_tui(
    spec: &TargetOsSpec,
    devices: Vec<UsbDevice>,
) -> Result<Option<(UsbDevice, InstallationPlan)>> {
    if devices.is_empty() {
        anyhow::bail!("no removable devices detected; insert a USB and re-run");
    }

    let mut terminal = setup_terminal()?;
    let mut app = App::new(spec, devices);

    let outcome = (|| -> Result<()> {
        loop {
            terminal.draw(|f| draw(f, &app))?;
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key(&mut app, key);
                if app.outcome.is_some() {
                    break;
                }
            }
        }
        Ok(())
    })();

    teardown_terminal(&mut terminal)?;
    outcome?;

    match app.outcome.unwrap() {
        Outcome::Cancelled => Ok(None),
        Outcome::Confirmed => {
            let device = app.devices[app.device_idx].clone();
            let plan = build_plan(&app)?;
            Ok(Some((device, plan)))
        }
    }
}

// ============================================================================
// State
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    DeviceSelect,
    Partition,
    User,
    Keyboard,
    Summary,
}

#[derive(Debug, Clone, Copy)]
enum Outcome {
    Confirmed,
    Cancelled,
}

#[derive(Debug)]
struct App {
    screen: Screen,
    outcome: Option<Outcome>,
    error: Option<String>,

    devices: Vec<UsbDevice>,
    device_state: ListState,
    device_idx: usize,

    // Partition fields
    partition_focus: usize, // 0 = esp, 1 = root
    esp_mib_input: String,
    root_gib_input: String,

    // User fields
    user_focus: usize, // 0..=3
    user_name: String,
    user_realname: String,
    user_password: String,
    user_password_confirm: String,
    show_passwords: bool,

    // Keyboard fields
    keyboard_focus: usize, // 0..=1
    kb_layout: String,
    kb_variant: String,

    // Pre-resolved values from spec (so we know what NOT to mutate).
    spec_user_shell: Option<String>,
    spec_user_groups: Option<Vec<String>>,
}

impl App {
    fn new(spec: &TargetOsSpec, devices: Vec<UsbDevice>) -> Self {
        let p = spec.partitions.as_ref();
        let u = spec.user.as_ref();
        let k = spec.keyboard.as_ref();

        let mut state = ListState::default();
        state.select(Some(0));

        Self {
            screen: Screen::DeviceSelect,
            outcome: None,
            error: None,
            devices,
            device_state: state,
            device_idx: 0,
            partition_focus: 0,
            esp_mib_input: p
                .and_then(|s| s.esp_mib)
                .map(|v| v.to_string())
                .unwrap_or_else(|| DEFAULT_ESP_MIB.to_string()),
            root_gib_input: p
                .and_then(|s| s.root_gib)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "0".to_string()),
            user_focus: 0,
            user_name: u.and_then(|s| s.name.clone()).unwrap_or_default(),
            user_realname: u.and_then(|s| s.real_name.clone()).unwrap_or_default(),
            user_password: String::new(),
            user_password_confirm: String::new(),
            show_passwords: false,
            keyboard_focus: 0,
            kb_layout: k
                .and_then(|s| s.layout.clone())
                .unwrap_or_else(|| DEFAULT_KEYBOARD.to_string()),
            kb_variant: k.and_then(|s| s.variant.clone()).unwrap_or_default(),
            spec_user_shell: u.and_then(|s| s.shell.clone()),
            spec_user_groups: u.and_then(|s| s.groups.clone()),
        }
    }

    fn current_device(&self) -> &UsbDevice {
        &self.devices[self.device_idx]
    }
}

// ============================================================================
// Input handling
// ============================================================================

fn handle_key(app: &mut App, key: KeyEvent) {
    // Universal escape & ctrl-c
    if matches!(key.code, KeyCode::Esc)
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
    {
        app.outcome = Some(Outcome::Cancelled);
        return;
    }

    app.error = None; // dismiss any prior validation error on any key

    match app.screen {
        Screen::DeviceSelect => handle_devices(app, key),
        Screen::Partition => handle_partition(app, key),
        Screen::User => handle_user(app, key),
        Screen::Keyboard => handle_keyboard(app, key),
        Screen::Summary => handle_summary(app, key),
    }
}

fn handle_devices(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.device_idx > 0 {
                app.device_idx -= 1;
                app.device_state.select(Some(app.device_idx));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.device_idx + 1 < app.devices.len() {
                app.device_idx += 1;
                app.device_state.select(Some(app.device_idx));
            }
        }
        KeyCode::Enter => {
            let d = app.current_device();
            if !d.removable {
                app.error = Some(format!(
                    "{} is not removable; refusing.",
                    d.path.display()
                ));
                return;
            }
            app.screen = Screen::Partition;
        }
        _ => {}
    }
}

fn handle_partition(app: &mut App, key: KeyEvent) {
    let buf = if app.partition_focus == 0 {
        &mut app.esp_mib_input
    } else {
        &mut app.root_gib_input
    };
    match key.code {
        KeyCode::Tab | KeyCode::Down => {
            app.partition_focus = (app.partition_focus + 1) % 2;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.partition_focus = (app.partition_focus + 1) % 2;
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            if buf.len() < 10 {
                buf.push(c);
            }
        }
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Enter => {
            // Validate then advance
            let esp: u32 = match app.esp_mib_input.trim().parse() {
                Ok(v) if (64..=8192).contains(&v) => v,
                _ => {
                    app.error = Some("ESP must be a number in 64–8192 MiB".into());
                    return;
                }
            };
            let root: Option<u32> = match app.root_gib_input.trim().parse::<u32>() {
                Ok(0) => None,
                Ok(v) => Some(v),
                Err(_) => {
                    app.error = Some("root GiB must be a non-negative integer (0 = rest)".into());
                    return;
                }
            };
            let disk_mib = (app.current_device().size_bytes / 1_048_576) as u32;
            let max_root_gib = disk_mib.saturating_sub(esp + 8) / 1024;
            if let Some(r) = root {
                if r > max_root_gib {
                    app.error = Some(format!(
                        "root {r} GiB exceeds available {max_root_gib} GiB after ESP"
                    ));
                    return;
                }
            }
            app.screen = Screen::User;
        }
        KeyCode::Left => {
            app.screen = Screen::DeviceSelect;
        }
        _ => {}
    }
}

fn handle_user(app: &mut App, key: KeyEvent) {
    if key.code == KeyCode::F(2) {
        app.show_passwords = !app.show_passwords;
        return;
    }
    if key.code == KeyCode::Tab || key.code == KeyCode::Down {
        app.user_focus = (app.user_focus + 1) % 4;
        return;
    }
    if key.code == KeyCode::BackTab || key.code == KeyCode::Up {
        app.user_focus = (app.user_focus + 3) % 4;
        return;
    }
    if key.code == KeyCode::Left {
        app.screen = Screen::Partition;
        return;
    }
    if key.code == KeyCode::Enter {
        // Validate
        if app.user_name.is_empty() || app.user_name == "root" {
            app.error = Some("username must be set and must not be 'root'".into());
            return;
        }
        if !app
            .user_name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            app.error = Some(
                "username must be lowercase alphanumeric + underscore only".into(),
            );
            return;
        }
        if app.user_password.len() < 6 {
            app.error = Some("password must be at least 6 characters".into());
            return;
        }
        if app.user_password != app.user_password_confirm {
            app.error = Some("password and confirmation do not match".into());
            return;
        }
        app.screen = Screen::Keyboard;
        return;
    }

    let buf = match app.user_focus {
        0 => &mut app.user_name,
        1 => &mut app.user_realname,
        2 => &mut app.user_password,
        _ => &mut app.user_password_confirm,
    };

    match key.code {
        KeyCode::Char(c) => buf.push(c),
        KeyCode::Backspace => {
            buf.pop();
        }
        _ => {}
    }
}

fn handle_keyboard(app: &mut App, key: KeyEvent) {
    if key.code == KeyCode::Tab || key.code == KeyCode::Down {
        app.keyboard_focus = (app.keyboard_focus + 1) % 2;
        return;
    }
    if key.code == KeyCode::BackTab || key.code == KeyCode::Up {
        app.keyboard_focus = (app.keyboard_focus + 1) % 2;
        return;
    }
    if key.code == KeyCode::Left {
        app.screen = Screen::User;
        return;
    }
    if key.code == KeyCode::Enter {
        if app.kb_layout.is_empty() {
            app.error = Some("layout cannot be empty".into());
            return;
        }
        if !valid_keymap(&app.kb_layout) {
            app.error = Some("layout must be lowercase alphanumeric + underscore".into());
            return;
        }
        if !app.kb_variant.is_empty() && !valid_keymap(&app.kb_variant) {
            app.error = Some("variant must be lowercase alphanumeric + underscore".into());
            return;
        }
        app.screen = Screen::Summary;
        return;
    }

    let buf = if app.keyboard_focus == 0 {
        &mut app.kb_layout
    } else {
        &mut app.kb_variant
    };
    match key.code {
        KeyCode::Char(c) => buf.push(c),
        KeyCode::Backspace => {
            buf.pop();
        }
        _ => {}
    }
}

fn handle_summary(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.outcome = Some(Outcome::Confirmed),
        KeyCode::Left | KeyCode::Backspace => app.screen = Screen::Keyboard,
        _ => {}
    }
}

fn valid_keymap(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

// ============================================================================
// Build the final plan from app state
// ============================================================================

fn build_plan(app: &App) -> Result<InstallationPlan> {
    let esp_mib: u32 = app.esp_mib_input.trim().parse()?;
    let root_gib: Option<u32> = match app.root_gib_input.trim().parse::<u32>() {
        Ok(0) => None,
        Ok(v) => Some(v),
        Err(_) => return Err(anyhow!("invalid root GiB")),
    };

    let password_hash = hash_password_sha512(&app.user_password)
        .context("hash password via openssl passwd -6")?;
    validate_hash(&password_hash)?;

    let shell = app
        .spec_user_shell
        .clone()
        .unwrap_or_else(|| DEFAULT_SHELL.to_string());
    let groups = app.spec_user_groups.clone().unwrap_or_else(|| {
        DEFAULT_GROUPS.iter().map(|s| s.to_string()).collect()
    });

    Ok(InstallationPlan {
        partition: PartitionPlan { esp_mib, root_gib },
        user: ResolvedUser {
            name: app.user_name.clone(),
            real_name: app.user_realname.clone(),
            password_hash,
            shell,
            groups,
            uid: 1000,
            gid: 1000,
        },
        keyboard: ResolvedKeyboard {
            layout: app.kb_layout.clone(),
            variant: if app.kb_variant.is_empty() {
                None
            } else {
                Some(app.kb_variant.clone())
            },
        },
        // TUI doesn't expose a network toggle yet — desktop default.
        // Headless / SSH-only setups go via the CLI prompt path which
        // reads the spec's `network.enabled_at_boot` field.
        network: crate::spec::ResolvedNetwork::default(),
    })
}

// ============================================================================
// Rendering
// ============================================================================

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(10),    // body
            Constraint::Length(3),  // footer (key hints)
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);
    match app.screen {
        Screen::DeviceSelect => draw_devices(f, chunks[1], app),
        Screen::Partition => draw_partition(f, chunks[1], app),
        Screen::User => draw_user(f, chunks[1], app),
        Screen::Keyboard => draw_keyboard(f, chunks[1], app),
        Screen::Summary => draw_summary(f, chunks[1], app),
    }
    draw_footer(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = match app.screen {
        Screen::DeviceSelect => "WriteOnce Installer · [1/5] Select target device",
        Screen::Partition => "WriteOnce Installer · [2/5] Partition layout",
        Screen::User => "WriteOnce Installer · [3/5] User account",
        Screen::Keyboard => "WriteOnce Installer · [4/5] Keyboard",
        Screen::Summary => "WriteOnce Installer · [5/5] Review + confirm",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let p = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let hints = match app.screen {
        Screen::DeviceSelect => "↑↓ select   Enter continue   Esc cancel",
        Screen::Partition => "Tab cycle   Enter continue   ←Back   Esc cancel",
        Screen::User => "Tab cycle   F2 toggle password visibility   Enter continue   ←Back   Esc cancel",
        Screen::Keyboard => "Tab cycle   Enter continue   ←Back   Esc cancel",
        Screen::Summary => "Enter to CONFIRM and install   ←Back   Esc cancel",
    };
    let footer = if let Some(err) = &app.error {
        format!("⚠ {err}    │    {hints}")
    } else {
        hints.to_string()
    };
    let style = if app.error.is_some() {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let p = Paragraph::new(footer)
        .style(style)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_devices(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|d| {
            let label = format!(
                "{:<10}  {:>6.1} GB  {} {}{}",
                d.path.display(),
                d.size_gb(),
                d.vendor,
                d.model,
                if d.removable { "" } else { "  [NOT REMOVABLE]" }
            );
            let style = if d.removable {
                Style::default()
            } else {
                Style::default().fg(Color::Red)
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Removable block devices "))
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = app.device_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_partition(f: &mut Frame, area: Rect, app: &App) {
    let dev = app.current_device();
    let disk_mib = (dev.size_bytes / 1_048_576) as u32;
    let esp_n: u32 = app.esp_mib_input.trim().parse().unwrap_or(0);
    let max_root_gib = disk_mib.saturating_sub(esp_n + 8) / 1024;

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let esp_label = format!(" ESP size (MiB) — default {DEFAULT_ESP_MIB}, range 64–8192");
    let root_label = format!(" Root size (GiB) — 0 = use rest (max {max_root_gib} GiB)");

    f.render_widget(input_widget(&esp_label, &app.esp_mib_input, app.partition_focus == 0), body[0]);
    f.render_widget(input_widget(&root_label, &app.root_gib_input, app.partition_focus == 1), body[1]);

    let info = vec![
        Line::raw(""),
        Line::raw(format!(" Target: {} ({:.2} GB)", dev.path.display(), dev.size_gb())),
        Line::raw(format!(" Vendor / Model: {} {}", dev.vendor, dev.model)),
    ];
    f.render_widget(Paragraph::new(info), body[2]);
}

fn draw_user(f: &mut Frame, area: Rect, app: &App) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    f.render_widget(
        input_widget(" Username (not root)", &app.user_name, app.user_focus == 0),
        body[0],
    );
    f.render_widget(
        input_widget(" Real name (optional)", &app.user_realname, app.user_focus == 1),
        body[1],
    );

    let pw_display = if app.show_passwords {
        app.user_password.clone()
    } else {
        "●".repeat(app.user_password.len())
    };
    let cf_display = if app.show_passwords {
        app.user_password_confirm.clone()
    } else {
        "●".repeat(app.user_password_confirm.len())
    };
    f.render_widget(
        input_widget(" Password (min 6 chars)", &pw_display, app.user_focus == 2),
        body[2],
    );
    f.render_widget(
        input_widget(" Confirm password", &cf_display, app.user_focus == 3),
        body[3],
    );
}

fn draw_keyboard(f: &mut Frame, area: Rect, app: &App) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    f.render_widget(
        input_widget(" Layout (us, uk, de, fr, es, …)", &app.kb_layout, app.keyboard_focus == 0),
        body[0],
    );
    f.render_widget(
        input_widget(" Variant (optional — dvorak, intl, …)", &app.kb_variant, app.keyboard_focus == 1),
        body[1],
    );
}

fn draw_summary(f: &mut Frame, area: Rect, app: &App) {
    let dev = app.current_device();
    let esp_n: u32 = app.esp_mib_input.trim().parse().unwrap_or(0);
    let root_str = match app.root_gib_input.trim().parse::<u32>() {
        Ok(0) | Err(_) => "rest of disk".to_string(),
        Ok(v) => format!("{v} GiB"),
    };

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(" Target device ", Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::styled(
                format!("{}  ({:.2} GB, {} {})", dev.path.display(), dev.size_gb(), dev.vendor, dev.model),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Partitioning ", Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::raw(format!("ESP {esp_n} MiB · root {root_str}")),
        ]),
        Line::from(vec![
            Span::styled(" User         ", Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::raw(format!(
                "{} (uid 1000, {})",
                app.user_name,
                if app.user_realname.is_empty() {
                    "no real name".to_string()
                } else {
                    app.user_realname.clone()
                }
            )),
        ]),
        Line::from(vec![
            Span::styled(" Shell        ", Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::raw(app.spec_user_shell.clone().unwrap_or_else(|| DEFAULT_SHELL.to_string())),
        ]),
        Line::from(vec![
            Span::styled(" Groups       ", Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::raw(
                app.spec_user_groups
                    .clone()
                    .unwrap_or_else(|| DEFAULT_GROUPS.iter().map(|s| s.to_string()).collect())
                    .join(","),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Keyboard     ", Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::raw(format!(
                "{}{}",
                app.kb_layout,
                if app.kb_variant.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", app.kb_variant)
                }
            )),
        ]),
        Line::raw(""),
        Line::styled(
            " ⚠ Pressing Enter wipes the entire target device.",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
    ];

    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Installation summary "),
    );
    f.render_widget(p, area);
}

fn input_widget<'a>(label: &'a str, value: &'a str, focused: bool) -> Paragraph<'a> {
    let block_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let cursor = if focused { "_" } else { "" };
    let content = vec![Line::from(vec![
        Span::raw(format!(" {value}")),
        Span::styled(cursor, Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK)),
    ])];
    Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .title(label.to_string())
            .border_style(block_style),
    )
}

// ============================================================================
// Terminal lifecycle
// ============================================================================

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("enable_raw_mode")?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)
        .context("EnterAlternateScreen")?;
    let backend = CrosstermBackend::new(out);
    Terminal::new(backend).context("Terminal::new")
}

fn teardown_terminal(term: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().ok();
    execute!(term.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).ok();
    term.show_cursor().ok();
    Ok(())
}
