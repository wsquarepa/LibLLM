pub struct CommandInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub args: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo { name: "/help", aliases: &[], args: "", description: "Show available commands" },
    CommandInfo { name: "/clear", aliases: &["/new"], args: "", description: "Clear conversation history" },
    CommandInfo { name: "/save", aliases: &[], args: "<path>", description: "Save session to file" },
    CommandInfo { name: "/load", aliases: &[], args: "<path>", description: "Load session from file" },
    CommandInfo { name: "/model", aliases: &[], args: "", description: "Show current model name" },
    CommandInfo { name: "/system", aliases: &[], args: "<prompt>", description: "Set or show system prompt" },
    CommandInfo { name: "/retry", aliases: &[], args: "", description: "Regenerate last response (new branch)" },
    CommandInfo { name: "/edit", aliases: &[], args: "<text>", description: "Replace last message and regenerate" },
    CommandInfo { name: "/branch", aliases: &[], args: "", description: "Browse branches at current position" },
    CommandInfo { name: "/character", aliases: &[], args: "[import <path>]", description: "Select a character or import a card" },
    CommandInfo { name: "/self", aliases: &["/user", "/me"], args: "", description: "Set your name and persona" },
    CommandInfo { name: "/worldbook", aliases: &["/lore", "/world", "/lorebook"], args: "", description: "Toggle worldbooks for this session" },
    CommandInfo { name: "/config", aliases: &[], args: "", description: "Open configuration dialog" },
    CommandInfo { name: "/quit", aliases: &["/exit"], args: "", description: "Exit the chat" },
];

pub fn resolve_alias(input: &str) -> &str {
    for cmd in COMMANDS {
        if cmd.aliases.contains(&input) {
            return cmd.name;
        }
    }
    input
}

pub fn matching_commands(prefix: &str) -> Vec<&'static CommandInfo> {
    COMMANDS
        .iter()
        .filter(|c| {
            c.name.starts_with(prefix)
                || c.aliases.iter().any(|a| a.starts_with(prefix))
        })
        .collect()
}
