use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A scripted completion response: a sequence of tokens, or an error body with HTTP 503.
enum Scripted {
    Tokens(Vec<String>),
    Error(String),
}

#[derive(Clone, Default)]
struct Queue(Arc<Mutex<VecDeque<Scripted>>>);

struct CompletionResponder {
    queue: Queue,
}

impl Respond for CompletionResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        match self
            .queue
            .0
            .lock()
            .expect("completion queue lock")
            .pop_front()
        {
            Some(Scripted::Tokens(toks)) => {
                ResponseTemplate::new(200).set_body_string(sse_body(&toks))
            }
            Some(Scripted::Error(msg)) => ResponseTemplate::new(503).set_body_string(msg),
            None => ResponseTemplate::new(200).set_body_string(sse_body(&[])),
        }
    }
}

/// Builds an SSE body matching the format `ApiClient` parses: one `data:` line per token,
/// terminated by `data: [DONE]`. wiremock delivers the whole body at once; the client's
/// line-by-line parser handles that correctly because each line ends with `\n`.
fn sse_body(tokens: &[String]) -> String {
    let mut out = String::new();
    for t in tokens {
        let payload = serde_json::json!({ "choices": [{ "text": t }] });
        out.push_str(&format!("data: {payload}\n\n"));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

/// A wiremock server impersonating the llama.cpp endpoints the TUI calls at startup and
/// during inference. Completion responses are scripted via the queue; all other endpoints
/// return fixed deterministic bodies so any probe or tokenize call the TUI issues succeeds.
pub struct MockApi {
    server: MockServer,
    queue: Queue,
}

impl MockApi {
    /// Starts the mock server and mounts all required endpoint handlers.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        let queue = Queue::default();

        Mock::given(method("POST"))
            .and(path("/v1/completions"))
            .respond_with(CompletionResponder {
                queue: queue.clone(),
            })
            .mount(&server)
            .await;

        // `fetch_model_name` reads `body["data"][0]["id"]` as a string.
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "id": "test-model" }]
            })))
            .mount(&server)
            .await;

        // `fetch_server_context_size` reads `body["default_generation_settings"]["n_ctx"]`.
        // `fetch_server_chat_template` reads `body["chat_template"]`.
        // Both call GET /props (relative to the server root, not /v1).
        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "chat_template": "{% for m in messages %}{{ m.role }}: {{ m.content }}\n{% endfor %}",
                "default_generation_settings": { "n_ctx": 4096 }
            })))
            .mount(&server)
            .await;

        // `tokenize` reads `body["tokens"]` as an array and returns its length.
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": [1, 2, 3]
            })))
            .mount(&server)
            .await;

        Self { server, queue }
    }

    /// Returns the base URL for `ApiClient::new`, pointing at `/v1` on the mock server.
    pub fn base_url(&self) -> String {
        format!("{}/v1", self.server.uri())
    }

    /// Enqueues a scripted success response. The next completion request will stream these
    /// tokens in order, followed by `[DONE]`.
    pub fn enqueue_completion(&self, tokens: &[&str]) {
        self.queue
            .0
            .lock()
            .expect("completion queue lock")
            .push_back(Scripted::Tokens(
                tokens.iter().map(|s| s.to_string()).collect(),
            ));
    }

    /// Enqueues a scripted error response (HTTP 503). The next completion request will
    /// fail with this message as the response body.
    pub fn enqueue_error(&self, msg: &str) {
        self.queue
            .0
            .lock()
            .expect("completion queue lock")
            .push_back(Scripted::Error(msg.to_owned()));
    }
}
