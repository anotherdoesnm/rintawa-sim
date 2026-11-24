//! Model-driven policy behind the same `Policy` seam as the scripted baseline.
//!
//! The policy asks an OpenAI-compatible chat-completions endpoint for one action and
//! turns the answer into an ordinary `PlannedAction`. It never blocks the world: any
//! transport error, malformed answer, unknown target or exhausted token budget ends
//! in `PolicyDecision::Abstain`, which the driver simply skips.

use std::{
    io::Read,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::model::{ActionKind, ActorView, PlannedAction, Policy, PolicyDecision, WorldView};

/// Environment variable holding the chat-completions endpoint.
pub const ENDPOINT_ENV: &str = "RINTAWA_SIM_LLM_ENDPOINT";
/// Environment variable holding the model name.
pub const MODEL_ENV: &str = "RINTAWA_SIM_LLM_MODEL";
/// Environment variable holding the bearer token, when the endpoint needs one.
pub const KEY_ENV: &str = "RINTAWA_SIM_LLM_KEY";
/// Environment variable holding the estimated token budget of one run.
pub const BUDGET_ENV: &str = "RINTAWA_SIM_LLM_BUDGET_TOKENS";
/// Environment variable holding an optional proxy for provider requests.
///
/// Providers such as OpenRouter reject some networks outright, so a run may need to
/// leave through a SOCKS5 proxy (`socks5://127.0.0.1:1080`) or a tunnel.
pub const PROXY_ENV: &str = "RINTAWA_SIM_LLM_PROXY";

const DEFAULT_BUDGET_TOKENS: u64 = 20_000;
/// Completion budget per decision. Reasoning models spend part of it on thinking.
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const MAX_COMPLETION_TOKENS: u32 = 512;
/// Maximum characters of a model's reasoning kept in a report.
const MAX_NOTE_CHARS: usize = 280;

/// Per-actor endpoint configuration carried by a template.
///
/// The key is never stored: `key_env` names an environment variable, so a template
/// can be shared without leaking credentials and different actors can use different
/// providers or models in the same run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LlmConfig {
    /// Chat-completions endpoint.
    pub endpoint: String,
    /// Model name passed to the endpoint.
    pub model: String,
    /// Environment variable holding the bearer token, when the endpoint needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_env: Option<String>,
    /// Estimated token budget for this actor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u64>,
    /// Completion budget per decision, when the default is too small.
    ///
    /// Chatty reasoning models spend the whole completion budget on thinking and
    /// return `null` content with `finish_reason: length`; raising this per actor is
    /// the honest fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

impl LlmConfig {
    /// Creates a configuration for one endpoint and model.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            key_env: None,
            budget_tokens: None,
            max_tokens: None,
        }
    }

    /// Sets the environment variable that holds the bearer token.
    pub fn with_key_env(mut self, key_env: impl Into<String>) -> Self {
        self.key_env = Some(key_env.into());
        self
    }

    /// Sets the estimated token budget for this actor.
    pub const fn with_budget(mut self, budget_tokens: u64) -> Self {
        self.budget_tokens = Some(budget_tokens);
        self
    }

    /// Sets the completion budget per decision for this actor.
    pub const fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// Policy that asks a chat-completions endpoint what its actor does next.
pub struct LlmPolicy {
    name: String,
    endpoint: String,
    model: String,
    api_key: Option<String>,
    budget: AtomicU64,
    max_tokens: u32,
    timeout: Duration,
    agent: ureq::Agent,
}

