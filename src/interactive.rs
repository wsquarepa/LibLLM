use std::io::{Write, stdout};
use std::path::PathBuf;

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use termimad::MadSkin;

use crate::client::ApiClient;
use crate::context::{ContextManager, ContextStatus};
use crate::prompt::Template;
use crate::sampling::SamplingParams;
use crate::session::{self, Message, Role, SaveMode, Session};

const GREEN_BOLD: &str = "\x1b[1;32m";
const BLUE_BOLD: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

struct ChatContext<'a> {
    client: &'a ApiClient,
    session: &'a mut Session,
    save_mode: &'a SaveMode,
    template: Template,
    stop_tokens: &'static [&'static str],
    sampling: &'a SamplingParams,
    context_mgr: ContextManager,
}

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    save_mode: &SaveMode,
    template: Template,
    sampling: &SamplingParams,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    println!("Chat with {model_name} (Ctrl+C to quit, /help for commands)\n");

    let mut editor = DefaultEditor::new()?;

    let mut ctx = ChatContext {
        client,
        session,
        save_mode,
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
    let parent = ctx.session.tree.head();
    ctx.session.tree.push(parent, Message::new(Role::User, content.to_owned()));

    let branch_path = ctx.session.tree.branch_path();
    let truncated = ctx.context_mgr.truncated_path(&branch_path);

    if truncated.len() < branch_path.len() {
        let removed = branch_path.len() - truncated.len();
        println!("{YELLOW}[context truncated: {removed} old messages hidden]{RESET}");
    } else if let ContextStatus::Warning { used, limit } = ctx.context_mgr.check(&branch_path) {
        println!("{YELLOW}[context usage: ~{used}/{limit} tokens]{RESET}");
    }

    let prompt = ctx.template.render(truncated, ctx.session.system_prompt.as_deref());

    print!("{BLUE_BOLD}Assistant:{RESET} ");
    stdout().flush()?;

    let response = ctx
        .client
        .stream_completion(&prompt, ctx.stop_tokens, ctx.sampling, &mut stdout().lock())
        .await?;

    println!();

    let user_node = ctx.session.tree.head().unwrap();
    ctx.session.tree.push(Some(user_node), Message::new(Role::Assistant, response));
    ctx.session.maybe_save(ctx.save_mode)?;

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
            println!("{DIM}Available commands:\n{}\n{RESET}", crate::commands::format_help());
        }
        "/clear" => {
            ctx.session.tree.clear();
            println!("{YELLOW}Conversation cleared.{RESET}");
            ctx.session.maybe_save(ctx.save_mode)?;
        }
        "/save" => {
            if arg.is_empty() {
                ctx.session.maybe_save(ctx.save_mode)?;
                match ctx.save_mode.path() {
                    Some(p) => println!("{YELLOW}Session saved to {}.{RESET}", p.display()),
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
                let count = ctx.session.tree.branch_path().len();
                println!("{YELLOW}Session loaded from {arg} ({count} messages on current branch).{RESET}");
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
                ctx.session.maybe_save(ctx.save_mode)?;
            }
        }
        "/retry" => {
            ctx.session.pop_trailing_assistant();

            let last_user_content = ctx.session.tree.head()
                .and_then(|id| ctx.session.tree.node(id))
                .filter(|n| n.message.role == Role::User)
                .map(|n| n.message.content.clone());

            match last_user_content {
                Some(content) => {
                    ctx.session.tree.pop_head();
                    send_message(&content, ctx).await?;
                }
                None => println!("{YELLOW}No user message to retry.{RESET}"),
            }
        }
        "/edit" => {
            if arg.is_empty() {
                println!("{YELLOW}Usage: /edit <new message text>{RESET}");
            } else {
                ctx.session.pop_trailing_assistant();
                if ctx.session.tree.head()
                    .and_then(|id| ctx.session.tree.node(id))
                    .is_some_and(|n| n.message.role == Role::User)
                {
                    ctx.session.tree.pop_head();
                }
                send_message(arg, ctx).await?;
            }
        }
        "/history" => {
            let path = ctx.session.tree.branch_path();
            if path.is_empty() {
                println!("{YELLOW}No messages in history.{RESET}");
            } else {
                let path_ids = ctx.session.tree.branch_path_ids();
                for (i, (msg, &node_id)) in path.iter().zip(path_ids.iter()).enumerate() {
                    let role_color = role_ansi_color(msg.role);
                    let truncated: String = msg.content.chars().take(100).collect();
                    let ellipsis = if msg.content.chars().count() > 100 { "..." } else { "" };
                    let (sib_idx, sib_total) = ctx.session.tree.sibling_info(node_id);
                    let branch_marker = if sib_total > 1 {
                        format!(" {DIM}[{}/{}]{RESET}", sib_idx + 1, sib_total)
                    } else {
                        String::new()
                    };
                    println!(
                        "{DIM}{:>3}.{RESET} {role_color}{}{RESET}{branch_marker}: {truncated}{ellipsis}",
                        msg.role,
                        i + 1
                    );
                }
            }
        }
        "/render" => {
            let path = ctx.session.tree.branch_path();
            match path.iter().rev().find(|m| m.role == Role::Assistant) {
                Some(msg) => {
                    println!("{BLUE_BOLD}Assistant:{RESET}");
                    MadSkin::default().print_text(&msg.content);
                }
                None => println!("{YELLOW}No assistant response to render.{RESET}"),
            }
        }
        "/branch" => {
            match arg {
                "list" => {
                    let path = ctx.session.tree.branch_path_ids();
                    let mut found_any = false;
                    for &node_id in &path {
                        let (idx, total) = ctx.session.tree.sibling_info(node_id);
                        if total > 1 {
                            if let Some(node) = ctx.session.tree.node(node_id) {
                            println!(
                                "{YELLOW}Node {node_id} ({}): branch {}/{total}{RESET}",
                                node.message.role,
                                idx + 1
                            );
                            found_any = true;
                            }
                        }
                    }
                    if !found_any {
                        println!("{YELLOW}No branch points in current conversation.{RESET}");
                    }
                }
                "next" => {
                    ctx.session.tree.switch_sibling(1);
                    println!("{YELLOW}Switched to next branch.{RESET}");
                    ctx.session.maybe_save(ctx.save_mode)?;
                }
                "prev" => {
                    ctx.session.tree.switch_sibling(-1);
                    println!("{YELLOW}Switched to previous branch.{RESET}");
                    ctx.session.maybe_save(ctx.save_mode)?;
                }
                _ => {
                    if let Ok(id) = arg.parse::<usize>() {
                        ctx.session.tree.switch_to(id);
                        println!("{YELLOW}Switched to node {id}.{RESET}");
                        ctx.session.maybe_save(ctx.save_mode)?;
                    } else {
                        println!("{YELLOW}Usage: /branch list|next|prev|<id>{RESET}");
                    }
                }
            }
        }
        "/quit" | "/exit" => return Ok(true),
        _ => println!("{YELLOW}Unknown command: {cmd}. Type /help for available commands.{RESET}"),
    }

    Ok(false)
}

fn role_ansi_color(role: Role) -> &'static str {
    match role {
        Role::User => GREEN_BOLD,
        Role::Assistant => BLUE_BOLD,
        Role::System => DIM,
    }
}
