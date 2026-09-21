use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::Context;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::config::Config;

pub struct FileMatcher {
    include: GlobSet,
    exclude: GlobSet,
    overrides: Vec<OverrideMatcher>,
}

struct OverrideMatcher {
    files: GlobSet,
    excluded_files: GlobSet,
    rules: Vec<String>,
}

impl FileMatcher {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let overrides = config
            .overrides
            .iter()
            .map(|rule_override| {
                Ok(OverrideMatcher {
                    files: build_globs(&rule_override.files)?,
                    excluded_files: build_globs(&rule_override.excluded_files)?,
                    rules: rule_override.rules.clone(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            include: build_globs(&config.include)?,
            exclude: build_globs(&config.exclude)?,
            overrides,
        })
    }

    pub fn matches(&self, relative_path: &Path) -> bool {
        self.include.is_match(relative_path) && !self.exclude.is_match(relative_path)
    }

    pub fn is_lintable(&self, relative_path: &Path) -> bool {
        self.matches(relative_path)
            && (self.overrides.is_empty() || !self.rule_ids(relative_path).is_empty())
    }

    pub fn rule_ids<'a>(&'a self, relative_path: &Path) -> BTreeSet<&'a str> {
        self.overrides
            .iter()
            .filter(|rule_override| {
                rule_override.files.is_match(relative_path)
                    && !rule_override.excluded_files.is_match(relative_path)
            })
            .flat_map(|rule_override| rule_override.rules.iter().map(String::as_str))
            .collect()
    }

    pub fn uses_overrides(&self) -> bool {
        !self.overrides.is_empty()
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

    use crate::config::{Config, RuleOverride};

    use super::FileMatcher;

    #[test]
    fn selects_only_rules_from_matching_overrides() {
        let config = Config {
            include: vec!["**/*.py".into(), "**/*.ts".into()],
            overrides: vec![
                RuleOverride {
                    files: vec!["**/*.py".into()],
                    excluded_files: vec!["generated/**".into()],
                    rules: vec!["python-boundaries".into()],
                },
                RuleOverride {
                    files: vec!["**/*.ts".into()],
                    excluded_files: Vec::new(),
                    rules: vec!["typescript-boundaries".into()],
                },
                RuleOverride {
                    files: vec!["src/**".into()],
                    excluded_files: Vec::new(),
                    rules: vec!["shared-source-rule".into()],
                },
            ],
            ..Config::default()
        };
        let matcher = FileMatcher::new(&config).unwrap();

        assert_eq!(
            matcher
                .rule_ids(Path::new("src/service.py"))
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["python-boundaries", "shared-source-rule"]
        );
        assert_eq!(
            matcher
                .rule_ids(Path::new("src/client.ts"))
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["shared-source-rule", "typescript-boundaries"]
        );
        assert!(!matcher.is_lintable(Path::new("generated/models.py")));
        assert!(!matcher.is_lintable(Path::new("README.md")));
    }

    #[test]
    fn no_overrides_applies_all_rules_to_selected_files() {
        let matcher = FileMatcher::new(&Config::default()).unwrap();

        assert!(matcher.is_lintable(Path::new("src/main.rs")));
        assert!(!matcher.uses_overrides());
        assert!(matcher.rule_ids(Path::new("src/main.rs")).is_empty());
    }
}
