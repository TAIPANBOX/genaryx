//! `OpenAiCompat`: the one client that covers Ollama, LM Studio, vLLM,
//! OpenRouter and OpenAI (they all speak the `/chat/completions` wire format).
//! Bodies are built and parsed with `serde_json` (no reqwest `json` feature),
//! matching `CloudClient`'s style.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    ChatRequest, ChatTurn, LlmProvider, Message, ProviderDescriptor, ProviderError, Role, ToolCall,
    Usage, check_residency, residency_client, system_lookup,
};
use crate::config::ProviderKind;
use crate::resolver::HostnameLookup;

#[derive(Debug)]
pub struct OpenAiCompat {
    kind: ProviderKind,
    base_url: String,
    model: String,
    api_key: Option<String>,
    local: bool,
    /// C2 self-budget: sent as `x-fuse-run-id` so a TokenFuse gateway meters the
    /// copilot's own inference spend (harmless against a raw endpoint).
    run_id: String,
    /// 2026-09-27 defect fix: sent as `x-fuse-agent-id` beside `x-fuse-run-id`,
    /// so a TokenFuse gateway running its Wardryx hook in enforce mode can
    /// identify Felyx's own calls instead of refusing them with
    /// `identity_required` for carrying no agent identity at all. Resolved by
    /// `CopilotConfig::resolved_agent_id` (explicit or the
    /// `agent://<org_domain>/genaryx/felyx` default), never logged above
    /// `debug` and never alongside `api_key`.
    agent_id: String,
    http: reqwest::Client,
}

impl OpenAiCompat {
    /// `local_hostnames` (`GENARYX_COPILOT_LOCAL_HOSTNAMES`): the allow-list
    /// of hostnames the residency gate may resolve and check (invariant 14).
    /// Empty keeps the original behaviour - any hostname other than
    /// `localhost` refused outright, no DNS call at all.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: ProviderKind,
        base_url: String,
        model: String,
        api_key: Option<String>,
        allow_non_local_endpoints: bool,
        local_hostnames: Vec<String>,
        run_id: String,
        agent_id: String,
    ) -> Result<Self, ProviderError> {
        Self::new_with_lookup(
            kind,
            base_url,
            model,
            api_key,
            allow_non_local_endpoints,
            local_hostnames,
            run_id,
            agent_id,
            system_lookup(),
        )
    }

    /// Test-support back door: build against an injected [`HostnameLookup`]
    /// instead of the real OS resolver, so a test can prove the hostname
    /// half of the residency gate (invariant 14) without touching real DNS.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_lookup(
        kind: ProviderKind,
        base_url: String,
        model: String,
        api_key: Option<String>,
        allow_non_local_endpoints: bool,
        local_hostnames: Vec<String>,
        run_id: String,
        agent_id: String,
        lookup: Arc<dyn HostnameLookup>,
    ) -> Result<Self, ProviderError> {
        let outcome = check_residency(
            &base_url,
            allow_non_local_endpoints,
            &local_hostnames,
            lookup,
        )?;
        let local = outcome.local;
        let http = residency_client(outcome, allow_non_local_endpoints)?;
        Ok(Self {
            kind,
            base_url,
            model,
            api_key,
            local,
            run_id,
            agent_id,
            http,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompat {
    async fn chat(&self, req: ChatRequest) -> Result<ChatTurn, ProviderError> {
        let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
        if !req.system.is_empty() {
            messages.push(json!({"role": "system", "content": req.system}));
        }
        for m in &req.messages {
            messages.push(message_to_openai(m));
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": false,
        });
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.params_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = Value::Array(tools);
            body["tool_choice"] = json!("auto");
        }

        let mut request = self
            .http
            .post(self.endpoint())
            .header("x-fuse-run-id", &self.run_id)
            .header("x-fuse-agent-id", &self.agent_id)
            .json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let resp = request
            .send()
            .await
            .map_err(|e| super::map_send_error(&self.base_url, e))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: text,
            });
        }
        parse_openai_response(&text)
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: self.kind.label().to_string(),
            model: self.model.clone(),
            endpoint: self.base_url.clone(),
            local: self.local,
        }
    }
}

