use std::{net::IpAddr, path::Path};

use clap::{Parser, Subcommand};

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
        #[arg(long, default_value = "127.0.0.1")]
        listen: IpAddr,
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    Update,
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        let config = Config::load()?;
        let database_path = database_path()?;
        match self.command {
            Command::Start { listen, port } => {
                if !Database::exists(&database_path) {
                    update_database(&database_path, &config)?;
                }
                crate::web::serve(&database_path, listen, port).await?;
            }
            Command::Update => {
                update_database(&database_path, &config)?;
            }
        }
        Ok(())
    }
}

fn update_database(path: &Path, config: &Config) -> anyhow::Result<()> {
    println!("Updating database...");
    let mut database = Database::open(path)?;
    let count = index::rebuild(&mut database.connection, config)?;
    println!("Indexed {count} files.");
    Ok(())
}
