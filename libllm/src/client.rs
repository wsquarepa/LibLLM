//! HTTP client for the llama.cpp completions API with streaming support.

use std::io::Write;

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
    pub fn new(base_url: &str, tls_skip_verify: bool) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(tls_skip_verify)
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// Queries `GET /models` and returns the first model ID, or an error string on failure.
    pub async fn fetch_model_name(&self) -> std::result::Result<String, String> {
        let url = format!("{}/models", self.base_url);
        let result: Result<String> = async {
            let resp = self
                .client
                .get(&url)
                .timeout(std::time::Duration::from_secs(5))
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
        let resp = self.start_completion(prompt, stop_tokens, sampling).await?;
        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::<u8>::new();
        let mut consumed = 0usize;
        let mut full_response = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            buffer.extend_from_slice(&chunk);

            while let Some((line_start, line_end)) = next_line_bounds(&buffer, consumed) {
                let line_bytes = &buffer[line_start..line_end];
                consumed = line_end + 1;

                if let Some(text) = parse_token_line(line_bytes) {
                    full_response.push_str(&text);
                    if sender.send(StreamToken::Token(text)).await.is_err() {
                        return Ok(full_response);
                    }
                }
            }

            if consumed > 0 {
                buffer.drain(..consumed);
                consumed = 0;
            }
        }

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

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("POST /completions (non-streaming) failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("API returned {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await.context("failed to parse response JSON")?;
        let content = json["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok(content)
    }

    /// Queries the llama.cpp server for its context size (`n_ctx`).
    /// Returns `None` if the server doesn't support the endpoint or the field is missing.
    pub async fn fetch_server_context_size(&self) -> Option<usize> {
        let url = format!("{}/props", self.base_url.trim_end_matches("/v1"));
        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let json: serde_json::Value = resp.json().await.ok()?;
        json["default_generation_settings"]["n_ctx"]
            .as_u64()
            .map(|n| n as usize)
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
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("POST /completions failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
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

            if let Some(text) = parse_token_line(line_bytes) {
                if !on_token(&text)? {
                    return Ok(());
                }
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
