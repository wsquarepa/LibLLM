use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: String,
}

impl Message {
    pub fn new(role: Role, content: String) -> Self {
        Self {
            role,
            content,
            timestamp: now_iso8601(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Session {
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

impl Session {
    pub fn pop_trailing_assistant(&mut self) {
        while self.messages.last().is_some_and(|m| m.role == Role::Assistant) {
            self.messages.pop();
        }
    }

    pub fn maybe_save(&self, path: Option<&Path>) -> Result<()> {
        match path {
            Some(p) => save(p, self),
            None => Ok(()),
        }
    }
}

#[derive(Deserialize)]
struct LegacySession {
    prompt_history: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    template: Option<String>,
}

pub fn load(path: &Path) -> Result<Session> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
        Err(e) => return Err(e).context(format!("failed to read session file: {}", path.display())),
    };

    if let Ok(session) = serde_json::from_str::<Session>(&contents) {
        return Ok(session);
    }

    if let Ok(legacy) = serde_json::from_str::<LegacySession>(&contents) {
        return Ok(Session {
            messages: vec![Message::new(Role::User, legacy.prompt_history)],
            model: legacy.model,
            template: legacy.template,
            system_prompt: None,
        });
    }

    Ok(Session {
        messages: vec![Message::new(Role::User, contents)],
        model: None,
        template: None,
        system_prompt: None,
    })
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    let json = serde_json::to_string_pretty(session).context("failed to serialize session")?;
    std::fs::write(path, json).context(format!("failed to write session file: {}", path.display()))
}

fn now_iso8601() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (year, month, day) = days_to_ymd(secs / 86400);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: [u64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0u64;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u64 + 1;
            break;
        }
        days -= md;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
