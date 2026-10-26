# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-26

### Added

- CLI search with one or more search words and one path per output line
- Lazy image previews and previews of the first image among the three filename-sorted ZIP entries
- Deduplicated MRU search history with up to 30 links on the top page
- Clipboard controls and Windows file links for search result paths
- Search URLs synchronized with `/?q=...`
- SVG favicon and GitHub Actions CI
- Access logs with HTTP method, URI, status code, and elapsed time for every Web request

### Changed

- Listen on `0.0.0.0` by default
- Prefer Windows drive paths for `/mnt/c`, `/mnt/d`, `/mnt/e`, and `/mnt/f`
- Install directly from GitHub in the setup instructions
- Skip unchanged files and scan directories in parallel during `update`

(git commit: 2ba2e7c)

## [0.1.0] - 2026-10-26

### Added

- Local file indexing with SQLite FTS5
- CLI commands for updating the index and starting the Web server
- Web search page and JSON API
- WSL path conversion

(git commit: cef8415)
