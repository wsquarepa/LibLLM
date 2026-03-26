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
