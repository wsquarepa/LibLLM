//! End-to-end pre-send token-counting behavior against a mock llama.cpp server.

#[expect(
    dead_code,
    reason = "each test binary uses a different subset of common helpers"
)]
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use libllm_protocol::client::ApiClient;
use libllm_protocol::tokenizer::TokenCounter;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn count_authoritative_calls_server_on_miss_and_caches_on_hit() {
    let server = MockServer::start().await;
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(move |_: &wiremock::Request| {
            hits_clone.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": vec![0u32; 7]
            }))
        })
        .mount(&server)
        .await;

    let base = format!("{}/v1", server.uri());
    let client = ApiClient::new(&base, false, libllm_core::config::Auth::None);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let counter = TokenCounter::new(client, tx).await;
    let hits_after_probe = hits.load(Ordering::SeqCst);

    let n = counter.count_authoritative("same text").await.unwrap();
    assert_eq!(n, 7);
    assert_eq!(hits.load(Ordering::SeqCst), hits_after_probe + 1);

    // Second call with same text should be cached.
    let n2 = counter.count_authoritative("same text").await.unwrap();
    assert_eq!(n2, 7);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        hits_after_probe + 1,
        "a cache hit must not make another HTTP call"
    );
}
