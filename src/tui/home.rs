//! The main screen: a single unified dashboard.
//!
//! Layout:
//! - top-left: the action **menu** (settings + quit)
//! - middle-left: curated and custom **problem sets**
//! - bottom-left: the user's **profile** (solved counts by difficulty)
//! - right: the **search page** (type to filter the cached problem list, open a
//!   problem in the solve view)
//!
//! `Tab` moves focus between the menu, sets, and search page.

use std::time::Duration;
use std::collections::HashMap;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::{block_on, help_line, login, solve, Backend};
use crate::cache::{Cache, ListFilter};
use crate::client::graphql::DailyChallenge;
use crate::client::models::{DifficultyStat, ProblemSummary, ProfileStats};
use crate::client::LeetCodeClient;
use crate::config::Config;
use crate::lang;
use crate::sets::{self, ProblemSet};

/// Result of a background startup fetch, delivered to the event loop so the UI
/// can paint immediately instead of blocking on the network.
enum LoadMsg {
    Profile(Result<ProfileStats>),
    Daily(Result<(String, DailyChallenge)>),
}

/// Which side currently receives navigation input.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Menu,
    Sets,
    Search,
}

/// Action menu entries, in display order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuItem {
    SetLang,
    Login,
    DeleteCache,
    ResetConfig,
    Quit,
}

impl MenuItem {
    const ALL: [MenuItem; 5] = [
        MenuItem::SetLang,
        MenuItem::Login,
        MenuItem::DeleteCache,
        MenuItem::ResetConfig,
        MenuItem::Quit,
    ];

    fn label(self, cfg: &Config) -> String {
        match self {
            MenuItem::SetLang => format!("Set default language ({})", cfg.lang),
            MenuItem::Login => "Log in / switch account".to_string(),
            MenuItem::DeleteCache => "Delete cache".to_string(),
            MenuItem::ResetConfig => "Reset config".to_string(),
            MenuItem::Quit => "Quit".to_string(),
        }
    }
}

/// Loaded state of the profile panel.
enum Profile {
    Loading,
    Ready(Box<ProfileStats>),
    Unavailable(String),
}

struct App {
    cache: Cache,
    // Search page.
    search: String,
    results: Vec<ProblemSummary>,
    list_state: ListState,
    sets: Vec<ProblemSet>,
    set_state: ListState,
    set_category_mode: bool,
    active_set: Option<usize>,
    active_category: Option<String>,
    /// Today's daily challenge, pinned to the top of results when the search
    /// box is empty. Loaded once at startup.
    daily: Option<ProblemSummary>,
    // Menu.
    menu_selected: usize,
    // Shared.
    focus: Focus,
    profile: Profile,
    status: String,
    confirm: Option<MenuItem>,
    lang_state: Option<ListState>,
    // Deferred requests that need the terminal (handled in the run loop).
    open_login: bool,
    open_solve: Option<String>,
    quit: bool,
}

impl App {
    fn new(cache: Cache) -> Result<Self> {
        let mut set_state = ListState::default();
        set_state.select(Some(0));
        let mut app = Self {
            cache,
            search: String::new(),
            results: Vec::new(),
            list_state: ListState::default(),
            sets: sets::all()?,
            set_state,
            set_category_mode: false,
            active_set: None,
            active_category: None,
            daily: None,
            menu_selected: 0,
            focus: Focus::Search,
            profile: Profile::Loading,
            status: "Type to search; <Tab> cycles Menu, Problem Sets, and Search.".to_string(),
            confirm: None,
            lang_state: None,
            open_login: false,
            open_solve: None,
            quit: false,
        };
        app.refilter();
        Ok(app)
    }

    fn menu_item(&self) -> MenuItem {
        MenuItem::ALL[self.menu_selected]
    }

