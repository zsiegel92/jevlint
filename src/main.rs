use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use clap::Parser;
use jevlint::{Config, Engine, files, jev::JevClient, report};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, default_value = ".jevlintrc.toml")]
    config: PathBuf,
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("jevlint: {error:#}");
        std::process::exit(2);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = absolute(&cli.config)?;
    let root = config_path
        .parent()
        .context("config path has no parent")?
        .to_owned();
    let config = Config::load(&config_path).await?;
    if cli.dry_run {
        let selected = files::discover(&root, &config)?;
        println!("jevlint: would lint {} files", selected.len());
        for path in selected {
            println!("{}", path.display());
        }
        return Ok(());
    }
    let provider = Arc::new(JevClient::from_env(
        config.model.clone(),
        config.request_timeout(),
    )?);
    let engine = Engine::new(root, config, provider);
    let result = engine.run().await?;
    report::print(&result, std::io::stdout(), std::io::stderr())?;
    if result.precondition_failure.is_some() || !result.violations.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
