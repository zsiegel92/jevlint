use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::hash;

#[derive(Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub source_path: PathBuf,
    pub message: String,
    pub markdown: String,
    pub hash: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
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

pub async fn load(directory: &Path) -> anyhow::Result<Vec<Rule>> {
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
            id,
            hash: hash::bytes(&markdown),
            message: message(&markdown),
            markdown,
            source_path,
        });
    }
    Ok(rules)
}

fn message(markdown: &str) -> String {
    let section = markdown
        .lines()
        .take_while(|line| !is_horizontal_rule(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    if section.is_empty() {
        markdown.trim().to_owned()
    } else {
        section
    }
}

fn is_horizontal_rule(line: &str) -> bool {
    let compact = line.chars().filter(|character| !character.is_whitespace());
    let mut count = 0;
    for character in compact {
        if character != '-' {
            return false;
        }
        count += 1;
    }
    count >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loads_markdown_rules() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("security.md"), "Check security.").unwrap();
        let rules = load(directory.path()).await.unwrap();
        assert_eq!(rules[0].id, "security");
    }

    #[tokio::test]
    async fn uses_markdown_before_the_first_horizontal_rule_as_the_message() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("security.md"),
            "# Unsafe boundary\n\nValidate external values.\n\n---\n\nLong instructions.",
        )
        .unwrap();

        let rules = load(directory.path()).await.unwrap();

        assert_eq!(
            rules[0].message,
            "# Unsafe boundary\n\nValidate external values."
        );
    }
}
