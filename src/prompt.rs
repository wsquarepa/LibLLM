use crate::session::{Message, Role};

#[derive(Debug, Clone, Copy)]
pub enum Template {
    Llama2,
    ChatMl,
    Mistral,
    Phi,
    Raw,
}

impl Template {
    pub fn from_name(name: &str) -> Self {
        match name {
            "chatml" => Self::ChatMl,
            "mistral" => Self::Mistral,
            "phi" => Self::Phi,
            "raw" => Self::Raw,
            _ => Self::Llama2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Llama2 => "llama2",
            Self::ChatMl => "chatml",
            Self::Mistral => "mistral",
            Self::Phi => "phi",
            Self::Raw => "raw",
        }
    }

    pub fn render(self, messages: &[&Message], system_prompt: Option<&str>) -> String {
        let mut prompt = self.bos(system_prompt);
        for msg in messages {
            match msg.role {
                Role::User => prompt.push_str(&self.wrap_user(&msg.content)),
                Role::Assistant => {
                    prompt.push_str(&msg.content);
                    prompt.push_str(self.assistant_end());
                }
                Role::System => prompt.push_str(&self.wrap_system(&msg.content)),
            }
        }
        prompt
    }

    pub fn stop_tokens(self) -> &'static [&'static str] {
        match self {
            Self::Llama2 => &["</s>", "[INST]"],
            Self::ChatMl => &["<|im_end|>", "<|im_start|>"],
            Self::Mistral => &["</s>"],
            Self::Phi => &["<|end|>", "<|user|>"],
            Self::Raw => &[],
        }
    }

    fn bos(self, system_prompt: Option<&str>) -> String {
        match self {
            Self::Llama2 | Self::Mistral => match system_prompt {
                Some(sys) => format!("<s>[INST] {sys}\n\n"),
                None => "<s>".to_owned(),
            },
            Self::ChatMl => match system_prompt {
                Some(sys) => format!("<|im_start|>system\n{sys}<|im_end|>\n"),
                None => String::new(),
            },
            Self::Phi => match system_prompt {
                Some(sys) => format!("<|system|>\n{sys}<|end|>\n"),
                None => String::new(),
            },
            Self::Raw => match system_prompt {
                Some(sys) => format!("{sys}\n\n"),
                None => String::new(),
            },
        }
    }

    fn wrap_system(self, content: &str) -> String {
        match self {
            Self::Llama2 | Self::Mistral => format!("[INST] {content} [/INST]"),
            Self::ChatMl => format!("<|im_start|>system\n{content}<|im_end|>\n"),
            Self::Phi => format!("<|system|>\n{content}<|end|>\n"),
            Self::Raw => format!("{content}\n"),
        }
    }

    fn wrap_user(self, content: &str) -> String {
        match self {
            Self::Llama2 | Self::Mistral => format!("[INST] {content} [/INST]"),
            Self::ChatMl => {
                format!("<|im_start|>user\n{content}<|im_end|>\n<|im_start|>assistant\n")
            }
            Self::Phi => format!("<|user|>\n{content}<|end|>\n<|assistant|>\n"),
            Self::Raw => format!("{content}\n"),
        }
    }

    fn assistant_end(self) -> &'static str {
        match self {
            Self::Llama2 | Self::Mistral => "</s>",
            Self::ChatMl => "<|im_end|>\n",
            Self::Phi => "<|end|>\n",
            Self::Raw => "\n",
        }
    }
}
