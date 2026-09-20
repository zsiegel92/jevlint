use std::{collections::BTreeMap, env, time::Duration};

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::rule::Rule;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuleAnswer {
    pub rule_id: String,
    pub verdict: Verdict,
    pub confidence: f64,
    pub resolved_model: String,
}

#[derive(Clone, Debug)]
pub struct LineQuestion<'a> {
    pub rule: &'a Rule,
    pub line: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LineAnswer {
    pub rule_id: String,
    pub line: usize,
    pub violated: bool,
    pub confidence: f64,
    pub resolved_model: String,
}

#[async_trait]
pub trait LintProvider: Send + Sync {
    async fn lint(
        &self,
        system_prompt: &str,
        path: &str,
        source: &str,
        rules: &[Rule],
    ) -> anyhow::Result<Vec<RuleAnswer>>;
    async fn locate(
        &self,
        system_prompt: &str,
        path: &str,
        numbered_source: &str,
        questions: &[LineQuestion<'_>],
    ) -> anyhow::Result<Vec<LineAnswer>>;
}

pub struct JevClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl JevClient {
    pub fn from_env(model: String, timeout: Duration) -> anyhow::Result<Self> {
        let api_key = env::var("TYPESAFE_API_KEY").context("TYPESAFE_API_KEY is not set")?;
        let base_url =
            env::var("TYPESAFE_BASE_URL").unwrap_or_else(|_| "https://api.typesafe.ai".into());
        let http = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Self {
            http,
            api_key,
            base_url: base_url.trim_end_matches('/').into(),
            model,
        })
    }

    async fn request(
        &self,
        state: Value,
        questions: BTreeMap<String, Value>,
    ) -> anyhow::Result<ApiResponse> {
        let response = self
            .http
            .post(format!("{}/v1/systemone", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&json!({ "model": self.model, "state": state, "questions": questions }))
            .send()
            .await
            .context("Jev request failed")?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            bail!("Jev returned {status}: {}", body.trim());
        }
        serde_json::from_str(&body).context("Jev returned an invalid response")
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    model: String,
    answers: BTreeMap<String, ApiAnswer>,
}

#[derive(Deserialize)]
struct ApiAnswer {
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    noul: Option<f64>,
}

#[async_trait]
impl LintProvider for JevClient {
    async fn lint(
        &self,
        system_prompt: &str,
        path: &str,
        source: &str,
        rules: &[Rule],
    ) -> anyhow::Result<Vec<RuleAnswer>> {
        let questions = rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                (
                    format!("q{index}"),
                    json!({
                        "type": "choice",
                        "instructions": format!("Apply this lint rule:\n\n{}", rule.markdown),
                        "criteria": {
                            "pass": "The supplied file complies with this rule.",
                            "fail": "The supplied file violates this rule."
                        }
                    }),
                )
            })
            .collect();
        let response = self
            .request(
                json!({
                    "system_prompt": system_prompt,
                    "file": { "path": path, "source": source }
                }),
                questions,
            )
            .await?;
        rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                let answer = response
                    .answers
                    .get(&format!("q{index}"))
                    .with_context(|| format!("Jev omitted answer for rule {}", rule.id))?;
                let verdict = match answer.choice.as_deref() {
                    Some("pass") => Verdict::Pass,
                    Some("fail") => Verdict::Fail,
                    other => bail!("unexpected verdict {other:?} for rule {}", rule.id),
                };
                Ok(RuleAnswer {
                    rule_id: rule.id.clone(),
                    verdict,
                    confidence: answer
                        .confidence
                        .context("choice answer omitted confidence")?,
                    resolved_model: response.model.clone(),
                })
            })
            .collect()
    }

    async fn locate(
        &self,
        system_prompt: &str,
        path: &str,
        numbered_source: &str,
        questions: &[LineQuestion<'_>],
    ) -> anyhow::Result<Vec<LineAnswer>> {
        let request_questions = questions.iter().enumerate().map(|(index, question)| (
            format!("q{index}"),
            json!({
                "type": "noul",
                "instructions": format!(
                    "Does line {} contain or participate in a violation of this rule?\n\n{}",
                    question.line, question.rule.markdown
                ),
                "criteria": {
                    "true": "This line is part of this rule violation.",
                    "false": "This line is not part of this rule violation."
                }
            }),
        )).collect();
        let response = self.request(json!({
            "system_prompt": system_prompt,
            "task": "Locate already-confirmed lint violations by line. Judge each rule and line independently.",
            "file": { "path": path, "numbered_source": numbered_source }
        }), request_questions).await?;
        questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                let answer = response
                    .answers
                    .get(&format!("q{index}"))
                    .with_context(|| format!("Jev omitted location answer q{index}"))?;
                let probability = answer.noul.context("noul answer omitted probability")?;
                Ok(LineAnswer {
                    rule_id: question.rule.id.clone(),
                    line: question.line,
                    violated: probability >= 0.5,
                    confidence: if probability >= 0.5 {
                        probability
                    } else {
                        1.0 - probability
                    },
                    resolved_model: response.model.clone(),
                })
            })
            .collect()
    }
}
