use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, ensure};
use serde::Deserialize;

use crate::rule::Severity;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub rules_dir: PathBuf,
    pub rule_severity: BTreeMap<String, Severity>,
    pub system_prompt: PathBuf,
    pub cache_dir: PathBuf,
    pub model: String,
    pub concurrency: NonZeroUsize,
    pub request_timeout_seconds: u64,
    pub line_detection: LineDetectionConfig,
    pub watch: WatchConfig,
    pub precondition: Option<PreconditionConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LineDetectionConfig {
    pub enabled: bool,
    pub max_questions_per_request: NonZeroUsize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreconditionConfig {
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WatchConfig {
    pub debounce_milliseconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            include: vec!["**/*.rs".into()],
            exclude: vec!["target/**".into(), ".jevlint/**".into()],
            rules_dir: ".jevlint-rules".into(),
            rule_severity: BTreeMap::new(),
            system_prompt: ".jevlint-system.md".into(),
            cache_dir: ".jevlint".into(),
            model: "jev-latest".into(),
            concurrency: NonZeroUsize::new(5).unwrap(),
            request_timeout_seconds: 30,
            line_detection: LineDetectionConfig::default(),
            watch: WatchConfig::default(),
            precondition: None,
        }
    }
}

impl Default for LineDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_questions_per_request: NonZeroUsize::new(200).unwrap(),
        }
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_milliseconds: 300,
        }
    }
}

impl Config {
    pub async fn load(path: &Path) -> anyhow::Result<Self> {
        let text = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.include.is_empty(),
            "config must include at least one file glob"
        );
        if let Some(precondition) = &self.precondition {
            ensure!(
                !precondition.command.is_empty(),
                "precondition.command cannot be empty"
            );
        }
        Ok(())
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_seconds)
    }

    pub fn watch_debounce(&self) -> Duration {
        Duration::from_millis(self.watch.debounce_milliseconds)
    }
}
