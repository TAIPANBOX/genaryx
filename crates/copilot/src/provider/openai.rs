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

    let usage = usage_from(&v);

    Ok(ChatTurn {
        content,
        tool_calls,
        usage,
    })
}

/// The token usage of one OpenAI-shaped answer, output counted the way the
/// provider bills it.
///
/// Output is the larger of `completion_tokens` and `total_tokens` less
/// `prompt_tokens`. Google's OpenAI-compatible endpoint leaves a thinking
/// model's reasoning out of `completion_tokens` and bills it at the output
/// rate (measured 2026-10-07 on Vertex AI, `gemini-2.5-flash`: prompt 14,
/// completion 59, reasoning 560, total 633), so reading the completion count
/// alone showed about a tenth of the output a gateway in front now charges.
/// OpenAI's completion count already holds its reasoning, so there the gap
/// equals the completion count and nothing changes; the reasoning detail is
/// never added on top, which would count it twice. A total short of the sum
/// never lowers the output below the completion count, and a total with no
/// prompt count is output whole. The rule is TokenFuse's own (its invariant
/// 80) and CostCrew's, so the three read one answer the same way.
///
/// Counts arrive as JSON numbers and are kept in `u32`: a figure past it
/// saturates rather than wrapping into a small, plausible number, and a
/// field that is not a non-negative integer reads as absent.
fn usage_from(v: &Value) -> Usage {
    let count = |field: &str| {
        v.pointer(&format!("/usage/{field}"))
            .and_then(Value::as_u64)
    };
    let prompt = count("prompt_tokens").unwrap_or(0);
    let completion = count("completion_tokens").unwrap_or(0);
    let output = match count("total_tokens") {
        Some(total) => completion.max(total.saturating_sub(prompt)),
        None => completion,
    };
    let clamp = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    Usage {
        prompt_tokens: clamp(prompt),
        completion_tokens: clamp(output),
    }
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

    fn usage_of(usage: &str) -> Usage {
        let body = format!(r#"{{"choices":[{{"message":{{"content":"ok"}}}}],"usage":{usage}}}"#);
        parse_openai_response(&body).expect("parses").usage
    }

    /// Measured 2026-10-07 against Vertex AI's OpenAI-compatible endpoint
    /// (`google/gemini-2.5-flash`, one non-streamed answer), the figures
    /// TokenFuse's invariant 80 records: Google's `completion_tokens` leaves
    /// the reasoning out (633 = 14 + 59 + 560) and Google bills reasoning at
    /// the output rate, so the output a gateway now charges is 619, not 59.
    #[test]
    fn a_reasoning_model_usage_counts_its_reasoning_as_output() {
        let u = usage_of(
            r#"{"prompt_tokens":14,"completion_tokens":59,"total_tokens":633,
                "completion_tokens_details":{"reasoning_tokens":560}}"#,
        );
        assert_eq!(u.prompt_tokens, 14);
        assert_eq!(
            u.completion_tokens, 619,
            "the reasoning the provider bills as output is shown as output"
        );
    }

    /// OpenAI's own completion count already holds its reasoning
    /// (total = prompt + completion), so nothing changes, and the reasoning
    /// detail is never added on top, which would count it twice.
    #[test]
    fn an_openai_shaped_usage_is_read_unchanged() {
        let u = usage_of(
            r#"{"prompt_tokens":10,"completion_tokens":25,"total_tokens":35,
                "completion_tokens_details":{"reasoning_tokens":20}}"#,
        );
        assert_eq!((u.prompt_tokens, u.completion_tokens), (10, 25));
        let u = usage_of(r#"{"prompt_tokens":10,"completion_tokens":5}"#);
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens),
            (10, 5),
            "no total: read as before"
        );
    }

    /// A total that is short of prompt + completion never lowers the output
    /// below the completion count, and a total below the prompt never wraps.
    #[test]
    fn a_short_total_never_lowers_the_output_below_the_completion_count() {
        let u = usage_of(r#"{"prompt_tokens":100,"completion_tokens":50,"total_tokens":120}"#);
        assert_eq!(u.completion_tokens, 50);
        let u = usage_of(r#"{"prompt_tokens":100,"completion_tokens":50,"total_tokens":3}"#);
        assert_eq!(u.completion_tokens, 50);
    }

    /// The same rule TokenFuse applies: a total with no prompt count is shown
    /// whole as output rather than dropped.
    #[test]
    fn a_total_with_no_prompt_count_is_shown_as_output() {
        let u = usage_of(r#"{"completion_tokens":5,"total_tokens":40}"#);
        assert_eq!((u.prompt_tokens, u.completion_tokens), (0, 40));
    }

    /// Figures past `u32` saturate rather than wrap (a wrapped count reads as
    /// a small, plausible number), and a field that is not a count reads as
    /// absent. A 200-seed sweep of present, absent and out-of-range figures
    /// holds the rule against an independent statement of it.
    #[test]
    fn hostile_usage_figures_saturate_and_never_wrap() {
        let u = usage_of(r#"{"prompt_tokens":4294967297,"completion_tokens":4294967296}"#);
        assert_eq!((u.prompt_tokens, u.completion_tokens), (u32::MAX, u32::MAX));
        let u = usage_of(r#"{"prompt_tokens":-3,"completion_tokens":"7","total_tokens":null}"#);
        assert_eq!((u.prompt_tokens, u.completion_tokens), (0, 0));

        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..200 {
            let mut fields = Vec::new();
            let mut pick = |name: &str, fields: &mut Vec<String>| -> Option<u64> {
                let r = next();
                if r % 4 == 0 {
                    return None;
                }
                let v = if r % 4 == 1 {
                    next() >> 20
                } else {
                    next() % 5_000
                };
                fields.push(format!("\"{name}\":{v}"));
                Some(v)
            };
            let p = pick("prompt_tokens", &mut fields);
            let c = pick("completion_tokens", &mut fields);
            let t = pick("total_tokens", &mut fields);
            let u = usage_of(&format!("{{{}}}", fields.join(",")));
            let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
            let p = p.unwrap_or(0);
            let c = c.unwrap_or(0);
            let want = match t {
                Some(t) => c.max(t.saturating_sub(p)),
                None => c,
            };
            assert_eq!(u.prompt_tokens, clamp(p), "fields {fields:?}");
            assert_eq!(u.completion_tokens, clamp(want), "fields {fields:?}");
        }
    }
}
