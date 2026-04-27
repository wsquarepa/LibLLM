//! HTTP client for the llama.cpp completions API with streaming support.

use std::io::Write;
use std::time::Instant;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::sampling::SamplingParams;

/// HTTP client for the llama.cpp `/completions` and `/models` endpoints.
#[derive(Clone)]
pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
    auth: crate::config::Auth,
}

#[derive(Deserialize)]
struct SseChoice {
    text: Option<String>,
}

#[derive(Deserialize)]
struct SseEvent {
    choices: Vec<SseChoice>,
}

/// A token event from a streaming completion response.
pub enum StreamToken {
    /// An incremental text fragment received during generation.
    Token(String),
    /// Generation completed; contains the full concatenated response text.
    Done(String),
    /// An error occurred during streaming; contains the error description.
    Error(String),
}

impl ApiClient {
    /// Creates a new client targeting the given base URL (e.g. `http://localhost:5001/v1`).
    ///
    /// When `tls_skip_verify` is true, TLS certificate validation is disabled.
    pub fn new(base_url: &str, tls_skip_verify: bool, auth: crate::config::Auth) -> Self {
        crate::crypto_provider::install_default_crypto_provider();
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(tls_skip_verify)
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            auth,
        }
    }

    /// Queries `GET /models` and returns the first model ID, or an error string on failure.
    pub async fn fetch_model_name(&self) -> std::result::Result<String, String> {
        let url = format!("{}/models", self.base_url);
        let start = Instant::now();
        let result: Result<String> = async {
            let resp = self
                .auth
                .apply(
                    self.client
                        .get(&url)
                        .timeout(std::time::Duration::from_secs(5)),
                )
                .context("apply auth")?
                .send()
                .await
                .context("GET /models failed")?;
            let body: serde_json::Value = resp
                .json()
                .await
                .context("failed to parse /models response")?;
            body["data"][0]["id"]
                .as_str()
                .map(String::from)
                .context("no model id in response")
        }
        .await;

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        match &result {
            Ok(name) => tracing::info!(
                phase = "request",
                result = "ok",
                elapsed_ms = elapsed_ms,
                model_name_bytes = name.len(),
                "client.models"
            ),
            Err(err) => tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                error = %err,
                "client.models"
            ),
        }

        result.map_err(|e| e.to_string())
    }

    /// Streams a completion request, writing each token to `writer` as it arrives.
    ///
    /// Returns the full concatenated response on success.
    pub async fn stream_completion(
        &self,
        prompt: &str,
        stop_tokens: &[&str],
        sampling: &SamplingParams,
        writer: &mut impl Write,
    ) -> Result<String> {
        let resp = self.start_completion(prompt, stop_tokens, sampling).await?;
        let mut full_response = String::new();
        consume_completion_stream(resp, |text| {
            write!(writer, "{text}")?;
            writer.flush()?;
            full_response.push_str(text);
            Ok(true)
        })
        .await?;
        Ok(full_response)
    }

    /// Streams a completion request, sending each token as a `StreamToken` over `sender`.
    ///
    /// Sends `StreamToken::Done` on success or `StreamToken::Error` on failure.
    pub async fn stream_completion_to_channel(
        &self,
        prompt: &str,
        stop_tokens: &[&str],
        sampling: &SamplingParams,
        sender: mpsc::Sender<StreamToken>,
    ) {
        let result = self
            .stream_completion_channel_inner(prompt, stop_tokens, sampling, &sender)
            .await;
        match result {
            Ok(full) => {
                let _ = sender.send(StreamToken::Done(full)).await;
            }
            Err(e) => {
                let _ = sender.send(StreamToken::Error(e.to_string())).await;
            }
        }
    }

    async fn stream_completion_channel_inner(
        &self,
        prompt: &str,
        stop_tokens: &[&str],
        sampling: &SamplingParams,
        sender: &mpsc::Sender<StreamToken>,
    ) -> Result<String> {
        let start = Instant::now();
        tracing::info!(
            phase = "start",
            prompt_bytes = prompt.len(),
            stop_token_count = stop_tokens.len(),
            max_tokens = sampling.max_tokens,
            temperature = sampling.temperature,
            "client.stream"
        );
        let resp = match self.start_completion(prompt, stop_tokens, sampling).await {
            Ok(resp) => resp,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "done",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.stream"
                );
                return Err(err);
            }
        };
        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::<u8>::new();
        let mut consumed = 0usize;
        let mut full_response = String::new();
        let mut token_chunks = 0usize;

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk.context("stream read error") {
                Ok(chunk) => chunk,
                Err(err) => {
                    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                    tracing::error!(
                        phase = "done",
                        result = "error",
                        elapsed_ms = elapsed_ms,
                        response_bytes = full_response.len(),
                        token_chunks = token_chunks,
                        error = %err,
                        "client.stream"
                    );
                    return Err(err);
                }
            };
            buffer.extend_from_slice(&chunk);

            while let Some((line_start, line_end)) = next_line_bounds(&buffer, consumed) {
                let line_bytes = &buffer[line_start..line_end];
                consumed = line_end + 1;

                if let Some(text) = parse_token_line(line_bytes) {
                    full_response.push_str(&text);
                    token_chunks += 1;
                    if sender.send(StreamToken::Token(text)).await.is_err() {
                        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                        tracing::info!(
                            phase = "done",
                            result = "ok",
                            reason = "receiver_dropped",
                            elapsed_ms = elapsed_ms,
                            response_bytes = full_response.len(),
                            token_chunks = token_chunks,
                            "client.stream"
                        );
                        return Ok(full_response);
                    }
                }
            }

            if consumed > 0 {
                buffer.drain(..consumed);
                consumed = 0;
            }
        }

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::info!(
            phase = "done",
            result = "ok",
            elapsed_ms = elapsed_ms,
            response_bytes = full_response.len(),
            token_chunks = token_chunks,
            "client.stream"
        );
        Ok(full_response)
    }

    /// Sends a non-streaming completion request and returns the full response content.
    pub async fn complete(
        &self,
        prompt: &str,
        stop_tokens: &[&str],
        sampling: &SamplingParams,
    ) -> Result<String> {
        let url = format!("{}/completions", self.base_url);
        let body = json!({
            "prompt": prompt,
            "stream": false,
            "temperature": sampling.temperature,
            "max_tokens": sampling.max_tokens,
            "top_k": sampling.top_k,
            "top_p": sampling.top_p,
            "min_p": sampling.min_p,
            "repeat_last_n": sampling.repeat_last_n,
            "repeat_penalty": sampling.repeat_penalty,
            "stop": stop_tokens,
            "samplers": ["top_k", "top_p", "min_p", "temperature"],
        });

        let start = Instant::now();
        tracing::info!(
            phase = "start",
            prompt_bytes = prompt.len(),
            stop_token_count = stop_tokens.len(),
            max_tokens = sampling.max_tokens,
            temperature = sampling.temperature,
            "client.complete"
        );
        let send_result = self
            .auth
            .apply(self.client.post(&url).json(&body))
            .context("apply auth")?
            .send()
            .await
            .context("POST /completions (non-streaming) failed");
        let resp = match send_result {
            Ok(resp) => resp,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "done",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.complete"
                );
                return Err(err);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "done",
                result = "error",
                elapsed_ms = elapsed_ms,
                status = status.as_u16(),
                body_bytes = text.len(),
                "client.complete"
            );
            anyhow::bail!("API returned {status}: {text}");
        }

        let json: serde_json::Value =
            match resp.json().await.context("failed to parse response JSON") {
                Ok(json) => json,
                Err(err) => {
                    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                    tracing::error!(
                        phase = "done",
                        result = "error",
                        elapsed_ms = elapsed_ms,
                        error = %err,
                        "client.complete"
                    );
                    return Err(err);
                }
            };
        let content = json["choices"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::info!(
            phase = "done",
            result = "ok",
            elapsed_ms = elapsed_ms,
            response_bytes = content.len(),
            "client.complete"
        );
        Ok(content)
    }

    /// Queries the llama.cpp server for its context size (`n_ctx`).
    /// Returns `None` if the server doesn't support the endpoint or the field is missing.
    pub async fn fetch_server_context_size(&self) -> Option<usize> {
        let url = format!("{}/props", self.base_url.trim_end_matches("/v1"));
        let start = Instant::now();
        let builder = match self.auth.apply(self.client.get(&url)) {
            Ok(b) => b,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "request",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.props"
                );
                return None;
            }
        };
        let resp = match builder.send().await {
            Ok(resp) => resp,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "request",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.props"
                );
                return None;
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                status = status.as_u16(),
                "client.props"
            );
            return None;
        }
        let json: serde_json::Value = match resp.json().await {
            Ok(json) => json,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "request",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.props"
                );
                return None;
            }
        };
        let n_ctx = json["default_generation_settings"]["n_ctx"]
            .as_u64()
            .map(|n| n as usize);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        match n_ctx {
            Some(size) => tracing::info!(
                phase = "request",
                result = "ok",
                elapsed_ms = elapsed_ms,
                n_ctx = size,
                "client.props"
            ),
            None => tracing::info!(
                phase = "request",
                result = "missing",
                elapsed_ms = elapsed_ms,
                "client.props"
            ),
        }
        n_ctx
    }

    /// Queries the server for its chat template (`chat_template` field of `/props`).
    /// Returns `None` if the field is missing/empty or the request fails.
    /// Strips trailing NUL bytes (a known llama.cpp quirk).
    pub async fn fetch_server_chat_template(&self) -> Option<String> {
        let url = format!("{}/props", self.base_url.trim_end_matches("/v1"));
        let start = Instant::now();
        let builder = match self.auth.apply(self.client.get(&url)) {
            Ok(b) => b,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "request",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.props.chat_template"
                );
                return None;
            }
        };
        let resp = match builder.send().await {
            Ok(resp) => resp,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "request",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.props.chat_template"
                );
                return None;
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                status = status.as_u16(),
                "client.props.chat_template"
            );
            return None;
        }
        let json: serde_json::Value = match resp.json().await {
            Ok(json) => json,
            Err(err) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                tracing::error!(
                    phase = "request",
                    result = "error",
                    elapsed_ms = elapsed_ms,
                    error = %err,
                    "client.props.chat_template"
                );
                return None;
            }
        };
        let template = json["chat_template"]
            .as_str()
            .map(|s| s.trim_end_matches('\0').to_owned());
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        match &template {
            Some(t) if !t.is_empty() => {
                tracing::info!(
                    phase = "request",
                    result = "ok",
                    elapsed_ms = elapsed_ms,
                    template_bytes = t.len(),
                    "client.props.chat_template"
                );
                Some(t.clone())
            }
            _ => {
                tracing::info!(
                    phase = "request",
                    result = "missing",
                    elapsed_ms = elapsed_ms,
                    "client.props.chat_template"
                );
                None
            }
        }
    }

    /// Calls `POST {server}/tokenize` with the given text and returns the token count.
    /// When `add_special` is true, BOS/EOS tokens are included in the count.
    pub async fn tokenize(&self, text: &str, add_special: bool) -> anyhow::Result<usize> {
        let url = format!("{}/tokenize", self.base_url.trim_end_matches("/v1"));
        let start = Instant::now();
        let body = serde_json::json!({
            "content": text,
            "add_special": add_special,
        });
        let builder = self
            .auth
            .apply(self.client.post(&url).json(&body))
            .map_err(|err| {
                tracing::error!(
                    phase = "auth",
                    result = "error",
                    error = %err,
                    "client.tokenize"
                );
                anyhow::anyhow!("auth apply failed: {err}")
            })?;

        let resp = builder.send().await.map_err(|err| {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                error = %err,
                "client.tokenize"
            );
            anyhow::anyhow!("tokenize request failed: {err}")
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                status = status.as_u16(),
                "client.tokenize"
            );
            return Err(anyhow::anyhow!(
                "tokenize returned HTTP {}",
                status.as_u16()
            ));
        }

        let json: serde_json::Value = resp.json().await.map_err(|err| {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                error = %err,
                "client.tokenize"
            );
            anyhow::anyhow!("tokenize response parse failed: {err}")
        })?;

        let count = json["tokens"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("tokenize response missing `tokens` array"))?
            .len();

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            phase = "request",
            result = "ok",
            elapsed_ms = elapsed_ms,
            count = count,
            "client.tokenize"
        );
        Ok(count)
    }

    /// Calls `POST {server}/api/extra/tokencount` (KoboldCPP) with the given text and returns the token count.
    /// KoboldCPP does not expose an `add_special` toggle; it always counts as the configured model does.
    pub async fn tokenize_kobold(&self, text: &str) -> anyhow::Result<usize> {
        let url = format!(
            "{}/api/extra/tokencount",
            self.base_url.trim_end_matches("/v1")
        );
        let start = Instant::now();
        let body = serde_json::json!({ "prompt": text });
        let builder = self
            .auth
            .apply(self.client.post(&url).json(&body))
            .map_err(|err| {
                tracing::error!(
                    phase = "auth",
                    result = "error",
                    error = %err,
                    "client.tokenize_kobold"
                );
                anyhow::anyhow!("auth apply failed: {err}")
            })?;

        let resp = builder.send().await.map_err(|err| {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                error = %err,
                "client.tokenize_kobold"
            );
            anyhow::anyhow!("tokenize_kobold request failed: {err}")
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                status = status.as_u16(),
                "client.tokenize_kobold"
            );
            return Err(anyhow::anyhow!(
                "tokenize_kobold returned HTTP {}",
                status.as_u16()
            ));
        }

        let json: serde_json::Value = resp.json().await.map_err(|err| {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(
                phase = "request",
                result = "error",
                elapsed_ms = elapsed_ms,
                error = %err,
                "client.tokenize_kobold"
            );
            anyhow::anyhow!("tokenize_kobold response parse failed: {err}")
        })?;

        let count = json["value"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("tokenize_kobold response missing numeric `value`"))?
            as usize;

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            phase = "request",
            result = "ok",
            elapsed_ms = elapsed_ms,
            count = count,
            "client.tokenize_kobold"
        );
        Ok(count)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn start_completion(
        &self,
        prompt: &str,
        stop_tokens: &[&str],
        sampling: &SamplingParams,
    ) -> Result<reqwest::Response> {
        let url = format!("{}/completions", self.base_url);
        let body = json!({
            "prompt": prompt,
            "stream": true,
            "temperature": sampling.temperature,
            "max_tokens": sampling.max_tokens,
            "top_k": sampling.top_k,
            "top_p": sampling.top_p,
            "min_p": sampling.min_p,
            "repeat_last_n": sampling.repeat_last_n,
            "repeat_penalty": sampling.repeat_penalty,
            "stop": stop_tokens,
            "samplers": ["top_k", "top_p", "min_p", "temperature"],
        });

        let resp = self
            .auth
            .apply(self.client.post(&url).json(&body))
            .context("apply auth")?
            .send()
            .await
            .context("POST /completions failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::error!(
                phase = "start",
                result = "error",
                status = status.as_u16(),
                body_bytes = text.len(),
                "client.stream"
            );
            anyhow::bail!("API returned {status}: {text}");
        }

        Ok(resp)
    }
}

