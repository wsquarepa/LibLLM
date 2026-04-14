use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingParams {
    pub temperature: f64,
    pub top_k: i64,
    pub top_p: f64,
    pub min_p: f64,
    pub repeat_last_n: i64,
    pub repeat_penalty: f64,
    pub max_tokens: i64,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: 0.8,
            top_k: 40,
            top_p: 0.95,
            min_p: 0.05,
            repeat_last_n: 64,
            repeat_penalty: 1.0,
            max_tokens: -1,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SamplingOverrides {
    pub temperature: Option<f64>,
    pub top_k: Option<i64>,
    pub top_p: Option<f64>,
    pub min_p: Option<f64>,
    pub repeat_last_n: Option<i64>,
    pub repeat_penalty: Option<f64>,
    pub max_tokens: Option<i64>,
}

impl SamplingParams {
    pub fn with_overrides(self, overrides: &SamplingOverrides) -> Self {
        Self {
            temperature: overrides.temperature.unwrap_or(self.temperature),
            top_k: overrides.top_k.unwrap_or(self.top_k),
            top_p: overrides.top_p.unwrap_or(self.top_p),
            min_p: overrides.min_p.unwrap_or(self.min_p),
            repeat_last_n: overrides.repeat_last_n.unwrap_or(self.repeat_last_n),
            repeat_penalty: overrides.repeat_penalty.unwrap_or(self.repeat_penalty),
            max_tokens: overrides.max_tokens.unwrap_or(self.max_tokens),
        }
    }
}
