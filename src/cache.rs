use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::client::models::ProblemSummary;
use crate::config;

/// Local SQLite cache of the problem list, so `list`/lookups are fast and work
/// without a round-trip to LeetCode on every invocation.
pub struct Cache {
    conn: Connection,
}

/// Filters applied when querying the cached problem list.
#[derive(Debug, Default)]
pub struct ListFilter {
    pub difficulty: Option<String>,
    pub tag: Option<String>,
    pub status: Option<String>,
    pub query: Option<String>,
    pub limit: Option<i64>,
}

impl Cache {
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let cache = Self {
            conn: Connection::open_in_memory()?,
        };
        cache.init()?;
        Ok(cache)
    }

    /// Open (creating if needed) the cache database and ensure the schema.
    pub fn open() -> Result<Self> {
        let path = config::cache_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating cache dir {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening cache db {}", path.display()))?;
        let cache = Self { conn };
        cache.init()?;
        Ok(cache)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS problems (
                question_id INTEGER PRIMARY KEY,
                frontend_id TEXT NOT NULL,
                title TEXT NOT NULL,
                slug TEXT NOT NULL,
                difficulty TEXT NOT NULL,
                paid_only INTEGER NOT NULL,
                ac_rate REAL NOT NULL,
                status TEXT,
                tags TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_problems_slug ON problems(slug);
            CREATE INDEX IF NOT EXISTS idx_problems_frontend ON problems(frontend_id);",
        )?;
        Ok(())
    }

    /// Number of cached problems.
    pub fn count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM problems", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Replace the entire cache with a fresh set of problems.
    pub fn replace_all(&mut self, problems: &[ProblemSummary]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM problems", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO problems
                    (question_id, frontend_id, title, slug, difficulty, paid_only, ac_rate, status, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for p in problems {
                stmt.execute(params![
                    p.question_id,
                    p.frontend_id,
                    p.title,
                    p.slug,
                    p.difficulty,
                    p.paid_only as i64,
                    p.ac_rate,
                    p.status,
                    p.tags.join(","),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Drop all cached problems.
    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM problems", [])?;
        Ok(())
    }

    /// Query cached problems with the given filters.
    pub fn query(&self, filter: &ListFilter) -> Result<Vec<ProblemSummary>> {
        let mut sql = String::from(
            "SELECT question_id, frontend_id, title, slug, difficulty, paid_only, ac_rate, status, tags
             FROM problems WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(diff) = &filter.difficulty {
            sql.push_str(" AND difficulty = ? COLLATE NOCASE");
            args.push(Box::new(capitalize(diff)));
        }
        if let Some(tag) = &filter.tag {
            sql.push_str(" AND (',' || tags || ',') LIKE ?");
            args.push(Box::new(format!("%,{},%", tag.to_lowercase())));
        }
        if let Some(status) = &filter.status {
            match status.to_lowercase().as_str() {
                "solved" | "ac" => sql.push_str(" AND status = 'ac'"),
                "todo" | "unsolved" => sql.push_str(" AND (status IS NULL OR status = '')"),
                "attempted" | "notac" => sql.push_str(" AND status = 'notac'"),
                _ => {}
            }
        }
        if let Some(q) = &filter.query {
            sql.push_str(" AND (title LIKE ? OR slug LIKE ? OR frontend_id = ?)");
            let like = format!("%{q}%");
            args.push(Box::new(like.clone()));
            args.push(Box::new(like));
            args.push(Box::new(q.clone()));
        }

        sql.push_str(" ORDER BY question_id ASC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), row_to_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Look up a single problem by frontend id or slug.
    pub fn find(&self, key: &str) -> Result<Option<ProblemSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT question_id, frontend_id, title, slug, difficulty, paid_only, ac_rate, status, tags
             FROM problems WHERE frontend_id = ?1 OR slug = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![key], row_to_summary)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }
}

fn row_to_summary(row: &rusqlite::Row) -> rusqlite::Result<ProblemSummary> {
    let tags: String = row.get(8)?;
    Ok(ProblemSummary {
        question_id: row.get(0)?,
        frontend_id: row.get(1)?,
        title: row.get(2)?,
        slug: row.get(3)?,
        difficulty: row.get(4)?,
        paid_only: row.get::<_, i64>(5)? != 0,
        ac_rate: row.get(6)?,
        status: row.get(7)?,
        tags: if tags.is_empty() {
            Vec::new()
        } else {
            tags.split(',').map(|s| s.to_string()).collect()
        },
    })
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}
