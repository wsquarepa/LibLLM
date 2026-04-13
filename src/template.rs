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
