use std::{
    collections::BTreeSet,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::{
    Config, Engine,
    engine::{RunReport, Violation},
    files::FileMatcher,
    jev::JevClient,
    precondition::PreconditionFailure,
};

const PROTOCOL_VERSION: u8 = 1;

pub struct WatchOptions {
    pub snapshot_file: Option<PathBuf>,
    pub stdout: bool,
}

pub async fn run(config_path: PathBuf, options: WatchOptions) -> anyhow::Result<()> {
    let root = config_path
        .parent()
        .context("config path has no parent")?
        .to_owned();
    let mut config = Config::load(&config_path).await?;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = sender.send(event);
        },
        notify::Config::default(),
    )?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    let mut sequence = 1;
    emit_started(sequence, "initial", &[], &options)?;
    emit_snapshot(run_once(&root, &config, sequence).await, &options).await?;

    loop {
        let first = tokio::select! {
            event = receiver.recv() => match event {
                Some(event) => event,
                None => break,
            },
            signal = tokio::signal::ctrl_c() => {
                signal?;
                break;
            }
        };
        let mut paths = event_paths(first)?;
        let debounce = config.watch_debounce();
        while let Ok(Some(event)) = tokio::time::timeout(debounce, receiver.recv()).await {
            paths.extend(event_paths(event)?);
        }
        paths.sort();
        paths.dedup();
        if !is_relevant(&root, &config_path, &config, &options, &paths)? {
            continue;
        }

        sequence += 1;
        let changed_paths = relative_paths(&root, &paths);
        emit_started(sequence, "filesystem", &changed_paths, &options)?;
        match Config::load(&config_path).await {
            Ok(updated) => config = updated,
            Err(error) => {
                emit_snapshot(
                    Snapshot::error(&root, sequence, format!("{error:#}")),
                    &options,
                )
                .await?;
                continue;
            }
        }
        emit_snapshot(run_once(&root, &config, sequence).await, &options).await?;
    }
    drop(watcher);
    Ok(())
}

async fn run_once(root: &Path, config: &Config, sequence: u64) -> Snapshot {
    let result = async {
        let provider = Arc::new(JevClient::from_env(
            config.model.clone(),
            config.request_timeout(),
        )?);
        Engine::new(root.to_owned(), config.clone(), provider)
            .run()
            .await
    }
    .await;
    match result {
        Ok(report) => Snapshot::from_report(root, sequence, report),
        Err(error) => Snapshot::error(root, sequence, format!("{error:#}")),
    }
}

fn event_paths(event: notify::Result<Event>) -> anyhow::Result<Vec<PathBuf>> {
    Ok(event.context("filesystem watcher failed")?.paths)
}

fn is_relevant(
    root: &Path,
    config_path: &Path,
    config: &Config,
    options: &WatchOptions,
    paths: &[PathBuf],
) -> anyhow::Result<bool> {
    let matcher = FileMatcher::new(config)?;
    let rules = root.join(&config.rules_dir);
    let prompt = root.join(&config.system_prompt);
    let cache = root.join(&config.cache_dir);
    Ok(paths.iter().any(|path| {
        if options
            .snapshot_file
            .as_ref()
            .is_some_and(|file| path == file)
            || path.starts_with(&cache)
        {
            return false;
        }
        if path == config_path || path == &prompt || path.starts_with(&rules) {
            return true;
        }
        path.strip_prefix(root)
            .is_ok_and(|relative| matcher.matches(relative))
    }))
}

