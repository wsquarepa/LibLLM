//! Session and character export to Markdown and JSON formats.

use crate::session::{self, Message, Role};
use crate::template;

pub fn render_markdown(messages: &[&Message], char_name: &str, user_name: &str) -> String {
    let _span = tracing::info_span!("export.markdown", message_count = messages.len()).entered();
    let mut out = String::new();
    for msg in messages {
        let role_label = match msg.role {
            Role::User => user_name,
            Role::Assistant => char_name,
            Role::System | Role::Summary => "System",
        };
        let content = template::apply_template_vars(&msg.content, char_name, user_name);
        out.push_str(&format!("## {role_label}\n\n{content}\n\n---\n\n"));
    }
    tracing::info!(phase = "done", output_bytes = out.len(), "export.markdown");
    out
}

pub fn render_html(messages: &[&Message], char_name: &str, user_name: &str) -> String {
    let _span = tracing::info_span!("export.html", message_count = messages.len()).entered();
    {
        let mut body = String::new();
            for msg in messages {
                let role_label = match msg.role {
                    Role::User => user_name,
                    Role::Assistant => char_name,
                    Role::System | Role::Summary => "System",
                };
                let content = template::apply_template_vars(&msg.content, char_name, user_name);
                let formatted = html_format_content(&content);
                let class = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System | Role::Summary => "system",
                };
                let tag = match msg.role {
                    Role::System | Role::Summary => "em",
                    _ => "span",
                };
                body.push_str(&format!(
                    "    <article class=\"message {class}\">\n\
                     \x20     <div class=\"role\">{}</div>\n\
                     \x20     <div class=\"content\"><{tag}>{formatted}</{tag}></div>\n\
                     \x20     <time>{}</time>\n\
                     \x20   </article>\n",
                    html_escape(role_label),
                    html_escape(&msg.timestamp),
                ));
            }

            let char_escaped = html_escape(char_name);
            let user_escaped = html_escape(user_name);

            let out = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Chat: {user_escaped} &amp; {char_escaped}</title>
  <style>
    :root {{
      --bg: #1a1a2e;
      --surface: #16213e;
      --surface-alt: #0f3460;
      --text: #e0e0e0;
      --text-dim: #8a8a9a;
      --accent-user: #4fc3f7;
      --accent-assistant: #ce93d8;
      --accent-system: #ffb74d;
      --user-bg: rgba(79, 195, 247, 0.08);
      --user-border: rgba(79, 195, 247, 0.3);
      --assistant-bg: rgba(206, 147, 216, 0.08);
      --assistant-border: rgba(206, 147, 216, 0.3);
      --system-bg: rgba(255, 183, 77, 0.06);
      --system-border: rgba(255, 183, 77, 0.25);
    }}

    @media (prefers-color-scheme: light) {{
      :root {{
        --bg: #f5f5f5;
        --surface: #ffffff;
        --surface-alt: #e8eaf6;
        --text: #212121;
        --text-dim: #757575;
        --accent-user: #1565c0;
        --accent-assistant: #7b1fa2;
        --accent-system: #e65100;
        --user-bg: rgba(21, 101, 192, 0.06);
        --user-border: rgba(21, 101, 192, 0.25);
        --assistant-bg: rgba(123, 31, 162, 0.06);
        --assistant-border: rgba(123, 31, 162, 0.25);
        --system-bg: rgba(230, 81, 0, 0.05);
        --system-border: rgba(230, 81, 0, 0.2);
      }}
    }}

    *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}

    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
      background: var(--bg);
      color: var(--text);
      line-height: 1.6;
      padding: 0;
    }}

    header {{
      background: var(--surface);
      border-bottom: 1px solid var(--user-border);
      padding: 1.5rem 2rem;
      text-align: center;
    }}

    header h1 {{
      font-size: 1.25rem;
      font-weight: 600;
      letter-spacing: -0.01em;
    }}

    header p {{
      color: var(--text-dim);
      font-size: 0.85rem;
      margin-top: 0.25rem;
    }}

    main {{
      max-width: 52rem;
      margin: 0 auto;
      padding: 1.5rem 1rem;
      display: flex;
      flex-direction: column;
      gap: 0.75rem;
    }}

    .message {{
      padding: 1rem 1.25rem;
      border-radius: 12px;
      border-left: 3px solid transparent;
    }}

    .message.user {{
      background: var(--user-bg);
      border-left-color: var(--accent-user);
    }}

    .message.assistant {{
      background: var(--assistant-bg);
      border-left-color: var(--accent-assistant);
    }}

    .message.system {{
      background: var(--system-bg);
      border-left-color: var(--accent-system);
      font-size: 0.9rem;
    }}

    .role {{
      font-weight: 600;
      font-size: 0.8rem;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      margin-bottom: 0.4rem;
    }}

    .user .role {{ color: var(--accent-user); }}
    .assistant .role {{ color: var(--accent-assistant); }}
    .system .role {{ color: var(--accent-system); }}

    .content {{
      white-space: pre-wrap;
      word-wrap: break-word;
      font-size: 0.95rem;
    }}

    .system .content em {{
      font-style: italic;
    }}

    .content q {{
      quotes: none;
      color: var(--accent-assistant);
    }}

    time {{
      display: block;
      color: var(--text-dim);
      font-size: 0.75rem;
      margin-top: 0.5rem;
      text-align: right;
    }}

    @media (max-width: 600px) {{
      main {{ padding: 1rem 0.5rem; }}
      .message {{ padding: 0.75rem 1rem; }}
    }}
  </style>
