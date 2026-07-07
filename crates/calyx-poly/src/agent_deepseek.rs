//! DeepSeek chat-completions transport for the Poly forecast-agent launcher.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::{AgentLauncherRequest, DeepSeekRuntimeSecrets, DeepSeekUsage, PolyError, Result};

pub(crate) struct DeepSeekCompletion {
    pub id: String,
    pub finish_reason: String,
    pub content_json: String,
    pub usage: DeepSeekUsage,
}

pub(crate) fn call_deepseek(
    secrets: &DeepSeekRuntimeSecrets,
    request: &AgentLauncherRequest,
    rendered_prompt: &str,
) -> Result<DeepSeekCompletion> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(request.timeout_secs)))
        .build()
        .into();
    let auth = secrets.bearer_authorization();
    let body = json!({
        "model": secrets.model(),
        "messages": [
            {
                "role": "system",
                "content": "You are a Poly local forecast agent. Return strict JSON only. You cannot trade, sign orders, submit orders, manage bankroll, request tools, or use Polymarket trading surfaces."
            },
            {
                "role": "user",
                "content": rendered_prompt
            }
        ],
        "thinking": {"type": "disabled"},
        "response_format": {"type": "json_object"},
        "max_tokens": request.max_tokens,
        "temperature": 0,
        "stream": false
    });
    let mut response = agent
        .post(&secrets.chat_completions_url())
        .header("Authorization", auth.as_str())
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|err| {
            PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_HTTP_FAILED",
                redact_secret(&err.to_string()),
            )
        })?;
    let value: DeepSeekChatCompletion = response.body_mut().read_json().map_err(|err| {
        PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_DEEPSEEK_RESPONSE_DECODE_FAILED",
            redact_secret(&err.to_string()),
        )
    })?;
    value.into_completion(secrets.model())
}

fn redact_secret(message: &str) -> String {
    message
        .split_whitespace()
        .map(|part| {
            if part.starts_with("sk-") {
                "<redacted-secret>"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Deserialize)]
struct DeepSeekChatCompletion {
    id: String,
    object: String,
    model: String,
    choices: Vec<DeepSeekChoice>,
    usage: Option<DeepSeekUsagePayload>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    finish_reason: String,
    message: DeepSeekMessage,
}

#[derive(Debug, Deserialize)]
struct DeepSeekMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekUsagePayload {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

impl DeepSeekChatCompletion {
    fn into_completion(self, expected_model: &str) -> Result<DeepSeekCompletion> {
        if self.object != "chat.completion" {
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_OBJECT_INVALID",
                format!("DeepSeek object was {}", self.object),
            ));
        }
        if self.model != expected_model {
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_MODEL_MISMATCH",
                format!(
                    "DeepSeek model was {}, expected {expected_model}",
                    self.model
                ),
            ));
        }
        if self.choices.len() != 1 {
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_CHOICE_COUNT_INVALID",
                format!("expected exactly one choice, got {}", self.choices.len()),
            ));
        }
        let choice = self.choices.into_iter().next().expect("one choice checked");
        if choice.finish_reason != "stop" {
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_FINISH_REASON_INVALID",
                format!("DeepSeek finish_reason was {}", choice.finish_reason),
            ));
        }
        let content_json = choice.message.content.ok_or_else(|| {
            PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_CONTENT_MISSING",
                "DeepSeek response message content was missing",
            )
        })?;
        if content_json.trim().is_empty() {
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_DEEPSEEK_CONTENT_EMPTY",
                "DeepSeek response message content was empty",
            ));
        }
        let usage = self.usage.map_or(
            DeepSeekUsage {
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
            },
            |usage| DeepSeekUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            },
        );
        Ok(DeepSeekCompletion {
            id: self.id,
            finish_reason: choice.finish_reason,
            content_json,
            usage,
        })
    }
}
