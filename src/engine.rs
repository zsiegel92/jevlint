use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, ensure};
use serde::Serialize;
use tokio::task::JoinSet;

use crate::{
    cache::Cache,
    config::Config,
    files, hash,
    jev::{LineAnswer, LineQuestion, LintProvider, RuleAnswer, Verdict},
    precondition::{CommandPrecondition, Precondition, PreconditionFailure},
    rule::{self, Rule, Severity},
};

#[derive(Debug)]
pub struct RunReport {
    pub files_checked: usize,
    pub rules: usize,
    pub api_requests: usize,
    pub cache_hits: usize,
    pub violations: Vec<Violation>,
    pub precondition_failure: Option<PreconditionFailure>,
}

impl RunReport {
    pub fn count(&self, severity: Severity) -> usize {
        self.violations
            .iter()
            .filter(|violation| violation.severity == severity)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.violations
            .iter()
            .any(|violation| violation.severity == Severity::Error)
    }
}

#[derive(Debug)]
pub struct Violation {
    pub path: PathBuf,
    pub rule_id: String,
    pub confidence: f64,
    pub severity: Severity,
    pub regions: Vec<LineRegion>,
}

#[derive(Debug)]
pub struct LineRegion {
    pub start: usize,
    pub end: usize,
}

pub struct Engine {
    root: PathBuf,
    config: Config,
    provider: Arc<dyn LintProvider>,
    precondition: Option<Arc<dyn Precondition>>,
}

impl Engine {
    pub fn new(root: PathBuf, config: Config, provider: Arc<dyn LintProvider>) -> Self {
        let precondition = config
            .precondition
            .clone()
            .map(|value| Arc::new(CommandPrecondition::new(value)) as Arc<dyn Precondition>);
        Self {
            root,
            config,
            provider,
            precondition,
        }
    }

    pub fn selected_files(&self) -> anyhow::Result<Vec<PathBuf>> {
        files::discover(&self.root, &self.config)
    }

    pub async fn run(&self) -> anyhow::Result<RunReport> {
        let files = self.selected_files()?;
        let rules = Arc::new(
            rule::load(
                &self.root.join(&self.config.rules_dir),
                &self.config.rule_severity,
            )
            .await?,
        );
        validate_override_rules(&self.config, &rules)?;
        let matcher = files::FileMatcher::new(&self.config)?;
        let jobs = files
            .into_iter()
            .map(|path| {
                let selected = matcher.rule_ids(&path);
                let rule_indices = if matcher.uses_overrides() {
                    rules
                        .iter()
                        .enumerate()
                        .filter_map(|(index, rule)| {
                            selected.contains(rule.id.as_str()).then_some(index)
                        })
                        .collect()
                } else {
                    (0..rules.len()).collect()
                };
                FileJob { path, rule_indices }
            })
            .collect::<Vec<_>>();
        let precondition_files = jobs.iter().map(|job| job.path.clone()).collect::<Vec<_>>();
        if let Some(precondition) = &self.precondition
            && let Some(failure) = precondition.check(&self.root, &precondition_files).await?
        {
            return Ok(RunReport {
                files_checked: 0,
                rules: rules.len(),
                api_requests: 0,
                cache_hits: 0,
                violations: Vec::new(),
                precondition_failure: Some(failure),
            });
        }

        let prompt = tokio::fs::read_to_string(self.root.join(&self.config.system_prompt)).await?;
        let prompt_hash = hash::bytes(&prompt);
        let cache = Cache::open(&self.root.join(&self.config.cache_dir).join("cache.redb")).await?;
        let context = Arc::new(FileContext {
            root: self.root.clone(),
            model: self.config.model.clone(),
            prompt,
            prompt_hash,
            line_detection: self.config.line_detection.clone(),
            rules,
            cache,
            provider: Arc::clone(&self.provider),
        });

        let mut pending = jobs.into_iter();
        let mut tasks = JoinSet::new();
        let mut reports = Vec::new();
        for _ in 0..self.config.concurrency.get() {
            if let Some(job) = pending.next() {
                spawn_file(&mut tasks, Arc::clone(&context), job);
            }
        }
        while let Some(result) = tasks.join_next().await {
            reports.push(result??);
            if let Some(job) = pending.next() {
                spawn_file(&mut tasks, Arc::clone(&context), job);
            }
        }

        reports.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(RunReport {
            files_checked: reports.len(),
            rules: context.rules.len(),
            api_requests: reports.iter().map(|x| x.api_requests).sum(),
            cache_hits: reports.iter().map(|x| x.cache_hits).sum(),
            violations: reports.into_iter().flat_map(|x| x.violations).collect(),
            precondition_failure: None,
        })
    }
}

fn validate_override_rules(config: &Config, rules: &[Rule]) -> anyhow::Result<()> {
    let known = rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    for (override_index, rule_override) in config.overrides.iter().enumerate() {
        for rule_id in &rule_override.rules {
            ensure!(
                known.contains(rule_id.as_str()),
                "overrides[{override_index}] references unknown rule {rule_id:?}"
            );
        }
    }
    Ok(())
}

