use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::lang;

/// Metadata describing which problem a solution file belongs to.
#[derive(Debug, Clone)]
pub struct SolutionMeta {
    pub slug: String,
    pub lang_slug: String,
}

/// Compute the on-disk path for a problem's solution file.
pub fn solution_path(cfg: &Config, frontend_id: &str, slug: &str, lang_slug: &str) -> PathBuf {
    let ext = lang::extension_for(lang_slug);
    cfg.workspace_dir
        .join(format!("{frontend_id}.{slug}.{ext}"))
}

/// Build the solution file from starter code, appending the problem description
/// as comments for Python. `test`/`submit` identify the problem from the file
/// name (`{id}.{slug}.{ext}`).
pub fn render_file(lang_slug: &str, code: &str, description_html: &str) -> String {
    let mut contents = format!("{}\n", clean_snippet(code));
    if lang_slug == "python3" && !description_html.trim().is_empty() {
        contents.push_str("\n# --- Problem description ---\n");
        let description = crate::render::html_to_text(description_html);
        for line in description.trim_end().lines() {
            if line.trim().is_empty() {
                contents.push_str("#\n");
            } else {
                contents.push_str("# ");
                contents.push_str(line);
                contents.push('\n');
            }
        }
    }
    contents
}

/// Normalize a LeetCode starter snippet: strip trailing whitespace from every
/// line (LeetCode ships empty method bodies and separators as lines full of
/// spaces) and drop leading/trailing blank lines.
fn clean_snippet(code: &str) -> String {
    code.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

/// Parse the `@lcx` metadata header from a solution file's contents.
pub fn parse_meta(contents: &str) -> Option<SolutionMeta> {
    let line = contents.lines().find(|l| l.contains("@lcx"))?;
    let mut slug = None;
    let mut lang_slug = None;
    for token in line.split_whitespace() {
        if let Some(v) = token.strip_prefix("slug=") {
            slug = Some(v.to_string());
        } else if let Some(v) = token.strip_prefix("lang=") {
            lang_slug = Some(v.to_string());
        }
    }
    Some(SolutionMeta {
        slug: slug?,
        lang_slug: lang_slug?,
    })
}

/// Read a solution file and best-effort resolve its metadata. Falls back to the
/// filename convention `{id}.{slug}.{ext}` when no header is present.
pub fn read_solution(path: &Path) -> Result<(String, Option<SolutionMeta>)> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading solution file {}", path.display()))?;
    let meta = parse_meta(&contents).or_else(|| meta_from_filename(path));
    Ok((contents, meta))
}

fn meta_from_filename(path: &Path) -> Option<SolutionMeta> {
    let stem = path.file_name()?.to_str()?;
    // Expect `{id}.{slug}.{ext}`.
    let ext = path.extension()?.to_str()?;
    let without_ext = stem.strip_suffix(&format!(".{ext}"))?;
    let (_id, slug) = without_ext.split_once('.')?;
    let lang_slug = lang::slug_from_extension(ext)?;
    Some(SolutionMeta {
        slug: slug.to_string(),
        lang_slug: lang_slug.to_string(),
    })
}

/// Locate an existing solution file for a problem in the workspace, trying the
/// preferred language first, then any language.
pub fn find_existing(
    cfg: &Config,
    frontend_id: &str,
    slug: &str,
    preferred: &str,
) -> Option<PathBuf> {
    let preferred_path = solution_path(cfg, frontend_id, slug, preferred);
    if preferred_path.exists() {
        return Some(preferred_path);
    }
    let prefix = format!("{frontend_id}.{slug}.");
    let entries = std::fs::read_dir(&cfg.workspace_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_str().unwrap_or("");
        if name.starts_with(&prefix) {
            return Some(entry.path());
        }
    }
    None
}

/// Ensure the workspace directory exists.
pub fn ensure_workspace(cfg: &Config) -> Result<()> {
    std::fs::create_dir_all(&cfg.workspace_dir)
        .with_context(|| format!("creating workspace {}", cfg.workspace_dir.display()))?;
    Ok(())
}

/// Open a file in the configured editor.
pub fn open_in_editor(cfg: &Config, path: &Path) -> Result<()> {
    let editor = cfg.resolve_editor();
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("launching editor '{editor}'"))?;
    if !status.success() {
        return Err(anyhow!("editor '{editor}' exited with an error"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clean_snippet, render_file};

    #[test]
    fn strips_trailing_whitespace_and_normalizes_endings() {
        // Mimics a LeetCode snippet: empty bodies / separators are lines of
        // spaces, and endings may be CRLF.
        let raw = "class MinStack {\r\n    void pop() {\r\n        \r\n    }\r\n    \r\n    int top() {\r\n        \r\n    }\r\n};\r\n";
        let cleaned = clean_snippet(raw);
        assert_eq!(
            cleaned,
            "class MinStack {\n    void pop() {\n\n    }\n\n    int top() {\n\n    }\n};"
        );
        assert!(!cleaned.lines().any(|l| l != l.trim_end()));
    }

    #[test]
    fn python_file_ends_with_full_commented_description() {
        let file = render_file(
            "python3",
            "class Solution:\n    pass",
            "<p>Swap adjacent nodes.</p><p><strong>Example:</strong> 1 → 2 becomes 2 → 1.</p>",
        );
        assert!(file.contains("class Solution:\n    pass\n\n# --- Problem description ---\n"));
        assert!(file.starts_with("class Solution:"));
        assert!(!file.contains("Solved with LCX"));
        assert!(file.contains("# Swap adjacent nodes."));
        assert!(file.contains("Example:"));
        assert!(file
            .lines()
            .skip_while(|line| *line != "# --- Problem description ---")
            .skip(1)
            .all(|line| line.starts_with('#')));
        assert!(!render_file("rust", "fn main() {}", "<p>Description</p>")
            .contains("Problem description"));
    }
}
