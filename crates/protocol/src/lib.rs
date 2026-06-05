//! HTTP protocol layer for LibLLM: the llama.cpp API client, tokenizer, and
//! summarization orchestration.

pub mod client;
pub mod crypto_provider;
pub mod summarize;
pub mod tokenizer;
