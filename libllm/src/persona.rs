//! User persona profiles for character roleplay sessions.

use serde::{Deserialize, Serialize};

/// A user persona with a display name and freeform persona description text.
#[derive(Debug, Serialize, Deserialize)]
pub struct PersonaFile {
    pub name: String,
    pub persona: String,
}
