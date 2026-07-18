//! `MockProvider` (test-only): a scripted [`LlmProvider`] that returns a queued
//! sequence of [`ChatTurn`]s, one per `chat` call. It lets the agent loop be
//! tested end to end - tool-call turn, then final-answer turn - with real tool
//! execution but no model and no network. The residency story is unaffected: a
//! mock never touches an endpoint.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use super::{ChatRequest, ChatTurn, LlmProvider, ProviderDescriptor, ProviderError};

pub struct MockProvider {
    turns: Mutex<VecDeque<ChatTurn>>,
    /// Every request the loop sent, captured so tests can assert the tool
    /// results were actually fed back on the second turn.
    pub seen: Mutex<Vec<ChatRequest>>,
}

impl MockProvider {
    pub fn new(turns: Vec<ChatTurn>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            seen: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatTurn, ProviderError> {
        self.seen.lock().unwrap().push(req);
        self.turns
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ProviderError::Decode("mock provider ran out of scripted turns".into()))
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "mock".to_string(),
            model: "mock".to_string(),
            endpoint: "mock://local".to_string(),
            local: true,
        }
    }
}