    fn refilter(&mut self) {
        let is_empty = self.search.trim().is_empty();
        if let Some(set_index) = self.active_set {
            let set = &self.sets[set_index];
            let cached: HashMap<String, ProblemSummary> = self
                .cache
                .query(&ListFilter::default())
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p.slug.clone(), p))
                .collect();
            let query = self.search.trim().to_lowercase();
            self.results = set
                .entries
                .iter()
                .filter(|entry| {
                    self.active_category
                        .as_ref()
                        .is_none_or(|category| &entry.category == category)
                })
                .filter_map(|entry| {
                    let problem = cached.get(&entry.slug).cloned().unwrap_or_else(|| {
                        ProblemSummary {
                            question_id: 0,
                            frontend_id: String::new(),
                            title: if entry.title.is_empty() {
                                entry.slug.clone()
                            } else {
                                entry.title.clone()
                            },
                            slug: entry.slug.clone(),
                            difficulty: entry.difficulty.clone(),
                            paid_only: false,
                            ac_rate: 0.0,
                            status: None,
                            tags: Vec::new(),
                        }
                    });
                    (query.is_empty()
                        || problem.title.to_lowercase().contains(&query)
                        || problem.slug.contains(&query)
                        || entry.category.to_lowercase().contains(&query))
                    .then_some(problem)
                })
                .collect();
            self.list_state.select((!self.results.is_empty()).then_some(0));
            return;
        }
        let filter = ListFilter {
            difficulty: None,
            tag: None,
            status: None,
            query: if is_empty {
                None
            } else {
                Some(self.search.trim().to_string())
            },
            limit: None,
        };
        let mut results = self.cache.query(&filter).unwrap_or_default();
        // With no search query, pin today's daily challenge to the top (and
        // drop its duplicate from the rest of the list).
        if is_empty {
            if let Some(daily) = &self.daily {
                results.retain(|p| p.slug != daily.slug);
                results.insert(0, daily.clone());
            }
        }
        self.results = results;
        self.list_state.select(if self.results.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    /// Whether the given slug is the daily challenge currently pinned to the top
    /// (only pinned while the search box is empty).
    fn is_daily(&self, slug: &str) -> bool {
        self.active_set.is_none()
            && self.search.trim().is_empty()
            && self.daily.as_ref().is_some_and(|d| d.slug == slug)
    }

    fn categories(&self) -> Vec<String> {
        let Some(index) = self.active_set else { return Vec::new() };
        let mut categories = Vec::new();
        for entry in &self.sets[index].entries {
            if !entry.category.is_empty() && !categories.contains(&entry.category) {
                categories.push(entry.category.clone());
            }
        }
        categories
    }

    fn set_items(&self) -> Vec<String> {
        if self.set_category_mode {
            let mut items = vec!["← Choose set".to_string(), "All categories".to_string()];
            items.extend(self.categories());
            items
        } else {
            let mut items = vec!["All problems".to_string()];
            items.extend(self.sets.iter().map(|s| s.name.clone()));
            items
        }
    }

    fn move_set(&mut self, delta: i32) {
        let len = self.set_items().len() as i32;
        let cur = self.set_state.selected().unwrap_or(0) as i32;
        self.set_state.select(Some((cur + delta).rem_euclid(len) as usize));
    }

    fn activate_set(&mut self) {
        let selected = self.set_state.selected().unwrap_or(0);
        if self.set_category_mode {
            if selected == 0 {
                self.set_category_mode = false;
                self.set_state.select(Some(self.active_set.map_or(0, |i| i + 1)));
                return;
            }
            self.active_category = if selected == 1 {
                None
            } else {
                self.categories().get(selected - 2).cloned()
            };
        } else if selected == 0 {
            self.active_set = None;
            self.active_category = None;
        } else {
            self.active_set = Some(selected - 1);
            self.active_category = None;
            self.set_category_mode = true;
            self.set_state.select(Some(1));
        }
        self.refilter();
        let label = self.active_set.map_or("All problems", |i| &self.sets[i].name);
        self.status = format!("Showing {label} ({} problems).", self.results.len());
    }

    fn active_category_for(&self, slug: &str) -> Option<&str> {
        let index = self.active_set?;
        self.sets[index]
            .entries
            .iter()
            .find(|entry| entry.slug == slug)
            .map(|entry| entry.category.as_str())
    }

    /// Pin a fetched daily challenge to the top of the list. Best-effort: called
    /// once the background fetch resolves (on error the daily just isn't pinned).
    fn apply_daily(&mut self, daily: DailyChallenge) {
        let q = daily.question;
        // Prefer the cached summary (has status/ac rate/tags); fall back to
        // a minimal entry built from the daily payload.
        let summary = self
            .cache
            .find(&q.title_slug)
            .ok()
            .flatten()
            .unwrap_or_else(|| ProblemSummary {
                question_id: 0,
                frontend_id: q.question_frontend_id,
                title: q.title,
                slug: q.title_slug,
                difficulty: q.difficulty,
                paid_only: false,
                ac_rate: 0.0,
                status: None,
                tags: Vec::new(),
            });
        self.daily = Some(summary);
        self.refilter();
    }

    fn move_result(&mut self, delta: i32) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len() as i32;
        let cur = self.list_state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, len - 1);
        self.list_state.select(Some(next as usize));
    }

    fn move_menu(&mut self, delta: i32) {
        let len = MenuItem::ALL.len() as i32;
        self.menu_selected = ((self.menu_selected as i32 + delta).rem_euclid(len)) as usize;
    }

    fn open_selected(&mut self) {
        if let Some(p) = self.list_state.selected().and_then(|i| self.results.get(i)) {
            self.open_solve = Some(p.slug.clone());
        }
    }

    /// Fetch the full problem list from LeetCode and rebuild the cache, showing
    /// page-by-page progress in the status bar (like the CLI `cache --update`).
    fn refresh(&mut self, terminal: &mut Terminal<Backend>, cfg: &Config) -> Result<()> {
        const PAGE: i64 = 500;

        let client = match LeetCodeClient::from_config(cfg) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("Client error: {e:#}");
                return Ok(());
            }
        };

        self.status = "Updating problem list... (fetching from LeetCode)".to_string();
        terminal.draw(|f| ui(f, self, cfg))?;

        let mut all = Vec::new();
        let mut skip = 0i64;
        loop {
            let (total, batch) =
                match block_on(client.list_problems(PAGE, skip, serde_json::json!({}))) {
                    Ok(v) => v,
                    Err(e) => {
                        self.status = format!("Update failed: {e:#}");
                        return Ok(());
                    }
                };
            let fetched = batch.len() as i64;
            all.extend(batch);
            skip += fetched;
            self.status = format!("Updating problem list... {} / {total} problems", all.len());
            terminal.draw(|f| ui(f, self, cfg))?;
            if fetched == 0 || skip >= total {
                break;
            }
        }

        match self.cache.replace_all(&all) {
            Ok(()) => {
                let n = all.len();
                self.refilter();
                self.status = format!("Updated {n} problems.");
            }
            Err(e) => self.status = format!("Cache write failed: {e}"),
        }
        Ok(())
    }

    fn load_profile(&mut self, cfg: &Config) {
        if !cfg.is_authenticated() {
            self.profile = Profile::Unavailable("Not logged in.".to_string());
            return;
        }
        self.profile = Profile::Loading;
        let client = match LeetCodeClient::from_config(cfg) {
            Ok(c) => c,
            Err(e) => {
                self.profile = Profile::Unavailable(format!("Client error: {e:#}"));
                return;
            }
        };
        match block_on(client.profile_stats()) {
            Ok(stats) => self.profile = Profile::Ready(Box::new(stats)),
            Err(e) => self.profile = Profile::Unavailable(format!("Could not load profile: {e:#}")),
        }
    }

    fn open_lang_picker(&mut self, cfg: &Config) {
        let cur = lang::PICKABLE
            .iter()
            .position(|s| *s == cfg.lang)
            .unwrap_or(0);
        let mut state = ListState::default();
        state.select(Some(cur));
        self.lang_state = Some(state);
    }

    fn move_lang(&mut self, delta: i32) {
        if let Some(state) = &mut self.lang_state {
            let len = lang::PICKABLE.len() as i32;
            let cur = state.selected().unwrap_or(0) as i32;
            state.select(Some(((cur + delta).rem_euclid(len)) as usize));
        }
    }

    fn commit_lang(&mut self, cfg: &mut Config) {
        if let Some(i) = self.lang_state.as_ref().and_then(|s| s.selected()) {
            let slug = lang::PICKABLE[i].to_string();
            cfg.lang = slug.clone();
            self.status = match cfg.save() {
                Ok(()) => format!("Default language set to {slug}."),
                Err(e) => format!("Failed to save config: {e:#}"),
            };
        }
        self.lang_state = None;
    }

    fn delete_cache(&mut self) {
        self.status = match self.cache.clear() {
            Ok(()) => {
                self.refilter();
                "Cache deleted.".to_string()
            }
            Err(e) => format!("Failed to delete cache: {e:#}"),
        };
    }

    fn reset_config(&mut self, cfg: &mut Config) {
        *cfg = Config::default();
        self.status = match cfg.save() {
            Ok(()) => "Config reset to defaults (session cleared).".to_string(),
            Err(e) => format!("Failed to reset config: {e:#}"),
        };
        self.load_profile(cfg);
    }

    fn activate_menu(&mut self, cfg: &Config) {
        match self.menu_item() {
            MenuItem::SetLang => self.open_lang_picker(cfg),
            MenuItem::Login => self.open_login = true,
            MenuItem::DeleteCache => {
                self.confirm = Some(MenuItem::DeleteCache);
                self.status =
                    "Delete cache? Press <y> to confirm, any other key to cancel.".to_string();
            }
            MenuItem::ResetConfig => {
                self.confirm = Some(MenuItem::ResetConfig);
                self.status =
                    "Reset config (clears session + settings)? Press <y> to confirm, any other key to cancel.".to_string();
            }
            MenuItem::Quit => self.quit = true,
        }
    }
}

