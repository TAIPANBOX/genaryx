//! `AnthropicMessages`: the Anthropic Messages API client. Distinct wire shape
//! from OpenAI: `system` is a top-level field, tool calls are `tool_use`
//! content blocks, and tool results ride back as `tool_result` blocks inside a
//! USER message (consecutive tool results are folded into one user message, as
//! the API expects).

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    ChatRequest, ChatTurn, LlmProvider, Message, ProviderDescriptor, ProviderError, Role, ToolCall,
    Usage,
};
use crate::residency::is_local_endpoint;

const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug)]
pub struct AnthropicMessages {
    base_url: String,
    model: String,
    api_key: String,
    local: bool,
    /// C2 self-budget: sent as `x-fuse-run-id` for TokenFuse-gateway metering.
    run_id: String,
    http: reqwest::Client,
}

impl AnthropicMessages {
    pub fn new(
        base_url: String,
        model: String,
        api_key: String,
        allow_non_local_endpoints: bool,
        run_id: String,
    ) -> Result<Self, ProviderError> {
        let local = is_local_endpoint(&base_url);
        if !local && !allow_non_local_endpoints {
            return Err(ProviderError::NonLocalEndpointRefused { url: base_url });
        }
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(Self {
            base_url,
            model,
            api_key,
            local,
            run_id,
            http,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl LlmProvider for AnthropicMessages {
    async fn chat(&self, req: ChatRequest) -> Result<ChatTurn, ProviderError> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "messages": messages_to_anthropic(&req.messages),
        });
        if !req.system.is_empty() {
            body["system"] = json!(req.system);
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.params_schema,
                    })
                })
                .collect();
            body["tools"] = Value::Array(tools);
        }

        let resp = self
            .http
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("x-fuse-run-id", &self.run_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
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
        parse_anthropic_response(&text)
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "anthropic".to_string(),
            model: self.model.clone(),
            endpoint: self.base_url.clone(),
            local: self.local,
        }
    }
}

/// Fold our flat message list into Anthropic's shape: `system` is handled by the
/// caller; user/assistant map directly; a RUN of consecutive tool results
/// becomes a single user message carrying one `tool_result` block per result.
fn messages_to_anthropic(messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();

    let flush = |pending: &mut Vec<Value>, out: &mut Vec<Value>| {
        if !pending.is_empty() {
            out.push(json!({"role": "user", "content": std::mem::take(pending)}));
        }
    };

    for m in messages {
        match m.role {
            Role::System => {} // handled as the top-level `system` field
            Role::Tool => {
                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content,
                }));
            }
            Role::User => {
                flush(&mut pending_tool_results, &mut out);
                out.push(json!({"role": "user", "content": [{"type": "text", "text": m.content}]}));
            }
            Role::Assistant => {
                flush(&mut pending_tool_results, &mut out);
                let mut content: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    content.push(json!({"type": "text", "text": m.content}));
                }
                for c in &m.tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": c.id,
                        "name": c.name,
                        "input": c.arguments,
                    }));
                }
                out.push(json!({"role": "assistant", "content": content}));
            }
        }
    }
    flush(&mut pending_tool_results, &mut out);
    out
}

fn parse_anthropic_response(text: &str) -> Result<ChatTurn, ProviderError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ProviderError::Decode(e.to_string()))?;
    let blocks = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Decode("no content array".into()))?;

    let mut text_out = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text_out.push_str(t);
                }
            }
            Some("tool_use") => {
                tool_calls.push(ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                });
            }
            _ => {}
        }
    }

    let usage = Usage {
        prompt_tokens: v
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: v
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    };

    Ok(ChatTurn {
        content: (!text_out.is_empty()).then_some(text_out),
        tool_calls,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_public_by_default_allows_when_opted_in() {
        assert!(matches!(
            AnthropicMessages::new(
                "https://api.anthropic.com".into(),
                "claude".into(),
                "k".into(),
                false,
                "genaryx-copilot".into()
            ),
            Err(ProviderError::NonLocalEndpointRefused { .. })
        ));
        let p = AnthropicMessages::new(
            "https://api.anthropic.com".into(),
            "claude".into(),
            "k".into(),
            true,
            "genaryx-copilot".into(),
        )
        .unwrap();
        assert_eq!(p.endpoint(), "https://api.anthropic.com/v1/messages");
        assert!(!p.descriptor().local);
    }

    #[test]
    fn tool_results_fold_into_one_user_message() {
        let messages = vec![
            Message::user("q"),
            Message::assistant_tool_calls(
                None,
                vec![
                    ToolCall {
                        id: "a".into(),
                        name: "alerts".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "incidents".into(),
                        arguments: json!({}),
                    },
                ],
            ),
            Message::tool_result("a", "alerts", &json!([1, 2])),
            Message::tool_result("b", "incidents", &json!([])),
        ];
        let mapped = messages_to_anthropic(&messages);
        // user(q), assistant(tool_use x2), user(tool_result x2)
        assert_eq!(mapped.len(), 3);
        assert_eq!(mapped[2]["role"], "user");
        assert_eq!(mapped[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(mapped[2]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn parses_text_and_tool_use() {
        let body = r#"{
            "content": [
                {"type": "text", "text": "Looking..."},
                {"type": "tool_use", "id": "tu_1", "name": "alerts", "input": {}}
            ],
            "usage": {"input_tokens": 30, "output_tokens": 12}
        }"#;
        let turn = parse_anthropic_response(body).unwrap();
        assert_eq!(turn.content.as_deref(), Some("Looking..."));
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "alerts");
        assert_eq!(turn.usage.completion_tokens, 12);
    }
}
