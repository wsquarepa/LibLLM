mod cli;
mod client;
mod interactive;
mod prompt;
mod session;

use std::io::{self, Read, Write};

use anyhow::Result;
use clap::Parser;

use cli::Args;
use client::ApiClient;
use prompt::{Llama2Template, PromptTemplate};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let template = Llama2Template;
    let client = ApiClient::new(&args.api_url);

    let mut session = match &args.session {
        Some(path) => session::load(path)?,
        None => session::Session::default(),
    };

    if session.prompt_history.is_empty() {
        session.prompt_history = template.bos(args.system_prompt.as_deref());
    }

    if let Some(message) = &args.message {
        let text = if message == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            message.clone()
        };

        session.prompt_history.push_str(&template.wrap_user(&text));

        let stop_tokens = template.stop_tokens();
        let mut stdout = io::stdout().lock();
        let response = client
            .stream_completion(&session.prompt_history, &stop_tokens, &mut stdout)
            .await?;
        writeln!(stdout)?;

        session.prompt_history.push_str(&response);
        session.prompt_history.push_str(template.assistant_end());

        if let Some(path) = &args.session {
            session::save(path, &session)?;
        }

        return Ok(());
    }

    interactive::run(&client, &mut session, args.session.as_deref(), &template).await
}
