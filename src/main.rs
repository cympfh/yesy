use clap::Parser;
use yesy::cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Cli::parse().run().await
}
