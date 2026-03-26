use std::io::Write;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::json;

pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
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
        stop_tokens: &[String],
        writer: &mut impl Write,
    ) -> Result<String> {
        let url = format!("{}/completions", self.base_url);
        let body = json!({
            "prompt": prompt,
            "stream": true,
            "temperature": 0.8,
            "max_tokens": -1,
            "top_k": 40,
            "top_p": 0.95,
            "min_p": 0.05,
            "repeat_last_n": 64,
            "repeat_penalty": 1.0,
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
        let mut full_response = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream read error")?;
            buffer.extend_from_slice(&chunk);

            while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buffer.drain(..=newline_pos).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                let line = line.trim();

                if line.is_empty() {
                    continue;
                }

                let data = line.strip_prefix("data: ").unwrap_or(line);

                if data == "[DONE]" {
                    continue;
                }

                let parsed: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(text) = parsed["choices"][0]["text"].as_str() {
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