async fn consume_completion_stream<F>(resp: reqwest::Response, mut on_token: F) -> Result<()>
where
    F: FnMut(&str) -> Result<bool>,
{
    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut consumed = 0usize;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("stream read error")?;
        buffer.extend_from_slice(&chunk);

        while let Some((line_start, line_end)) = next_line_bounds(&buffer, consumed) {
            let line_bytes = &buffer[line_start..line_end];
            consumed = line_end + 1;

            if let Some(text) = parse_token_line(line_bytes)
                && !on_token(&text)?
            {
                return Ok(());
            }
        }

        if consumed > 0 {
            buffer.drain(..consumed);
            consumed = 0;
        }
    }

    Ok(())
}

fn next_line_bounds(buffer: &[u8], start: usize) -> Option<(usize, usize)> {
    let rel_end = buffer.get(start..)?.iter().position(|&b| b == b'\n')?;
    Some((start, start + rel_end))
}

fn parse_token_line(line_bytes: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(line_bytes).ok()?.trim();
    if line.is_empty() {
        return None;
    }

    let data = line.strip_prefix("data: ").unwrap_or(line);
    if data == "[DONE]" {
        return None;
    }

    let event: SseEvent = serde_json::from_str(data).ok()?;
    let text = event.choices.first().and_then(|c| c.text.as_deref())?;
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn tokenize_returns_token_count() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": [1, 2, 3, 4, 5]
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        let count = client.tokenize("hello world", true).await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn tokenize_returns_error_on_server_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        assert!(client.tokenize("hello", true).await.is_err());
    }

    #[tokio::test]
    async fn tokenize_kobold_returns_value_count() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/extra/tokencount"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "value": 5,
                "ids": [1, 22177, 1044, 4304, 1033]
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        let count = client.tokenize_kobold("Hello, world!").await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn tokenize_kobold_returns_error_on_server_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/extra/tokencount"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        assert!(client.tokenize_kobold("hello").await.is_err());
    }

    #[tokio::test]
    async fn tokenize_kobold_errors_when_value_missing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/extra/tokencount"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ids": [1, 2, 3]
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        assert!(client.tokenize_kobold("hello").await.is_err());
    }

    #[tokio::test]
    async fn fetch_server_chat_template_returns_template_field() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "chat_template": "{% for m in messages %}{{ m.role }}{% endfor %}",
                "default_generation_settings": { "n_ctx": 8192 }
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        let template = client.fetch_server_chat_template().await;
        assert!(template.is_some());
        assert!(template.unwrap().contains("messages"));
    }

    #[tokio::test]
    async fn fetch_server_chat_template_returns_none_when_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "default_generation_settings": { "n_ctx": 8192 }
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        assert!(client.fetch_server_chat_template().await.is_none());
    }

    #[tokio::test]
    async fn fetch_server_chat_template_strips_trailing_nul() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "chat_template": "hello\u{0000}\u{0000}"
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);

        assert_eq!(
            client.fetch_server_chat_template().await.as_deref(),
            Some("hello")
        );
    }
}
