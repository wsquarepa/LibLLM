//! CLI flag overrides: [`CliOverrides`] bundles per-invocation flag values that shadow config fields.

use super::auth::{AuthKind, AuthOverrides};

/// CLI flag values that override corresponding config fields; overridden fields display in red in `/config`.
#[derive(Default)]
pub struct CliOverrides {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub tls_skip_verify: bool,
    pub sampling: crate::sampling::SamplingOverrides,
    pub system_prompt: Option<String>,
    pub persona: Option<String>,
    pub characters: Vec<String>,
    pub chat_mode: Option<crate::group_chat::ChatMode>,
    pub scenario: Option<String>,
    pub talkativeness: std::collections::HashMap<String, f32>,
    pub author_note: Option<String>,
    pub author_note_depth: Option<u32>,
    pub author_note_at_top: Option<bool>,
    pub no_summarize: bool,
    pub auth_type: Option<AuthKind>,
    pub auth_basic_username: Option<String>,
    pub auth_basic_password: Option<String>,
    pub auth_bearer_token: Option<String>,
    pub auth_header_name: Option<String>,
    pub auth_header_value: Option<String>,
    pub auth_query_name: Option<String>,
    pub auth_query_value: Option<String>,
}

impl CliOverrides {
    pub fn auth_overrides(&self) -> AuthOverrides {
        AuthOverrides {
            auth_type: self.auth_type,
            auth_basic_username: self.auth_basic_username.clone(),
            auth_basic_password: self.auth_basic_password.clone(),
            auth_bearer_token: self.auth_bearer_token.clone(),
            auth_header_name: self.auth_header_name.clone(),
            auth_header_value: self.auth_header_value.clone(),
            auth_query_name: self.auth_query_name.clone(),
            auth_query_value: self.auth_query_value.clone(),
        }
    }
}