fn relative_paths(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn emit_started(
    sequence: u64,
    trigger: &'static str,
    changed_paths: &[String],
    options: &WatchOptions,
) -> anyhow::Result<()> {
    if options.stdout {
        write_json_line(&Started {
            schema_version: PROTOCOL_VERSION,
            kind: "run_started",
            sequence,
            trigger,
            changed_paths,
        })?;
    }
    Ok(())
}

async fn emit_snapshot(snapshot: Snapshot, options: &WatchOptions) -> anyhow::Result<()> {
    if options.stdout {
        write_json_line(&snapshot)?;
    }
    if let Some(path) = &options.snapshot_file {
        write_snapshot(path.clone(), snapshot).await?;
    }
    Ok(())
}

fn write_json_line(value: &impl Serialize) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

async fn write_snapshot(path: PathBuf, snapshot: Snapshot) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let parent = path.parent().context("snapshot path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, &snapshot)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path).map_err(|error| error.error)?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

#[derive(Serialize)]
struct Started<'a> {
    schema_version: u8,
    kind: &'static str,
    sequence: u64,
    trigger: &'static str,
    changed_paths: &'a [String],
}

#[derive(Serialize)]
struct Snapshot {
    schema_version: u8,
    kind: &'static str,
    sequence: u64,
    root: PathBuf,
    line_base: u8,
    status: Status,
    diagnostics: Vec<Diagnostic>,
    stats: Option<Stats>,
    precondition: Option<PreconditionStatus>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Clean,
    Violations,
    PreconditionFailed,
    Error,
}

#[derive(Serialize)]
struct Diagnostic {
    path: PathBuf,
    rule_id: String,
    severity: &'static str,
    confidence: f64,
    regions: Vec<Region>,
}

#[derive(Serialize)]
struct Region {
    start_line: usize,
    end_line: usize,
}

#[derive(Serialize)]
struct Stats {
    files_checked: usize,
    rules: usize,
    api_requests: usize,
    cache_hits: usize,
    violations: usize,
}

#[derive(Serialize)]
struct PreconditionStatus {
    command: Vec<String>,
    exit_code: Option<i32>,
    output: String,
    skipped_files: Vec<PathBuf>,
}

impl Snapshot {
    fn from_report(root: &Path, sequence: u64, report: RunReport) -> Self {
        let status = if report.precondition_failure.is_some() {
            Status::PreconditionFailed
        } else if report.violations.is_empty() {
            Status::Clean
        } else {
            Status::Violations
        };
        let stats = Stats {
            files_checked: report.files_checked,
            rules: report.rules,
            api_requests: report.api_requests,
            cache_hits: report.cache_hits,
            violations: report.violations.len(),
        };
        Self {
            schema_version: PROTOCOL_VERSION,
            kind: "snapshot",
            sequence,
            root: root.to_owned(),
            line_base: 1,
            status,
            diagnostics: report
                .violations
                .into_iter()
                .map(Diagnostic::from)
                .collect(),
            stats: Some(stats),
            precondition: report.precondition_failure.map(PreconditionStatus::from),
            error: None,
        }
    }

    fn error(root: &Path, sequence: u64, error: String) -> Self {
        Self {
            schema_version: PROTOCOL_VERSION,
            kind: "snapshot",
            sequence,
            root: root.to_owned(),
            line_base: 1,
            status: Status::Error,
            diagnostics: Vec::new(),
            stats: None,
            precondition: None,
            error: Some(error),
        }
    }
}

impl From<Violation> for Diagnostic {
    fn from(value: Violation) -> Self {
        Self {
            path: value.path,
            rule_id: value.rule_id,
            severity: "error",
            confidence: value.confidence,
            regions: value
                .regions
                .into_iter()
                .map(|region| Region {
                    start_line: region.start,
                    end_line: region.end,
                })
                .collect(),
        }
    }
}

impl From<PreconditionFailure> for PreconditionStatus {
    fn from(value: PreconditionFailure) -> Self {
        Self {
            command: value.command,
            exit_code: value.status,
            output: value.output,
            skipped_files: value.files,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomically_writes_a_machine_readable_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state/diagnostics.json");
        write_snapshot(
            path.clone(),
            Snapshot::error(directory.path(), 7, "temporary failure".into()),
        )
        .await
        .unwrap();

        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["kind"], "snapshot");
        assert_eq!(value["sequence"], 7);
        assert_eq!(value["status"], "error");
        assert_eq!(value["diagnostics"], serde_json::json!([]));
    }

    #[test]
    fn watches_sources_and_linter_inputs_but_not_cache_files() {
        let root = Path::new("/project");
        let config_path = root.join(".jevlintrc.toml");
        let config = Config {
            include: vec!["**/*.ts".into()],
            ..Config::default()
        };
        let options = WatchOptions {
            snapshot_file: None,
            stdout: true,
        };

        assert!(
            is_relevant(
                root,
                &config_path,
                &config,
                &options,
                &[root.join("src/app.ts")]
            )
            .unwrap()
        );
        assert!(
            is_relevant(
                root,
                &config_path,
                &config,
                &options,
                &[root.join(".jevlint-rules/security.md")]
            )
            .unwrap()
        );
        assert!(
            !is_relevant(
                root,
                &config_path,
                &config,
                &options,
                &[root.join(".jevlint/cache.redb")]
            )
            .unwrap()
        );
    }
}