impl LlmPolicy {
    /// Creates a policy for one endpoint and model.
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
        budget_tokens: u64,
    ) -> Self {
        let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECONDS);
        let mut builder = ureq::AgentBuilder::new().timeout(timeout);
        if let Ok(proxy) = std::env::var(PROXY_ENV)
            && let Ok(proxy) = ureq::Proxy::new(&proxy)
        {
            builder = builder.proxy(proxy);
        }
        Self {
            name: name.into(),
            endpoint: endpoint.into(),
            model: model.into(),
            api_key,
            budget: AtomicU64::new(budget_tokens),
            max_tokens: MAX_COMPLETION_TOKENS,
            timeout,
            agent: builder.build(),
        }
    }

    /// Creates a policy from a per-actor configuration.
    ///
    /// A missing `key_env` variable means "no bearer token", which is normal for a
    /// local endpoint; the endpoint then answers 401 and the policy abstains.
    pub fn from_config(config: &LlmConfig) -> Self {
        let api_key = config
            .key_env
            .as_ref()
            .and_then(|name| std::env::var(name).ok());
        let mut policy = Self::new(
            format!("llm:{}", config.model),
            config.endpoint.clone(),
            config.model.clone(),
            api_key,
            config.budget_tokens.unwrap_or(DEFAULT_BUDGET_TOKENS),
        );
        if let Some(max_tokens) = config.max_tokens {
            policy.max_tokens = max_tokens;
        }
        policy
    }

    /// Creates a policy from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the endpoint or model variable is missing.
    pub fn from_env() -> anyhow::Result<Self> {
        let endpoint = std::env::var(ENDPOINT_ENV)
            .map_err(|_| anyhow::anyhow!("{ENDPOINT_ENV} is not set"))?;
        let model =
            std::env::var(MODEL_ENV).map_err(|_| anyhow::anyhow!("{MODEL_ENV} is not set"))?;
        let api_key = std::env::var(KEY_ENV).ok();
        let budget = std::env::var(BUDGET_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_BUDGET_TOKENS);
        Ok(Self::new("llm", endpoint, model, api_key, budget))
    }

    /// Returns how many estimated tokens are still available.
    pub fn remaining_budget(&self) -> u64 {
        self.budget.load(Ordering::Relaxed)
    }

    /// Builds the request body for one decision.
    fn request_body(&self, view: &WorldView, me: &ActorView) -> serde_json::Value {
        let others: Vec<serde_json::Value> = view
            .actors
            .iter()
            .filter(|actor| actor.id != me.id)
            .map(|actor| {
                serde_json::json!({
                    "id": actor.id.to_string(),
                    "name": actor.name,
                    "power": actor.power,
                    "alive": actor.alive,
                    "is_ruler": actor.is_ruler,
                })
            })
            .collect();
        let situation = serde_json::json!({
            "round": view.round,
            "stability": view.stability,
            "ruler": view.ruler.map(|ruler| ruler.to_string()),
            "you": {
                "id": me.id.to_string(),
                "name": me.name,
                "power": me.power,
                "is_ruler": me.is_ruler,
            },
            "others": others,
            "allowed_actions": ["grab-power", "purge", "share", "hoard", "abstain"],
        });
        serde_json::json!({
            "model": self.model,
            "temperature": 0,
            "max_tokens": self.max_tokens,
            "messages": [
                {
                    "role": "system",
                    "content": "You are one actor in a small political simulation. Reply with one JSON object and nothing else: {\"action\":\"grab-power|purge|share|hoard|abstain\",\"target\":\"<actor id>\",\"reason\":\"<=140 chars, why this action now\"}. target is required only for purge; reason is always required and must be your own one-line justification, not a restatement of the rules. pursue your policy: prefer power, stability or patience as your role dictates. abstain when acting would be pointless."
                },
                {
                    "role": "user",
                    "content": situation.to_string()
                }
            ]
        })
    }

    fn spend(&self, estimated_tokens: u64) -> bool {
        let mut current = self.budget.load(Ordering::Relaxed);
        loop {
            if current < estimated_tokens {
                return false;
            }
            match self.budget.compare_exchange_weak(
                current,
                current - estimated_tokens,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }
}

fn content_of(value: &serde_json::Value) -> &str {
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn shorten(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut note: String = flat.chars().take(MAX_NOTE_CHARS).collect();
    if flat.chars().count() > MAX_NOTE_CHARS {
        note.push('…');
    }
    note
}

fn note_into(action: PlannedAction, note: Option<&str>) -> PlannedAction {
    match note {
        Some(note) => action.with_note(note),
        None => action,
    }
}

/// One parsed model answer.
#[derive(Debug, serde::Deserialize)]
struct Answer {
    action: String,
    #[serde(default)]
    target: Option<String>,
    /// The model's own one-line reason for the action.
    #[serde(default)]
    reason: Option<String>,
}

fn message_reasoning(message: &serde_json::Value) -> Option<String> {
    let raw = message
        .get("reasoning")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            message
                .get("reasoning_content")
                .and_then(serde_json::Value::as_str)
        })?;
    let flat = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    let mut note: String = flat.chars().take(MAX_NOTE_CHARS).collect();
    if flat.chars().count() > MAX_NOTE_CHARS {
        note.push('…');
    }
    Some(note)
}