/// Run the unified main screen until the user quits.
pub fn run(terminal: &mut Terminal<Backend>, cfg: &mut Config, cache: Cache) -> Result<()> {
    let mut app = App::new(cache)?;

    // Load the profile and daily challenge in the background so the cached
    // problem list paints instantly instead of waiting on two network calls.
    let (tx, rx) = std::sync::mpsc::channel::<LoadMsg>();
    let mut pending = 0usize;

    if cfg.is_authenticated() {
        match LeetCodeClient::from_config(cfg) {
            Ok(client) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(LoadMsg::Profile(client.profile_stats().await));
                });
                pending += 1;
            }
            Err(e) => app.profile = Profile::Unavailable(format!("Client error: {e:#}")),
        }
    } else {
        app.profile = Profile::Unavailable("Not logged in.".to_string());
    }

    if let Ok(client) = LeetCodeClient::from_config(cfg) {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(LoadMsg::Daily(client.daily().await));
        });
        pending += 1;
    }
    drop(tx);

    terminal.clear()?;

    loop {
        // Apply any background loads that have finished.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                LoadMsg::Profile(res) => {
                    app.profile = match res {
                        Ok(stats) => Profile::Ready(Box::new(stats)),
                        Err(e) => Profile::Unavailable(format!("Could not load profile: {e:#}")),
                    };
                }
                LoadMsg::Daily(res) => {
                    if let Ok((_date, daily)) = res {
                        app.apply_daily(daily);
                    }
                }
            }
            pending = pending.saturating_sub(1);
        }

        terminal.draw(|f| ui(f, &mut app, cfg))?;
        if app.quit {
            return Ok(());
        }

        // Deferred actions that need the terminal.
        if app.open_login {
            app.open_login = false;
            match login::run(terminal, cfg, false)? {
                login::LoginOutcome::Saved {
                    cfg: new_cfg,
                    username,
                } => {
                    *cfg = new_cfg;
                    app.status = format!("Logged in as {username}.");
                    app.load_profile(cfg);
                }
                login::LoginOutcome::Offline | login::LoginOutcome::Cancelled => {
                    app.status = "Login cancelled.".to_string();
                }
            }
            terminal.clear()?;
            continue;
        }
        if let Some(slug) = app.open_solve.take() {
            let client = LeetCodeClient::from_config(cfg)?;
            match solve::prepare(cfg, &client, &slug, None) {
                Ok(mut s) => solve::run(terminal, &mut s)?,
                Err(e) => app.status = format!("Could not open problem: {e:#}"),
            }
            terminal.clear()?;
            continue;
        }

        // While startup loads are in flight, poll so freshly arrived results are
        // picked up and drawn without waiting for a keypress.
        if pending > 0 && !event::poll(Duration::from_millis(50))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Language sub-picker captures input while open.
        if app.lang_state.is_some() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_lang(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_lang(1),
                KeyCode::Enter => app.commit_lang(cfg),
                KeyCode::Esc => {
                    app.lang_state = None;
                    app.status = "Language unchanged.".to_string();
                }
                _ => {}
            }
            continue;
        }

        // A destructive action awaiting confirmation.
        if let Some(pending) = app.confirm {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    app.confirm = None;
                    match pending {
                        MenuItem::DeleteCache => app.delete_cache(),
                        MenuItem::ResetConfig => app.reset_config(cfg),
                        _ => {}
                    }
                }
                _ => {
                    app.confirm = None;
                    app.status = "Cancelled.".to_string();
                }
            }
            continue;
        }

        // Global keys.
        match key.code {
            KeyCode::Esc => {
                app.quit = true;
                continue;
            }
            KeyCode::Tab => {
                app.focus = match app.focus {
                    Focus::Menu => Focus::Sets,
                    Focus::Sets => Focus::Search,
                    Focus::Search => Focus::Menu,
                };
                continue;
            }
            KeyCode::F(1) => {
                app.open_login = true;
                continue;
            }
            KeyCode::F(5) => {
                app.refresh(terminal, cfg)?;
                continue;
            }
            _ => {}
        }

        match app.focus {
            Focus::Menu => match key.code {
                KeyCode::Up => app.move_menu(-1),
                KeyCode::Down => app.move_menu(1),
                KeyCode::Enter => app.activate_menu(cfg),
                _ => {}
            },
            Focus::Sets => match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_set(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_set(1),
                KeyCode::Enter => app.activate_set(),
                KeyCode::Backspace if app.set_category_mode => {
                    app.set_category_mode = false;
                    app.set_state.select(Some(app.active_set.map_or(0, |i| i + 1)));
                }
                _ => {}
            },
            Focus::Search => match key.code {
                KeyCode::Up => app.move_result(-1),
                KeyCode::Down => app.move_result(1),
                KeyCode::Enter => app.open_selected(),
                KeyCode::Backspace => {
                    app.search.pop();
                    app.refilter();
                }
                KeyCode::Char(c) if !ctrl => {
                    app.search.push(c);
                    app.refilter();
                }
                _ => {}
            },
        }
    }
}

