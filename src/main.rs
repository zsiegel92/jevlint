use std::{
    fs::OpenOptions,
    io::Write,
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
    #[arg(long, global = true, default_value = ".jevlintrc.json")]
    config: PathBuf,
    #[arg(long)]
    dry_run: bool,
    /// Print a one-shot check as structured JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
    /// Ignore cached results for this check without changing the project's cache.
    #[arg(long)]
    force_fresh: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a starter JSON config, prompt, and rule in the current project.
    Init,
    /// Print the JSON Schema derived from the Rust config types.
    Schema {
        /// Write the schema to this path instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
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
    let status = match run().await {
        Ok(status) => status,
        Err(error) => {
            eprintln!("jevlint: {error:#}");
            2
        }
    };
    std::process::exit(status);
}

async fn run() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    let config_path = absolute(&cli.config)?;
    match cli.command {
        Some(Command::Init) => {
            ensure!(!cli.json, "--json is only valid for one-shot checks");
            ensure!(
                !cli.force_fresh,
                "--force-fresh is only valid for one-shot checks"
            );
            init(&config_path)?;
            Ok(0)
        }
        Some(Command::Schema { output }) => {
            ensure!(!cli.json, "--json is only valid for one-shot checks");
            ensure!(
                !cli.force_fresh,
                "--force-fresh is only valid for one-shot checks"
            );
            schema(output.as_deref())?;
            Ok(0)
        }
        Some(Command::Watch { output, no_stdout }) => {
            ensure!(!cli.json, "watch already emits JSON Lines");
            ensure!(
                !cli.force_fresh,
                "--force-fresh is only valid for one-shot checks"
            );
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
            .await?;
            Ok(0)
        }
        None => check(config_path, cli.dry_run, cli.json, cli.force_fresh).await,
    }
}

fn schema(output: Option<&Path>) -> anyhow::Result<()> {
    let value = serde_json::to_string_pretty(&schemars::schema_for!(Config))?;
    match output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, format!("{value}\n"))?;
        }
        None => println!("{value}"),
    }
    Ok(())
}

fn init(path: &Path) -> anyhow::Result<()> {
    let root = path.parent().context("config path has no parent")?;
    let mut config = Config::default();
    config.rule_sets = vec![jevlint::config::RuleSet {
        patterns: vec!["*.rs".into(), "**/*.rs".into()],
        exclude: Vec::new(),
        error: [("example-rule".into(), 0.8)].into(),
        warn: Default::default(),
    }];
    let schema_path = std::env::current_exe()?
        .parent()
        .and_then(Path::parent)
        .map(|parent| parent.join("share/jevlint/jevlint.schema.json"));
    if let Some(schema_path) = schema_path.filter(|path| path.is_file()) {
        config.schema = Some(
            url::Url::from_file_path(schema_path)
                .map_err(|_| anyhow::anyhow!("invalid installed schema path"))?
                .to_string(),
        );
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("cannot create {} (it may already exist)", path.display()))?;
    writeln!(file, "{}", serde_json::to_string_pretty(&config)?)?;
    let rules_dir = root.join(&config.rules_dir);
    std::fs::create_dir_all(&rules_dir)?;
    create_if_absent(
        &root.join(&config.system_prompt),
        "You are a conservative semantic code linter. Report only clear violations visible in the supplied file.\n",
    )?;
    create_if_absent(
        &rules_dir.join("example-rule.md"),
        "# Example rule\n\nDescribe the issue here.\n\n---\n\nWrite detailed instructions for Jev here.\n",
    )?;
    println!("Created {}", path.display());
    Ok(())
}

fn create_if_absent(path: &Path, content: &str) -> anyhow::Result<()> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes())?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn check(
    config_path: PathBuf,
    dry_run: bool,
    json: bool,
    force_fresh: bool,
) -> anyhow::Result<i32> {
    ensure!(!dry_run || !json, "--dry-run and --json cannot be combined");
    ensure!(
        !dry_run || !force_fresh,
        "--dry-run and --force-fresh cannot be combined"
    );
    let root = config_path
        .parent()
        .context("config path has no parent")?
        .to_owned();
    let mut config = Config::load(&config_path).await?;
    if dry_run {
        let selected = files::discover(&root, &config)?;
        println!("jevlint: would lint {} files", selected.len());
        for path in selected {
            println!("{}", path.display());
        }
        return Ok(0);
    }
    let temporary_cache = force_fresh.then(tempfile::tempdir).transpose()?;
    if let Some(directory) = &temporary_cache {
        config.cache_dir = directory.path().to_path_buf();
    }
    let api_key = config.typesafe_api_key(&root).await?;
    let provider = Arc::new(JevClient::new(
        config.model.clone(),
        config.request_timeout(),
        api_key,
    )?);
    let engine = Engine::new(root, config, provider);
    let result = engine.run().await?;
    if json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        report::print(&result, std::io::stdout(), std::io::stderr())?;
    }
    Ok(
        if result.precondition_failure.is_some() || result.has_errors() {
            1
        } else {
            0
        },
    )
}

fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
