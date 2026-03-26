use std::io::{Write, stdout};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use termimad::MadSkin;

use crate::client::ApiClient;
use crate::context::{ContextManager, ContextStatus};
use crate::prompt::Template;
use crate::sampling::SamplingParams;
use crate::session::{self, Message, Role, Session};

const GREEN_BOLD: &str = "\x1b[1;32m";
const BLUE_BOLD: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

struct ChatContext<'a> {
    client: &'a ApiClient,
    session: &'a mut Session,
    session_path: Option<&'a Path>,
    template: Template,
    stop_tokens: &'static [&'static str],
    sampling: &'a SamplingParams,
    context_mgr: ContextManager,
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    session_path: Option<&Path>,
    template: Template,
    sampling: &SamplingParams,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    println!("Chat with {model_name} (Ctrl+C to quit, /help for commands)\n");

    let mut editor = DefaultEditor::new()?;

    let mut ctx = ChatContext {
        client,
        session,
        session_path,
        template,
        stop_tokens: template.stop_tokens(),
        sampling,
        context_mgr: ContextManager::default(),
    };

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
            if handle_command(trimmed, &mut ctx).await? {
                break;
            }
            continue;
        }

        send_message(trimmed, &mut ctx).await?;
    }

    Ok(())
}

async fn send_message(content: &str, ctx: &mut ChatContext<'_>) -> Result<()> {
    ctx.session.messages.push(Message::new(Role::User, content.to_owned()));

    let removed = ctx.context_mgr.truncate(&mut ctx.session.messages);
    if removed > 0 {
        println!("{YELLOW}[context truncated: ~{removed} tokens of old messages removed]{RESET}");
    } else if let ContextStatus::Warning { used, limit } = ctx.context_mgr.check(&ctx.session.messages) {
        println!("{YELLOW}[context usage: ~{used}/{limit} tokens]{RESET}");
    }

    let prompt = ctx.template.render(&ctx.session.messages, ctx.session.system_prompt.as_deref());

    print!("{BLUE_BOLD}Assistant:{RESET} ");
    stdout().flush()?;

    let response = ctx
        .client
        .stream_completion(&prompt, ctx.stop_tokens, ctx.sampling, &mut stdout().lock())
        .await?;

    println!();

    ctx.session.messages.push(Message::new(Role::Assistant, response));
    ctx.session.maybe_save(ctx.session_path)?;

    println!();
    Ok(())
}

/// Returns true if the REPL should quit.
async fn handle_command(input: &str, ctx: &mut ChatContext<'_>) -> Result<bool> {
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
        }
        "/clear" => {
            ctx.session.messages.clear();
            println!("{YELLOW}Conversation cleared.{RESET}");
            ctx.session.maybe_save(ctx.session_path)?;
        }
        "/save" => {
            if arg.is_empty() {
                match ctx.session_path {
                    Some(path) => {
                        session::save(path, ctx.session)?;
                        println!("{YELLOW}Session saved to {}.{RESET}", path.display());
                    }
                    None => println!("{YELLOW}Usage: /save <path>{RESET}"),
                }
            } else {
                let path = PathBuf::from(arg);
                session::save(&path, ctx.session)?;
                println!("{YELLOW}Session saved to {arg}.{RESET}");
            }
        }
        "/load" => {
            if arg.is_empty() {
                println!("{YELLOW}Usage: /load <path>{RESET}");
            } else {
                let path = PathBuf::from(arg);
                *ctx.session = session::load(&path)?;
                println!(
                    "{YELLOW}Session loaded from {arg} ({} messages).{RESET}",
                    ctx.session.messages.len()
                );
            }
        }
        "/model" => {
            let model = ctx.client.fetch_model_name().await;
            println!("{YELLOW}Current model: {model}{RESET}");
        }
        "/system" => {
            if arg.is_empty() {
                match &ctx.session.system_prompt {
                    Some(sp) => println!("{YELLOW}Current system prompt: {sp}{RESET}"),
                    None => println!("{YELLOW}No system prompt set. Usage: /system <prompt>{RESET}"),
                }
            } else {
                ctx.session.system_prompt = Some(arg.to_owned());
                println!("{YELLOW}System prompt updated.{RESET}");
                ctx.session.maybe_save(ctx.session_path)?;
            }
        }
        "/retry" => {
            ctx.session.pop_trailing_assistant();

            let last_user = ctx
                .session
                .messages
                .iter()
                .rposition(|m| m.role == Role::User)
                .map(|i| ctx.session.messages.remove(i).content);

            match last_user {
                Some(content) => send_message(&content, ctx).await?,
                None => println!("{YELLOW}No user message to retry.{RESET}"),
            }
        }
        "/edit" => {
            if arg.is_empty() {
                println!("{YELLOW}Usage: /edit <new message text>{RESET}");
            } else {
                ctx.session.pop_trailing_assistant();
                if ctx.session.messages.last().is_some_and(|m| m.role == Role::User) {
                    ctx.session.messages.pop();
                }
                send_message(arg, ctx).await?;
            }
        }
        "/history" => {
            if ctx.session.messages.is_empty() {
                println!("{YELLOW}No messages in history.{RESET}");
            } else {
                for (i, msg) in ctx.session.messages.iter().enumerate() {
                    let role_color = match msg.role {
                        Role::User => GREEN_BOLD,
                        Role::Assistant => BLUE_BOLD,
                        Role::System => DIM,
                    };
                    let role_name = match msg.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::System => "system",
                    };
                    let truncated: String = msg.content.chars().take(100).collect();
                    let ellipsis = if msg.content.chars().count() > 100 { "..." } else { "" };
                    println!("{DIM}{:>3}.{RESET} {role_color}{role_name}{RESET}: {truncated}{ellipsis}", i + 1);
                }
            }
        }
        "/render" => {
            match ctx.session.messages.iter().rev().find(|m| m.role == Role::Assistant) {
                Some(msg) => {
                    println!("{BLUE_BOLD}Assistant:{RESET}");
                    MadSkin::default().print_text(&msg.content);
                }
                None => println!("{YELLOW}No assistant response to render.{RESET}"),
            }
        }
        "/quit" | "/exit" => return Ok(true),
        _ => println!("{YELLOW}Unknown command: {cmd}. Type /help for available commands.{RESET}"),
    }

    Ok(false)
}
