use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::config::PreconditionConfig;

#[derive(Debug, Deserialize, Serialize)]
pub struct PreconditionFailure {
    pub files: Vec<PathBuf>,
    pub command: Vec<String>,
    pub status: Option<i32>,
    pub output: String,
}

#[async_trait]
pub trait Precondition: Send + Sync {
    async fn check(
        &self,
        root: &Path,
        files: &[PathBuf],
    ) -> anyhow::Result<Option<PreconditionFailure>>;
}

pub struct CommandPrecondition {
    config: PreconditionConfig,
}

impl CommandPrecondition {
    pub fn new(config: PreconditionConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Precondition for CommandPrecondition {
    async fn check(
        &self,
        root: &Path,
        files: &[PathBuf],
    ) -> anyhow::Result<Option<PreconditionFailure>> {
        let (program, args) = self
            .config
            .command
            .split_first()
            .expect("validated command");
        let output = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("failed to run precondition command {program:?}"))?;
        if output.status.success() {
            return Ok(None);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(Some(PreconditionFailure {
            files: files.to_vec(),
            command: self.config.command.clone(),
            status: output.status.code(),
            output: format!("{stdout}{stderr}").trim().to_owned(),
        }))
    }
}
