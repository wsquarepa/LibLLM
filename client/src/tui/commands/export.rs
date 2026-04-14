use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

use anyhow::Result;

use libllm::session;

use super::App;

enum ExportFormat {
    Markdown,
    Html,
    Jsonl,
}

impl ExportFormat {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "" | "html" => Ok(Self::Html),
            "md" | "markdown" => Ok(Self::Markdown),
            "jsonl" | "json" => Ok(Self::Jsonl),
            other => Err(format!("Unknown export format: {other}. Use md, html, or jsonl")),
        }
    }

    fn extension(&self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Jsonl => "jsonl",
        }
    }
}

pub(in crate::tui::commands) fn cmd_export(app: &mut App, arg: &str) {
    let format = match ExportFormat::parse(arg.trim()) {
        Ok(f) => f,
        Err(err) => {
            app.set_status(err, super::super::StatusLevel::Error);
            return;
        }
    };

    let messages = app.session.tree.branch_path();
    if messages.is_empty() {
        app.set_status(
            "Nothing to export (empty conversation)".to_owned(),
            super::super::StatusLevel::Warning,
        );
        return;
    }

    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            app.set_status(
                format!("Cannot resolve current directory: {err}"),
                super::super::StatusLevel::Error,
            );
            return;
        }
    };

    let char_name = app.session.character.as_deref().unwrap_or("Assistant");
    let user_name = app.active_persona_name.as_deref().unwrap_or("User");

    let content = match format {
        ExportFormat::Markdown => libllm::export::render_markdown(&messages, char_name, user_name),
        ExportFormat::Html => libllm::export::render_html(&messages, char_name, user_name),
        ExportFormat::Jsonl => libllm::export::render_jsonl(&messages, char_name, user_name),
    };

    let timestamp = session::now_compact();
    let filename = format!("export-{timestamp}.{}", format.extension());
    let output_path = current_dir.join(&filename);

    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&output_path)
        .and_then(|mut f| f.write_all(content.as_bytes()))
    {
        Ok(()) => app.set_status(
            format!("Exported to {}", output_path.display()),
            super::super::StatusLevel::Info,
        ),
        Err(err) => app.set_status(
            format!("Failed to write export: {err}"),
            super::super::StatusLevel::Error,
        ),
    }
}
