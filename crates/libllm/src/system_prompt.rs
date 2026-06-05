//! System prompt storage and builtin defaults for assistant and roleplay modes.

use serde::{Deserialize, Serialize};

pub const BUILTIN_ASSISTANT: &str = "assistant";
pub const BUILTIN_ROLEPLAY: &str = "roleplay";

pub const BUILTIN_ASSISTANT_CONTENT: &str = "";
pub const BUILTIN_ROLEPLAY_CONTENT: &str = "";

/// A named system prompt with its content text, used for both builtins and user-created prompts.
#[derive(Debug, Serialize, Deserialize)]
pub struct SystemPromptFile {
    pub name: String,
    pub content: String,
}
