use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use globset::{Glob, GlobMatcher};
use rusqlite::{Connection, params};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified_ns: Option<i64>,
    size: i64,
}

pub fn rebuild(connection: &mut Connection, config: &Config) -> anyhow::Result<usize> {
    let ignore = IgnoreMatcher::new(&config.ignore)?;
    let mut existing = HashMap::new();
    {
        let mut statement = connection.prepare("SELECT path, modified_ns, size FROM documents")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                FileStamp {
                    modified_ns: row.get(1)?,
                    size: row.get(2)?,
                },
            ))
        })?;
        for row in rows {
            let (path, stamp) = row?;
            existing.insert(path, stamp);
        }
    }
    let transaction = connection.transaction()?;
    let mut seen = HashSet::new();
    let mut count = 0;

    let (result_sender, result_receiver) = std::sync::mpsc::channel();
    for directory in &config.directories {
        let root = directory
            .canonicalize()
            .with_context(|| format!("failed to open search directory: {}", directory.display()))?;
        ignore::WalkBuilder::new(&root)
            .hidden(false)
            .parents(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .follow_links(false)
            .build_parallel()
            .run(|| {
                let results = result_sender.clone();
                let ignore = &ignore;
                let existing = &existing;
                let root = &root;
                Box::new(move |entry| {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            eprintln!("Skipping {error}");
                            return ignore::WalkState::Continue;
                        }
                    };
                    let path = entry.path();
                    let relative = path.strip_prefix(root).unwrap_or(path);
                    let Some(file_type) = entry.file_type() else {
                        return ignore::WalkState::Continue;
                    };
                    if file_type.is_dir() {
                        if entry.depth() > 0 && ignore.is_ignored(relative, true) {
                            return ignore::WalkState::Skip;
                        }
                        return ignore::WalkState::Continue;
                    }
                    if !file_type.is_file() || ignore.is_ignored(relative, false) {
                        return ignore::WalkState::Continue;
                    }
                    if results
                        .send(inspect_file(path.to_path_buf(), existing))
                        .is_err()
                    {
                        return ignore::WalkState::Quit;
                    }
                    ignore::WalkState::Continue
                })
            });
    }
    drop(result_sender);
    for (path, update) in result_receiver {
        seen.insert(path.clone());
        count += 1;
        let Some((content, stamp)) = update else {
            continue;
        };
        transaction.execute(
            "INSERT INTO documents (path, content, modified_ns, size) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(path) DO UPDATE SET \
             content = excluded.content, \
             modified_ns = excluded.modified_ns, \
             size = excluded.size",
            params![path, content, stamp.modified_ns, stamp.size],
        )?;
    }
    for path in existing.keys() {
        if !seen.contains(path) {
            transaction.execute("DELETE FROM documents WHERE path = ?1", params![path])?;
        }
    }
    transaction.commit()?;
    Ok(count)
}

fn inspect_file(
    path: std::path::PathBuf,
    existing: &HashMap<String, FileStamp>,
) -> (String, Option<(Option<String>, FileStamp)>) {
    let path_string = path.to_string_lossy().into_owned();
    let stamp = match std::fs::metadata(&path) {
        Ok(metadata) => FileStamp {
            modified_ns: metadata.modified().ok().and_then(modified_nanoseconds),
            size: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
        },
        Err(_) => FileStamp {
            modified_ns: None,
            size: 0,
        },
    };
    let update = (existing.get(&path_string) != Some(&stamp)).then(|| (read_text(&path), stamp));
    (path_string, update)
}

fn modified_nanoseconds(time: SystemTime) -> Option<i64> {
    let nanoseconds = time.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some(nanoseconds.min(i64::MAX as u128) as i64)
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
        fs::write(directory.path().join(".hidden.txt"), "visible dotfile").unwrap();
        fs::create_dir(directory.path().join("private")).unwrap();
        fs::write(directory.path().join("private/secret.txt"), "hidden words").unwrap();
        let config = Config {
            directories: vec![directory.path().to_path_buf()],
            ignore: vec!["*.log".into(), "private/".into()],
        };
        let mut database = Database::open_in_memory().unwrap();

        assert_eq!(rebuild(&mut database.connection, &config).unwrap(), 3);
        assert_eq!(database.search("searchable", 100).unwrap().len(), 1);
        assert_eq!(database.search("binary.dat", 100).unwrap().len(), 1);
        assert_eq!(database.search("visible dotfile", 100).unwrap().len(), 1);
        assert!(database.search("hidden words", 100).unwrap().is_empty());

        let changes = database.connection.total_changes();
        assert_eq!(rebuild(&mut database.connection, &config).unwrap(), 3);
        assert_eq!(database.connection.total_changes(), changes);

        fs::write(
            directory.path().join("note.txt"),
            "a changed searchable phrase",
        )
        .unwrap();
        rebuild(&mut database.connection, &config).unwrap();
        assert_eq!(database.search("changed searchable", 100).unwrap().len(), 1);

        fs::remove_file(directory.path().join("note.txt")).unwrap();
        rebuild(&mut database.connection, &config).unwrap();
        assert!(database.search("searchable", 100).unwrap().is_empty());
    }
}
