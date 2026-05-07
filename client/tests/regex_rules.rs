//! Integration tests for regex find/replace rules across the four pipeline scopes.

#[expect(dead_code, reason = "each test binary uses a different subset of common helpers")]
mod common;

use libllm::regex_rules::{RegexRule, Scope, Target};
use libllm::session::{Message, Role};

fn smart_quote_to_ascii_rule() -> RegexRule {
    RegexRule {
        name: "smart-quotes".to_owned(),
        pattern: "[\u{201c}\u{201d}]".to_owned(),
        replacement: "\"".to_owned(),
        scope: vec![Scope::PromptSend],
        target: vec![Target::User],
        enabled: true,
        compile_error: None,
    }
}

#[test]
fn prompt_send_rule_rewrites_outgoing_text_without_mutating_tree() {
    let rule = smart_quote_to_ascii_rule();
    let compiled = libllm::regex_rules::compile_rules(&[rule]);

    let original = "He said \u{201c}hi\u{201d}";
    let stored = Message::new(Role::User, original.to_owned());

    let transformed = libllm::regex_rules::apply(
        &compiled,
        Scope::PromptSend,
        stored.role,
        &stored.content,
    );

    assert_eq!(transformed, "He said \"hi\"");
    assert_eq!(stored.content, original, "tree-stored content must not change");
}
