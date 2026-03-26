use crate::session::Message;

pub trait PromptTemplate {
    fn name(&self) -> &str;
    fn bos(&self, system_prompt: Option<&str>) -> String;
    fn render(&self, messages: &[Message], system_prompt: Option<&str>) -> String;
    fn wrap_user(&self, content: &str) -> String;
    fn assistant_end(&self) -> &str;
    fn stop_tokens(&self) -> Vec<String>;
}

pub fn template_by_name(name: &str) -> Box<dyn PromptTemplate> {
    match name {
        "chatml" => Box::new(ChatMlTemplate),
        "mistral" => Box::new(MistralTemplate),
        "phi" => Box::new(PhiTemplate),
        "raw" => Box::new(RawTemplate),
        _ => Box::new(Llama2Template),
    }
}

fn render_from_messages(
    messages: &[Message],
    system_prompt: Option<&str>,
    template: &dyn PromptTemplate,
) -> String {
    let mut prompt = template.bos(system_prompt);
    for msg in messages {
        match msg.role.as_str() {
            "user" => prompt.push_str(&template.wrap_user(&msg.content)),
            "assistant" => {
                prompt.push_str(&msg.content);
                prompt.push_str(template.assistant_end());
            }
            _ => {}
        }
    }
    prompt
}

// --- Llama2 ---

pub struct Llama2Template;

impl PromptTemplate for Llama2Template {
    fn name(&self) -> &str {
        "llama2"
    }

    fn bos(&self, system_prompt: Option<&str>) -> String {
        match system_prompt {
            Some(sys) => format!("<s>[INST] {sys}\n\n"),
            None => "<s>".to_owned(),
        }
    }

    fn render(&self, messages: &[Message], system_prompt: Option<&str>) -> String {
        render_from_messages(messages, system_prompt, self)
    }

    fn wrap_user(&self, content: &str) -> String {
        format!("[INST] {content} [/INST]")
    }

    fn assistant_end(&self) -> &str {
        "</s>"
    }

    fn stop_tokens(&self) -> Vec<String> {
        vec!["</s>".to_owned(), "[INST]".to_owned()]
    }
}

// --- ChatML ---

pub struct ChatMlTemplate;

impl PromptTemplate for ChatMlTemplate {
    fn name(&self) -> &str {
        "chatml"
    }

    fn bos(&self, system_prompt: Option<&str>) -> String {
        match system_prompt {
            Some(sys) => format!("<|im_start|>system\n{sys}<|im_end|>\n"),
            None => String::new(),
        }
    }

    fn render(&self, messages: &[Message], system_prompt: Option<&str>) -> String {
        render_from_messages(messages, system_prompt, self)
    }

    fn wrap_user(&self, content: &str) -> String {
        format!("<|im_start|>user\n{content}<|im_end|>\n<|im_start|>assistant\n")
    }

    fn assistant_end(&self) -> &str {
        "<|im_end|>\n"
    }

    fn stop_tokens(&self) -> Vec<String> {
        vec!["<|im_end|>".to_owned(), "<|im_start|>".to_owned()]
    }
}

// --- Mistral ---

pub struct MistralTemplate;

impl PromptTemplate for MistralTemplate {
    fn name(&self) -> &str {
        "mistral"
    }

    fn bos(&self, system_prompt: Option<&str>) -> String {
        match system_prompt {
            Some(sys) => format!("<s>[INST] {sys}\n\n"),
            None => "<s>".to_owned(),
        }
    }

    fn render(&self, messages: &[Message], system_prompt: Option<&str>) -> String {
        render_from_messages(messages, system_prompt, self)
    }

    fn wrap_user(&self, content: &str) -> String {
        format!("[INST] {content} [/INST]")
    }

    fn assistant_end(&self) -> &str {
        "</s>"
    }

    fn stop_tokens(&self) -> Vec<String> {
        vec!["</s>".to_owned()]
    }
}

// --- Phi ---

pub struct PhiTemplate;

impl PromptTemplate for PhiTemplate {
    fn name(&self) -> &str {
        "phi"
    }

    fn bos(&self, system_prompt: Option<&str>) -> String {
        match system_prompt {
            Some(sys) => format!("<|system|>\n{sys}<|end|>\n"),
            None => String::new(),
        }
    }

    fn render(&self, messages: &[Message], system_prompt: Option<&str>) -> String {
        render_from_messages(messages, system_prompt, self)
    }

    fn wrap_user(&self, content: &str) -> String {
        format!("<|user|>\n{content}<|end|>\n<|assistant|>\n")
    }

    fn assistant_end(&self) -> &str {
        "<|end|>\n"
    }

    fn stop_tokens(&self) -> Vec<String> {
        vec!["<|end|>".to_owned(), "<|user|>".to_owned()]
    }
}

// --- Raw (no template wrapping) ---

pub struct RawTemplate;

impl PromptTemplate for RawTemplate {
    fn name(&self) -> &str {
        "raw"
    }

    fn bos(&self, system_prompt: Option<&str>) -> String {
        match system_prompt {
            Some(sys) => format!("{sys}\n\n"),
            None => String::new(),
        }
    }

    fn render(&self, messages: &[Message], system_prompt: Option<&str>) -> String {
        render_from_messages(messages, system_prompt, self)
    }

    fn wrap_user(&self, content: &str) -> String {
        format!("{content}\n")
    }

    fn assistant_end(&self) -> &str {
        "\n"
    }

    fn stop_tokens(&self) -> Vec<String> {
        vec![]
    }
}
