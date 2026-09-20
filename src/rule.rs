use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::hash;

#[derive(Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub source_path: PathBuf,
    pub markdown: String,
    pub hash: String,
    pub severity: Severity,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Error,
    Warning,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => formatter.write_str("error"),
            Self::Warning => formatter.write_str("warning"),
        }
    }
}

pub async fn load(
    directory: &Path,
    severities: &BTreeMap<String, Severity>,
) -> anyhow::Result<Vec<Rule>> {
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .with_context(|| format!("failed to read rules directory {}", directory.display()))?;
    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if entry.file_type().await?.is_file() && path.extension().is_some_and(|x| x == "md") {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        bail!("no .md lint rules found in {}", directory.display());
    }

    let mut rules = Vec::with_capacity(paths.len());
    for source_path in paths {
        let markdown = tokio::fs::read_to_string(&source_path).await?;
        if markdown.trim().is_empty() {
            bail!("lint rule is empty: {}", source_path.display());
        }
        let id = source_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        rules.push(Rule {
            severity: severities.get(&id).copied().unwrap_or_default(),
            id,
            hash: hash::bytes(&markdown),
            markdown,
            source_path,
        });
    }
    for id in severities.keys() {
        ensure!(
            rules.iter().any(|rule| &rule.id == id),
            "severity configured for unknown rule {id:?}"
        );
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn applies_severity_and_rejects_unknown_rule_ids() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("security.md"), "Check security.").unwrap();
        let severities = BTreeMap::from([("security".into(), Severity::Warning)]);
        let rules = load(directory.path(), &severities).await.unwrap();
        assert_eq!(rules[0].severity, Severity::Warning);

        let unknown = BTreeMap::from([("typo".into(), Severity::Error)]);
        assert!(load(directory.path(), &unknown).await.is_err());
    }
}
