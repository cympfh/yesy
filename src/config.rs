use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub directories: Vec<PathBuf>,
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let path = config_path()?;
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Self::from_toml(&source, base)
            .with_context(|| format!("failed to parse config: {}", path.display()))
    }

    pub fn from_toml(source: &str, base: &Path) -> anyhow::Result<Self> {
        let mut config: Self = toml::from_str(source)?;
        if config.directories.is_empty() {
            bail!("directories must contain at least one path");
        }
        for directory in &mut config.directories {
            *directory = expand_path(directory, base);
        }
        Ok(config)
    }
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join("yesy.toml"))
        .context("could not determine the config directory")
}

pub fn database_path() -> anyhow::Result<PathBuf> {
    dirs::data_dir()
        .map(|path| path.join("yesy").join("yesy.db"))
        .context("could not determine the data directory")
}

fn expand_path(path: &Path, base: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_directories_and_ignore_patterns() {
        let config = Config::from_toml(
            r#"
            directories = ["docs", "/var/log"]
            ignore = ["*.log", ".git/"]
            "#,
            Path::new("/config"),
        )
        .unwrap();

        assert_eq!(
            config.directories,
            vec![PathBuf::from("/config/docs"), PathBuf::from("/var/log")]
        );
        assert_eq!(config.ignore, vec!["*.log", ".git/"]);
    }

    #[test]
    fn rejects_an_empty_directory_list() {
        let error = Config::from_toml("directories = []", Path::new(".")).unwrap_err();
        assert!(error.to_string().contains("at least one"));
    }
}
