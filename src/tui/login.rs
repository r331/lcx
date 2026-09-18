//! In-TUI login modal. Shown at startup when there are no credentials (or the
//! saved ones fail to verify), and reachable any time with `Ctrl+L`.
//!
//! Authentication is hands-free by default: the modal continuously polls your
//! browsers for LeetCode cookies (via the `rookie` crate) and, once found,
//! verifies and saves them automatically. So the usual flow is just `F3` to open
//! the login page, sign in, and the modal logs you in on its own.
//!
//! Polling stops the moment you type into a field, so manual entry is never
//! clobbered. You can then paste `LEETCODE_SESSION` + `csrftoken` and press
//! `Enter`. `F2` toggles auto-detection on/off.
//!
//! F-keys are the primary bindings because terminal multiplexers such as tmux
//! commonly reserve a Ctrl prefix (e.g. `Ctrl+A`) and would swallow it.

use std::time::Duration;

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use super::{block_on, Backend};
use crate::client::LeetCodeClient;
use crate::config::Config;

const LOGIN_URL: &str = "https://leetcode.com/accounts/login/";

/// Outcome of the login modal.
pub enum LoginOutcome {
    /// Credentials verified and saved; carries the updated config + username.
    Saved { cfg: Config, username: String },
    /// User dismissed the modal without logging in.
    Cancelled,
    /// User chose to browse cached problems without signing in.
    Offline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Session,
    Csrf,
}

struct LoginApp {
    base: Config,
    session: String,
    csrf: String,
    field: Field,
    status: String,
    done: Option<LoginOutcome>,
    /// While true, the modal keeps auto-importing browser cookies on a timer.
    polling: bool,
    /// Cookie pairs already tried, to avoid re-verifying them on every tick.
    tried_imports: Vec<(String, String)>,
    /// Whether cancelling (Esc) quits the app (startup) vs. returns (re-login).
    cancel_quits: bool,
}

impl LoginApp {
    fn new(cfg: &Config, cancel_quits: bool) -> Self {
        Self {
            base: cfg.clone(),
            session: cfg.session.clone().unwrap_or_default(),
            csrf: cfg.csrf_token.clone().unwrap_or_default(),
            field: Field::Session,
            status: "Auto-detecting browser login... (<F3> to open login page, or type to enter manually)".to_string(),
            done: None,
            polling: true,
            tried_imports: Vec::new(),
            cancel_quits,
        }
    }

    /// Timer tick: while polling, try to import cookies from the browser and, if
    /// found (and not already tried), verify + save them automatically.
    fn poll_tick(&mut self) {
        if !self.polling {
            return;
        }
        match import_cookies() {
            Ok(pairs) => {
                for pair in pairs {
                    if self.tried_imports.contains(&pair) {
                        continue;
                    }
                    self.tried_imports.push(pair.clone());
                    self.session = pair.0;
                    self.csrf = pair.1;
                    self.status = "Detected browser cookies, verifying...".to_string();
                    self.submit();
                    if self.done.is_some() {
                        return;
                    }
                }
            }
            Err(e) => {
                // Surface the reason (e.g. "not found, sign in first" or the
                // Windows app-bound/admin hint) instead of a generic spinner.
                self.status = format!("{e}");
            }
        }
    }

    /// Disable auto-import once the user starts entering values manually.
    fn stop_polling(&mut self) {
        if self.polling {
            self.polling = false;
            self.status =
                "Auto-detect paused (manual input). <Enter> to verify & save.".to_string();
        }
    }

    /// Toggle auto-detection on/off (bound to F2).
    fn toggle_polling(&mut self) {
        self.polling = !self.polling;
        if self.polling {
            self.tried_imports.clear(); // allow a fresh attempt
            self.status = "Auto-detect enabled.".to_string();
            self.poll_tick();
        } else {
            self.status =
                "Auto-detect disabled. Type cookies manually, then <Enter> to save.".to_string();
        }
    }

    fn active_mut(&mut self) -> &mut String {
        match self.field {
            Field::Session => &mut self.session,
            Field::Csrf => &mut self.csrf,
        }
    }

    fn toggle_field(&mut self) {
        self.field = match self.field {
            Field::Session => Field::Csrf,
            Field::Csrf => Field::Session,
        };
    }

