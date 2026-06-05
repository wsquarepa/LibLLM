//! Facade crate that re-exports the LibLLM library crates under a single `libllm`
//! path. Application crates depend on this facade; the real implementations live
//! in `libllm-core` (pure domain), `libllm-config` (config I/O), `libllm-storage`
//! (database), and `libllm-protocol` (HTTP client).

pub use libllm_core::{
    archive, author_note, character, commands, context, crypto, diagnostics, export, files,
    group_chat, persona, preset, regex_rules, sampling, session, side_character, system_prompt,
    template, thought, worldinfo,
};

pub use libllm_core::timed_result;

pub mod config {
    pub use libllm_config::*;
    pub use libllm_core::config::*;
}

pub mod migration {
    pub fn migrate_config_path() {
        libllm_config::migrate_config();
    }
}

pub use libllm_protocol::{client, crypto_provider, summarize, tokenizer};

pub use libllm_storage::{db, search};
