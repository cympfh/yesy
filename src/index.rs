use std::{fs::File, io::Read, path::Path};

use anyhow::Context;
use globset::{Glob, GlobMatcher};
use rusqlite::{Connection, params};
use walkdir::WalkDir;

use crate::config::Config;

const MAX_TEXT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
struct IgnorePattern {
    matcher: GlobMatcher,
    negated: bool,
    directory_only: bool,
    basename_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct IgnoreMatcher {
    patterns: Vec<IgnorePattern>,
}

impl IgnoreMatcher {
    pub fn new(patterns: &[String]) -> anyhow::Result<Self> {
        let patterns = patterns
            .iter()
            .filter(|line| {
                let line = line.trim();
                !line.is_empty() && !line.starts_with('#')
            })
            .map(|line| IgnorePattern::parse(line))
            .collect::<anyhow::Result<_>>()?;
        Ok(Self { patterns })
    }

    pub fn is_ignored(&self, relative_path: &Path, is_dir: bool) -> bool {
        if self.matches(relative_path, is_dir) {
            return true;
        }
        for ancestor in relative_path.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            if self.matches(ancestor, true) {
                return true;
            }
        }
        false
    }

    fn matches(&self, path: &Path, is_dir: bool) -> bool {
        let basename = path.file_name().map(Path::new);
        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.directory_only && !is_dir {
                continue;
            }
            let candidate = if pattern.basename_only {
                basename
            } else {
                Some(path)
            };
            if candidate.is_some_and(|candidate| pattern.matcher.is_match(candidate)) {
                ignored = !pattern.negated;
            }
        }
        ignored
    }
}

impl IgnorePattern {
    fn parse(line: &str) -> anyhow::Result<Self> {
        let mut pattern = line.trim();
        let negated = pattern.starts_with('!');
        if negated {
            pattern = &pattern[1..];
        }
        let directory_only = pattern.ends_with('/');
        pattern = pattern.trim_end_matches('/');
        let anchored = pattern.starts_with('/');
        pattern = pattern.strip_prefix('/').unwrap_or(pattern);
        let basename_only = !anchored && !pattern.contains('/');
        let matcher = Glob::new(pattern)
            .with_context(|| format!("invalid ignore pattern: {line}"))?
            .compile_matcher();
        Ok(Self {
            matcher,
            negated,
            directory_only,
            basename_only,
        })
    }
}

pub fn rebuild(connection: &mut Connection, config: &Config) -> anyhow::Result<usize> {
    let ignore = IgnoreMatcher::new(&config.ignore)?;
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM documents", [])?;
    let mut count = 0;

    for directory in &config.directories {
        let root = directory
            .canonicalize()
            .with_context(|| format!("failed to open search directory: {}", directory.display()))?;
        let entries = WalkDir::new(&root).follow_links(false).into_iter();
        for entry in entries.filter_entry(|entry| {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                return true;
            }
            let path = entry.path();
            let relative = path.strip_prefix(&root).unwrap_or(path);
            !ignore.is_ignored(relative, true)
        }) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    eprintln!("Skipping {error}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(&root).unwrap_or(path);
            if ignore.is_ignored(relative, false) {
                continue;
            }
            let content = read_text(path);
            transaction.execute(
                "INSERT INTO documents (path, content) VALUES (?1, ?2) \
                 ON CONFLICT(path) DO UPDATE SET content = excluded.content",
                params![path.to_string_lossy(), content],
            )?;
            count += 1;
        }
    }
    transaction.commit()?;
    Ok(count)
}

fn read_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_TEXT_BYTES || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::db::Database;

    #[test]
    fn matches_globs_directories_and_negations() {
        let matcher = IgnoreMatcher::new(&[
            "*.log".into(),
            "private/".into(),
            "!important.log".into(),
            "/root-only.txt".into(),
        ])
        .unwrap();

        assert!(matcher.is_ignored(Path::new("debug.log"), false));
        assert!(matcher.is_ignored(Path::new("nested/debug.log"), false));
        assert!(!matcher.is_ignored(Path::new("important.log"), false));
        assert!(matcher.is_ignored(Path::new("private/secret.txt"), false));
        assert!(matcher.is_ignored(Path::new("root-only.txt"), false));
        assert!(!matcher.is_ignored(Path::new("nested/root-only.txt"), false));
    }

    #[test]
    fn indexes_text_and_replaces_the_previous_index() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("note.txt"), "a searchable phrase").unwrap();
        fs::write(directory.path().join("ignored.log"), "hidden words").unwrap();
        fs::write(directory.path().join("binary.dat"), [0, 159, 146, 150]).unwrap();
        fs::create_dir(directory.path().join("private")).unwrap();
        fs::write(directory.path().join("private/secret.txt"), "hidden words").unwrap();
        let config = Config {
            directories: vec![directory.path().to_path_buf()],
            ignore: vec!["*.log".into(), "private/".into()],
        };
        let mut database = Database::open_in_memory().unwrap();

        assert_eq!(rebuild(&mut database.connection, &config).unwrap(), 2);
        assert_eq!(database.search("searchable", 100).unwrap().len(), 1);
        assert_eq!(database.search("binary.dat", 100).unwrap().len(), 1);
        assert!(database.search("hidden", 100).unwrap().is_empty());

        fs::remove_file(directory.path().join("note.txt")).unwrap();
        rebuild(&mut database.connection, &config).unwrap();
        assert!(database.search("searchable", 100).unwrap().is_empty());
    }
}
