mod cli;
mod client;
mod config;
mod context;
mod interactive;
mod prompt;
mod render;
mod sampling;
mod session;

use std::io::{self, Read, Write};

use anyhow::Result;
use clap::Parser;

use cli::Args;
use client::ApiClient;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = config::load();

    let api_url = args
        .api_url
        .as_deref()
        .unwrap_or_else(|| cfg.api_url());
    let client = ApiClient::new(api_url);

    let template_name = args
        .template
        .as_deref()
        .or(cfg.template.as_deref())
        .unwrap_or("llama2");
    let template = prompt::template_by_name(template_name);

    let sampling = args.resolve_sampling(cfg.resolve_sampling());

    let mut session = match &args.session {
        Some(path) => session::load(path)?,
        None => session::Session::default(),
    };

    session.template = Some(template.name().to_owned());

    if session.system_prompt.is_none() {
        session.system_prompt = args
            .system_prompt
            .clone()
            .or_else(|| cfg.system_prompt.clone());
    }

    if let Some(message) = &args.message {
        let text = if message == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            message.clone()
        };

        session.messages.push(session::Message::new("user", &text));

        let prompt_text = template.render(&session.messages, session.system_prompt.as_deref());
        let stop_tokens = template.stop_tokens();
        let mut stdout = io::stdout().lock();
        let response = client
            .stream_completion(&prompt_text, &stop_tokens, &sampling, &mut stdout)
            .await?;
        writeln!(stdout)?;

        session.messages.push(session::Message::new("assistant", &response));

        if let Some(path) = &args.session {
            session::save(path, &session)?;
        }

        return Ok(());
    }

    interactive::run(&client, &mut session, args.session.as_deref(), template.as_ref(), &sampling).await
}
