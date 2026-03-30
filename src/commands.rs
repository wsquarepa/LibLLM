pub struct CommandInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub args: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "/clear",
        aliases: &["/new"],
        args: "",
        description: "Clear conversation history",
    },
    CommandInfo {
        name: "/system",
        aliases: &[],
        args: "",
        description: "Select or edit system prompt",
    },
    CommandInfo {
        name: "/retry",
        aliases: &[],
        args: "",
        description: "Regenerate last response (new branch)",
    },
    CommandInfo {
        name: "/branch",
        aliases: &[],
        args: "",
        description: "Browse branches at current position",
    },
    CommandInfo {
        name: "/character",
        aliases: &[],
        args: "",
        description: "Select a character",
    },
    CommandInfo {
        name: "/persona",
        aliases: &["/self", "/user", "/me"],
        args: "",
        description: "Manage user personas",
    },
    CommandInfo {
        name: "/worldbook",
        aliases: &["/lore", "/world", "/lorebook"],
        args: "",
        description: "Toggle worldbooks for this session",
    },
    CommandInfo {
        name: "/passkey",
        aliases: &["/password", "/pass", "/auth"],
        args: "",
        description: "Set or change encryption passkey",
    },
    CommandInfo {
        name: "/config",
        aliases: &[],
        args: "",
        description: "Open configuration dialog",
    },
    CommandInfo {
        name: "/report",
        aliases: &[],
        args: "",
        description: "Copy current debug log to ./debug.log",
    },
    CommandInfo {
        name: "/quit",
        aliases: &["/exit"],
        args: "",
        description: "Exit the chat",
    },
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
        .filter(|c| c.name.starts_with(prefix) || c.aliases.iter().any(|a| a.starts_with(prefix)))
        .collect()
}
