use std::io::{Write, stdout};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::client::ApiClient;
use crate::context::{ContextManager, ContextStatus};
use crate::prompt::PromptTemplate;
use crate::sampling::SamplingParams;
use crate::session::{self, Message, Session};

const GREEN_BOLD: &str = "\x1b[1;32m";
const BLUE_BOLD: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

enum CommandResult {
    Continue,
    Handled,
    Quit,
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    session_path: Option<&Path>,
    template: &dyn PromptTemplate,
    sampling: &SamplingParams,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    println!("Chat with {model_name} (Ctrl+C to quit, /help for commands)\n");

    let mut editor = DefaultEditor::new()?;
    let stop_tokens = template.stop_tokens();
    let context_mgr = ContextManager::default();

    loop {
        let input = match editor.readline(&format!("{GREEN_BOLD}You:{RESET} ")) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        editor.add_history_entry(trimmed)?;

        if trimmed.starts_with('/') {
            match handle_command(trimmed, client, session, session_path, template, sampling, &context_mgr).await? {
                CommandResult::Continue => continue,
                CommandResult::Handled => continue,
                CommandResult::Quit => break,
            }
        }

        send_message(trimmed, client, session, session_path, template, &stop_tokens, sampling, &context_mgr).await?;
    }

    Ok(())
}

async fn send_message(
    content: &str,
    client: &ApiClient,
    session: &mut Session,
    session_path: Option<&Path>,
    template: &dyn PromptTemplate,
    stop_tokens: &[String],
    sampling: &SamplingParams,
    context_mgr: &ContextManager,
) -> Result<()> {
    session.messages.push(Message::new("user", content));

    match context_mgr.check_and_truncate(&mut session.messages) {
        ContextStatus::Truncated { removed_count } => {
            println!("{YELLOW}[context truncated: ~{removed_count} tokens of old messages removed]{RESET}");
        }
        ContextStatus::Warning { used, limit } => {
            println!("{YELLOW}[context usage: ~{used}/{limit} tokens]{RESET}");
        }
        ContextStatus::Ok => {}
    }

    let prompt = template.render(&session.messages, session.system_prompt.as_deref());

    print!("{BLUE_BOLD}Assistant:{RESET} ");
    stdout().flush()?;

    let response = client
        .stream_completion(&prompt, stop_tokens, sampling, &mut stdout().lock())
        .await?;

    println!();

    session.messages.push(Message::new("assistant", &response));

    if let Some(path) = session_path {
        session::save(path, session)?;
    }

    println!();
    Ok(())
}

