//! Pure domain layer for LibLLM: conversation/session types, characters,
//! personas, world info, presets, the file-ingestion pipeline, crypto, and
//! configuration types. This crate performs no database access, no network
//! I/O, has no async runtime, and holds no process-global state. Data-loading
//! functions take explicit `&Path` inputs; outer crates (`libllm-storage`,
//! `libllm-protocol`, `libllm-config`, `libllm-diagnostics`) build
//! infrastructure concerns on top of it.

// Conversation domain
pub mod group_chat;
pub mod session;
pub mod side_character;
pub mod thought;

// Content and personas
pub mod character;
pub mod persona;
pub mod system_prompt;
pub mod worldinfo;

// Prompt assembly
pub mod author_note;
pub mod commands;
pub mod context;
pub mod preset;
pub mod regex_rules;
pub mod sampling;
pub mod template;

// Configuration
pub mod config;

// Data pipeline and persistence support
pub mod archive;
pub mod crypto;
pub mod export;
pub mod files;

mod timing;
