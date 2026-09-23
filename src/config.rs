use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, ensure};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_rules_dir")]
    pub rules_dir: PathBuf,
    #[schemars(length(min = 1))]
    pub rule_sets: Vec<RuleSet>,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: PathBuf,
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
    #[serde(default)]
    pub typesafe_api_key_file: Option<PathBuf>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_concurrency")]
    pub concurrency: NonZeroUsize,
    #[serde(default = "default_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default)]
    pub line_detection: LineDetectionConfig,
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub precondition: Option<PreconditionConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LineDetectionConfig {
    pub enabled: bool,
    pub max_questions_per_request: NonZeroUsize,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreconditionConfig {
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    /// File globs relative to the config directory, such as **/*.ts.
    #[serde(rename = "match")]
    #[schemars(length(min = 1))]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    /// Rule IDs to minimum verdict confidence (0 to 1 inclusive).
    #[schemars(schema_with = "rule_thresholds_schema")]
    pub error: BTreeMap<String, f64>,
    #[serde(default)]
    /// Rule IDs to minimum verdict confidence (0 to 1 inclusive).
    #[schemars(schema_with = "rule_thresholds_schema")]
    pub warn: BTreeMap<String, f64>,
}

fn rule_thresholds_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "additionalProperties": {
            "type": "number",
            "minimum": 0,
            "maximum": 1
        }
    })
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WatchConfig {
    pub debounce_milliseconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: None,
            exclude: vec!["target/**".into(), ".jevlint/**".into()],
            rules_dir: default_rules_dir(),
            rule_sets: Vec::new(),
            system_prompt: default_system_prompt(),
            cache_dir: default_cache_dir(),
            typesafe_api_key_file: None,
            model: default_model(),
            concurrency: default_concurrency(),
            request_timeout_seconds: default_timeout(),
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
        let config: Self = serde_json::from_str(&text)
            .with_context(|| format!("invalid config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.rule_sets.is_empty(),
            "config requires at least one rule_sets entry"
        );
        if let Some(precondition) = &self.precondition {
            ensure!(
                !precondition.command.is_empty(),
                "precondition.command cannot be empty"
            );
        }
        for (index, rule_set) in self.rule_sets.iter().enumerate() {
            ensure!(
                !rule_set.patterns.is_empty(),
                "rule_sets[{index}].match cannot be empty"
            );
            ensure!(
                !rule_set.error.is_empty() || !rule_set.warn.is_empty(),
                "rule_sets[{index}] requires error or warn rules"
            );
            for (id, threshold) in &rule_set.error {
                ensure!(
                    *threshold < 0.0 || !rule_set.warn.get(id).is_some_and(|value| *value >= 0.0),
                    "rule_sets[{index}] declares {id:?} as both error and warn"
                );
            }
            for threshold in rule_set.error.values().chain(rule_set.warn.values()) {
                ensure!(
                    threshold.is_finite(),
                    "rule_sets[{index}] threshold must be finite"
                );
            }
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

fn default_rules_dir() -> PathBuf {
    ".jevlint-rules".into()
}
fn default_system_prompt() -> PathBuf {
    ".jevlint-system.md".into()
}
fn default_cache_dir() -> PathBuf {
    ".jevlint".into()
}
fn default_model() -> String {
    "jev-latest".into()
}
fn default_concurrency() -> NonZeroUsize {
    NonZeroUsize::new(5).expect("nonzero")
}
fn default_timeout() -> u64 {
    30
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

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn committed_schema_matches_config_type() {
        let generated = serde_json::to_value(schemars::schema_for!(Config)).unwrap();
        let committed: serde_json::Value =
            serde_json::from_str(include_str!("../schema/jevlint.schema.json")).unwrap();
        assert_eq!(generated, committed);
    }

    #[test]
    fn schema_bounds_rule_thresholds() {
        let schema = serde_json::to_value(schemars::schema_for!(Config)).unwrap();
        for severity in ["error", "warn"] {
            let threshold =
                &schema["$defs"]["RuleSet"]["properties"][severity]["additionalProperties"];
            assert_eq!(threshold["minimum"], 0);
            assert_eq!(threshold["maximum"], 1);
        }
    }

    #[test]
    fn json_rejects_unknown_fields_and_accepts_thresholds() {
        let config: Config =
            serde_json::from_str(r#"{"rule_sets":[{"match":["**/*.ts"],"warn":{"example":1.4}}]}"#)
                .unwrap();
        config.validate().unwrap();
        assert_eq!(config.rule_sets[0].warn["example"], 1.4);
        assert!(serde_json::from_str::<Config>(r#"{"rule_sets":[],"typo":true}"#).is_err());
    }
}