fn message_to_openai(m: &Message) -> Value {
    match m.role {
        Role::System => json!({"role": "system", "content": m.content}),
        Role::User => json!({"role": "user", "content": m.content}),
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
            "content": m.content,
        }),
        Role::Assistant => {
            if m.tool_calls.is_empty() {
                json!({"role": "assistant", "content": m.content})
            } else {
                let calls: Vec<Value> = m
                    .tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": {
                                "name": c.name,
                                // OpenAI wants arguments as a JSON *string*.
                                "arguments": c.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                json!({
                    "role": "assistant",
                    "content": if m.content.is_empty() { Value::Null } else { json!(m.content) },
                    "tool_calls": calls,
                })
            }
        }
    }
}

fn parse_openai_response(text: &str) -> Result<ChatTurn, ProviderError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ProviderError::Decode(e.to_string()))?;
    let message = v
        .pointer("/choices/0/message")
        .ok_or_else(|| ProviderError::Decode("no choices[0].message".into()))?;

    let content = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (i, call) in calls.iter().enumerate() {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{i}"));
            let func = call.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = func
                .and_then(|f| f.get("arguments"))
                .map(parse_arguments)
                .unwrap_or_else(|| json!({}));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    let usage = Usage {
        prompt_tokens: v
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: v
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    };

    Ok(ChatTurn {
        content,
        tool_calls,
        usage,
    })
}

/// Tool-call arguments arrive as a JSON-encoded string in the OpenAI wire
/// format; some local runtimes send an object directly. Handle both, and treat
/// an empty/blank string as no arguments.
fn parse_arguments(v: &Value) -> Value {
    match v {
        Value::String(s) if s.trim().is_empty() => json!({}),
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_public_endpoint_by_default() {
        let err = OpenAiCompat::new(
            ProviderKind::OpenRouter,
            "https://openrouter.ai/api/v1".into(),
            "x".into(),
            Some("k".into()),
            false,
            Vec::new(),
            "genaryx-copilot".into(),
            "agent://local/genaryx/felyx".into(),
        )
        .unwrap_err();
        assert!(matches!(err, ProviderError::NonLocalEndpointRefused { .. }));
    }

    #[test]
    fn allows_public_endpoint_when_opted_in() {
        let p = OpenAiCompat::new(
            ProviderKind::OpenRouter,
            "https://openrouter.ai/api/v1".into(),
            "x".into(),
            Some("k".into()),
            true,
            Vec::new(),
            "genaryx-copilot".into(),
            "agent://local/genaryx/felyx".into(),
        )
        .unwrap();
        assert!(!p.descriptor().local);
    }

    #[test]
    fn local_endpoint_needs_no_opt_in() {
        let p = OpenAiCompat::new(
            ProviderKind::Ollama,
            "http://127.0.0.1:11434/v1".into(),
            "qwen3:8b".into(),
            None,
            false,
            Vec::new(),
            "genaryx-copilot".into(),
            "agent://local/genaryx/felyx".into(),
        )
        .unwrap();
        assert!(p.descriptor().local);
        assert_eq!(p.endpoint(), "http://127.0.0.1:11434/v1/chat/completions");
    }

    #[test]
    fn parses_a_tool_call_response() {
        let body = r#"{
            "choices": [{"message": {"content": null, "tool_calls": [
                {"id": "call_1", "type": "function",
                 "function": {"name": "alerts", "arguments": "{}"}}
            ]}}],
            "usage": {"prompt_tokens": 42, "completion_tokens": 7}
        }"#;
        let turn = parse_openai_response(body).unwrap();
        assert!(turn.content.is_none());
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "alerts");
        assert_eq!(turn.usage.prompt_tokens, 42);
    }

    #[test]
    fn parses_a_text_response() {
        let body = r#"{"choices":[{"message":{"content":"3 runs are over cap."}}],
                       "usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let turn = parse_openai_response(body).unwrap();
        assert_eq!(turn.content.as_deref(), Some("3 runs are over cap."));
        assert!(turn.tool_calls.is_empty());
    }
}
