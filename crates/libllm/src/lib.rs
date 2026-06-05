//! Facade crate that re-exports the LibLLM library crates under a single `libllm`
//! path. Application crates depend on this facade; the real implementations live
//! in `libllm-core` (pure domain), `libllm-storage` (database), and
//! `libllm-protocol` (HTTP client).

pub use libllm_core::{
    archive, author_note, character, commands, config, context, crypto, diagnostics, export, files,
    group_chat, migration, persona, preset, regex_rules, sampling, session, side_character,
    system_prompt, template, thought, worldinfo,
};

pub use libllm_core::timed_result;

pub use libllm_protocol::{client, crypto_provider, summarize, tokenizer};

pub use libllm_storage::{db, search};
