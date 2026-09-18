use anyhow::{bail, Result};
use std::path::PathBuf;

use crate::config::{self, Config};

/// Print the current configuration.
pub fn show() -> Result<()> {
    let cfg = Config::load()?;
    let path = config::config_path()?;
    println!("Config file: {}", path.display());
    println!("lang:          {}", cfg.lang);
    println!(
        "editor:        {}",
        cfg.editor.clone().unwrap_or_else(|| "$EDITOR".to_string())
    );
    println!("workspace_dir: {}", cfg.workspace_dir.display());
    println!(
        "authenticated: {}",
        if cfg.is_authenticated() { "yes" } else { "no" }
    );
    println!(
        "cf_clearance:  {}",
        if cfg.cf_clearance.as_deref().is_some_and(|s| !s.is_empty()) {
            "saved"
        } else {
            "not set"
        }
    );
    Ok(())
}

/// Set a configuration value.
pub fn set(key: &str, value: &str) -> Result<()> {
    let mut cfg = Config::load()?;
    match key {
        "lang" => cfg.lang = value.to_string(),
        "editor" => cfg.editor = Some(value.to_string()),
        "workspace" | "workspace_dir" => cfg.workspace_dir = PathBuf::from(value),
        other => bail!("unknown config key '{other}'. Valid keys: lang, editor, workspace"),
    }
    cfg.save()?;
    println!("Set {key} = {value}");
    Ok(())
}
