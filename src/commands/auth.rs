use anyhow::{Context, Result};
use std::io::Write;

use crate::client::LeetCodeClient;
use crate::config::{self, Config};

/// Save session credentials. If either value is omitted, prompt for it.
pub async fn login(
    session: Option<String>,
    csrf: Option<String>,
    cf_clearance: Option<String>,
) -> Result<()> {
    let mut cfg = Config::load()?;

    let session = match session {
        Some(s) => s,
        None if cf_clearance.is_some() && cfg.session.is_some() => cfg.session.clone().unwrap(),
        None => prompt("LEETCODE_SESSION: ")?,
    };
    let csrf = match csrf {
        Some(c) => c,
        None if cf_clearance.is_some() && cfg.csrf_token.is_some() => cfg.csrf_token.clone().unwrap(),
        None => prompt("csrftoken: ")?,
    };

    cfg.session = Some(session.trim().to_string());
    cfg.csrf_token = Some(csrf.trim().to_string());
    if let Some(clearance) = cf_clearance {
        cfg.cf_clearance = Some(clearance.trim().to_string());
    }
    cfg.save()?;

    let path = config::config_path()?;
    println!("Saved credentials to {}", path.display());

    // Verify the credentials work.
    let client = LeetCodeClient::from_config(&cfg)?;
    match client.whoami().await {
        Ok(Some(user)) => println!("Logged in as {user}"),
        Ok(None) => println!("Warning: credentials saved but LeetCode reports not signed in."),
        Err(e) => println!("Warning: could not verify credentials: {e}"),
    }
    Ok(())
}

/// Print the currently authenticated user.
pub async fn whoami() -> Result<()> {
    let cfg = Config::load()?;
    super::require_auth(&cfg)?;
    let client = LeetCodeClient::from_config(&cfg)?;
    match client.whoami().await? {
        Some(user) => println!("{user}"),
        None => println!("Not signed in (credentials may be expired)."),
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush().ok();
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("reading input")?;
    Ok(buf.trim().to_string())
}
