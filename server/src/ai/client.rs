use async_stream::stream;
use futures::Stream;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

use super::circuit_breaker::CircuitBreaker;

/// Maximum user prompt size in bytes (16 KB). Prevents context window overflow
/// and excessive API costs from large email bodies.
const MAX_USER_PROMPT_BYTES: usize = 16_384;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIConfig {
    pub url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for AIConfig {
    fn default() -> Self {
        Self {
            url: "https://llm.aimighty.de/v1".into(),
            api_key: "ollama".into(),
            model: "llama3.2".into(),
            max_tokens: 4096,
            temperature: 0.7,
        }
    }
}

pub struct AIClient {
    http: HttpClient,
    config: AIConfig,
    max_retries: u32,
    base_delay: Duration,
    /// Semaphore(1) — serializes LLM calls. User requests wait, background skips if busy.
    semaphore: Arc<Semaphore>,
    /// Circuit Breaker: 3 failures in 60s → open for 120s
    circuit_breaker: Arc<CircuitBreaker>,
}

impl AIClient {
    pub fn circuit_breaker(&self) -> &Arc<CircuitBreaker> {
        &self.circuit_breaker
    }

    pub fn new(config: AIConfig) -> Self {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(5)
            .build()
            .unwrap_or_default();

        Self {
            http,
            config,
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            semaphore: Arc::new(Semaphore::new(1)),
            circuit_breaker: Arc::new(CircuitBreaker::new()),
        }
    }

    pub async fn health_check(&self) -> Result<bool, String> {
        let url = format!("{}/models", self.config.url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| format!("Health check fehlgeschlagen: {}", e))?;
        Ok(resp.status().is_success())
    }

    /// User-initiated request — waits for semaphore (high priority)
    pub async fn complete_user(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        // Check circuit breaker first
        self.circuit_breaker.allow_request()?;

        let _permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| "LLM-System heruntergefahren".to_string())?;

        let messages = vec![
            ChatMessage { role: "system".into(), content: system_prompt.to_string() },
            ChatMessage {
                role: "user".into(),
                content: truncate_prompt(user_prompt, MAX_USER_PROMPT_BYTES).into_owned(),
            },
        ];
        match self.complete_internal(messages, temperature, max_tokens).await {
            Ok(result) => {
                self.circuit_breaker.record_success();
                Ok(result)
            }
            Err(e) => {
                self.circuit_breaker.record_failure();
                Err(e)
            }
        }
    }

    /// Multi-message request (e.g. tool loop). Caller builds the message list and
    /// is responsible for keeping it within a reasonable size.
    pub async fn complete_messages(
        &self,
        messages: Vec<ChatMessage>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        self.circuit_breaker.allow_request()?;
        let _permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| "LLM-System heruntergefahren".to_string())?;
        match self.complete_internal(messages, temperature, max_tokens).await {
            Ok(result) => {
                self.circuit_breaker.record_success();
                Ok(result)
            }
            Err(e) => {
                self.circuit_breaker.record_failure();
                Err(e)
            }
        }
    }

    /// Background/automatic request — skips if LLM is busy (low priority)
    pub async fn complete_background(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Option<Result<String, String>> {
        // Check circuit breaker first
        if self.circuit_breaker.allow_request().is_err() {
            return None;
        }

        let permit = self.semaphore.clone().try_acquire_owned();
        match permit {
            Ok(_p) => {
                // Keep permit (_p) in scope so it's dropped only after complete_internal is finished!
                // This prevents overloading the local LLM with concurrent parallel requests.
            let messages = vec![
                ChatMessage { role: "system".into(), content: system_prompt.to_string() },
                ChatMessage {
                    role: "user".into(),
                    content: truncate_prompt(user_prompt, MAX_USER_PROMPT_BYTES).into_owned(),
                },
            ];
            let result = self.complete_internal(messages, temperature, max_tokens).await;
            if result.is_ok() {
                    self.circuit_breaker.record_success();
                } else {
                    self.circuit_breaker.record_failure();
                }
                // Permit is dropped here when _p goes out of scope!
                Some(result)
            }
            Err(_) => {
                tracing::debug!("LLM busy, skip automatic summary");
                None
            }
        }
    }

    /// Internal: execute the LLM call (semaphore must already be acquired by caller)
    async fn complete_internal(
        &self,
        messages: Vec<ChatMessage>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        let url = format!(
            "{}/chat/completions",
            self.config.url.trim_end_matches('/')
        );

        let body = ChatRequest {
            model: self.config.model.clone(),
            messages,
            stream: false,
            temperature: temperature.unwrap_or(self.config.temperature),
            max_tokens: max_tokens.unwrap_or(self.config.max_tokens),
        };

        for attempt in 0..=self.max_retries {
            match self
                .http
                .post(&url)
                .bearer_auth(&self.config.api_key)
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status() == 429 {
                        let delay = with_jitter(backoff_delay(self.base_delay, attempt));
                        tracing::warn!("Rate limited, retrying in {:?}", delay);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    if !resp.status().is_success() {
                        let status = resp.status().as_u16();
                        let text = resp.text().await.unwrap_or_default();
                        if attempt < self.max_retries {
                            tokio::time::sleep(with_jitter(backoff_delay(self.base_delay, attempt))).await;
                            continue;
                        }
                        return Err(format!("API Fehler {}: {}", status, text));
                    }
                    let data: ChatResponse = resp
                        .json()
                        .await
                        .map_err(|e| format!("Parse Fehler: {}", e))?;
                    // Defensive: a malformed/empty upstream response must not panic.
                    return data
                        .choices
                        .into_iter()
                        .next()
                        .map(|c| c.message.content)
                        .ok_or_else(|| "Leere AI-Antwort (keine choices)".to_string());
                }
                Err(e) => {
                    if attempt < self.max_retries {
                        tokio::time::sleep(with_jitter(backoff_delay(self.base_delay, attempt))).await;
                        continue;
                    }
                    return Err(format!(
                        "Verbindungsfehler nach {} Versuchen: {}",
                        self.max_retries, e
                    ));
                }
            }
        }
        Err("Maximale Wiederholungen ueberschritten".into())
    }

    pub fn stream_completion(
        self: Arc<Self>,
        system_prompt: String,
        user_prompt: String,
        temperature: Option<f32>,
    ) -> impl Stream<Item = Result<String, String>> {
        let config = self.config.clone();
        let http = self.http.clone();
        let max_retries = self.max_retries;
        let base_delay = self.base_delay;

        stream! {
            let url = format!("{}/chat/completions", config.url.trim_end_matches('/'));

            // Truncate user prompt to prevent context overflow
            let user_prompt = truncate_prompt(&user_prompt, MAX_USER_PROMPT_BYTES).into_owned();

            let body = ChatRequest {
                model: config.model,
                messages: vec![
                    ChatMessage { role: "system".into(), content: system_prompt },
                    ChatMessage { role: "user".into(), content: user_prompt },
                ],
                stream: true,
                temperature: temperature.unwrap_or(config.temperature),
                max_tokens: config.max_tokens,
            };

            'retry: for attempt in 0..=max_retries {
                let resp = match http
                    .post(&url)
                    .bearer_auth(&config.api_key)
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        if attempt < max_retries {
                            tokio::time::sleep(backoff_delay(base_delay, attempt)).await;
                            continue;
                        }
                        yield Err(format!("Senden fehlgeschlagen: {}", e));
                        return;
                    }
                };

                if resp.status() == 429 {
                    let delay = backoff_delay(base_delay, attempt);
                    tokio::time::sleep(delay).await;
                    continue 'retry;
                }

                if !resp.status().is_success() {
                    yield Err(format!("API Fehler {}", resp.status()));
                    return;
                }

                let mut stream = resp.bytes_stream();
                let mut buffer = String::new();
                const MAX_BUF: usize = 1_048_576;
                while let Some(chunk_result) = futures::StreamExt::next(&mut stream).await {
                    match chunk_result {
                        Ok(bytes) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                            if buffer.len() > MAX_BUF {
                                yield Err("Stream-Buffer überschritten (max 1MB)".into());
                                return;
                            }
                            while let Some(pos) = buffer.find('\n') {
                                let line = buffer[..pos].to_string();
                                buffer = buffer[pos + 1..].to_string();
                                let trimmed = line.trim_start_matches("data: ").trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                if trimmed == "[DONE]" {
                                    return;
                                }
                                if let Ok(chunk) = serde_json::from_str::<StreamChunk>(trimmed) {
                                    for choice in chunk.choices {
                                        if let Some(content) = choice.delta.content {
                                            yield Ok(content);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if attempt < max_retries {
                                tokio::time::sleep(backoff_delay(base_delay, attempt)).await;
                                continue 'retry;
                            }
                            yield Err(format!("Stream Fehler: {}", e));
                            return;
                        }
                    }
                }
                return;
            }
        }
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

/// Truncate a prompt to UTF-8 bytes, logging a warning if truncated.
/// Returns borrowed reference when no truncation is needed (zero allocation).
#[inline]
fn truncate_prompt<'a>(prompt: &'a str, max_bytes: usize) -> Cow<'a, str> {
    if prompt.len() <= max_bytes {
        return Cow::Borrowed(prompt);
    }
    tracing::warn!(
        "AI-Prompt auf {} Bytes gekürzt (war {} Bytes)",
        max_bytes,
        prompt.len()
    );
    let mut truncated = String::new();
    let mut bytes = 0;
    for ch in prompt.chars() {
        let ch_len = ch.len_utf8();
        if bytes + ch_len > max_bytes {
            break;
        }
        bytes += ch_len;
        truncated.push(ch);
    }
    truncated.push_str("\n\n[... Nachricht gekürzt, zu lang für KI-Verarbeitung]");
    Cow::Owned(truncated)
}

#[inline]
fn with_jitter(delay: std::time::Duration) -> std::time::Duration {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    delay + std::time::Duration::from_millis(ns % 250)
}

/// Exponential backoff that cannot overflow regardless of `attempt`.
/// Caps the multiplier at 2^6 (64x) to bound both overflow and wait time.
#[inline]
fn backoff_delay(base: std::time::Duration, attempt: u32) -> std::time::Duration {
    let factor = 2u32.saturating_pow(attempt.min(6));
    base.saturating_mul(factor)
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}
