//! Two-pane "solve" view: problem description on the left, a read-only preview
//! of your solution on the right. Reached by selecting a problem in the browser.
//!
//! Editing is done in your host editor: press `e` (or `Enter`) to open the
//! solution file in `$EDITOR`; on return the preview reloads from disk. The
//! right pane is review-only, so both panes are scrollable and nothing is typed
//! into the buffer directly. `r` starts over, restoring the original starter
//! code (with a confirmation press).

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::{block_on, leave, resume, strip_ansi, Backend};
use crate::client::models::ProblemDetail;
use crate::client::LeetCodeClient;
use crate::config::Config;
use crate::render;
use crate::solution;

/// Which pane scrolling keys currently act on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Desc,
    Code,
    Output,
}

/// Runtime state for the solve view.
pub struct SolveApp {
    cfg: Config,
    client: LeetCodeClient,
    detail: ProblemDetail,
    lang_slug: String,
    path: std::path::PathBuf,
    /// Current solution contents (read-only preview; edited via host editor).
    code: String,
    desc: Text<'static>,
    desc_scroll: u16,
    code_scroll: u16,
    output_scroll: u16,
    focus: Focus,
    status: String,
    /// True after a first `r` press, awaiting confirmation to start over.
    pending_reset: bool,
    /// Set when the user asks to return to the browser.
    back: bool,
}

