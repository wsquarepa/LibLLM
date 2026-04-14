use serde::{Deserialize, Serialize};

pub const BUILTIN_ASSISTANT: &str = "assistant";
pub const BUILTIN_ROLEPLAY: &str = "roleplay";

pub const BUILTIN_ASSISTANT_CONTENT: &str = "";
pub const BUILTIN_ROLEPLAY_CONTENT: &str = "";

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemPromptFile {
    pub name: String,
    pub content: String,
}
