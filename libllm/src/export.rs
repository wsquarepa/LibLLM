//! Session and character export to Markdown, HTML, and JSONL formats.

use crate::preset::ReasoningPreset;
use crate::session::{self, Message, Role};
use crate::template;
use crate::thought;

/// Conversation-wide metadata threaded through Markdown and HTML exports.
///
/// Holds borrowed names so callers retain ownership of their session strings.
/// `character` and `persona` are `None` when the conversation is a plain
/// assistant chat with no roleplay identities configured; the renderer omits
/// those fields from the metadata block and substitutes neutral defaults
/// ("Assistant" / "User") only where a label is structurally required
/// (per-message speaker, `{{char}}` / `{{user}}` template vars).
pub struct ExportMeta<'a> {
    pub character: Option<&'a str>,
    pub persona: Option<&'a str>,
    pub model: Option<&'a str>,
    pub template: Option<&'a str>,
    pub worldbooks: &'a [String],
    pub exported_at: &'a str,
}

impl<'a> ExportMeta<'a> {
    fn character_label(&self) -> &str {
        self.character.unwrap_or("Assistant")
    }

    fn persona_label(&self) -> &str {
        self.persona.unwrap_or("User")
    }

    fn title(&self) -> String {
        match (self.persona, self.character) {
            (None, None) => "Conversation".to_owned(),
            _ => format!("{} & {}", self.persona_label(), self.character_label()),
        }
    }
}

fn thought_label(seconds: Option<u32>) -> String {
    match seconds {
        Some(1) => "Thought for 1 second".to_owned(),
        Some(n) => format!("Thought for {n} seconds"),
        None => "Thought for a moment".to_owned(),
    }
}

fn role_label<'a>(role: &Role, char_name: &'a str, user_name: &'a str) -> &'a str {
    match role {
        Role::User => user_name,
        Role::Assistant => char_name,
        Role::System | Role::Summary => "System",
    }
}

fn period_range(messages: &[&Message]) -> Option<(String, String)> {
    let first = messages.first()?.timestamp.clone();
    let last = messages.last()?.timestamp.clone();
    Some((first, last))
}

fn markdown_format_assistant_body(
    content: &str,
    msg: &Message,
    preset: Option<&ReasoningPreset>,
) -> String {
    let split = thought::split_first_think_block(content, preset);
    let Some(thought) = split.thought else {
        return content.to_owned();
    };
    if !split.closed {
        return thought.to_owned();
    }
    let label = thought_label(msg.thought_seconds);
    if split.after.is_empty() {
        format!("<details>\n<summary>{label}</summary>\n\n{thought}\n\n</details>")
    } else {
        format!(
            "<details>\n<summary>{label}</summary>\n\n{thought}\n\n</details>\n\n{}",
            split.after
        )
    }
}

fn markdown_metadata_block(meta: &ExportMeta, messages: &[&Message]) -> String {
    let mut lines = Vec::new();
    lines.push(format!("- **Exported:** {}", meta.exported_at));
    if let Some(model) = meta.model {
        lines.push(format!("- **Model:** {model}"));
    }
    if let Some(template) = meta.template {
        lines.push(format!("- **Template:** {template}"));
    }
    if let Some(character) = meta.character {
        lines.push(format!("- **Character:** {character}"));
    }
    if let Some(persona) = meta.persona {
        lines.push(format!("- **Persona:** {persona}"));
    }
    if !meta.worldbooks.is_empty() {
        lines.push(format!("- **Worldbooks:** {}", meta.worldbooks.join(", ")));
    }
    lines.push(format!("- **Messages:** {}", messages.len()));
    if let Some((first, last)) = period_range(messages) {
        if first == last {
            lines.push(format!("- **Recorded:** {first}"));
        } else {
            lines.push(format!("- **Period:** {first} – {last}"));
        }
    }
    lines.join("\n")
}

