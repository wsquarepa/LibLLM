//! User-defined regex find/replace rules applied at four pipeline points:
//! display, prompt_send, prompt_recv, export. Rules form one global ordered
//! list; each call to `apply` filters by scope and target.

use std::borrow::Cow;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::session::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Display,
    PromptSend,
    PromptRecv,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    User,
    Assistant,
    System,
    Summary,
}

impl Target {
    pub fn from_role(role: Role) -> Self {
        match role {
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
            Role::System => Self::System,
            Role::Summary => Self::Summary,
        }
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexRule {
    pub name: String,
    pub pattern: String,
    pub replacement: String,
    #[serde(default)]
    pub scope: Vec<Scope>,
    #[serde(default)]
    pub target: Vec<Target>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_error: Option<String>,
}

#[derive(Debug)]
pub struct CompiledRule {
    pub rule: RegexRule,
    pub re: Regex,
}

/// Compile every enabled, error-free rule into a `Vec<CompiledRule>`. Rules whose
/// pattern fails to compile are dropped with a `tracing::warn`. Returns the live
/// rule set; the original `RegexRule` config is unchanged.
pub fn compile_rules(rules: &[RegexRule]) -> Vec<CompiledRule> {
    let mut compiled = Vec::new();
    for rule in rules {
        if !rule.enabled || rule.compile_error.is_some() {
            continue;
        }
        match Regex::new(&rule.pattern) {
            Ok(re) => compiled.push(CompiledRule {
                rule: rule.clone(),
                re,
            }),
            Err(err) => {
                tracing::warn!(
                    rule = %rule.name,
                    pattern = %rule.pattern,
                    error = %err,
                    "regex.compile",
                );
            }
        }
    }
    compiled
}

/// Apply every compiled rule whose `scope` and `target` match, top-to-bottom.
/// Each rule sees the previous rule's output. Returns `Cow::Borrowed` on the
/// no-rule-fired path so callers pay zero copy cost when no rule matches.
pub fn apply<'a>(
    rules: &[CompiledRule],
    scope: Scope,
    role: Role,
    text: &'a str,
) -> Cow<'a, str> {
    if rules.is_empty() {
        return Cow::Borrowed(text);
    }
    let target = Target::from_role(role);
    let mut current: Cow<'a, str> = Cow::Borrowed(text);
    for cr in rules {
        if !cr.rule.scope.contains(&scope) || !cr.rule.target.contains(&target) {
            continue;
        }
        let replaced = cr.re.replace_all(&current, cr.rule.replacement.as_str());
        if let Cow::Owned(s) = replaced {
            current = Cow::Owned(s);
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = Config {
            regex: vec![RegexRule {
                name: "strip-think".to_owned(),
                pattern: r"(?s)<think>.*?</think>\s*".to_owned(),
                replacement: String::new(),
                scope: vec![Scope::Display],
                target: vec![Target::Assistant],
                enabled: true,
                compile_error: None,
            }],
            ..Config::default()
        };

        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.regex.len(), 1);
        assert_eq!(back.regex[0].name, "strip-think");
        assert_eq!(back.regex[0].scope, vec![Scope::Display]);
        assert_eq!(back.regex[0].target, vec![Target::Assistant]);
        assert!(back.regex[0].enabled);
        assert!(back.regex[0].compile_error.is_none());
    }

    #[test]
    fn config_with_no_regex_table_parses_to_empty() {
        let toml_str = r#"api_url = "http://localhost:5001/v1""#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.regex.is_empty());
    }
}
