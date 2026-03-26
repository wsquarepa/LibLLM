pub trait PromptTemplate {
    fn bos(&self, system_prompt: Option<&str>) -> String;
    fn wrap_user(&self, content: &str) -> String;
    fn assistant_end(&self) -> &str;
    fn stop_tokens(&self) -> Vec<String>;
}

pub struct Llama2Template;

impl PromptTemplate for Llama2Template {
    fn bos(&self, system_prompt: Option<&str>) -> String {
        match system_prompt {
            Some(sys) => format!("<s>[INST] {sys}\n\n"),
            None => "<s>".to_owned(),
        }
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
