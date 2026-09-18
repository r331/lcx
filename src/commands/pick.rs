use anyhow::{bail, Result};

use crate::cache::Cache;
use crate::client::LeetCodeClient;
use crate::config::Config;
use crate::solution;

/// Generate a solution file for a problem (if it does not exist) and optionally
/// open it in the editor.
pub async fn run(key: &str, lang: Option<String>, open: bool) -> Result<()> {
    let cfg = Config::load()?;
    let cache = Cache::open()?;
    let client = LeetCodeClient::from_config(&cfg)?;

    let lang_slug = lang.unwrap_or(cfg.lang.clone());
    let detail = super::fetch_detail(&client, &cache, key).await?;

    let path = solution::solution_path(&cfg, &detail.frontend_id, &detail.slug, &lang_slug);

    if path.exists() {
        println!("Solution file already exists: {}", path.display());
    } else {
        let snippet = detail.snippet_for(&lang_slug).ok_or_else(|| {
            anyhow::anyhow!(
                "no starter code for language '{lang_slug}'. Available: {}",
                detail
                    .code_snippets
                    .iter()
                    .map(|s| s.lang_slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        solution::ensure_workspace(&cfg)?;
        let contents = solution::render_file(&lang_slug, &snippet.code, &detail.content);
        std::fs::write(&path, contents)?;
        println!("Created {}", path.display());
    }

    if open {
        if !path.exists() {
            bail!("solution file missing: {}", path.display());
        }
        solution::open_in_editor(&cfg, &path)?;
    }
    Ok(())
}
