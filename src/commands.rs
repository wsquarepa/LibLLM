pub struct CommandInfo {
    pub name: &'static str,
    pub args: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo { name: "/help", args: "", description: "Show this help message" },
    CommandInfo { name: "/clear", args: "", description: "Clear conversation history" },
    CommandInfo { name: "/save", args: "<path>", description: "Save session to file" },
    CommandInfo { name: "/load", args: "<path>", description: "Load session from file" },
    CommandInfo { name: "/model", args: "", description: "Show current model name" },
    CommandInfo { name: "/system", args: "<prompt>", description: "Set or show system prompt" },
    CommandInfo { name: "/retry", args: "", description: "Regenerate last response (new branch)" },
    CommandInfo { name: "/edit", args: "<text>", description: "Replace last message and regenerate" },
    CommandInfo { name: "/history", args: "", description: "Show conversation history" },
    CommandInfo { name: "/render", args: "", description: "Render last response as markdown" },
    CommandInfo { name: "/branch", args: "list|next|prev|<id>", description: "Navigate branches" },
    CommandInfo { name: "/quit", args: "", description: "Exit the chat" },
];

pub fn matching_commands(prefix: &str) -> Vec<&'static CommandInfo> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(prefix))
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
