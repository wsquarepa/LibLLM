use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PersonaFile {
    pub name: String,
    pub persona: String,
}
