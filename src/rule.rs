use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::hash;

#[derive(Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub source_path: PathBuf,
    pub markdown: String,
    pub hash: String,
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
            markdown,
            source_path,
        });
    }
    Ok(rules)
}
