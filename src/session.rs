use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct Session {
    pub prompt_history: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

pub fn load(path: &Path) -> Result<Session> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
        Err(e) => return Err(e).context(format!("failed to read session file: {}", path.display())),
    };

    serde_json::from_str::<Session>(&contents).or_else(|_| {
        Ok(Session {
            prompt_history: contents,
            model: None,
            template: None,
        })
    })
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    let json = serde_json::to_string_pretty(session).context("failed to serialize session")?;
    std::fs::write(path, json).context(format!("failed to write session file: {}", path.display()))
}