fn spawn_file(
    tasks: &mut JoinSet<anyhow::Result<FileReport>>,
    context: Arc<FileContext>,
    job: FileJob,
) {
    tasks.spawn(async move { lint_file(context, job).await });
}

struct FileJob {
    path: PathBuf,
    rule_indices: Vec<usize>,
}

struct FileContext {
    root: PathBuf,
    model: String,
    prompt: String,
    prompt_hash: String,
    line_detection: crate::config::LineDetectionConfig,
    rules: Arc<Vec<Rule>>,
    cache: Cache,
    provider: Arc<dyn LintProvider>,
}

struct FileReport {
    path: PathBuf,
    api_requests: usize,
    cache_hits: usize,
    violations: Vec<Violation>,
}

#[derive(Serialize)]
struct RuleCacheIdentity<'a> {
    path: &'a str,
    file_hash: &'a str,
    rule_hash: &'a str,
    prompt_hash: &'a str,
    model: &'a str,
    schema: &'static str,
}

#[derive(Serialize)]
struct LineCacheIdentity<'a> {
    path: &'a str,
    file_hash: &'a str,
    rule_hash: &'a str,
    prompt_hash: &'a str,
    model: &'a str,
    line: usize,
    schema: &'static str,
}

async fn lint_file(context: Arc<FileContext>, job: FileJob) -> anyhow::Result<FileReport> {
    let FileJob { path, rule_indices } = job;
    let rules = rule_indices
        .iter()
        .map(|&index| &context.rules[index])
        .collect::<Vec<_>>();
    let display_path = path.to_string_lossy().replace('\\', "/");
    let source = tokio::fs::read_to_string(context.root.join(&path))
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file_hash = hash::bytes(&source);
    let keys = rules
        .iter()
        .map(|rule| {
            hash::key(
                "rule-result",
                &RuleCacheIdentity {
                    path: &display_path,
                    file_hash: &file_hash,
                    rule_hash: &rule.hash,
                    prompt_hash: &context.prompt_hash,
                    model: &context.model,
                    schema: "verdict-v1",
                },
            )
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let cached: Vec<Option<RuleAnswer>> = context.cache.get_many(keys.clone()).await?;
    let mut answers = Vec::with_capacity(rules.len());
    let mut missing = Vec::new();
    let mut cache_hits = 0;
    for (index, answer) in cached.into_iter().enumerate() {
        if let Some(answer) = answer {
            cache_hits += 1;
            answers.push(answer);
        } else {
            missing.push(index);
        }
    }
    let mut api_requests = 0;
    if !missing.is_empty() {
        api_requests += 1;
        let requested = missing
            .iter()
            .map(|&index| (*rules[index]).clone())
            .collect::<Vec<_>>();
        let fresh = context
            .provider
            .lint(&context.prompt, &display_path, &source, &requested)
            .await?;
        ensure!(
            fresh.len() == missing.len(),
            "provider returned the wrong number of rule answers"
        );
        context
            .cache
            .put_many(
                missing
                    .iter()
                    .copied()
                    .zip(fresh.iter().cloned())
                    .map(|(index, answer)| (keys[index].clone(), answer))
                    .collect(),
            )
            .await?;
        answers.extend(fresh);
    }
    answers.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    let failed = answers
        .into_iter()
        .filter(|x| x.verdict == Verdict::Fail)
        .collect::<Vec<_>>();
    let locations = if context.line_detection.enabled && !failed.is_empty() {
        locate(
            &context,
            &display_path,
            &source,
            &file_hash,
            &failed,
            &mut api_requests,
            &mut cache_hits,
        )
        .await?
    } else {
        BTreeMap::new()
    };
    let violations = failed
        .into_iter()
        .map(|answer| Violation {
            path: path.clone(),
            rule_id: answer.rule_id.clone(),
            confidence: answer.confidence,
            severity: context
                .rules
                .iter()
                .find(|rule| rule.id == answer.rule_id)
                .expect("answer rule was requested")
                .severity,
            regions: regions(
                locations
                    .get(&answer.rule_id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            ),
        })
        .collect();
    Ok(FileReport {
        path,
        api_requests,
        cache_hits,
        violations,
    })
}

async fn locate(
    context: &FileContext,
    path: &str,
    source: &str,
    file_hash: &str,
    failed: &[RuleAnswer],
    api_requests: &mut usize,
    cache_hits: &mut usize,
) -> anyhow::Result<BTreeMap<String, Vec<usize>>> {
    let line_count = source.lines().count().max(1);
    let rules = failed
        .iter()
        .map(|answer| {
            context
                .rules
                .iter()
                .find(|rule| rule.id == answer.rule_id)
                .with_context(|| format!("unknown failed rule {}", answer.rule_id))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut entries = Vec::with_capacity(rules.len() * line_count);
    for rule in rules {
        for line in 1..=line_count {
            let key = hash::key(
                "line-result",
                &LineCacheIdentity {
                    path,
                    file_hash,
                    rule_hash: &rule.hash,
                    prompt_hash: &context.prompt_hash,
                    model: &context.model,
                    line,
                    schema: "line-noul-v1",
                },
            )?;
            entries.push((key, rule, line));
        }
    }
    let cached: Vec<Option<LineAnswer>> = context
        .cache
        .get_many(entries.iter().map(|x| x.0.clone()).collect())
        .await?;
    let mut answers = Vec::new();
    let mut missing = Vec::new();
    for (index, answer) in cached.into_iter().enumerate() {
        if let Some(answer) = answer {
            *cache_hits += 1;
            answers.push(answer);
        } else {
            missing.push(index);
        }
    }
    let numbered = if source.is_empty() {
        "1: ".to_owned()
    } else {
        source
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{}: {line}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for chunk in missing.chunks(context.line_detection.max_questions_per_request.get()) {
        *api_requests += 1;
        let questions = chunk
            .iter()
            .map(|&index| LineQuestion {
                rule: entries[index].1,
                line: entries[index].2,
            })
            .collect::<Vec<_>>();
        let fresh = context
            .provider
            .locate(&context.prompt, path, &numbered, &questions)
            .await?;
        ensure!(
            fresh.len() == chunk.len(),
            "provider returned the wrong number of line answers"
        );
        context
            .cache
            .put_many(
                chunk
                    .iter()
                    .copied()
                    .zip(fresh.iter().cloned())
                    .map(|(index, answer)| (entries[index].0.clone(), answer))
                    .collect(),
            )
            .await?;
        answers.extend(fresh);
    }
    let mut lines = BTreeMap::<String, Vec<usize>>::new();
    for answer in answers.into_iter().filter(|x| x.violated) {
        lines.entry(answer.rule_id).or_default().push(answer.line);
    }
    for values in lines.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    Ok(lines)
}

fn regions(lines: &[usize]) -> Vec<LineRegion> {
    let mut result = Vec::new();
    for &line in lines {
        match result.last_mut() {
            Some(LineRegion { end, .. }) if *end + 1 == line => *end = line,
            _ => result.push(LineRegion {
                start: line,
                end: line,
            }),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::rule::Severity;
    use async_trait::async_trait;

    struct FakeProvider {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LintProvider for FakeProvider {
        async fn lint(
            &self,
            _system_prompt: &str,
            _path: &str,
            _source: &str,
            rules: &[Rule],
        ) -> anyhow::Result<Vec<RuleAnswer>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(rules
                .iter()
                .map(|rule| RuleAnswer {
                    rule_id: rule.id.clone(),
                    verdict: Verdict::Fail,
                    confidence: 0.9,
                    resolved_model: "test".into(),
                })
                .collect())
        }

        async fn locate(
            &self,
            _system_prompt: &str,
            _path: &str,
            _source: &str,
            questions: &[LineQuestion<'_>],
        ) -> anyhow::Result<Vec<LineAnswer>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(questions
                .iter()
                .map(|question| LineAnswer {
                    rule_id: question.rule.id.clone(),
                    line: question.line,
                    violated: true,
                    confidence: 0.8,
                    resolved_model: "test".into(),
                })
                .collect())
        }
    }

    #[test]
    fn joins_contiguous_lines() {
        let got = regions(&[2, 3, 4, 7, 9, 10]);
        assert_eq!(
            got.iter().map(|x| (x.start, x.end)).collect::<Vec<_>>(),
            vec![(2, 4), (7, 7), (9, 10)]
        );
    }

    #[tokio::test]
    async fn reuses_verdict_and_line_cache() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join(".jevlint-rules")).unwrap();
        std::fs::write(project.path().join("sample.rs"), "unsafe {}\n").unwrap();
        std::fs::write(project.path().join(".jevlint-rules/safe.md"), "No unsafe.").unwrap();
        std::fs::write(project.path().join(".jevlint-system.md"), "Lint the file.").unwrap();
        let provider = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let config = Config {
            include: vec!["*.rs".into()],
            ..Config::default()
        };
        let mut config = config;
        config
            .rule_severity
            .insert("safe".into(), Severity::Warning);
        let engine = Engine::new(project.path().into(), config.clone(), provider.clone());
        let first = engine.run().await.unwrap();
        assert_eq!(first.api_requests, 2);
        assert_eq!(first.violations[0].severity, Severity::Warning);
        assert!(!first.has_errors());
        assert_eq!(first.violations[0].regions[0].start, 1);

        config.rule_severity.insert("safe".into(), Severity::Error);
        let second = Engine::new(project.path().into(), config, provider.clone())
            .run()
            .await
            .unwrap();
        assert_eq!(second.api_requests, 0);
        assert_eq!(second.cache_hits, 2);
        assert!(second.has_errors());
        assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    }
}