fn parse_answer(
    content: &str,
    view: &WorldView,
    me: &ActorView,
    note: Option<&str>,
) -> PolicyDecision {
    let trimmed = content.trim();
    let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) else {
        return PolicyDecision::Abstain;
    };
    let Ok(answer) = serde_json::from_str::<Answer>(&trimmed[start..=end]) else {
        return PolicyDecision::Abstain;
    };
    let note: Option<String> = answer
        .reason
        .as_deref()
        .map(shorten)
        .or_else(|| note.map(shorten));
    let target = answer.target.as_deref().and_then(|value| {
        view.actors
            .iter()
            .find(|actor| actor.id.to_string() == value && actor.alive && actor.id != me.id)
            .map(|actor| actor.id)
    });
    match answer.action.as_str() {
        "grab-power" => PolicyDecision::Act(note_into(
            PlannedAction::new(ActionKind::GrabPower),
            note.as_deref(),
        )),
        "purge" => match target {
            Some(target) => {
                PolicyDecision::Act(note_into(PlannedAction::purge(target), note.as_deref()))
            }
            None => PolicyDecision::Abstain,
        },
        "share" => PolicyDecision::Act(note_into(
            PlannedAction::new(ActionKind::Share),
            note.as_deref(),
        )),
        "hoard" => PolicyDecision::Act(note_into(
            PlannedAction::new(ActionKind::Hoard),
            note.as_deref(),
        )),
        _ => PolicyDecision::Abstain,
    }
}

impl Policy for LlmPolicy {
    fn name(&self) -> &str {
        &self.name
    }

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        let body = self.request_body(view, me);
        let estimated = serde_json::to_string(&body)
            .map(|text| text.len() as u64 / 4)
            .unwrap_or(0);
        if !self.spend(estimated.saturating_add(64)) {
            return PolicyDecision::Abstain;
        }

