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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    fn rule(
        pattern: &str,
        replacement: &str,
        scopes: &[Scope],
        targets: &[Target],
    ) -> RegexRule {
        RegexRule {
            name: "test".to_owned(),
            pattern: pattern.to_owned(),
            replacement: replacement.to_owned(),
            scope: scopes.to_vec(),
            target: targets.to_vec(),
            enabled: true,
            compile_error: None,
        }
    }

    #[test]
    fn apply_returns_borrowed_on_empty_rules() {
        let rules: Vec<CompiledRule> = Vec::new();
        let out = apply(&rules, Scope::Display, Role::Assistant, "hello");
        assert!(matches!(out, Cow::Borrowed("hello")));
    }

    #[test]
    fn apply_returns_borrowed_on_no_match() {
        let rules = compile_rules(&[rule(
            "zzz",
            "yyy",
            &[Scope::Display],
            &[Target::Assistant],
        )]);
        let out = apply(&rules, Scope::Display, Role::Assistant, "hello");
        assert!(matches!(out, Cow::Borrowed("hello")));
    }

    #[test]
    fn apply_runs_top_to_bottom() {
        let rules = compile_rules(&[
            rule("a", "b", &[Scope::Display], &[Target::Assistant]),
            rule("b", "c", &[Scope::Display], &[Target::Assistant]),
        ]);
        let out = apply(&rules, Scope::Display, Role::Assistant, "a");
        assert_eq!(out, "c");
    }

    #[test]
    fn apply_skips_non_matching_scope() {
        let rules = compile_rules(&[rule(
            "a",
            "b",
            &[Scope::PromptSend],
            &[Target::Assistant],
        )]);
        let out = apply(&rules, Scope::Display, Role::Assistant, "a");
        assert_eq!(out, "a");
    }

    #[test]
    fn apply_skips_non_matching_target() {
        let rules = compile_rules(&[rule(
            "a",
            "b",
            &[Scope::Display],
            &[Target::User],
        )]);
        let out = apply(&rules, Scope::Display, Role::Assistant, "a");
        assert_eq!(out, "a");
    }

    #[test]
    fn apply_skips_disabled_rules() {
        let mut r = rule("a", "b", &[Scope::Display], &[Target::Assistant]);
        r.enabled = false;
        let rules = compile_rules(&[r]);
        let out = apply(&rules, Scope::Display, Role::Assistant, "a");
        assert_eq!(out, "a");
    }

    #[test]
    fn apply_skips_rules_with_compile_error() {
        let mut r = rule("a", "b", &[Scope::Display], &[Target::Assistant]);
        r.compile_error = Some("any error".to_owned());
        let rules = compile_rules(&[r]);
        let out = apply(&rules, Scope::Display, Role::Assistant, "a");
        assert_eq!(out, "a");
    }

    #[test]
    fn apply_resolves_numbered_capture_group() {
        let rules = compile_rules(&[rule(
            r"\[OOC: (.*?)\]",
            "«$1»",
            &[Scope::Display],
            &[Target::User],
        )]);
        let out = apply(&rules, Scope::Display, Role::User, "hey [OOC: brb 5min]");
        assert_eq!(out, "hey «brb 5min»");
    }

    #[test]
    fn apply_resolves_named_capture_group() {
        let rules = compile_rules(&[rule(
            r"\[OOC: (?P<note>.*?)\]",
            "<<${note}>>",
            &[Scope::Display],
            &[Target::User],
        )]);
        let out = apply(&rules, Scope::Display, Role::User, "[OOC: hi]");
        assert_eq!(out, "<<hi>>");
    }

    #[test]
    fn apply_handles_multibyte_input() {
        let rules = compile_rules(&[rule(
            "[\u{201c}\u{201d}]",
            "\"",
            &[Scope::PromptSend],
            &[Target::User],
        )]);
        let out = apply(
            &rules,
            Scope::PromptSend,
            Role::User,
            "He said \u{201c}hi\u{201d}",
        );
        assert_eq!(out, "He said \"hi\"");
    }

    #[test]
    fn apply_strips_think_blocks_dotall() {
        let rules = compile_rules(&[rule(
            r"(?s)<think>.*?</think>\s*",
            "",
            &[Scope::Display],
            &[Target::Assistant],
        )]);
        let out = apply(
            &rules,
            Scope::Display,
            Role::Assistant,
            "<think>line1\nline2</think>\n\nactual answer",
        );
        assert_eq!(out, "actual answer");
    }

    #[test]
    fn apply_one_rule_multiple_scopes() {
        let rules = compile_rules(&[rule(
            "a",
            "b",
            &[Scope::Display, Scope::Export],
            &[Target::Assistant],
        )]);
        assert_eq!(apply(&rules, Scope::Display, Role::Assistant, "a"), "b");
        assert_eq!(apply(&rules, Scope::Export, Role::Assistant, "a"), "b");
        assert_eq!(apply(&rules, Scope::PromptSend, Role::Assistant, "a"), "a");
    }

    #[test]
    fn compile_rules_skips_invalid_pattern() {
        let bad = rule("(unclosed", "x", &[Scope::Display], &[Target::Assistant]);
        let rules = compile_rules(&[bad]);
        assert!(rules.is_empty());
    }

    #[test]
    fn config_round_trips_disabled_rule() {
        let cfg = Config {
            regex: vec![RegexRule {
                name: "off".to_owned(),
                pattern: "x".to_owned(),
                replacement: "y".to_owned(),
                scope: vec![Scope::Display],
                target: vec![Target::Assistant],
                enabled: false,
                compile_error: None,
            }],
            ..Config::default()
        };

        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            s.contains("enabled = false"),
            "serialized TOML must include `enabled = false` for disabled rules; got: {s}"
        );
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.regex.len(), 1);
        assert!(!back.regex[0].enabled, "deserialized rule must remain disabled");
    }
}