fn ui(f: &mut Frame, app: &mut App, cfg: &Config) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Title bar.
    let title = Line::from(vec![
        Span::styled(
            " lcx ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(255, 161, 22))
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(title), root[0]);

    // Left sidebar (menu, sets, profile) and right search page.
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(40), Constraint::Min(30)])
        .split(root[1]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Min(7),
        ])
        .split(body[0]);

    // Menu (top-left).
    let menu_focused = app.focus == Focus::Menu;
    let items: Vec<ListItem> = MenuItem::ALL
        .iter()
        .map(|m| ListItem::new(m.label(cfg)))
        .collect();
    let mut menu_state = ListState::default();
    menu_state.select(Some(app.menu_selected));
    let menu = List::new(items)
        .block(pane_block(" Menu ", menu_focused))
        .highlight_style(highlight(menu_focused))
        .highlight_symbol(if menu_focused { "> " } else { "  " });
    f.render_stateful_widget(menu, left[0], &mut menu_state);

    // Problem sets (middle-left). Enter a set to see its categories.
    let sets_focused = app.focus == Focus::Sets;
    let set_title = if app.set_category_mode {
        app.active_set.map_or(" Problem Sets ".to_string(), |i| {
            format!(" {} categories ", app.sets[i].name)
        })
    } else {
        " Problem Sets ".to_string()
    };
    let set_items: Vec<ListItem> = app.set_items().into_iter().map(ListItem::new).collect();
    let set_list = List::new(set_items)
        .block(pane_block(&set_title, sets_focused))
        .highlight_style(highlight(sets_focused))
        .highlight_symbol(if sets_focused { "> " } else { "  " });
    f.render_stateful_widget(set_list, left[1], &mut app.set_state);

    // Profile (bottom-left).
    f.render_widget(profile_widget(&app.profile, left[2]), left[2]);

    // Search page (right): search box + results.
    let search_focused = app.focus == Focus::Search;
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(body[1]);

    let cursor = if search_focused { "\u{2588}" } else { "" };
    let search = Paragraph::new(format!("{}{cursor}", app.search))
        .block(pane_block(" Search (name or id) ", search_focused));
    f.render_widget(search, right[0]);

    let list_items: Vec<ListItem> = app
        .results
        .iter()
        .map(|p| {
            ListItem::new(problem_line(
                p,
                app.is_daily(&p.slug),
                app.active_category_for(&p.slug),
            ))
        })
        .collect();
    let list_title = if let Some(index) = app.active_set {
        let category = app
            .active_category
            .as_ref()
            .map_or(String::new(), |s| format!(" / {s}"));
        let total = app.active_category.as_ref().map_or(app.sets[index].entries.len(), |s| {
            app.sets[index]
                .entries
                .iter()
                .filter(|entry| &entry.category == s)
                .count()
        });
        format!(
            " {}{} ({} / {}) ",
            app.sets[index].name,
            category,
            app.results.len(),
            total
        )
    } else {
        let total = app.cache.count().unwrap_or(0);
        format!(" Problems ({} / {total}) ", app.results.len())
    };
    let list = List::new(list_items)
        .block(pane_block(&list_title, search_focused))
        .highlight_style(highlight(search_focused))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, right[1], &mut app.list_state);

    // Status pane.
    let status = Paragraph::new(app.status.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Status "))
        .wrap(Wrap { trim: true });
    f.render_widget(status, root[2]);

    // Help line.
    let help = "<Tab> focus   <\u{2191}>/<\u{2193}> move   <Enter> open/select   <F5> refresh   <F1> login   <Esc> quit";
    f.render_widget(Paragraph::new(help_line(help)), root[3]);

    // Language picker overlay.
    if app.lang_state.is_some() {
        render_lang_picker(f, app);
    }
}