pub fn render_markdown(
    messages: &[&Message],
    meta: &ExportMeta,
    reasoning_preset: Option<&ReasoningPreset>,
) -> String {
    let _span = tracing::info_span!("export.markdown", message_count = messages.len()).entered();
    let mut out = String::new();

    let character_label = meta.character_label();
    let persona_label = meta.persona_label();

    out.push_str(&format!("# {}\n\n", meta.title()));
    out.push_str(&markdown_metadata_block(meta, messages));
    out.push_str("\n\n---\n\n");

    for msg in messages {
        let role = role_label(&msg.role, character_label, persona_label);
        let content = template::apply_template_vars(&msg.content, character_label, persona_label);
        let body = if msg.role == Role::Assistant {
            markdown_format_assistant_body(&content, msg, reasoning_preset)
        } else {
            content
        };
        out.push_str(&format!(
            "## {role} · {}\n\n{body}\n\n---\n\n",
            msg.timestamp
        ));
    }
    tracing::info!(phase = "done", output_bytes = out.len(), "export.markdown");
    out
}

pub fn render_html(
    messages: &[&Message],
    meta: &ExportMeta,
    reasoning_preset: Option<&ReasoningPreset>,
) -> String {
    let _span = tracing::info_span!("export.html", message_count = messages.len()).entered();

    let character_label = meta.character_label();
    let persona_label = meta.persona_label();

    let mut turns = String::new();
    for (idx, msg) in messages.iter().enumerate() {
        let role = role_label(&msg.role, character_label, persona_label);
        let content = template::apply_template_vars(&msg.content, character_label, persona_label);
        let formatted = if msg.role == Role::Assistant {
            html_format_assistant_body(&content, msg, reasoning_preset)
        } else {
            html_format_content(&content)
        };
        let class = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System | Role::Summary => "system",
        };
        turns.push_str(&format!(
            "      <li class=\"turn {class}\" style=\"--i: {idx};\">\n\
             \x20       <div class=\"speaker\">{}</div>\n\
             \x20       <div class=\"utterance\">\n\
             \x20         <div class=\"body\">{formatted}</div>\n\
             \x20         <time>{}</time>\n\
             \x20       </div>\n\
             \x20     </li>\n",
            html_escape(role),
            html_escape(&msg.timestamp),
        ));
    }

    let dossier = render_dossier(meta, messages);
    let dateline = render_dateline(messages);
    let title_text = meta.title();
    let title_escaped = html_escape(&title_text);
    let masthead = render_masthead(meta, &title_escaped);

    let out = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title_escaped} — Transcript</title>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Fraunces:ital,opsz,wght,SOFT@0,9..144,300..900,30..100;1,9..144,300..700,30..100&family=Newsreader:ital,opsz,wght@0,6..72,300..700;1,6..72,300..700&family=JetBrains+Mono:wght@400;600&display=swap">
  <style>
    :root {{
      --paper: #f3ead4;
      --paper-shadow: #ebe0c2;
      --ink: #1a1410;
      --ink-soft: #574c41;
      --ink-faint: #968a78;
      --rule: #d6c9a4;
      --accent: #a23a2a;
      --user-tone: #2c5044;
      --char-tone: #6b3a5a;
    }}

    @media (prefers-color-scheme: dark) {{
      :root {{
        --paper: #15171c;
        --paper-shadow: #1d1f25;
        --ink: #ece4cf;
        --ink-soft: #b6ad96;
        --ink-faint: #74705e;
        --rule: #2f3138;
        --accent: #d76a55;
        --user-tone: #a8c4b0;
        --char-tone: #d09bb6;
      }}
    }}

    *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}

    html {{ background: var(--paper); }}

    body {{
      background: var(--paper);
      color: var(--ink);
      font-family: 'Newsreader', Iowan Old Style, Georgia, 'Times New Roman', serif;
      font-size: 1.0625rem;
      line-height: 1.65;
      font-feature-settings: 'kern', 'liga', 'onum';
      text-rendering: optimizeLegibility;
      -webkit-font-smoothing: antialiased;
      -moz-osx-font-smoothing: grayscale;
    }}

    .transcript {{
      max-width: 60rem;
      margin: 0 auto;
      padding: 5rem 2.5rem 6rem;
    }}

    .masthead {{
      text-align: center;
      padding-bottom: 2.5rem;
      border-bottom: 1px solid var(--rule);
    }}

    .overline {{
      font-family: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace;
      text-transform: uppercase;
      letter-spacing: 0.45em;
      font-size: 0.68rem;
      color: var(--ink-faint);
      padding-left: 0.45em;
      margin-bottom: 1.5rem;
    }}

    .masthead h1 {{
      font-family: 'Fraunces', 'Iowan Old Style', Georgia, serif;
      font-weight: 380;
      font-size: clamp(2.4rem, 7.5vw, 4.75rem);
      line-height: 1.02;
      letter-spacing: -0.02em;
      font-variation-settings: 'opsz' 144, 'SOFT' 50;
    }}

    .masthead h1 .ampersand {{
      display: inline-block;
      margin: 0 0.18em;
      font-weight: 300;
      font-style: italic;
      color: var(--accent);
      font-variation-settings: 'opsz' 144, 'SOFT' 100;
      transform: translateY(-0.04em);
    }}

    .dateline {{
      margin-top: 1.5rem;
      font-family: 'JetBrains Mono', ui-monospace, monospace;
      font-size: 0.72rem;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      color: var(--ink-soft);
    }}

    .dossier {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
      gap: 1.5rem 2.5rem;
      padding: 2rem 0 2.5rem;
      margin-bottom: 3.5rem;
      border-bottom: 1px solid var(--rule);
    }}

    .dossier .field {{ display: flex; flex-direction: column; gap: 0.35rem; }}

    .dossier dt {{
      font-family: 'JetBrains Mono', ui-monospace, monospace;
      text-transform: uppercase;
      letter-spacing: 0.18em;
      font-size: 0.62rem;
      color: var(--ink-faint);
    }}

    .dossier dd {{
      font-family: 'Newsreader', serif;
      font-size: 1rem;
      font-weight: 500;
      color: var(--ink);
      word-break: break-word;
    }}

    .dialogue {{
      list-style: none;
      display: flex;
      flex-direction: column;
      gap: 2.75rem;
    }}

    .turn {{
      display: grid;
      grid-template-columns: 9rem 1fr;
      gap: 2.25rem;
      opacity: 0;
      transform: translateY(10px);
      animation: settle 700ms cubic-bezier(0.2, 0.75, 0.2, 1) forwards;
      animation-delay: calc(var(--i, 0) * 55ms);
    }}

    @keyframes settle {{
      to {{ opacity: 1; transform: none; }}
    }}

    @media (prefers-reduced-motion: reduce) {{
      .turn {{ animation: none; opacity: 1; transform: none; }}
    }}

    .speaker {{
      font-family: 'JetBrains Mono', ui-monospace, monospace;
      font-size: 0.72rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.2em;
      padding-top: 0.35rem;
      text-align: right;
      align-self: start;
      position: sticky;
      top: 1.25rem;
      word-break: break-word;
    }}

    .turn.user .speaker {{ color: var(--user-tone); }}
    .turn.assistant .speaker {{ color: var(--char-tone); }}
    .turn.system .speaker {{ color: var(--accent); }}

    .utterance {{
      border-left: 1px solid var(--rule);
      padding-left: 2rem;
    }}

    .turn.user .utterance {{ border-left-color: var(--user-tone); }}
    .turn.assistant .utterance {{ border-left-color: var(--char-tone); }}
    .turn.system .utterance {{ border-left-color: var(--accent); }}

    .body {{
      white-space: pre-wrap;
      word-wrap: break-word;
      hyphens: auto;
    }}

    .turn.system .body {{
      font-style: italic;
      color: var(--ink-soft);
      font-size: 0.97rem;
    }}

    .body q {{
      quotes: '\201C' '\201D' '\2018' '\2019';
      color: var(--accent);
      font-style: italic;
    }}

    .body strong {{ font-weight: 600; }}
    .body em {{ font-style: italic; color: var(--ink-soft); }}

    time {{
      display: block;
      margin-top: 1rem;
      font-family: 'JetBrains Mono', ui-monospace, monospace;
      font-size: 0.68rem;
      letter-spacing: 0.14em;
      color: var(--ink-faint);
    }}

    details.thought {{
      margin-bottom: 1.25rem;
      padding: 0.85rem 1.1rem 1rem;
      background: var(--paper-shadow);
      border-left: 2px solid var(--ink-faint);
      font-size: 0.95rem;
      color: var(--ink-soft);
    }}

    details.thought summary {{
      cursor: pointer;
      list-style: none;
      font-family: 'JetBrains Mono', ui-monospace, monospace;
      font-size: 0.68rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.18em;
      color: var(--ink-faint);
    }}

    details.thought summary::-webkit-details-marker {{ display: none; }}

    details.thought summary::before {{
      content: '+';
      display: inline-block;
      width: 1em;
      margin-right: 0.4em;
      transition: transform 200ms ease;
    }}

    details.thought[open] summary::before {{
      content: '−';
    }}

    details.thought > *:not(summary) {{
      margin-top: 0.7rem;
      font-style: italic;
      white-space: pre-wrap;
    }}

    .colophon {{
      margin-top: 5rem;
      padding-top: 2rem;
      border-top: 1px solid var(--rule);
      text-align: center;
      font-family: 'JetBrains Mono', ui-monospace, monospace;
      font-size: 0.68rem;
      letter-spacing: 0.3em;
      text-transform: uppercase;
      color: var(--ink-faint);
    }}

    .colophon .mark {{
      display: block;
      font-family: 'Fraunces', serif;
      font-style: italic;
      font-size: 1.5rem;
      letter-spacing: 0;
      color: var(--accent);
      margin-bottom: 0.75rem;
      font-variation-settings: 'opsz' 144, 'SOFT' 100;
    }}

    @media (max-width: 760px) {{
      .transcript {{ padding: 2.5rem 1.25rem 4rem; }}
      .turn {{
        grid-template-columns: 1fr;
        gap: 0.65rem;
      }}
      .speaker {{
        text-align: left;
        position: static;
        padding-top: 0;
      }}
      .utterance {{ padding-left: 1.1rem; }}
      .dossier {{ gap: 1.25rem 1.5rem; padding: 1.5rem 0 2rem; margin-bottom: 2.5rem; }}
    }}
  </style>
