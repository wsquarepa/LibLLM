//! Pure domain layer for LibLLM: conversation/session types, characters,
//! personas, world info, presets, the file-ingestion pipeline, crypto, and
//! configuration types. This crate performs no database access, no network I/O,
//! and holds no process-global state; outer crates (`libllm-storage`,
//! `libllm-protocol`, `libllm-config`) build those concerns on top of it.

pub mod archive;
pub mod author_note;
pub mod character;
pub mod commands;
pub mod config;
pub mod context;
pub mod crypto;
pub mod diagnostics;
pub mod export;
pub mod files;
pub mod group_chat;
pub mod persona;
pub mod preset;
pub mod regex_rules;
pub mod sampling;
pub mod session;
pub mod side_character;
pub mod system_prompt;
pub mod template;
pub mod thought;
pub mod worldinfo;