fn profile_widget(profile: &Profile, area: Rect) -> Paragraph<'static> {
    let block = Block::default().borders(Borders::ALL).title(" Profile ");
    match profile {
        Profile::Loading => Paragraph::new("Loading profile...").block(block),
        Profile::Unavailable(msg) => Paragraph::new(msg.clone())
            .block(block)
            .wrap(Wrap { trim: true }),
        Profile::Ready(stats) => {
            Paragraph::new(profile_lines(stats, area.width, area.height)).block(block)
        }
    }
}

fn profile_lines(stats: &ProfileStats, width: u16, height: u16) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::raw("Signed in as "),
            Span::styled(
                stats.username.clone(),
                Style::default()
                    .fg(Color::Rgb(255, 161, 22))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    if height > 8 {
        lines.push(Line::raw(""));
    }
    lines.extend([
        stat_line("Total ", &stats.total, Color::White, width),
        stat_line("Easy  ", &stats.easy, Color::Green, width),
        stat_line("Medium", &stats.medium, Color::Yellow, width),
        stat_line("Hard  ", &stats.hard, Color::Red, width),
    ]);
    if height <= 8 {
        return lines;
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        format!(
            "Solved {} of {} ({:.1}%)",
            stats.total.solved,
            stats.total.total,
            stats.total.percent()
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines
}

/// One difficulty row: `Easy   120 / 800   15.0%  [====      ]`.
fn stat_line(label: &str, stat: &DifficultyStat, color: Color, width: u16) -> Line<'static> {
    let pct = stat.percent();
    let bar_cells = (width.saturating_sub(30)).clamp(6, 20) as usize;
    Line::from(vec![
        Span::styled(
            format!("{label} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("{:>4}/{:<4} ", stat.solved, stat.total)),
        Span::raw(format!("{pct:>5.1}% ")),
        Span::styled(bar(pct, bar_cells), Style::default().fg(color)),
    ])
}

/// A progress bar of `cells` width for a 0-100 percentage.
fn bar(pct: f64, cells: usize) -> String {
    let filled = ((pct / 100.0 * cells as f64).round() as usize).min(cells);
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled),
        " ".repeat(cells - filled)
    )
}

fn problem_line(p: &ProblemSummary, is_daily: bool, category: Option<&str>) -> Line<'static> {
    let status = match p.status.as_deref() {
        Some("ac") => Span::styled("\u{2714} ", Style::default().fg(Color::Green)),
        Some("notac") => Span::styled("\u{2717} ", Style::default().fg(Color::Yellow)),
        _ => Span::raw("  "),
    };
    let diff_color = match p.difficulty.as_str() {
        "Easy" => Color::Green,
        "Medium" => Color::Yellow,
        "Hard" => Color::Red,
        _ => Color::Gray,
    };
    let lock = if p.paid_only { " \u{1f512}" } else { "" };
    let mut spans = vec![
        status,
        Span::raw(format!("{:>5}  ", p.frontend_id)),
        Span::raw(p.title.clone()),
        Span::raw("  "),
        Span::styled(p.difficulty.clone(), Style::default().fg(diff_color)),
        Span::raw(lock.to_string()),
    ];
    if is_daily {
        spans.push(Span::styled(
            "  \u{2605} Daily",
            Style::default()
                .fg(Color::Rgb(255, 161, 22))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(category) = category.filter(|s| !s.is_empty()) {
        spans.push(Span::styled(
            format!("  [{category}]"),
            Style::default().fg(Color::Cyan),
        ));
    }
    Line::from(spans)
}

fn render_lang_picker(f: &mut Frame, app: &App) {
    let area = centered_rect(40, 60, f.area());
    f.render_widget(Clear, area);

    let items: Vec<ListItem> = lang::PICKABLE.iter().map(|s| ListItem::new(*s)).collect();
    let mut state = app.lang_state.clone().unwrap_or_default();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Default language  (<Enter> select, <Esc> cancel) "),
        )
        .highlight_style(highlight(true))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, &mut state);
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string());
    if focused {
        block = block.border_style(
            Style::default()
                .fg(Color::Rgb(255, 161, 22))
                .add_modifier(Modifier::BOLD),
        );
    }
    block
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

/// A centered rectangle sized as a percentage of `area`.
fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vert[1])[1]
}

#[cfg(test)]
mod tests {
    use super::{ui, App};
    use crate::cache::Cache;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn selecting_neetcode_and_category_filters_problem_list() {
        let mut app = App::new(Cache::open_in_memory().unwrap()).unwrap();
        app.set_state.select(Some(1));
        app.activate_set();
        assert_eq!(app.results.len(), 150);
        assert!(app.set_category_mode);
        let two_pointers = app
            .set_items()
            .iter()
            .position(|label| label == "Two Pointers")
            .unwrap();
        app.set_state.select(Some(two_pointers));
        app.activate_set();
        assert_eq!(app.results.len(), 5);
        assert!(app.results.iter().all(|problem| {
            app.active_category_for(&problem.slug) == Some("Two Pointers")
        }));
    }

    #[test]
    fn dashboard_renders_all_three_left_tiles() {
        let mut app = App::new(Cache::open_in_memory().unwrap()).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| ui(frame, &mut app, &Config::default())).unwrap();
        let buffer = terminal.backend().buffer();
        let screen = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("Menu"));
        assert!(screen.contains("Problem Sets"));
        assert!(screen.contains("Profile"));
        assert!(screen.contains("NeetCode 150"));
    }
}