</head>
<body>
  <article class="transcript">
    <header class="masthead">
      <p class="overline">A Transcript</p>
{masthead}      <p class="dateline">{dateline}</p>
    </header>
    <dl class="dossier">
{dossier}    </dl>
    <ol class="dialogue">
{turns}    </ol>
    <footer class="colophon">
      <span class="mark">§</span>
      Exported from LibLLM
    </footer>
  </article>
</body>
</html>
"#
    );
    tracing::info!(phase = "done", output_bytes = out.len(), "export.html");
    out
}

fn render_masthead(meta: &ExportMeta, title_escaped: &str) -> String {
    match (meta.persona, meta.character) {
        (None, None) => format!("      <h1>{title_escaped}</h1>\n"),
        _ => {
            let persona = html_escape(meta.persona_label());
            let character = html_escape(meta.character_label());
            format!(
                "      <h1>\n\
                 \x20       <span class=\"user\">{persona}</span><span class=\"ampersand\">&amp;</span><span class=\"character\">{character}</span>\n\
                 \x20     </h1>\n"
            )
        }
    }
}

fn render_dossier(meta: &ExportMeta, messages: &[&Message]) -> String {
    let mut fields: Vec<(&str, String)> = Vec::new();
    fields.push(("Exported", meta.exported_at.to_owned()));
    if let Some(model) = meta.model {
        fields.push(("Model", model.to_owned()));
    }
    if let Some(template) = meta.template {
        fields.push(("Template", template.to_owned()));
    }
    if let Some(character) = meta.character {
        fields.push(("Character", character.to_owned()));
    }
    if let Some(persona) = meta.persona {
        fields.push(("Persona", persona.to_owned()));
    }
    if !meta.worldbooks.is_empty() {
        fields.push(("Worldbooks", meta.worldbooks.join(", ")));
    }
    fields.push(("Messages", messages.len().to_string()));

    let mut out = String::new();
    for (label, value) in fields {
        out.push_str(&format!(
            "      <div class=\"field\"><dt>{}</dt><dd>{}</dd></div>\n",
            html_escape(label),
            html_escape(&value),
        ));
    }
    out
}

