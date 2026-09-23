use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use jevlint::{RunReport, engine::LineRegion, rule::Severity};
use serde_json::{Value, json};

const MARKER: &str = "JEVLINT_SMOKE_BAD";
const SOURCES: [&str; 6] = [
    "alpha.py",
    "beta.py",
    "alpha.ts",
    "beta.ts",
    "alpha.json",
    "beta.json",
];

fn main() -> Result<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let force_fresh = match arguments.as_slice() {
        [] => false,
        [option] if option == "--force-fresh" => true,
        _ => anyhow::bail!("usage: jevlint-smoke [--force-fresh]"),
    };
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("smoke-tests");
    let temporary = tempfile::tempdir()?;
    prepare(&fixture_root, temporary.path())?;
    let server = MockJev::start()?;
    let cli = std::env::current_exe()?.with_file_name("jevlint");
    ensure!(
        cli.is_file(),
        "build the jevlint binary before running smoke tests"
    );

    for fixture in ["clean", "violations"] {
        let root = temporary.path().join(fixture);
        let first = check(&cli, &root, &server, force_fresh)?;
        ensure!(
            first.files_checked == 6,
            "{fixture}: selected {} files, expected 6",
            first.files_checked
        );
        ensure!(
            first.precondition_failure.is_none(),
            "{fixture}: unexpected precondition failure"
        );
        ensure!(
            first.api_requests >= 6,
            "{fixture}: expected fresh verdict requests"
        );
        ensure!(
            first.cache_hits == 0,
            "{fixture}: first pass used cached results"
        );
        let expected = if fixture == "clean" {
            Vec::new()
        } else {
            vec![
                Issue::new("alpha.json", "json-no-smoke-marker", Severity::Error, 2, 2),
                Issue::new("alpha.py", "python-no-smoke-marker", Severity::Error, 2, 2),
                Issue::new(
                    "alpha.ts",
                    "typescript-no-smoke-marker",
                    Severity::Warning,
                    2,
                    3,
                ),
            ]
        };
        ensure!(
            issues(&first) == expected,
            "{fixture}: wrong issues: {:#?}",
            issues(&first)
        );

        let calls = server.calls.load(Ordering::Relaxed);
        let second = check(&cli, &root, &server, force_fresh)?;
        ensure!(
            issues(&second) == expected,
            "{fixture}: cached issues changed"
        );
        if force_fresh {
            ensure!(
                second.api_requests == first.api_requests,
                "{fixture}: fresh pass made a different number of requests"
            );
            ensure!(
                second.cache_hits == 0,
                "{fixture}: fresh pass used cached results"
            );
            ensure!(
                server.calls.load(Ordering::Relaxed) > calls,
                "{fixture}: fresh pass did not reach provider"
            );
        } else {
            ensure!(
                second.api_requests == 0,
                "{fixture}: second run made API requests"
            );
            ensure!(
                server.calls.load(Ordering::Relaxed) == calls,
                "{fixture}: second run reached provider"
            );
            ensure!(
                second.cache_hits > 0,
                "{fixture}: second run did not use cache"
            );
        }
        println!(
            "{fixture}: {} files, {} issues; second pass {}",
            first.files_checked,
            expected.len(),
            if force_fresh {
                "forced fresh"
            } else {
                "fully cached"
            }
        );
        if fixture == "clean" {
            fs::write(
                root.join("alpha.py"),
                "def double(value: int) -> int:\n    return value * 2  # JEVLINT_SMOKE_BAD\n",
            )?;
            let changed = check(&cli, &root, &server, force_fresh)?;
            ensure!(
                issues(&changed)
                    == vec![Issue::new(
                        "alpha.py",
                        "python-no-smoke-marker",
                        Severity::Error,
                        2,
                        2
                    )],
                "editing one file did not refresh its diagnostic"
            );
            if force_fresh {
                ensure!(
                    changed.api_requests == 7 && changed.cache_hits == 0,
                    "fresh edit pass used cached results"
                );
                println!("file edit: all six files rerun fresh");
            } else {
                ensure!(
                    changed.api_requests == 2 && changed.cache_hits >= 5,
                    "editing one file should reuse the other five verdicts"
                );
                println!("file edit: one changed file rerun; five other verdicts reused");
            }
        }
    }

    check_cli_stream(&cli, &temporary.path().join("violations"), &server)?;
    check_watch_output(&cli, &temporary.path().join("violations"), &server, false)?;
    check_watch_output(&cli, &temporary.path().join("violations"), &server, true)?;

    let root = temporary.path().join("clean");
    let config_path = root.join("jevlint.smoke.jsonc");
    let mut config: Value = jsonc_parser::parse_to_serde_value(
        &fs::read_to_string(&config_path)?,
        &Default::default(),
    )?;
    config["precondition"] = json!({ "command": ["sh", "-c", "exit 1"] });
    fs::write(&config_path, serde_json::to_vec_pretty(&config)?)?;
    let calls = server.calls.load(Ordering::Relaxed);
    let skipped = check(&cli, &root, &server, force_fresh)?;
    let failure = skipped
        .precondition_failure
        .context("failed precondition was not reported")?;
    ensure!(
        failure.files.len() == 6,
        "precondition did not skip all six files"
    );
    ensure!(
        skipped.files_checked == 0 && skipped.api_requests == 0,
        "precondition did not stop linting"
    );
    ensure!(
        server.calls.load(Ordering::Relaxed) == calls,
        "precondition still reached provider"
    );
    println!("precondition: six files skipped without provider calls");
    Ok(())
}

