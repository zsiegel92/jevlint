use std::path::{Path, PathBuf};

use anyhow::Context;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::config::Config;

pub struct FileMatcher {
    include: GlobSet,
    exclude: GlobSet,
}

impl FileMatcher {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            include: build_globs(&config.include)?,
            exclude: build_globs(&config.exclude)?,
        })
    }

    pub fn matches(&self, relative_path: &Path) -> bool {
        self.include.is_match(relative_path) && !self.exclude.is_match(relative_path)
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
        if matcher.matches(relative) {
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
