mod cli;
mod client;
mod config;
mod context;
mod interactive;
mod prompt;
mod sampling;
mod session;

use std::io::{self, Read, Write};

use anyhow::Result;
use clap::Parser;

use cli::Args;
use client::ApiClient;
use prompt::Template;
use session::{Message, Role};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = config::load();

    let api_url = args.api_url.as_deref().unwrap_or_else(|| cfg.api_url());
    let client = ApiClient::new(api_url);

    let template_name = args
        .template
        .as_deref()
        .or(cfg.template.as_deref())
        .unwrap_or("llama2");
    let template = Template::from_name(template_name);

    let sampling = sampling::SamplingParams::default()
        .with_overrides(&cfg.sampling)
        .with_overrides(&args.sampling_overrides());

    let mut session = match &args.session {
        Some(path) => session::load(path)?,
        None => session::Session::default(),
    };

    session.template = Some(template.name().to_owned());

    if session.system_prompt.is_none() {
        session.system_prompt = args
            .system_prompt
            .or(cfg.system_prompt);
    }

    if let Some(ref message) = args.message {
        let text = if message == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            message.clone()
        };

        session.messages.push(Message::new(Role::User, text));

        let prompt_text = template.render(&session.messages, session.system_prompt.as_deref());
        let stop_tokens = template.stop_tokens();
        let mut stdout = io::stdout().lock();
        let response = client
            .stream_completion(&prompt_text, stop_tokens, &sampling, &mut stdout)
            .await?;
        writeln!(stdout)?;

        session.messages.push(Message::new(Role::Assistant, response));

        if let Some(path) = &args.session {
            session::save(path, &session)?;
        }

        return Ok(());
    }

    interactive::run(&client, &mut session, args.session.as_deref(), template, &sampling).await
}