/// Prepare a solve session for a problem slug: fetch detail, generate the
/// solution file if missing, and load its contents. Networking runs on the
/// shared tokio runtime via `block_on`.
pub fn prepare(
    cfg: &Config,
    client: &LeetCodeClient,
    slug: &str,
    lang: Option<String>,
) -> Result<SolveApp> {
    let lang_slug = lang.unwrap_or_else(|| cfg.lang.clone());
    let detail = block_on(client.problem_detail(slug))?;

    let path = solution::solution_path(cfg, &detail.frontend_id, &detail.slug, &lang_slug);
    if !path.exists() {
        let snippet = detail.snippet_for(&lang_slug).ok_or_else(|| {
            anyhow!(
                "no starter code for language '{lang_slug}'. Available: {}",
                detail
                    .code_snippets
                    .iter()
                    .map(|s| s.lang_slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        solution::ensure_workspace(cfg)?;
        let contents = solution::render_file(&lang_slug, &snippet.code, &detail.content);
        std::fs::write(&path, contents)?;
    }

    let (code, _meta) = solution::read_solution(&path)?;
    Ok(SolveApp::new(
        cfg.clone(),
        client.clone(),
        detail,
        lang_slug,
        path,
        code,
    ))
}

impl SolveApp {
    fn new(
        cfg: Config,
        client: LeetCodeClient,
        detail: ProblemDetail,
        lang_slug: String,
        path: std::path::PathBuf,
        code: String,
    ) -> Self {
        let mut desc = super::markup::render_html(&detail.content, 100);
        // Show the problem link at the top of the description panel (it used to
        // live in the generated solution file header).
        let url = format!("https://leetcode.com/problems/{}/", detail.slug);
        desc.lines.insert(
            0,
            Line::from(Span::styled(
                url,
                Style::default()
                    .fg(Color::Rgb(88, 166, 255))
                    .add_modifier(Modifier::UNDERLINED),
            )),
        );
        desc.lines.insert(1, Line::raw(""));
        Self {
            cfg,
            client,
            detail,
            lang_slug,
            path,
            code,
            desc,
            desc_scroll: 0,
            code_scroll: 0,
            output_scroll: 0,
            focus: Focus::Desc,
            status: "<e>/<Enter> edit in $EDITOR  <r> start over  <Tab> switch pane  <\u{2191}>/<\u{2193}>/<j>/<k> scroll  <t> test  <s> submit  <Esc> back"
                .to_string(),
            pending_reset: false,
            back: false,
        }
    }

    /// Restore the solution file to the original starter code from LeetCode.
    fn start_over(&mut self) -> String {
        let Some(snippet) = self.detail.snippet_for(&self.lang_slug) else {
            return format!("No starter code available for '{}'.", self.lang_slug);
        };
        let contents = solution::render_file(&self.lang_slug, &snippet.code, &self.detail.content);
        if let Err(e) = std::fs::write(&self.path, &contents) {
            return format!("Start over failed: {e}");
        }
        self.code = match solution::read_solution(&self.path) {
            Ok((code, _)) => code,
            Err(_) => contents,
        };
        self.code_scroll = 0;
        format!(
            "Started over: restored original starter code for '{}'.",
            self.lang_slug
        )
    }

    fn scroll(&mut self, delta: i32) {
        let target = match self.focus {
            Focus::Desc => &mut self.desc_scroll,
            Focus::Code => &mut self.code_scroll,
            Focus::Output => &mut self.output_scroll,
        };
        *target = (*target as i32 + delta).max(0) as u16;
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Desc => Focus::Code,
            Focus::Code => Focus::Output,
            Focus::Output => Focus::Desc,
        };
    }

    fn current_code(&self) -> String {
        self.code.clone()
    }

    fn test_input(&self) -> String {
        if !self.detail.example_testcases.is_empty() {
            self.detail.example_testcases.clone()
        } else {
            self.detail.sample_test_case.clone()
        }
    }

    fn run_test(&mut self) {
        if !self.cfg.is_authenticated() {
            self.status = "Not logged in. Run `lcx login` first.".to_string();
            return;
        }
        let input = self.test_input();
        if input.trim().is_empty() {
            self.status = "No sample test input available for this problem.".to_string();
            return;
        }
        let code = self.current_code();
        let result = block_on(self.client.test(
            &self.detail.slug,
            self.detail.question_id,
            &self.lang_slug,
            &code,
            &input,
        ));
        self.status = match result {
            Ok(r) => strip_ansi(&render::format_test_result(&r)),
            Err(err) => format!("Test failed: {err:#}"),
        };
        self.output_scroll = 0;
        self.focus = Focus::Output;
    }

    fn run_submit(&mut self) {
        if !self.cfg.is_authenticated() {
            self.status = "Not logged in. Run `lcx login` first.".to_string();
            return;
        }
        let code = self.current_code();
        let result = block_on(self.client.submit(
            &self.detail.slug,
            self.detail.question_id,
            &self.lang_slug,
            &code,
        ));
        self.status = match result {
            Ok(r) => strip_ansi(&render::format_submit_result(&r)),
            Err(err) => format!("Submit failed: {err:#}"),
        };
        self.output_scroll = 0;
        self.focus = Focus::Output;
    }
}

/// Run the solve view until the user goes back to the browser (returns `Ok`).
pub fn run(terminal: &mut Terminal<Backend>, app: &mut SolveApp) -> Result<()> {
    terminal.clear()?;
    loop {
        terminal.draw(|f| ui(f, app))?;
        if app.back {
            return Ok(());
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // Any key other than `r` cancels a pending start-over confirmation.
        let was_pending = app.pending_reset;
        if !matches!(key.code, KeyCode::Char('r')) {
            app.pending_reset = false;
        }

        match key.code {
            KeyCode::Esc => {
                if was_pending {
                    app.status = "Start over cancelled.".to_string();
                } else {
                    app.back = true;
                }
            }
            KeyCode::Char('r') => {
                if was_pending {
                    app.pending_reset = false;
                    app.status = app.start_over();
                } else {
                    app.pending_reset = true;
                    app.status =
                        "Start over? Press <r> again to reset to starter code (<Esc> to cancel)."
                            .to_string();
                }
            }
            KeyCode::Char('e') | KeyCode::Enter => {
                app.status = open_in_host_editor(terminal, app)
                    .unwrap_or_else(|e| format!("Editor failed: {e:#}"));
            }
            KeyCode::Tab => app.toggle_focus(),
            KeyCode::Char('j') | KeyCode::Down => app.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => app.scroll(-1),
            KeyCode::Char('t') => {
                app.status = "Testing... (running on LeetCode)".to_string();
                terminal.draw(|f| ui(f, app))?;
                app.run_test();
            }
            KeyCode::Char('s') => {
                app.status = "Submitting... (running on LeetCode)".to_string();
                terminal.draw(|f| ui(f, app))?;
                app.run_submit();
            }
            _ => {}
        }
    }
}

/// Suspend the TUI, open the solution file in the user's host editor, then
/// restore the TUI and reload the preview from disk.
fn open_in_host_editor(terminal: &mut Terminal<Backend>, app: &mut SolveApp) -> Result<String> {
    leave(terminal)?;
    let edit_result = solution::open_in_editor(&app.cfg, &app.path);
    resume(terminal)?;
    edit_result?;
    let (code, _meta) = solution::read_solution(&app.path)?;
    app.code = code;
    Ok(format!("Reloaded {} from host editor", app.path.display()))
}

fn ui(f: &mut Frame, app: &SolveApp) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(f.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(root[0]);

    let desc_title = format!(
        " {} {}  [{}] ",
        app.detail.frontend_id, app.detail.title, app.detail.difficulty
    );
    let desc = Paragraph::new(app.desc.clone())
        .block(pane_block(&desc_title, app.focus == Focus::Desc))
        .wrap(Wrap { trim: false })
        .scroll((app.desc_scroll, 0));
    f.render_widget(desc, panes[0]);

    let code_title = format!(" Solution ({}) [review] ", app.lang_slug);
    let code = Paragraph::new(code_text(&app.code))
        .block(pane_block(&code_title, app.focus == Focus::Code))
        .scroll((app.code_scroll, 0));
    f.render_widget(code, panes[1]);

    let output = Paragraph::new(app.status.as_str())
        .block(pane_block(" Output ", app.focus == Focus::Output))
        .wrap(Wrap { trim: false })
        .scroll((app.output_scroll, 0));
    f.render_widget(output, root[1]);

    let help_text =
        "<e>/<Enter> edit in $EDITOR   <r> start over   <Tab> switch pane   <\u{2191}>/<\u{2193}>/<j>/<k> scroll   <t> test   <s> submit   <Esc> back";
    f.render_widget(Paragraph::new(super::help_line(help_text)), root[2]);
}

/// Render the solution code with dim line numbers for review.
fn code_text(code: &str) -> Text<'static> {
    let lines = code
        .lines()
        .enumerate()
        .map(|(i, line)| {
            Line::from(vec![
                Span::styled(
                    format!("{:>4} ", i + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(line.to_string()),
            ])
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string());
    if focused {
        block = block.border_style(Style::default().add_modifier(Modifier::BOLD));
    }
    block
}
