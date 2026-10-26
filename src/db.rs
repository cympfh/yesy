use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;

#[derive(Debug)]
pub struct Database {
    pub connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResult {
    pub path: String,
    pub snippet: Option<String>,
}

impl Database {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::new(connection)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::new(Connection::open_in_memory()?)
    }

    fn new(connection: Connection) -> anyhow::Result<Self> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS documents (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                content TEXT,
                modified_ns INTEGER NOT NULL DEFAULT 0,
                size INTEGER NOT NULL DEFAULT 0
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
                path,
                content,
                content='documents',
                content_rowid='id',
                tokenize='trigram'
            );
            CREATE TRIGGER IF NOT EXISTS documents_insert AFTER INSERT ON documents BEGIN
                INSERT INTO documents_fts(rowid, path, content)
                VALUES (new.id, new.path, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS documents_delete AFTER DELETE ON documents BEGIN
                INSERT INTO documents_fts(documents_fts, rowid, path, content)
                VALUES ('delete', old.id, old.path, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS documents_update AFTER UPDATE ON documents BEGIN
                INSERT INTO documents_fts(documents_fts, rowid, path, content)
                VALUES ('delete', old.id, old.path, old.content);
                INSERT INTO documents_fts(rowid, path, content)
                VALUES (new.id, new.path, new.content);
            END;
            ",
        )?;
        let mut statement = connection.prepare("PRAGMA table_info(documents)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        if !columns.iter().any(|column| column == "modified_ns") {
            connection.execute(
                "ALTER TABLE documents ADD COLUMN modified_ns INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "size") {
            connection.execute(
                "ALTER TABLE documents ADD COLUMN size INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(Self { connection })
    }

    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }

    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 100) as i64;
        let mut results = Vec::new();
        if query.chars().count() >= 3 {
            let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
            let mut statement = self.connection.prepare(
                "SELECT d.path, d.content
                 FROM documents_fts
                 JOIN documents d ON d.id = documents_fts.rowid
                 WHERE documents_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![fts_query, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (path, content) = row?;
                results.push(SearchResult {
                    path,
                    snippet: content.as_deref().map(|text| snippet(text, query)),
                });
            }
        } else {
            let like_query = format!("%{}%", escape_like(query));
            let mut statement = self.connection.prepare(
                "SELECT path, content FROM documents
                 WHERE path LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\'
                 ORDER BY path LIMIT ?2",
            )?;
            let rows = statement.query_map(params![like_query, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (path, content) = row?;
                results.push(SearchResult {
                    path,
                    snippet: content.as_deref().map(|text| snippet(text, query)),
                });
            }
        }
        Ok(results)
    }

    pub fn search_cancellable(
        &self,
        query: &str,
        limit: usize,
        cancelled: Arc<AtomicBool>,
    ) -> anyhow::Result<Vec<SearchResult>> {
        self.connection
            .progress_handler(100, Some(move || cancelled.load(Ordering::Relaxed)));
        let results = self.search(query, limit);
        self.connection.progress_handler(0, None::<fn() -> bool>);
        results
    }
}

fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn snippet(text: &str, query: &str) -> String {
    const CONTEXT_CHARS: usize = 100;
    let start = text
        .find(query)
        .or_else(|| {
            let lower_text = text.to_lowercase();
            let lower_query = query.to_lowercase();
            lower_text.find(&lower_query).filter(|index| {
                text.is_char_boundary(*index) && text.is_char_boundary(index + lower_query.len())
            })
        })
        .unwrap_or(0);
    let before = text[..start].chars().count();
    let snippet_start = text[..start]
        .char_indices()
        .nth(before.saturating_sub(CONTEXT_CHARS))
        .map_or(0, |(index, _)| index);
    let snippet_end = text[start..]
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(text.len(), |(index, _)| start + index);
    let mut result = String::new();
    if snippet_start > 0 {
        result.push('…');
    }
    result.push_str(&text[snippet_start..snippet_end]);
    if snippet_end < text.len() {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_paths_and_contents() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection
            .execute(
                "INSERT INTO documents(path, content) VALUES ('/tmp/note.txt', 'Rust makes local search possible')",
                [],
            )
            .unwrap();

        assert_eq!(database.search("local search", 100).unwrap().len(), 1);
        assert_eq!(database.search("note.txt", 100).unwrap().len(), 1);
        assert_eq!(database.search("se", 100).unwrap().len(), 1);
        assert!(database.search("missing", 100).unwrap().is_empty());
        assert!(database.search("", 100).unwrap().is_empty());
    }

    #[test]
    fn searches_japanese_text() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection
            .execute(
                "INSERT INTO documents(path, content) VALUES ('/tmp/メモ.txt', 'ローカルファイルを全文検索する')",
                [],
            )
            .unwrap();

        assert_eq!(database.search("全文検索", 100).unwrap().len(), 1);
        assert_eq!(database.search("検索", 100).unwrap().len(), 1);
        assert_eq!(database.search("メモ", 100).unwrap().len(), 1);
    }

    #[test]
    fn cancels_search_and_reuses_the_connection() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection
            .execute(
                "WITH RECURSIVE numbers(n) AS (\
                    VALUES(1) UNION ALL SELECT n + 1 FROM numbers WHERE n < 1000\
                 ) \
                 INSERT INTO documents(path, content) \
                 SELECT printf('/tmp/%d.txt', n), 'find this text' FROM numbers",
                [],
            )
            .unwrap();

        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(database.search_cancellable("fi", 100, cancelled).is_err());
        assert!(!database.search("find", 100).unwrap().is_empty());
    }

    #[test]
    fn makes_bounded_snippets() {
        let text = format!("{}needle{}", "a".repeat(200), "b".repeat(200));
        let snippet = snippet(&text, "needle");
        assert!(snippet.contains("needle"));
        assert!(snippet.chars().count() <= 202);
        assert!(snippet.starts_with('…'));
        assert!(snippet.ends_with('…'));
    }
}