fn check_cli_stream(cli: &Path, root: &Path, server: &MockJev) -> Result<()> {
    let output = Command::new(cli)
        .arg("--config")
        .arg(root.join("jevlint.smoke.jsonc"))
        .arg("--stream")
        .env("TYPESAFE_API_KEY", "smoke-key")
        .env("TYPESAFE_BASE_URL", &server.url)
        .output()?;
    ensure!(
        output.status.code() == Some(1),
        "streamed CLI exit status changed"
    );
    let text = String::from_utf8(output.stdout)?;
    ensure!(
        text.contains("alpha.py:2:"),
        "streamed CLI omitted a finding"
    );
    ensure!(
        text.contains("jevlint: 6 files"),
        "streamed CLI omitted its summary"
    );
    println!("CLI stream: findings and final summary received");
    Ok(())
}

fn check_watch_output(cli: &Path, root: &Path, server: &MockJev, stream: bool) -> Result<()> {
    let mut command = Command::new(cli);
    command
        .arg("watch")
        .arg("--config")
        .arg(root.join("jevlint.smoke.jsonc"))
        .env("TYPESAFE_API_KEY", "smoke-key")
        .env("TYPESAFE_BASE_URL", &server.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if stream {
        command.arg("--stream");
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().context("watch stdout was not piped")?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let result = (|| -> Result<()> {
        let mut started = false;
        let mut completed = BTreeSet::new();
        loop {
            let line = receiver.recv_timeout(Duration::from_secs(30))??;
            let value: Value = serde_json::from_str(&line)?;
            match value["kind"].as_str() {
                Some("run_started") => started = true,
                Some("file_result") => {
                    ensure!(started, "file result arrived before run_started");
                    completed.insert(
                        value["path"]
                            .as_str()
                            .context("file result has no path")?
                            .to_owned(),
                    );
                }
                Some("snapshot") => {
                    ensure!(
                        completed.len() == if stream { 6 } else { 0 },
                        "watch emitted the wrong number of file results"
                    );
                    ensure!(
                        value["diagnostics"]
                            .as_array()
                            .is_some_and(|items| items.len() == 3),
                        "final snapshot has the wrong diagnostics"
                    );
                    break;
                }
                other => anyhow::bail!("unexpected watch message kind {other:?}"),
            }
        }
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result?;
    println!(
        "watch {}: {} file results before final snapshot",
        if stream { "stream" } else { "default" },
        if stream { 6 } else { 0 }
    );
    Ok(())
}

fn prepare(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination.join("rules"))?;
    fs::copy(source.join("system.md"), destination.join("system.md"))?;
    for rule in [
        "python-no-smoke-marker.md",
        "typescript-no-smoke-marker.md",
        "json-no-smoke-marker.md",
    ] {
        fs::copy(
            source.join("rules").join(rule),
            destination.join("rules").join(rule),
        )?;
    }
    for fixture in ["clean", "violations"] {
        let target = destination.join(fixture);
        fs::create_dir(&target)?;
        for name in SOURCES.into_iter().chain(["jevlint.smoke.jsonc"]) {
            fs::copy(source.join(fixture).join(name), target.join(name))?;
        }
    }
    Ok(())
}

fn check(cli: &Path, root: &Path, server: &MockJev, force_fresh: bool) -> Result<RunReport> {
    let mut command = Command::new(cli);
    command
        .arg("--config")
        .arg(root.join("jevlint.smoke.jsonc"))
        .arg("--json");
    if force_fresh {
        command.arg("--force-fresh");
    }
    let output = command
        .env("TYPESAFE_API_KEY", "smoke-key")
        .env("TYPESAFE_BASE_URL", &server.url)
        .output()
        .with_context(|| format!("failed to launch {}", cli.display()))?;
    ensure!(
        matches!(output.status.code(), Some(0 | 1)),
        "jevlint exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RunReport = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "invalid CLI JSON: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })?;
    ensure!(
        output.status.code()
            == Some(
                if report.has_errors() || report.precondition_failure.is_some() {
                    1
                } else {
                    0
                }
            ),
        "exit status disagrees with diagnostics"
    );
    Ok(report)
}

#[derive(Debug, PartialEq, Eq)]
struct Issue {
    path: PathBuf,
    rule: String,
    severity: Severity,
    regions: Vec<LineRegion>,
}

impl Issue {
    fn new(path: &str, rule: &str, severity: Severity, start: usize, end: usize) -> Self {
        Self {
            path: path.into(),
            rule: rule.into(),
            severity,
            regions: vec![LineRegion { start, end }],
        }
    }
}

fn issues(report: &RunReport) -> Vec<Issue> {
    let mut result = report
        .violations
        .iter()
        .map(|violation| Issue {
            path: violation.path.clone(),
            rule: violation.rule_id.clone(),
            severity: violation.severity,
            regions: violation
                .regions
                .iter()
                .map(|region| LineRegion {
                    start: region.start,
                    end: region.end,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| a.path.cmp(&b.path).then(a.rule.cmp(&b.rule)));
    result
}

struct MockJev {
    url: String,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockJev {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_calls = Arc::clone(&calls);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = serve(stream, &thread_calls) {
                            eprintln!("mock Jev request failed: {error:#}");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("mock Jev listener failed: {error}"),
                }
            }
        });
        Ok(Self {
            url,
            calls,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for MockJev {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: TcpStream, calls: &AtomicUsize) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    ensure!(
        request_line.starts_with("POST /v1/systemone "),
        "unexpected request: {request_line}"
    );
    let mut length = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let mut body = vec![0; length.context("request has no content length")?];
    reader.read_exact(&mut body)?;
    let request: Value = serde_json::from_slice(&body)?;
    let questions = request["questions"]
        .as_object()
        .context("missing questions")?;
    let source = request["state"]["file"]["source"].as_str();
    let mut answers = BTreeMap::new();
    for (id, question) in questions {
        let answer = if let Some(source) = source {
            json!({ "choice": if source.contains(MARKER) { "fail" } else { "pass" }, "confidence": 0.99 })
        } else {
            let instructions = question["instructions"]
                .as_str()
                .context("missing line instructions")?;
            let line = instructions
                .split(" (verbatim):\n")
                .nth(1)
                .context("missing target line")?
                .lines()
                .next()
                .context("empty target line")?;
            json!({ "noul": if line.contains(MARKER) { 0.99 } else { 0.01 } })
        };
        answers.insert(id, answer);
    }
    calls.fetch_add(1, Ordering::Relaxed);
    let response = serde_json::to_vec(&json!({ "model": "smoke-model", "answers": answers }))?;
    let mut stream = reader.into_inner();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    )?;
    stream.write_all(&response)?;
    Ok(())
}