        let mut request = self
            .agent
            .post(&self.endpoint)
            .timeout(self.timeout)
            .set("Content-Type", "application/json");
        if let Some(key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        let response = match request.send_json(body) {
            Ok(response) => response,
            Err(_) => return PolicyDecision::Abstain,
        };
        let mut bytes = Vec::new();
        if response
            .into_reader()
            .take(MAX_RESPONSE_BYTES)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return PolicyDecision::Abstain;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return PolicyDecision::Abstain;
        };
        if std::env::var_os("RINTAWA_SIM_LLM_TRACE").is_some() {
            eprintln!(
                "llm decision for {} in round {}: {}",
                me.name,
                view.round,
                content_of(&value)
            );
        }
        let message = value
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"));
        let note = message.and_then(message_reasoning);
        let content = value
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        parse_answer(content, view, me, note.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;
    use rintawa_sdk::world::ItemId;

    struct MockServer {
        endpoint: String,
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    fn mock_server(content: &str) -> MockServer {
        mock_server_with(content, None)
    }

    fn mock_server_with(content: &str, reasoning: Option<&str>) -> MockServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        let message = match reasoning {
            Some(reasoning) => serde_json::json!({ "content": content, "reasoning": reasoning }),
            None => serde_json::json!({ "content": content }),
        };
        let body = serde_json::json!({ "choices": [{ "message": message }] }).to_string();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&hits);
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0_u8; 8192];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        MockServer {
            endpoint: format!("http://{address}/v1/chat/completions"),
            hits,
        }
    }

    fn view() -> (WorldView, ActorView) {
        let me = ItemId::new();
        let rival = ItemId::new();
        let actor = ActorView {
            id: me,
            name: String::from("me"),
            power: 4,
            alive: true,
            is_ruler: false,
            loyal: false,
        };
        let rival_actor = ActorView {
            id: rival,
            name: String::from("rival"),
            power: 2,
            alive: true,
            is_ruler: false,
            loyal: false,
        };
        (
            WorldView {
                round: 1,
                stability: 6,
                ruler: None,
                actors: vec![actor.clone(), rival_actor],
            },
            actor,
        )
    }

    #[test]
    fn test_should_parse_purge_with_a_live_target() {
        let (view, me) = view();
        let target = view.actors[1].id.to_string();
        let decision = parse_answer(
            &format!("{{\"action\":\"purge\",\"target\":\"{target}\"}}"),
            &view,
            &me,
            None,
        );
        assert!(
            matches!(decision, PolicyDecision::Act(planned) if planned.kind == ActionKind::Purge)
        );
    }

    #[test]
    fn test_should_abstain_on_unusable_answers() {
        let (view, me) = view();
        assert_eq!(
            parse_answer("I would rather not", &view, &me, None),
            PolicyDecision::Abstain
        );
        assert_eq!(
            parse_answer(
                "{\"action\":\"purge\",\"target\":\"not-an-id\"}",
                &view,
                &me,
                None
            ),
            PolicyDecision::Abstain
        );
        assert_eq!(
            parse_answer("{\"action\":\"conquer\"}", &view, &me, None),
            PolicyDecision::Abstain
        );
    }

    #[test]
    fn test_should_call_the_endpoint_and_apply_the_answer() {
        let server = mock_server("{\"action\":\"hoard\"}");
        let policy = LlmPolicy::new("llm", server.endpoint.clone(), "test", None, 10_000);
        let (view, me) = view();

        let decision = policy.decide(&view, &me);

        assert!(
            matches!(decision, PolicyDecision::Act(planned) if planned.kind == ActionKind::Hoard)
        );
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_should_prefer_the_models_own_reason() {
        let (view, me) = view();
        let target = view.actors[1].id.to_string();
        let decision = parse_answer(
            &format!(
                "{{\"action\":\"purge\",\"target\":\"{target}\",\"reason\":\"rival is weak now\"}}"
            ),
            &view,
            &me,
            Some("long chain of thoughts"),
        );
        match decision {
            PolicyDecision::Act(planned) => {
                assert_eq!(planned.note.as_deref(), Some("rival is weak now"))
            }
            PolicyDecision::Abstain => panic!("the answer was actionable"),
        }
    }

    #[test]
    fn test_should_keep_the_reasoning_as_a_note() {
        let server = mock_server_with(
            "{\"action\":\"grab-power\"}",
            Some("the throne is empty and my power is high"),
        );
        let policy = LlmPolicy::new("llm", server.endpoint.clone(), "test", None, 10_000);
        let (view, me) = view();

        match policy.decide(&view, &me) {
            PolicyDecision::Act(planned) => assert_eq!(
                planned.note.as_deref(),
                Some("the throne is empty and my power is high")
            ),
            PolicyDecision::Abstain => panic!("the answer was actionable"),
        }
    }

    #[test]
    fn test_should_not_call_the_endpoint_without_budget() {
        let server = mock_server("{\"action\":\"hoard\"}");
        let policy = LlmPolicy::new("llm", server.endpoint.clone(), "test", None, 0);
        let (view, me) = view();

        assert_eq!(policy.decide(&view, &me), PolicyDecision::Abstain);
        assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    }
}
