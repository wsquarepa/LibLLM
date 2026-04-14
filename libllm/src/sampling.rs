//! Sampling parameter types and CLI override merging for the completions API.

use serde::{Deserialize, Serialize};

/// Resolved sampling parameters sent to the completions API.
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

/// Optional per-field overrides from CLI flags or config, merged onto `SamplingParams` defaults.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let params = SamplingParams::default();
        assert!((params.temperature - 0.8).abs() < f64::EPSILON);
        assert_eq!(params.top_k, 40);
        assert!((params.top_p - 0.95).abs() < f64::EPSILON);
        assert!((params.min_p - 0.05).abs() < f64::EPSILON);
        assert_eq!(params.repeat_last_n, 64);
        assert!((params.repeat_penalty - 1.0).abs() < f64::EPSILON);
        assert_eq!(params.max_tokens, -1);
    }

    #[test]
    fn full_override() {
        let params = SamplingParams::default();
        let overrides = SamplingOverrides {
            temperature: Some(0.5),
            top_k: Some(10),
            top_p: Some(0.8),
            min_p: Some(0.1),
            repeat_last_n: Some(32),
            repeat_penalty: Some(1.2),
            max_tokens: Some(512),
        };
        let result = params.with_overrides(&overrides);

        assert!((result.temperature - 0.5).abs() < f64::EPSILON);
        assert_eq!(result.top_k, 10);
        assert!((result.top_p - 0.8).abs() < f64::EPSILON);
        assert!((result.min_p - 0.1).abs() < f64::EPSILON);
        assert_eq!(result.repeat_last_n, 32);
        assert!((result.repeat_penalty - 1.2).abs() < f64::EPSILON);
        assert_eq!(result.max_tokens, 512);
    }

    #[test]
    fn partial_override() {
        let params = SamplingParams::default();
        let overrides = SamplingOverrides {
            temperature: Some(0.3),
            top_k: None,
            top_p: None,
            min_p: None,
            repeat_last_n: None,
            repeat_penalty: None,
            max_tokens: Some(256),
        };
        let result = params.with_overrides(&overrides);

        assert!((result.temperature - 0.3).abs() < f64::EPSILON);
        assert_eq!(result.top_k, 40);
        assert!((result.top_p - 0.95).abs() < f64::EPSILON);
        assert!((result.min_p - 0.05).abs() < f64::EPSILON);
        assert_eq!(result.repeat_last_n, 64);
        assert!((result.repeat_penalty - 1.0).abs() < f64::EPSILON);
        assert_eq!(result.max_tokens, 256);
    }

    #[test]
    fn no_override() {
        let params = SamplingParams {
            temperature: 0.7,
            top_k: 50,
            top_p: 0.9,
            min_p: 0.02,
            repeat_last_n: 128,
            repeat_penalty: 1.1,
            max_tokens: 1024,
        };
        let overrides = SamplingOverrides {
            temperature: None,
            top_k: None,
            top_p: None,
            min_p: None,
            repeat_last_n: None,
            repeat_penalty: None,
            max_tokens: None,
        };
        let result = params.clone().with_overrides(&overrides);

        assert!((result.temperature - 0.7).abs() < f64::EPSILON);
        assert_eq!(result.top_k, 50);
        assert!((result.top_p - 0.9).abs() < f64::EPSILON);
        assert!((result.min_p - 0.02).abs() < f64::EPSILON);
        assert_eq!(result.repeat_last_n, 128);
        assert!((result.repeat_penalty - 1.1).abs() < f64::EPSILON);
        assert_eq!(result.max_tokens, 1024);
    }

    #[test]
    fn override_does_not_mutate_original() {
        let original = SamplingParams::default();
        let original_clone = original.clone();
        let overrides = SamplingOverrides {
            temperature: Some(0.1),
            top_k: Some(5),
            top_p: Some(0.5),
            min_p: Some(0.5),
            repeat_last_n: Some(10),
            repeat_penalty: Some(2.0),
            max_tokens: Some(100),
        };
        let _result = original.clone().with_overrides(&overrides);

        assert!((original.temperature - original_clone.temperature).abs() < f64::EPSILON);
        assert_eq!(original.top_k, original_clone.top_k);
        assert!((original.top_p - original_clone.top_p).abs() < f64::EPSILON);
        assert!((original.min_p - original_clone.min_p).abs() < f64::EPSILON);
        assert_eq!(original.repeat_last_n, original_clone.repeat_last_n);
        assert!((original.repeat_penalty - original_clone.repeat_penalty).abs() < f64::EPSILON);
        assert_eq!(original.max_tokens, original_clone.max_tokens);
    }
}
