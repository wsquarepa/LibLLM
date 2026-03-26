pub struct CommandInfo {
    pub name: &'static str,
    pub args: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo { name: "/help", args: "", description: "Show available commands" },
    CommandInfo { name: "/clear", args: "", description: "Clear conversation history" },
    CommandInfo { name: "/save", args: "<path>", description: "Save session to file" },
    CommandInfo { name: "/load", args: "<path>", description: "Load session from file" },
    CommandInfo { name: "/model", args: "", description: "Show current model name" },
    CommandInfo { name: "/system", args: "<prompt>", description: "Set or show system prompt" },
    CommandInfo { name: "/retry", args: "", description: "Regenerate last response (new branch)" },
    CommandInfo { name: "/edit", args: "<text>", description: "Replace last message and regenerate" },
    CommandInfo { name: "/branch", args: "list|next|prev|<id>", description: "Navigate branches" },
    CommandInfo { name: "/character", args: "list|load <name>|import <path>", description: "Manage character cards" },
    CommandInfo { name: "/self", args: "", description: "Set your name and persona" },
    CommandInfo { name: "/worldbook", args: "list|on <name>|off <name>|active", description: "Manage worldbooks" },
    CommandInfo { name: "/config", args: "", description: "Open configuration dialog" },
    CommandInfo { name: "/quit", args: "", description: "Exit the chat" },
    CommandInfo { name: "/exit", args: "", description: "Exit the chat" },
];

pub fn matching_commands(prefix: &str) -> Vec<&'static CommandInfo> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(prefix))
        .collect()
}

