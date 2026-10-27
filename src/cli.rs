use std::{net::IpAddr, path::Path};

use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{
    config::{Config, database_path},
    db::Database,
    index,
};

#[derive(Debug, Parser)]
#[command(name = "yesy", about = "Search local file names and text contents")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Start {
        #[arg(long, default_value = "0.0.0.0")]
        listen: IpAddr,
        #[arg(long, default_value_t = 8080)]
        port: u16,
        #[arg(long)]
        update: bool,
    },
    Update,
    Search {
        #[arg(required = true)]
        query: Vec<String>,
    },
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        let config = Config::load()?;
        let database_path = database_path()?;
        match self.command {
            Command::Start {
                listen,
                port,
                update,
            } => {
                if update || !Database::exists(&database_path) {
                    update_database(&database_path, &config)?;
                }
                crate::web::serve(&database_path, listen, port).await?;
            }
            Command::Update => {
                update_database(&database_path, &config)?;
            }
            Command::Search { query } => {
                if !Database::exists(&database_path) {
                    update_database(&database_path, &config)?;
                }
                let database = Database::open(&database_path)?;
                for result in database.search(&query.join(" "), 100)? {
                    println!("{}", result.path);
                }
            }
        }
        Ok(())
    }
}

fn update_database(path: &Path, config: &Config) -> anyhow::Result<()> {
    let mut database = Database::open(path)?;
    let progress = ProgressBar::new(0);
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {msg} [{bar:40.cyan/blue}] {pos}/{len} files ({eta})",
        )?
        .progress_chars("=>-"),
    );
    progress.set_message("Updating database");
    let count = index::rebuild_with_progress(&mut database.connection, config, |done, total| {
        progress.set_length(total as u64);
        progress.set_position(done as u64);
    })?;
    progress.finish_and_clear();
    println!("Indexed {count} files.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_search_words() {
        let cli = Cli::try_parse_from(["yesy", "search", "local", "search"]).unwrap();
        let Command::Search { query } = cli.command else {
            panic!("expected search command");
        };
        assert_eq!(query, ["local", "search"]);
    }

    #[test]
    fn requires_a_search_word() {
        assert!(Cli::try_parse_from(["yesy", "search"]).is_err());
    }

    #[test]
    fn parses_start_with_update() {
        let cli = Cli::try_parse_from(["yesy", "start", "--update"]).unwrap();
        let Command::Start { update, .. } = cli.command else {
            panic!("expected start command");
        };
        assert!(update);
    }
}
