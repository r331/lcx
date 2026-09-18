//! Curated problem sets. Built-in lists are available offline;
//! user sets are stored separately from the LeetCode problem cache.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetEntry {
    pub slug: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub difficulty: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProblemSet {
    pub name: String,
    #[serde(default)]
    pub source_url: String,
    pub entries: Vec<SetEntry>,
}

fn path() -> Result<PathBuf> {
    Ok(config::project_dir()?.join("sets.json"))
}

pub fn builtins() -> Vec<ProblemSet> {
    vec![
        serde_json::from_str(include_str!("../data/neetcode150.json"))
            .expect("bundled NeetCode 150 data is valid"),
        serde_json::from_str(include_str!("../data/amazon.json"))
            .expect("bundled Amazon data is valid"),
    ]
}

pub fn load_custom() -> Result<Vec<ProblemSet>> {
    let path = path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn save_custom(sets: &[ProblemSet]) -> Result<()> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(sets)?;
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn validate_name(name: &str, sets: &[ProblemSet]) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("set name cannot be empty");
    }
    if name.eq_ignore_ascii_case("All problems")
        || builtins().iter().any(|set| set.name.eq_ignore_ascii_case(name))
    {
        bail!("'{name}' is a reserved set name");
    }
    if sets.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
        bail!("set '{name}' already exists");
    }
    Ok(name.to_string())
}

fn normalize_slug(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let slug = raw
        .strip_prefix("https://leetcode.com/problems/")
        .unwrap_or(raw)
        .trim_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    if slug.is_empty()
        || slug.contains(char::is_whitespace)
        || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        bail!("provide a LeetCode slug or problem URL such as two-sum");
    }
    Ok(slug.to_string())
}

pub fn all() -> Result<Vec<ProblemSet>> {
    let mut sets = builtins();
    let builtin_names: Vec<String> = sets.iter().map(|set| set.name.to_lowercase()).collect();
    sets.extend(load_custom()?.into_iter().filter(|custom| {
        !builtin_names.contains(&custom.name.to_lowercase())
    }));
    Ok(sets)
}

pub fn create(name: &str) -> Result<()> {
    let mut sets = load_custom()?;
    let name = validate_name(name, &sets)?;
    sets.push(ProblemSet {
        name,
        source_url: String::new(),
        entries: Vec::new(),
    });
    save_custom(&sets)
}

pub fn add(set_name: &str, slug: &str, category: &str) -> Result<()> {
    let slug = normalize_slug(slug)?;
    let mut sets = load_custom()?;
    let set = sets
        .iter_mut()
        .find(|s| s.name.eq_ignore_ascii_case(set_name))
        .with_context(|| format!("custom set '{set_name}' not found; create it first"))?;
    if set.entries.iter().any(|e| e.slug == slug) {
        bail!("'{slug}' is already in '{}'", set.name);
    }
    set.entries.push(SetEntry {
        slug,
        title: String::new(),
        category: category.trim().to_string(),
        difficulty: String::new(),
    });
    save_custom(&sets)
}

/// Import a set from lines of `slug,category` or LeetCode problem URLs.
pub fn import_file(name: &str, file: &std::path::Path) -> Result<usize> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("reading {}", file.display()))?;
    let mut sets = load_custom()?;
    let name = validate_name(name, &sets)?;
    let entries = parse_import(&text)?;
    let count = entries.len();
    sets.push(ProblemSet {
        name,
        source_url: String::new(),
        entries,
    });
    save_custom(&sets)?;
    Ok(count)
}

fn parse_import(text: &str) -> Result<Vec<SetEntry>> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.eq_ignore_ascii_case("slug,category") {
            continue;
        }
        let (raw_slug, category) = line.split_once(',').unwrap_or((line, "Other"));
        let slug = normalize_slug(raw_slug)
            .with_context(|| format!("line {}", index + 1))?;
        if entries.iter().any(|e: &SetEntry| e.slug == slug) {
            bail!("duplicate problem '{slug}' on line {}", index + 1);
        }
        entries.push(SetEntry {
            slug,
            title: String::new(),
            category: category.trim().to_string(),
            difficulty: String::new(),
        });
    }
    if entries.is_empty() {
        bail!("import file contains no problems");
    }
    Ok(entries)
}

pub fn remove(set_name: &str, slug: &str) -> Result<()> {
    let mut sets = load_custom()?;
    let set = sets
        .iter_mut()
        .find(|s| s.name.eq_ignore_ascii_case(set_name))
        .with_context(|| format!("custom set '{set_name}' not found"))?;
    let before = set.entries.len();
    set.entries.retain(|e| e.slug != slug);
    if set.entries.len() == before {
        bail!("'{slug}' is not in '{}'", set.name);
    }
    save_custom(&sets)
}

pub fn delete(set_name: &str) -> Result<()> {
    let mut sets = load_custom()?;
    let before = sets.len();
    sets.retain(|s| !s.name.eq_ignore_ascii_case(set_name));
    if sets.len() == before {
        bail!("custom set '{set_name}' not found");
    }
    save_custom(&sets)
}

#[cfg(test)]
mod tests {
    use super::{builtins, parse_import};
    use std::collections::HashSet;

    #[test]
    fn neetcode_has_150_unique_problems_and_categories() {
        let set = &builtins()[0];
        let slugs: HashSet<_> = set.entries.iter().map(|e| &e.slug).collect();
        assert_eq!(set.entries.len(), 150);
        assert_eq!(slugs.len(), 150);
        assert!(set.entries.iter().any(|e| e.category == "Two Pointers"));
        assert!(set.entries.iter().any(|e| e.category == "Stack"));
        assert!(set.entries.iter().all(|e| !e.difficulty.is_empty()));
    }

    #[test]
    fn amazon_has_450_unique_problems_and_difficulties() {
        let set = &builtins()[1];
        let slugs: HashSet<_> = set.entries.iter().map(|e| &e.slug).collect();
        assert_eq!(set.name, "Amazon");
        assert_eq!(set.entries.len(), 450);
        assert_eq!(slugs.len(), 450);
        assert!(set.entries.iter().all(|e| !e.title.is_empty() && !e.difficulty.is_empty()));
    }

    #[test]
    fn imports_slugs_urls_and_categories() {
        let entries = parse_import(
            "slug,category\ntwo-sum,Arrays & Hashing\nhttps://leetcode.com/problems/valid-parentheses/,Stack\n",
        )
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].slug, "two-sum");
        assert_eq!(entries[1].slug, "valid-parentheses");
        assert_eq!(entries[1].category, "Stack");
    }
}
