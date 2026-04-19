//! Shared library for LibLLM: types, database access, API client, and preset management.

pub mod character;
pub mod client;
pub mod commands;
pub mod config;
pub mod context;
pub mod crypto;
pub mod crypto_provider;
pub mod db;
pub mod diagnostics;
pub mod export;
pub mod migration;
pub mod persona;
pub mod preset;
pub mod sampling;
pub mod session;
pub mod summarize;
pub mod system_prompt;
pub mod template;
pub mod worldinfo;