fn render_dateline(messages: &[&Message]) -> String {
    let count = messages.len();
    let exchange_word = if count == 1 { "exchange" } else { "exchanges" };
    match period_range(messages) {
        Some((first, last)) if first == last => {
            html_escape(&format!("{first} · {count} {exchange_word}"))
        }
        Some((first, last)) => html_escape(&format!("{first} → {last} · {count} {exchange_word}")),
        None => html_escape(&format!("{count} {exchange_word}")),
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
        if bytes[i] == b'*'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'*'
            && let Some(end) = find_delimiter(&escaped[i + 2..], "**")
        {
            let inner = &escaped[i + 2..i + 2 + end];
            out.push_str("<strong>");
            out.push_str(inner);
            out.push_str("</strong>");
            i += 2 + end + 2;
            continue;
        }

        if bytes[i] == b'*'
            && let Some(end) = find_delimiter(&escaped[i + 1..], "*")
        {
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

fn html_format_assistant_body(
    content: &str,
    msg: &Message,
    preset: Option<&ReasoningPreset>,
) -> String {
    let split = thought::split_first_think_block(content, preset);
    let Some(thought) = split.thought else {
        return html_format_content(content);
    };
    if !split.closed {
        return html_format_content(thought);
    }
    let label = html_escape(&thought_label(msg.thought_seconds));
    let thought_formatted = html_format_content(thought);
    if split.after.is_empty() {
        format!(
            "<details class=\"thought\"><summary>{label}</summary>\n{thought_formatted}\n</details>"
        )
    } else {
        let after_formatted = html_format_content(split.after);
        format!(
            "<details class=\"thought\"><summary>{label}</summary>\n{thought_formatted}\n</details>\n{after_formatted}"
        )
    }
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

    fn deepseek() -> ReasoningPreset {
        ReasoningPreset {
            name: "DeepSeek".to_owned(),
            prefix: "<think>\n".to_owned(),
            suffix: "\n</think>".to_owned(),
            separator: "\n\n".to_owned(),
        }
    }

    fn test_messages() -> Vec<Message> {
        vec![
            Message {
                role: Role::User,
                content: "Hello {{char}}".to_owned(),
                timestamp: "2026-01-15T10:00:00Z".to_owned(),
                thought_seconds: None,
            },
            Message {
                role: Role::Assistant,
                content: "Hi {{user}}!".to_owned(),
                timestamp: "2026-01-15T10:00:05Z".to_owned(),
                thought_seconds: None,
            },
        ]
    }

    fn meta_with(character: &'static str, persona: &'static str) -> ExportMeta<'static> {
        ExportMeta {
            character: Some(character),
            persona: Some(persona),
            model: None,
            template: None,
            worldbooks: &[],
            exported_at: "2026-05-05T12:00:00Z",
        }
    }

    fn meta_blank() -> ExportMeta<'static> {
        ExportMeta {
            character: None,
            persona: None,
            model: None,
            template: None,
            worldbooks: &[],
            exported_at: "2026-05-05T12:00:00Z",
        }
    }

    #[test]
    fn markdown_emits_title_and_metadata() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_markdown(&refs, &meta, None);
        assert!(result.starts_with("# Bob & Alice\n\n"));
        assert!(result.contains("- **Exported:** 2026-05-05T12:00:00Z"));
        assert!(result.contains("- **Character:** Alice"));
        assert!(result.contains("- **Persona:** Bob"));
        assert!(result.contains("- **Messages:** 2"));
        assert!(result.contains("- **Period:** 2026-01-15T10:00:00Z – 2026-01-15T10:00:05Z"));
    }

    #[test]
    fn markdown_messages_use_role_and_timestamp_heading() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_markdown(&refs, &meta, None);
        assert!(result.contains("## Bob · 2026-01-15T10:00:00Z\n\nHello Alice"));
        assert!(result.contains("## Alice · 2026-01-15T10:00:05Z\n\nHi Bob!"));
    }

    #[test]
    fn markdown_includes_optional_metadata_when_present() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let worldbooks = vec!["Lore".to_owned(), "Settings".to_owned()];
        let meta = ExportMeta {
            character: Some("Alice"),
            persona: Some("Bob"),
            model: Some("gpt-test"),
            template: Some("chatml"),
            worldbooks: &worldbooks,
            exported_at: "2026-05-05T12:00:00Z",
        };
        let result = render_markdown(&refs, &meta, None);
        assert!(result.contains("- **Model:** gpt-test"));
        assert!(result.contains("- **Template:** chatml"));
        assert!(result.contains("- **Worldbooks:** Lore, Settings"));
    }

    #[test]
    fn markdown_omits_optional_metadata_when_absent() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_markdown(&refs, &meta, None);
        assert!(!result.contains("- **Model:**"));
        assert!(!result.contains("- **Template:**"));
        assert!(!result.contains("- **Worldbooks:**"));
    }

    #[test]
    fn markdown_omits_character_and_persona_when_unset() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_blank();
        let result = render_markdown(&refs, &meta, None);
        assert!(result.starts_with("# Conversation\n\n"));
        assert!(!result.contains("- **Character:**"));
        assert!(!result.contains("- **Persona:**"));
        assert!(result.contains("## User · 2026-01-15T10:00:00Z\n\nHello Assistant"));
        assert!(result.contains("## Assistant · 2026-01-15T10:00:05Z\n\nHi User!"));
    }

    #[test]
    fn markdown_includes_character_only_when_persona_unset() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = ExportMeta {
            character: Some("Alice"),
            persona: None,
            model: None,
            template: None,
            worldbooks: &[],
            exported_at: "2026-05-05T12:00:00Z",
        };
        let result = render_markdown(&refs, &meta, None);
        assert!(result.starts_with("# User & Alice\n\n"));
        assert!(result.contains("- **Character:** Alice"));
        assert!(!result.contains("- **Persona:**"));
    }

    #[test]
    fn markdown_system_message() {
        let msgs = [system_msg("You are helpful.")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, None);
        assert!(result.contains("## System · "));
        assert!(result.contains("You are helpful."));
    }

    #[test]
    fn markdown_empty_still_renders_header() {
        let refs: Vec<&Message> = vec![];
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, None);
        assert!(result.starts_with("# User & Char\n\n"));
        assert!(result.contains("- **Character:** Char"));
        assert!(result.contains("- **Persona:** User"));
        assert!(result.contains("- **Messages:** 0"));
        assert!(!result.contains("- **Period:**"));
        assert!(!result.contains("- **Recorded:**"));
    }

    #[test]
    fn markdown_recorded_when_single_message() {
        let msgs = [user_msg("Hello")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, None);
        assert!(result.contains("- **Recorded:** "));
        assert!(!result.contains("- **Period:**"));
    }

    #[test]
    fn html_escapes_content() {
        let msgs = [user_msg("<script>alert('xss')</script>")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("&lt;script&gt;"));
        assert!(!result.contains("<script>alert"));
    }

    #[test]
    fn html_has_structure() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_html(&refs, &meta, None);
        assert!(result.starts_with("<!DOCTYPE html>"));
        assert!(result.contains("class=\"transcript\""));
        assert!(result.contains("class=\"masthead\""));
        assert!(result.contains("class=\"dossier\""));
        assert!(result.contains("class=\"dialogue\""));
        assert!(result.contains("class=\"turn user\""));
        assert!(result.contains("class=\"turn assistant\""));
        assert!(result.contains("class=\"colophon\""));
    }

    #[test]
    fn html_dossier_includes_optional_metadata() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let worldbooks = vec!["Lore".to_owned()];
        let meta = ExportMeta {
            character: Some("Alice"),
            persona: Some("Bob"),
            model: Some("gpt-test"),
            template: Some("chatml"),
            worldbooks: &worldbooks,
            exported_at: "2026-05-05T12:00:00Z",
        };
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("<dt>Model</dt><dd>gpt-test</dd>"));
        assert!(result.contains("<dt>Template</dt><dd>chatml</dd>"));
        assert!(result.contains("<dt>Worldbooks</dt><dd>Lore</dd>"));
        assert!(result.contains("<dt>Character</dt><dd>Alice</dd>"));
        assert!(result.contains("<dt>Persona</dt><dd>Bob</dd>"));
        assert!(result.contains("<dt>Messages</dt><dd>2</dd>"));
        assert!(result.contains("<dt>Exported</dt><dd>2026-05-05T12:00:00Z</dd>"));
    }

    #[test]
    fn html_dossier_omits_absent_optional_metadata() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_html(&refs, &meta, None);
        assert!(!result.contains("<dt>Model</dt>"));
        assert!(!result.contains("<dt>Template</dt>"));
        assert!(!result.contains("<dt>Worldbooks</dt>"));
    }

    #[test]
    fn html_omits_character_and_persona_when_unset() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_blank();
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("<title>Conversation — Transcript</title>"));
        assert!(result.contains("<h1>Conversation</h1>"));
        assert!(!result.contains("<dt>Character</dt>"));
        assert!(!result.contains("<dt>Persona</dt>"));
        assert!(!result.contains("class=\"ampersand\""));
        assert!(result.contains("<div class=\"speaker\">User</div>"));
        assert!(result.contains("<div class=\"speaker\">Assistant</div>"));
    }

    #[test]
    fn html_dateline_includes_period_and_count() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("2026-01-15T10:00:00Z"));
        assert!(result.contains("2026-01-15T10:00:05Z"));
        assert!(result.contains("2 exchanges"));
    }

    #[test]
    fn html_uses_stagger_index_per_turn() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("style=\"--i: 0;\""));
        assert!(result.contains("style=\"--i: 1;\""));
    }

    #[test]
    fn html_applies_template_vars() {
        let msgs = test_messages();
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Alice", "Bob");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("Hello Alice"));
        assert!(result.contains("Hi Bob!"));
    }

    #[test]
    fn html_formats_bold() {
        let msgs = [user_msg("This is **bold** text")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("<strong>bold</strong>"));
        assert!(!result.contains("**bold**"));
    }

    #[test]
    fn html_formats_italic() {
        let msgs = [user_msg("This is *italic* text")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("<em>italic</em>"));
        assert!(!result.contains("*italic*"));
    }

    #[test]
    fn html_formats_dialogue() {
        let msgs = [assistant_msg("She said \"hello there\" softly")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, None);
        assert!(result.contains("<q>hello there</q>"));
    }

    #[test]
    fn html_formats_mixed_markdown() {
        let msgs = [user_msg("**bold** and *italic* and \"dialogue\"")];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, None);
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

    fn assistant_thought_msg(content: &str, thought_seconds: Option<u32>) -> Message {
        Message {
            role: Role::Assistant,
            content: content.to_owned(),
            timestamp: "2026-01-15T10:00:05Z".to_owned(),
            thought_seconds,
        }
    }

    #[test]
    fn markdown_collapses_explicit_thought_block() {
        let msgs = [assistant_thought_msg(
            "<think>planning</think>Answer",
            Some(12),
        )];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, Some(&preset));
        assert!(result.contains("<details>\n<summary>Thought for 12 seconds</summary>"));
        assert!(result.contains("\n\nplanning\n\n"));
        assert!(result.contains("</details>\n\nAnswer"));
        assert!(!result.contains("<think>"));
    }

    #[test]
    fn markdown_uses_moment_label_when_duration_unknown() {
        let msgs = [assistant_thought_msg("<think>musing</think>Answer", None)];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, Some(&preset));
        assert!(result.contains("<summary>Thought for a moment</summary>"));
        assert!(result.contains("</details>\n\nAnswer"));
    }

    #[test]
    fn markdown_content_without_opener_is_not_collapsed() {
        let msgs = [assistant_thought_msg("musing</think>Answer", Some(5))];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, Some(&preset));
        assert!(!result.contains("<details>"));
        assert!(result.contains("musing</think>Answer"));
    }

    #[test]
    fn markdown_unclosed_explicit_thought_drops_opening_marker() {
        let msgs = [assistant_thought_msg("<think>still thinking", None)];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, Some(&preset));
        assert!(!result.contains("<think>"));
        assert!(!result.contains("<details>"));
        assert!(result.contains("still thinking"));
    }

    #[test]
    fn markdown_leaves_later_literal_think_tags_in_body() {
        let msgs = [assistant_thought_msg(
            "<think>a</think>code: <think>b</think>",
            Some(3),
        )];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, Some(&preset));
        assert!(result.contains("</details>\n\ncode: <think>b</think>"));
    }

    #[test]
    fn markdown_without_preset_preserves_raw_think_tags() {
        let msgs = [assistant_thought_msg(
            "<think>planning</think>Answer",
            Some(12),
        )];
        let refs: Vec<&Message> = msgs.iter().collect();
        let meta = meta_with("Char", "User");
        let result = render_markdown(&refs, &meta, None);
        assert!(!result.contains("<details>"));
        assert!(result.contains("<think>planning</think>Answer"));
    }

    #[test]
    fn html_collapses_explicit_thought_block() {
        let msgs = [assistant_thought_msg(
            "<think>planning</think>Hi!",
            Some(7),
        )];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, Some(&preset));
        assert!(
            result.contains("<details class=\"thought\"><summary>Thought for 7 seconds</summary>")
        );
        assert!(result.contains("</summary>\nplanning\n</details>"));
        assert!(result.contains("Hi!"));
    }

    #[test]
    fn html_assistant_without_thought_is_unaffected() {
        let msgs = [assistant_thought_msg("Just an answer", None)];
        let refs: Vec<&Message> = msgs.iter().collect();
        let preset = deepseek();
        let meta = meta_with("Char", "User");
        let result = render_html(&refs, &meta, Some(&preset));
        assert!(!result.contains("<details"));
        assert!(result.contains("Just an answer"));
    }
}
