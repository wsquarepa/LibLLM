use std::io::{Write, stdout};
use std::path::Path;

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::client::ApiClient;
use crate::prompt::PromptTemplate;
use crate::session::{self, Session};

const GREEN_BOLD: &str = "\x1b[1;32m";
const BLUE_BOLD: &str = "\x1b[1;34m";
const RESET: &str = "\x1b[0m";

pub async fn run(
    client: &ApiClient,
    session: &mut Session,
    session_path: Option<&Path>,
    template: &dyn PromptTemplate,
) -> Result<()> {
    let model_name = client.fetch_model_name().await;
    println!("Chat with {model_name} (Ctrl+C to quit)\n");

    let mut editor = DefaultEditor::new()?;
    let stop_tokens = template.stop_tokens();

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
        session.prompt_history.push_str(&template.wrap_user(trimmed));

        print!("{BLUE_BOLD}Assistant:{RESET} ");
        stdout().flush()?;

        let response = client
            .stream_completion(&session.prompt_history, &stop_tokens, &mut stdout().lock())
            .await?;

        println!();

        session.prompt_history.push_str(&response);
        session.prompt_history.push_str(template.assistant_end());

        if let Some(path) = session_path {
            session::save(path, session)?;
        }

        println!();
    }

    Ok(())
}
