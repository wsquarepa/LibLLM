pub struct CommandInfo {
    pub name: &'static str,
    pub args: &'static str,
    pub description: &'static str,
    pub repl_only: bool,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo { name: "/help", args: "", description: "Show this help message", repl_only: false },
    CommandInfo { name: "/clear", args: "", description: "Clear conversation history", repl_only: false },
    CommandInfo { name: "/save", args: "<path>", description: "Save session to file", repl_only: false },
    CommandInfo { name: "/load", args: "<path>", description: "Load session from file", repl_only: false },
    CommandInfo { name: "/model", args: "", description: "Show current model name", repl_only: false },
    CommandInfo { name: "/system", args: "<prompt>", description: "Set or show system prompt", repl_only: false },
    CommandInfo { name: "/retry", args: "", description: "Regenerate last response (new branch)", repl_only: false },
    CommandInfo { name: "/edit", args: "<text>", description: "Replace last message and regenerate", repl_only: false },
    CommandInfo { name: "/history", args: "", description: "Show conversation history", repl_only: true },
    CommandInfo { name: "/render", args: "", description: "Render last response as markdown", repl_only: true },
    CommandInfo { name: "/branch", args: "list|next|prev|<id>", description: "Navigate branches", repl_only: false },
    CommandInfo { name: "/quit", args: "", description: "Exit the chat", repl_only: false },
];

pub fn matching_commands(prefix: &str, exclude_repl_only: bool) -> Vec<&'static CommandInfo> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(prefix))
        .filter(|c| !exclude_repl_only || !c.repl_only)
        .collect()
}

pub fn format_help() -> String {
    let max_width = COMMANDS
        .iter()
        .map(|c| {
            if c.args.is_empty() {
                c.name.len()
            } else {
                c.name.len() + 1 + c.args.len()
            }
        })
        .max()
        .unwrap_or(0);

    COMMANDS
        .iter()
        .map(|c| {
            let usage = if c.args.is_empty() {
                c.name.to_owned()
            } else {
                format!("{} {}", c.name, c.args)
            };
            format!("  {:<width$}  {}", usage, c.description, width = max_width)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
