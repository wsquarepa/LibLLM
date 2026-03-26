use std::io::Write;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::sampling::SamplingParams;

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

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    pub async fn fetch_model_name(&self) -> String {
        let url = format!("{}/models", self.base_url);
        let result: Result<String> = async {
            let resp = self.client.get(&url).send().await.context("GET /models failed")?;
            let body: serde_json::Value = resp.json().await.context("failed to parse /models response")?;
            body["data"][0]["id"]
                .as_str()
                .map(String::from)
                .context("no model id in response")
        }
        .await;

        result.unwrap_or_else(|_| "unknown model".to_owned())
    }

    pub async fn stream_completion(
        &self,
        prompt: &str,
        stop_tokens: &[&str],
        sampling: &SamplingParams,
        writer: &mut impl Write,
    ) -> Result<String> {
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

        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::<u8>::new();
        let mut line_buf = String::new();
        let mut full_response = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            buffer.extend_from_slice(&chunk);

            while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                line_buf.clear();
                let line_bytes = &buffer[..newline_pos];
                line_buf.push_str(&String::from_utf8_lossy(line_bytes));
                buffer.drain(..=newline_pos);

                let line = line_buf.trim();
                if line.is_empty() {
                    continue;
                }

                let data = line.strip_prefix("data: ").unwrap_or(line);
                if data == "[DONE]" {
                    continue;
                }

                let event: SseEvent = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(text) = event.choices.first().and_then(|c| c.text.as_deref()) {
                    if !text.is_empty() {
                        write!(writer, "{text}")?;
                        writer.flush()?;
                        full_response.push_str(text);
                    }
                }
            }
        }

        Ok(full_response)
    }
}
