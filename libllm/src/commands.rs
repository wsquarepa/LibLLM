//! Slash command parsing and dispatch for the TUI chat interface.

/// Static metadata for a slash command: canonical name, aliases, argument pattern, and help text.
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
        name: "/continue",
        aliases: &["/cont"],
        args: "",
        description: "Continue the last assistant response",
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
        name: "/theme",
        aliases: &[],
        args: "[name]",
        description: "Switch color theme (dark, light)",
    },
    CommandInfo {
        name: "/export",
        aliases: &[],
        args: "[md|html|jsonl]",
        description: "Export current branch to file",
    },
    CommandInfo {
        name: "/macro",
        aliases: &["/m"],
        args: "<name> <args...>",
        description: "Run a user-defined macro",
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

/// Maps an alias (e.g. "/new") to its canonical command name (e.g. "/clear"), or returns the input unchanged.
pub fn resolve_alias(input: &str) -> &str {
    for cmd in COMMANDS {
        if cmd.aliases.contains(&input) {
            return cmd.name;
        }
    }
    input
}

/// Returns all commands whose name or aliases start with `prefix`, excluding those in `hidden`, sorted by shortest match.
pub fn matching_commands(prefix: &str, hidden: &[&str]) -> Vec<&'static CommandInfo> {
    let mut matches: Vec<&'static CommandInfo> = COMMANDS
        .iter()
        .filter(|c| {
            !hidden.contains(&c.name)
                && (c.name.starts_with(prefix) || c.aliases.iter().any(|a| a.starts_with(prefix)))
        })
        .collect();
    matches.sort_by_key(|c| {
        
        std::iter::once(c.name)
            .chain(c.aliases.iter().copied())
            .filter(|n| n.starts_with(prefix))
            .map(|n| n.len())
            .min()
            .unwrap_or(usize::MAX)
    });
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_alias_maps_known() {
        assert_eq!(resolve_alias("/new"), "/clear");
    }

    #[test]
    fn resolve_alias_canonical_unchanged() {
        assert_eq!(resolve_alias("/clear"), "/clear");
    }

    #[test]
    fn resolve_alias_unknown_passthrough() {
        assert_eq!(resolve_alias("/nonexistent"), "/nonexistent");
    }

    #[test]
    fn matching_commands_prefix() {
        let matches = matching_commands("/b", &[]);
        assert!(
            matches.iter().any(|c| c.name == "/branch"),
            "expected /branch to match /b prefix"
        );
    }

    #[test]
    fn matching_commands_empty_returns_all() {
        let all = matching_commands("/", &[]);
        assert_eq!(all.len(), COMMANDS.len());
    }

    #[test]
    fn matching_commands_shorter_first() {
        let matches = matching_commands("/m", &[]);
        assert!(matches.len() >= 2, "expected at least /macro and /persona (via /me)");
        assert_eq!(
            matches[0].name, "/macro",
            "/macro (via /m alias) should rank before /persona (via /me)"
        );
    }

    #[test]
    fn matching_commands_excludes_hidden() {
        let matches = matching_commands("/", &["/quit"]);
        assert!(
            !matches.iter().any(|c| c.name == "/quit"),
            "/quit should be excluded"
        );
        assert!(matches.len() < COMMANDS.len());
    }

    #[test]
    fn all_commands_have_slash_prefix() {
        for cmd in COMMANDS {
            assert!(
                cmd.name.starts_with('/'),
                "command name must start with /: {}",
                cmd.name
            );
            assert!(
                !cmd.description.is_empty(),
                "command {} must have a description",
                cmd.name
            );
        }
    }
}
