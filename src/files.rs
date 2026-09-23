use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::{config::Config, rule::Severity};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RuleSetting {
    pub severity: Severity,
    pub threshold: f64,
}

pub struct FileMatcher {
    exclude: GlobSet,
    rule_sets: Vec<RuleSetMatcher>,
}

struct RuleSetMatcher {
    files: GlobSet,
    excluded_files: GlobSet,
    rules: BTreeMap<String, RuleSetting>,
}

impl FileMatcher {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let rule_sets = config
            .rule_sets
            .iter()
            .map(|rule_set| {
                let rules = rule_set
                    .error
                    .iter()
                    .map(|(id, threshold)| {
                        (
                            id.clone(),
                            RuleSetting {
                                severity: Severity::Error,
                                threshold: threshold.min(1.0),
                            },
                        )
                    })
                    .chain(rule_set.warn.iter().map(|(id, threshold)| {
                        (
                            id.clone(),
                            RuleSetting {
                                severity: Severity::Warning,
                                threshold: threshold.min(1.0),
                            },
                        )
                    }))
                    .filter(|(_, setting)| setting.threshold >= 0.0)
                    .collect();
                Ok(RuleSetMatcher {
                    files: build_globs(&rule_set.patterns)?,
                    excluded_files: build_globs(&rule_set.exclude)?,
                    rules,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            exclude: build_globs(&config.exclude)?,
            rule_sets,
        })
    }

    pub fn is_lintable(&self, relative_path: &Path) -> bool {
        !self.exclude.is_match(relative_path) && !self.rules_for(relative_path).is_empty()
    }

    pub fn rules_for<'a>(&'a self, relative_path: &Path) -> BTreeMap<&'a str, RuleSetting> {
        if self.exclude.is_match(relative_path) {
            return BTreeMap::new();
        }
        self.rule_sets
            .iter()
            .filter(|rule_set| {
                rule_set.files.is_match(relative_path)
                    && !rule_set.excluded_files.is_match(relative_path)
            })
            .flat_map(|rule_set| {
                rule_set
                    .rules
                    .iter()
                    .map(|(id, severity)| (id.as_str(), *severity))
            })
            .collect()
    }
}

pub fn discover(root: &Path, config: &Config) -> anyhow::Result<Vec<PathBuf>> {
    let matcher = FileMatcher::new(config)?;
    let mut files = Vec::new();
    for entry in WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .build()
    {
        let entry = entry.context("failed while walking project files")?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        if matcher.is_lintable(relative) {
            files.push(relative.to_path_buf());
        }
    }
    files.sort();
    Ok(files)
}

fn build_globs(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("invalid glob {pattern:?}"))?);
    }
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use std::collections::BTreeMap;

    use crate::{
        config::{Config, RuleSet},
        rule::Severity,
    };

    use super::{FileMatcher, RuleSetting};

    #[test]
    fn selects_rules_and_severities_from_matching_sets() {
        let config = Config {
            rule_sets: vec![
                RuleSet {
                    patterns: vec!["**/*.py".into()],
                    exclude: vec!["generated/**".into()],
                    error: BTreeMap::from([("python-boundaries".into(), 0.7)]),
                    warn: BTreeMap::new(),
                },
                RuleSet {
                    patterns: vec!["**/*.ts".into()],
                    exclude: Vec::new(),
                    error: BTreeMap::new(),
                    warn: BTreeMap::from([("typescript-boundaries".into(), 0.8)]),
                },
                RuleSet {
                    patterns: vec!["src/**".into()],
                    exclude: Vec::new(),
                    error: BTreeMap::from([("shared-source-rule".into(), 1.5)]),
                    warn: BTreeMap::new(),
                },
            ],
            ..Config::default()
        };
        let matcher = FileMatcher::new(&config).unwrap();

        assert_eq!(
            matcher
                .rules_for(Path::new("src/service.py"))
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                (
                    "python-boundaries",
                    RuleSetting {
                        severity: Severity::Error,
                        threshold: 0.7
                    }
                ),
                (
                    "shared-source-rule",
                    RuleSetting {
                        severity: Severity::Error,
                        threshold: 1.0
                    }
                )
            ]
        );
        assert_eq!(
            matcher
                .rules_for(Path::new("src/client.ts"))
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                (
                    "shared-source-rule",
                    RuleSetting {
                        severity: Severity::Error,
                        threshold: 1.0
                    }
                ),
                (
                    "typescript-boundaries",
                    RuleSetting {
                        severity: Severity::Warning,
                        threshold: 0.8
                    }
                )
            ]
        );
        assert!(!matcher.is_lintable(Path::new("generated/models.py")));
        assert!(!matcher.is_lintable(Path::new("README.md")));
    }

    #[test]
    fn no_rule_sets_selects_nothing() {
        let matcher = FileMatcher::new(&Config::default()).unwrap();

        assert!(!matcher.is_lintable(Path::new("src/main.rs")));
        assert!(matcher.rules_for(Path::new("src/main.rs")).is_empty());
    }

    #[test]
    fn negative_threshold_disables_a_rule() {
        let config = Config {
            rule_sets: vec![RuleSet {
                patterns: vec!["*.ts".into()],
                exclude: Vec::new(),
                error: BTreeMap::from([("disabled".into(), -0.1)]),
                warn: BTreeMap::new(),
            }],
            ..Config::default()
        };
        assert!(
            !FileMatcher::new(&config)
                .unwrap()
                .is_lintable(Path::new("app.ts"))
        );
    }
}