</head>
<body>
  <header>
    <h1>{user_escaped} &amp; {char_escaped}</h1>
    <p>Exported from LibLLM</p>
  </header>
  <main>
{body}  </main>
</body>
</html>
"#
            );
        tracing::info!(phase = "done", output_bytes = out.len(), "export.html");
        out
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_format_line(line: &str) -> String {
    let escaped = html_escape(line);
    let mut out = String::with_capacity(escaped.len());
    let bytes = escaped.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*'
            && let Some(end) = find_delimiter(&escaped[i + 2..], "**") {
                let inner = &escaped[i + 2..i + 2 + end];
                out.push_str("<strong>");
                out.push_str(inner);
                out.push_str("</strong>");
                i += 2 + end + 2;
                continue;
            }

        if bytes[i] == b'*'
            && let Some(end) = find_delimiter(&escaped[i + 1..], "*") {
                let inner = &escaped[i + 1..i + 1 + end];
                out.push_str("<em>");
                out.push_str(inner);
                out.push_str("</em>");
                i += 1 + end + 1;
                continue;
            }

        if bytes[i] == b'&' && escaped[i..].starts_with("&quot;") {
            let after = i + 6;
            if let Some(end) = escaped[after..].find("&quot;") {
                let inner = &escaped[after..after + end];
                out.push_str("<q>");
                out.push_str(inner);
                out.push_str("</q>");
                i = after + end + 6;
                continue;
            }
        }

        let ch = escaped[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn find_delimiter(text: &str, delim: &str) -> Option<usize> {
    if text.len() <= delim.len() {
        return None;
    }
    let start = text.char_indices().nth(1).map(|(i, _)| i)?;
    text[start..].find(delim).map(|pos| pos + start)
}

pub fn html_format_content(content: &str) -> String {
    content
        .lines()
        .map(html_format_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_jsonl(messages: &[&Message], char_name: &str, user_name: &str) -> String {
    let _span = tracing::info_span!("export.jsonl", message_count = messages.len()).entered();
    let mut lines = Vec::new();

    let header = serde_json::json!({
        "user_name": user_name,
        "character_name": char_name,
        "create_date": session::now_compact(),
    });
    lines.push(serde_json::to_string(&header).unwrap_or_default());

    for msg in messages {
        let content = template::apply_template_vars(&msg.content, char_name, user_name);
        let name = match msg.role {
            Role::User => user_name,
            Role::Assistant => char_name,
            Role::System | Role::Summary => "System",
        };
        let entry = serde_json::json!({
            "name": name,
            "is_user": msg.role == Role::User,
            "is_system": msg.role == Role::System,
            "mes": content,
            "send_date": msg.timestamp,
        });
        lines.push(serde_json::to_string(&entry).unwrap_or_default());
    }

    let mut result = lines.join("\n");
    result.push('\n');
    tracing::info!(phase = "done", output_bytes = result.len(), "export.jsonl");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Message, Role};

    fn user_msg(content: &str) -> Message {
        Message::new(Role::User, content.to_string())
    }

    fn assistant_msg(content: &str) -> Message {
        Message::new(Role::Assistant, content.to_string())
    }

    fn system_msg(content: &str) -> Message {
        Message::new(Role::System, content.to_string())
    }

    fn test_messages() -> Vec<Message> {
        vec![
            Message {
                role: Role::User,
                content: "Hello {{char}}".to_owned(),
                timestamp: "2026-01-15T10:00:00Z".to_owned(),
            },
            Message {
                role: Role::Assistant,
                content: "Hi {{user}}!".to_owned(),
                timestamp: "2026-01-15T10:00:05Z".to_owned(),
            },
        ]
    }

    #[test]
    fn markdown_basic() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_markdown(&refs, "Alice", "Bob");
        assert!(result.contains("## Bob\n\nHello Alice"));
        assert!(result.contains("## Alice\n\nHi Bob!"));
    }

    #[test]
    fn markdown_system_message() {
        let msgs = [system_msg("You are helpful.")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_markdown(&refs, "Char", "User");
        assert!(result.contains("## System\n\nYou are helpful."));
    }

    #[test]
    fn markdown_empty() {
        let refs: Vec<&Message> = vec![];
        let result = render_markdown(&refs, "Char", "User");
        assert!(result.is_empty());
    }

    #[test]
    fn html_escapes_content() {
        let msgs = [user_msg("<script>alert('xss')</script>")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Char", "User");
        assert!(result.contains("&lt;script&gt;"));
        assert!(!result.contains("<script>alert"));
    }

    #[test]
    fn html_has_structure() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Alice", "Bob");
        assert!(result.starts_with("<!DOCTYPE html>"));
        assert!(result.contains("class=\"message user\""));
        assert!(result.contains("class=\"message assistant\""));
    }

    #[test]
    fn html_applies_template_vars() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Alice", "Bob");
        assert!(result.contains("Hello Alice"));
        assert!(result.contains("Hi Bob!"));
    }

    #[test]
    fn html_formats_bold() {
        let msgs = [user_msg("This is **bold** text")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Char", "User");
        assert!(result.contains("<strong>bold</strong>"));
        assert!(!result.contains("**bold**"));
    }

    #[test]
    fn html_formats_italic() {
        let msgs = [user_msg("This is *italic* text")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Char", "User");
        assert!(result.contains("<em>italic</em>"));
        assert!(!result.contains("*italic*"));
    }

    #[test]
    fn html_formats_dialogue() {
        let msgs = [assistant_msg("She said \"hello there\" softly")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Char", "User");
        assert!(result.contains("<q>hello there</q>"));
    }

    #[test]
    fn html_formats_mixed_markdown() {
        let msgs = [user_msg("**bold** and *italic* and \"dialogue\"")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_html(&refs, "Char", "User");
        assert!(result.contains("<strong>bold</strong>"));
        assert!(result.contains("<em>italic</em>"));
        assert!(result.contains("<q>dialogue</q>"));
    }

    #[test]
    fn jsonl_has_header() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_jsonl(&refs, "Alice", "Bob");
        let first_line = result.lines().next().unwrap();
        let header: serde_json::Value = serde_json::from_str(first_line).unwrap();
        assert_eq!(header["user_name"], "Bob");
        assert_eq!(header["character_name"], "Alice");
        assert!(header["create_date"].is_string());
    }

    #[test]
    fn jsonl_message_fields() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_jsonl(&refs, "Alice", "Bob");
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);

        let user_entry: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(user_entry["name"], "Bob");
        assert_eq!(user_entry["is_user"], true);
        assert_eq!(user_entry["mes"], "Hello Alice");
        assert_eq!(user_entry["send_date"], "2026-01-15T10:00:00Z");

        let asst_entry: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(asst_entry["name"], "Alice");
        assert_eq!(asst_entry["is_user"], false);
        assert_eq!(asst_entry["mes"], "Hi Bob!");
    }

    #[test]
    fn jsonl_system_message() {
        let msgs = [system_msg("System prompt")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_jsonl(&refs, "Char", "User");
        let lines: Vec<&str> = result.lines().collect();
        let sys_entry: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(sys_entry["name"], "System");
        assert_eq!(sys_entry["is_user"], false);
        assert_eq!(sys_entry["is_system"], true);
    }

    #[test]
    fn jsonl_applies_template_vars() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let result = render_jsonl(&refs, "Alice", "Bob");
        let lines: Vec<&str> = result.lines().collect();
        let user_entry: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(user_entry["mes"], "Hello Alice");
    }
}
