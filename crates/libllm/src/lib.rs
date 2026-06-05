//! Facade crate that re-exports the LibLLM library crates under a single `libllm`
//! path. Application crates depend on this facade; the real implementations live
//! in `libllm-core` (pure domain), `libllm-storage` (database), and
//! `libllm-protocol` (HTTP client).

pub use libllm_core::{
    archive, author_note, character, client, commands, config, context, crypto, crypto_provider,
    db, diagnostics, export, files, group_chat, migration, persona, preset, regex_rules, sampling,
    search, session, side_character, summarize, system_prompt, template, thought, tokenizer,
    worldinfo,
};

pub use libllm_core::timed_result;