    /// Verify the entered cookies against LeetCode and persist on success.
    fn submit(&mut self) {
        let session = self.session.trim().to_string();
        let csrf = self.csrf.trim().to_string();
        if session.is_empty() || csrf.is_empty() {
            self.status = "Both LEETCODE_SESSION and csrftoken are required.".to_string();
            return;
        }

        let mut cfg = self.base.clone();
        cfg.session = Some(session);
        cfg.csrf_token = Some(csrf);

        let client = match LeetCodeClient::from_config(&cfg) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("Client error: {e:#}");
                return;
            }
        };

        self.status = "Verifying...".to_string();
        match block_on(client.whoami()) {
            Ok(Some(username)) => {
                if let Err(e) = cfg.save() {
                    self.status = format!("Verified but failed to save: {e:#}");
                    return;
                }
                self.done = Some(LoginOutcome::Saved { cfg, username });
            }
            Ok(None) => {
                self.status = "Rejected: not signed in. Check the cookie values.".to_string();
            }
            Err(e) => {
                self.status = format!("Verify failed: {e:#}");
            }
        }
    }

    fn open_browser(&mut self) {
        self.status = match open::that(LOGIN_URL) {
            Ok(()) => {
                "Opened LeetCode login in your browser. Sign in; auto-detect will log you in."
                    .to_string()
            }
            Err(e) => format!("Could not open browser: {e}"),
        };
    }
}

/// Read LeetCode cookies from any local browser the user is logged into.
///
/// Browsers are tried individually (instead of `rookie::load`, which swallows
/// per-browser errors) so we can surface *why* detection failed. The common
/// culprit on Windows is Chrome/Edge/Brave "app-bound" cookie encryption, which
/// can only be decrypted when running as administrator.
fn import_cookies() -> Result<Vec<(String, String)>> {
    type Loader = fn(Option<Vec<String>>) -> rookie::Result<Vec<rookie::enums::Cookie>>;
    let domains = Some(vec!["leetcode.com".to_string()]);
    let loaders: [Loader; 6] = [
        rookie::chrome,
        rookie::firefox,
        rookie::edge,
        rookie::brave,
        rookie::chromium,
        rookie::vivaldi,
    ];

    let mut pairs = Vec::new();
    // Set when a browser was present but its cookies couldn't be decrypted
    // (e.g. app-bound encryption on Windows without admin rights).
    let mut blocked = false;

    for load in loaders {
        let mut session = None;
        let mut csrf = None;
        let cookies = match load(domains.clone()) {
            Ok(c) => c,
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("admin") || msg.contains("appbound") || msg.contains("app-bound") {
                    blocked = true;
                }
                continue;
            }
        };
        for c in cookies {
            if !c.domain.contains("leetcode") {
                continue;
            }
            match c.name.as_str() {
                "LEETCODE_SESSION" => session = Some(c.value),
                "csrftoken" => csrf = Some(c.value),
                _ => {}
            }
        }
        if let (Some(s), Some(c)) = (session, csrf) {
            let pair = (s, c);
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
    }

    if !pairs.is_empty() {
        Ok(pairs)
    } else if blocked {
        Err(anyhow!(
            "Found a Chromium browser but couldn't read its cookies. On Windows, \
             Chrome/Edge/Brave use app-bound encryption and can only be read when \
             lcx runs as administrator. Use Firefox, run lcx as admin, or enter \
             cookies manually (<Tab> to a field and type)."
        ))
    } else {
        Err(anyhow!(
            "LeetCode cookies not found. Open the login page (<F3>) and sign in first, \
             or enter cookies manually."
        ))
    }
}

/// Run the login modal until the user saves valid credentials or cancels.
///
/// `cancel_quits` only affects the help wording (Esc = "quit" at startup vs.
/// "cancel" when reopened from the browser).
pub fn run(
    terminal: &mut Terminal<Backend>,
    cfg: &Config,
    cancel_quits: bool,
) -> Result<LoginOutcome> {
    let mut app = LoginApp::new(cfg, cancel_quits);
    // Try immediately so an already-logged-in browser logs us in without delay.
    app.poll_tick();

    loop {
        terminal.draw(|f| ui(f, &app))?;
        if let Some(outcome) = app.done.take() {
            return Ok(outcome);
        }

        // Wait for a key, but wake up on a timer to re-poll for cookies.
        if !event::poll(Duration::from_millis(1000))? {
            app.poll_tick();
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc => return Ok(LoginOutcome::Cancelled),
            // F-keys are used instead of Ctrl shortcuts because terminal
            // multiplexers like tmux/screen reserve a Ctrl prefix and would
            // swallow them (tmux defaults to Ctrl+B; Ctrl+A is a common custom).
            KeyCode::F(2) => app.toggle_polling(),
            KeyCode::F(3) => app.open_browser(),
            KeyCode::F(4) => return Ok(LoginOutcome::Offline),
            KeyCode::Enter => app.submit(),
            KeyCode::Tab | KeyCode::BackTab => app.toggle_field(),
            KeyCode::Backspace => {
                app.stop_polling();
                app.active_mut().pop();
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                app.stop_polling();
                app.active_mut().push(c);
            }
            _ => {}
        }
    }
}

