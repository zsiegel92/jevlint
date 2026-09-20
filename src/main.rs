use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, ensure};
use clap::{Parser, Subcommand};
use jevlint::{
    Config, Engine, files,
    jev::JevClient,
    report,
    watch::{self, WatchOptions},
};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, global = true, default_value = ".jevlintrc.toml")]
    config: PathBuf,
    #[arg(long)]
    dry_run: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Watch the project and emit complete diagnostic snapshots as JSON Lines.
    Watch {
        /// Atomically replace this file with the latest JSON snapshot.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Write snapshots only to --output, not standard output.
        #[arg(long)]
        no_stdout: bool,
    },
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
    match cli.command {
        Some(Command::Watch { output, no_stdout }) => {
            ensure!(!cli.dry_run, "--dry-run cannot be used with watch");
            ensure!(
                output.is_some() || !no_stdout,
                "watch needs stdout or --output"
            );
            watch::run(
                config_path,
                WatchOptions {
                    snapshot_file: output.map(|path| absolute(&path)).transpose()?,
                    stdout: !no_stdout,
                },
            )
            .await
        }
        None => check(config_path, cli.dry_run).await,
    }
}

async fn check(config_path: PathBuf, dry_run: bool) -> anyhow::Result<()> {
    let root = config_path
        .parent()
        .context("config path has no parent")?
        .to_owned();
    let config = Config::load(&config_path).await?;
    if dry_run {
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
    if result.precondition_failure.is_some() || result.has_errors() {
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