async fn handle_command(
    input: &str,
    client: &ApiClient,
    session: &mut Session,
    session_path: Option<&Path>,
    template: &dyn PromptTemplate,
    sampling: &SamplingParams,
    context_mgr: &ContextManager,
) -> Result<CommandResult> {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

    match cmd {
        "/help" => {
            println!("{DIM}Available commands:");
            println!("  /help                Show this help message");
            println!("  /clear               Clear conversation history");
            println!("  /save <path>         Save session to file");
            println!("  /load <path>         Load session from file");
            println!("  /model               Show current model name");
            println!("  /system <prompt>     Set system prompt");
            println!("  /retry               Regenerate last assistant response");
            println!("  /edit <text>          Replace last user message and regenerate");
            println!("  /history             Show conversation history");
            println!("  /render              Re-render last response with markdown formatting");
            println!("  /quit                Exit the chat{RESET}");
            Ok(CommandResult::Handled)
        }
        "/clear" => {
            session.messages.clear();
            println!("{YELLOW}Conversation cleared.{RESET}");
            if let Some(path) = session_path {
                session::save(path, session)?;
            }
            Ok(CommandResult::Handled)
        }
        "/save" => {
            if arg.is_empty() {
                match session_path {
                    Some(path) => {
                        session::save(path, session)?;
                        println!("{YELLOW}Session saved to {}.{RESET}", path.display());
                    }
                    None => println!("{YELLOW}Usage: /save <path>{RESET}"),
                }
            } else {
                let path = PathBuf::from(arg);
                session::save(&path, session)?;
                println!("{YELLOW}Session saved to {arg}.{RESET}");
            }
            Ok(CommandResult::Handled)
        }
        "/load" => {
            if arg.is_empty() {
                println!("{YELLOW}Usage: /load <path>{RESET}");
            } else {
                let path = PathBuf::from(arg);
                *session = session::load(&path)?;
                println!("{YELLOW}Session loaded from {arg} ({} messages).{RESET}", session.messages.len());
            }
            Ok(CommandResult::Handled)
        }
        "/model" => {
            let model = client.fetch_model_name().await;
            println!("{YELLOW}Current model: {model}{RESET}");
            Ok(CommandResult::Handled)
        }
        "/system" => {
            if arg.is_empty() {
                match &session.system_prompt {
                    Some(sp) => println!("{YELLOW}Current system prompt: {sp}{RESET}"),
                    None => println!("{YELLOW}No system prompt set. Usage: /system <prompt>{RESET}"),
                }
            } else {
                session.system_prompt = Some(arg.to_owned());
                println!("{YELLOW}System prompt updated.{RESET}");
                if let Some(path) = session_path {
                    session::save(path, session)?;
                }
            }
            Ok(CommandResult::Handled)
        }
        "/retry" => {
            while session.messages.last().is_some_and(|m| m.role == "assistant") {
                session.messages.pop();
            }

            let last_user = session
                .messages
                .iter()
                .rposition(|m| m.role == "user")
                .map(|i| session.messages.remove(i).content);

            match last_user {
                Some(content) => {
                    let stop_tokens = template.stop_tokens();
                    send_message(&content, client, session, session_path, template, &stop_tokens, sampling, context_mgr).await?;
                }
                None => println!("{YELLOW}No user message to retry.{RESET}"),
            }
            Ok(CommandResult::Handled)
        }
        "/edit" => {
            if arg.is_empty() {
                println!("{YELLOW}Usage: /edit <new message text>{RESET}");
                return Ok(CommandResult::Handled);
            }

            while session.messages.last().is_some_and(|m| m.role == "assistant") {
                session.messages.pop();
            }
            if session.messages.last().is_some_and(|m| m.role == "user") {
                session.messages.pop();
            }

            let stop_tokens = template.stop_tokens();
            send_message(arg, client, session, session_path, template, &stop_tokens, sampling, context_mgr).await?;
            Ok(CommandResult::Handled)
        }
        "/history" => {
            if session.messages.is_empty() {
                println!("{YELLOW}No messages in history.{RESET}");
            } else {
                for (i, msg) in session.messages.iter().enumerate() {
                    let role_color = match msg.role.as_str() {
                        "user" => GREEN_BOLD,
                        "assistant" => BLUE_BOLD,
                        _ => DIM,
                    };
                    let truncated = if msg.content.len() > 100 {
                        format!("{}...", &msg.content[..100])
                    } else {
                        msg.content.clone()
                    };
                    println!("{DIM}{:>3}.{RESET} {role_color}{}{RESET}: {}", i + 1, msg.role, truncated);
                }
            }
            Ok(CommandResult::Handled)
        }
        "/render" => {
            match session.messages.iter().rev().find(|m| m.role == "assistant") {
                Some(msg) => {
                    println!("{BLUE_BOLD}Assistant:{RESET}");
                    crate::render::render_markdown(&msg.content);
                }
                None => println!("{YELLOW}No assistant response to render.{RESET}"),
            }
            Ok(CommandResult::Handled)
        }
        "/quit" | "/exit" => Ok(CommandResult::Quit),
        _ => {
            println!("{YELLOW}Unknown command: {cmd}. Type /help for available commands.{RESET}");
            Ok(CommandResult::Continue)
        }
    }
}