fn ui(f: &mut Frame, app: &LoginApp) {
    let area = f.area();
    f.render_widget(Clear, area);

    let outer = Block::default().borders(Borders::ALL).title(format!(
        " Log in to LeetCode  \u{2022}  auto-detect: {} ",
        if app.polling { "ON" } else { "OFF" }
    ));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Logo on the left, form on the right.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(50), Constraint::Min(28)])
        .split(inner);

    // Vertically center the logo in the left column.
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(23),
            Constraint::Min(0),
        ])
        .split(cols[0]);
    f.render_widget(logo_widget(), left[1]);

    // Vertically center the form in the right column.
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(cols[1]);

    f.render_widget(
        field_widget(
            "LEETCODE_SESSION",
            &app.session,
            app.field == Field::Session,
        ),
        right[1],
    );
    f.render_widget(
        field_widget("csrftoken", &app.csrf, app.field == Field::Csrf),
        right[2],
    );

    let status = Paragraph::new(app.status.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    f.render_widget(status, right[3]);

    let state = if app.polling { "ON" } else { "OFF" };
    let esc = if app.cancel_quits { "quit" } else { "cancel" };
    let help = format!(
        "<F2> auto-detect [{state}]  <F3> open login page  <F4> browse offline  <Tab> switch field  <Enter> save  <Esc> {esc}"
    );
    f.render_widget(Paragraph::new(super::help_line(&help)), right[4]);
}

/// ASCII-art rendition of the LeetCode logo. Characters forming the chevron
/// (`@ % *`) are drawn in gray/white; the curve/bar characters (`- = +`) in
/// LeetCode brand orange.
fn logo_widget() -> Paragraph<'static> {
    const ART: [&str; 23] = [
        "                            @@@@",
        "                          @@@@@@@",
        "                        @@@@@@@@",
        "                      @@@@@@@@@",
        "                    @@@@@@@@%",
        "                  @@@@@@@%+=----",
        "                 @@@@@@@+=-------",
        "               @@@@@@@%==----------",
        "             @@@@@@@@        --------",
        "           @@@@@@@@            ------",
        "         @@@@@@@@@              ----",
        "        @@@@@@@@",
        "        @@@@@@        ---------------------",
        "        @@@@@@       ----------------------",
        "        @@@@@@       ----------------------",
        "        @@@@@@@@",
        "         @@@@@@@@@               ---",
        "           @@@@@@@@            ------",
        "             @@@@@@@@        --------",
        "               @@@@@@@*-  ---------",
        "                 @@@@@*-----------",
        "                  @%*=---------=",
        "                     =----===",
    ];
    let lines: Vec<Line> = ART.iter().map(|row| color_logo_row(row)).collect();
    Paragraph::new(Text::from(lines))
}

/// Split a logo row into colored spans by character class.
fn color_logo_row(row: &str) -> Line<'static> {
    let gray = Style::default().fg(Color::Gray);
    let orange = Style::default().fg(Color::Rgb(255, 161, 22));

    // 0 = blank, 1 = chevron (gray), 2 = curve (orange).
    let class = |c: char| match c {
        '@' | '%' | '*' => 1u8,
        '-' | '=' | '+' => 2u8,
        _ => 0u8,
    };

    let mut spans: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let mut cur: Option<u8> = None;
    for ch in row.chars() {
        let cat = class(ch);
        if Some(cat) != cur {
            if let Some(prev) = cur {
                spans.push(styled_span(std::mem::take(&mut buf), prev, gray, orange));
            }
            cur = Some(cat);
        }
        buf.push(ch);
    }
    if let Some(prev) = cur {
        spans.push(styled_span(buf, prev, gray, orange));
    }
    Line::from(spans)
}

fn styled_span(text: String, cat: u8, gray: Style, orange: Style) -> Span<'static> {
    match cat {
        1 => Span::styled(text, gray),
        2 => Span::styled(text, orange),
        _ => Span::raw(text),
    }
}

/// A single input field, masked to keep the (secret) value from filling the box.
fn field_widget<'a>(label: &'a str, value: &str, focused: bool) -> Paragraph<'a> {
    let shown = mask(value, focused);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {label} "));
    if focused {
        block = block.border_style(Style::default().add_modifier(Modifier::BOLD));
    }
    Paragraph::new(shown).block(block)
}

/// Show the value length and a tail preview so the user can confirm a paste
/// without exposing the full secret on screen.
fn mask(value: &str, focused: bool) -> String {
    if value.is_empty() {
        return if focused {
            "\u{2588}".to_string()
        } else {
            String::new()
        };
    }
    let len = value.chars().count();
    let tail: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let cursor = if focused { "\u{2588}" } else { "" };
    format!("{} chars \u{2022} \u{2026}{tail}{cursor}", len)
}
