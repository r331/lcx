use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persisted configuration and authentication data, stored as TOML at
/// `~/.config/lcx/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// LeetCode session cookie (`LEETCODE_SESSION`).
    pub session: Option<String>,
    /// CSRF token cookie (`csrftoken`).
    pub csrf_token: Option<String>,
    /// Optional Cloudflare challenge clearance cookie (`cf_clearance`).
    pub cf_clearance: Option<String>,
    /// Default programming language slug (e.g. `rust`, `python3`, `cpp`).
    pub lang: String,
    /// Editor command used to open solution files. Falls back to `$EDITOR`.
    pub editor: Option<String>,
    /// Directory where solution files are written.
    pub workspace_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            session: None,
            csrf_token: None,
            cf_clearance: None,
            lang: "python3".to_string(),
            editor: None,
            workspace_dir: default_workspace_dir(),
        }
    }
}

fn default_workspace_dir() -> PathBuf {
    BaseDirs::new()
        .map(|d| d.home_dir().join("lcx"))
        .unwrap_or_else(|| PathBuf::from("lcx"))
}

/// Returns the directory used to store config and cache (`~/.config/lcx`).
pub fn project_dir() -> Result<PathBuf> {
    let dirs =
        ProjectDirs::from("", "", "lcx").context("could not determine a home/config directory")?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Path to the config file.
pub fn config_path() -> Result<PathBuf> {
    Ok(project_dir()?.join("config.toml"))
}

/// Path to the SQLite cache database.
pub fn cache_path() -> Result<PathBuf> {
    Ok(project_dir()?.join("cache.sqlite"))
}

impl Config {
    /// Load config from disk, returning defaults if the file does not exist.
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        let cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("parsing config at {}", path.display()))?;
        Ok(cfg)
    }

    /// Persist config to disk, creating parent directories and restricting
    /// permissions since it holds session credentials.
    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        let raw = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(&path, raw)
            .with_context(|| format!("writing config to {}", path.display()))?;
        restrict_permissions(&path)?;
        Ok(())
    }

    /// Returns true when session + csrf token are both present.
    pub fn is_authenticated(&self) -> bool {
        self.session
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            && self
                .csrf_token
                .as_deref()
                .map(|s| !s.is_empty())
                .unwrap_or(false)
    }

    /// Resolve the editor command, preferring config then `$EDITOR`.
    pub fn resolve_editor(&self) -> String {
        self.editor
            .clone()
            .or_else(|| std::env::var("EDITOR").ok())
            .unwrap_or_else(|| "vi".to_string())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}
