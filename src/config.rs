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
    pub exclude: Vec<String>,
    pub rules_dir: PathBuf,
    pub rule_sets: Vec<RuleSet>,
    pub system_prompt: PathBuf,
    pub cache_dir: PathBuf,
    pub typesafe_api_key_file: Option<PathBuf>,
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
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub files: Vec<String>,
    #[serde(default)]
    pub excluded_files: Vec<String>,
    pub rules: BTreeMap<String, Severity>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WatchConfig {
    pub debounce_milliseconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: vec!["target/**".into(), ".jevlint/**".into()],
            rules_dir: ".jevlint-rules".into(),
            rule_sets: Vec::new(),
            system_prompt: ".jevlint-system.md".into(),
            cache_dir: ".jevlint".into(),
            typesafe_api_key_file: None,
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
            !self.rule_sets.is_empty(),
            "config requires a [[rule_sets]] block"
        );
        if let Some(precondition) = &self.precondition {
            ensure!(
                !precondition.command.is_empty(),
                "precondition.command cannot be empty"
            );
        }
        for (index, rule_set) in self.rule_sets.iter().enumerate() {
            ensure!(
                !rule_set.files.is_empty(),
                "rule_sets[{index}].files cannot be empty"
            );
            ensure!(
                !rule_set.rules.is_empty(),
                "rule_sets[{index}].rules cannot be empty"
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

    pub async fn typesafe_api_key(&self, root: &Path) -> anyhow::Result<String> {
        if let Ok(value) = std::env::var("TYPESAFE_API_KEY")
            && !value.trim().is_empty()
        {
            return Ok(value.trim().to_owned());
        }
        let path = match &self.typesafe_api_key_file {
            Some(path) => resolve_key_path(root, path)?,
            None => default_key_path()?,
        };
        let value = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read TypeSafe API key from {}", path.display()))?;
        ensure!(
            !value.trim().is_empty(),
            "TypeSafe API key file is empty: {}",
            path.display()
        );
        Ok(value.trim().to_owned())
    }
}

fn resolve_key_path(root: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    if let Ok(suffix) = path.strip_prefix("~") {
        return Ok(home_directory()?.join(suffix));
    }
    Ok(root.join(path))
}

fn default_key_path() -> anyhow::Result<PathBuf> {
    Ok(home_directory()?.join(".config/jevlint/typesafe-api-key"))
}

fn home_directory() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; configure typesafe_api_key_file explicitly")
}
