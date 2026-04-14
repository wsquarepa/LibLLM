//! Template variable substitution for character and persona placeholders.

/// Replaces `{{char}}` and `{{user}}` placeholders in `text` with the given names.
///
/// Returns the input unchanged (without allocation) when neither placeholder is present.
pub fn apply_template_vars(text: &str, char_name: &str, user_name: &str) -> String {
    if !text.contains("{{char}}") && !text.contains("{{user}}") {
        return text.to_owned();
    }

    let mut rendered = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(rel_idx) = text[cursor..].find("{{") {
        let idx = cursor + rel_idx;
        rendered.push_str(&text[cursor..idx]);

        if text[idx..].starts_with("{{char}}") {
            rendered.push_str(char_name);
            cursor = idx + "{{char}}".len();
        } else if text[idx..].starts_with("{{user}}") {
            rendered.push_str(user_name);
            cursor = idx + "{{user}}".len();
        } else {
            rendered.push_str("{{");
            cursor = idx + 2;
        }
    }

    rendered.push_str(&text[cursor..]);
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_char_variable() {
        let result = apply_template_vars("Hello {{char}}", "Alice", "Bob");
        assert_eq!(result, "Hello Alice");
    }

    #[test]
    fn replaces_both_variables() {
        let result = apply_template_vars("{{user}} talks to {{char}}", "Alice", "Bob");
        assert_eq!(result, "Bob talks to Alice");
    }

    #[test]
    fn replaces_multiple_occurrences() {
        let result = apply_template_vars("{{char}} meets {{char}}", "Alice", "Bob");
        assert_eq!(result, "Alice meets Alice");
    }

    #[test]
    fn no_variables_returns_unchanged() {
        let result = apply_template_vars("plain text", "Alice", "Bob");
        assert_eq!(result, "plain text");
    }

    #[test]
    fn empty_names_produce_empty_substitutions() {
        let result = apply_template_vars("Hi {{char}} and {{user}}", "", "");
        assert_eq!(result, "Hi  and ");
    }

    #[test]
    fn nested_braces_not_substituted() {
        let result = apply_template_vars("{{{char}}}", "Alice", "Bob");
        assert_eq!(result, "{{{char}}}");
    }

    #[test]
    fn partial_brace_not_substituted() {
        let result = apply_template_vars("{char}", "Alice", "Bob");
        assert_eq!(result, "{char}");
    }
}
