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

#[test]
fn display_rule_does_not_mutate_stored_content() {
    use libllm::session::MessageTree;

    let rule = RegexRule {
        name: "strip-think".to_owned(),
        pattern: r"(?s)<think>.*?</think>\s*".to_owned(),
        replacement: String::new(),
        scope: vec![Scope::Display],
        target: vec![Target::Assistant],
        enabled: true,
        compile_error: None,
    };
    let compiled = libllm::regex_rules::compile_rules(&[rule]);

    let mut tree = MessageTree::new();
    let original = "<think>plan</think>\n\nhello";
    let id = tree.push(None, Message::new(Role::Assistant, original.to_owned()));

    let displayed = libllm::regex_rules::apply(
        &compiled,
        Scope::Display,
        Role::Assistant,
        &tree.node(id).unwrap().message.content,
    );

    assert_eq!(displayed, "hello");
    assert_eq!(
        tree.node(id).unwrap().message.content,
        original,
        "display rules must not change stored content"
    );
}

#[test]
fn prompt_recv_rule_mutates_stored_assistant_content() {
    let rule = RegexRule {
        name: "verbal-tic".to_owned(),
        pattern: "y'know".to_owned(),
        replacement: "you know".to_owned(),
        scope: vec![Scope::PromptRecv],
        target: vec![Target::Assistant],
        enabled: true,
        compile_error: None,
    };
    let compiled = libllm::regex_rules::compile_rules(&[rule]);

    let raw_response = "Well, y'know, that's how it is.";
    let stored = libllm::regex_rules::apply(
        &compiled,
        Scope::PromptRecv,
        Role::Assistant,
        raw_response,
    )
    .into_owned();

    assert_eq!(stored, "Well, you know, that's how it is.");
}
