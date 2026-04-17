//! Tracing subscriber assembly and `EnvFilter` resolution.

pub(super) struct ResolvedFilter {
    pub directive: String,
    pub source: &'static str,
}

pub(super) fn resolve_filter(
    flag: Option<&str>,
    env: Option<&str>,
    debug_opted_in: bool,
) -> ResolvedFilter {
    if let Some(f) = flag {
        return ResolvedFilter {
            directive: f.to_owned(),
            source: "--log-filter",
        };
    }
    match (debug_opted_in, env) {
        (true, Some(e)) => ResolvedFilter {
            directive: e.to_owned(),
            source: "LIBLLM_LOG",
        },
        (false, Some(_)) => ResolvedFilter {
            directive: "info".to_owned(),
            source: "default (LIBLLM_LOG ignored: --debug not set)",
        },
        _ => ResolvedFilter {
            directive: "info".to_owned(),
            source: "default",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_wins_over_env() {
        let r = resolve_filter(Some("debug"), Some("trace"), true);
        assert_eq!(r.directive, "debug");
        assert_eq!(r.source, "--log-filter");
    }

    #[test]
    fn env_used_when_debug_opted_in() {
        let r = resolve_filter(None, Some("trace"), true);
        assert_eq!(r.directive, "trace");
        assert_eq!(r.source, "LIBLLM_LOG");
    }

    #[test]
    fn env_ignored_without_debug() {
        let r = resolve_filter(None, Some("trace"), false);
        assert_eq!(r.directive, "info");
        assert!(r.source.contains("LIBLLM_LOG ignored"));
    }

    #[test]
    fn default_when_nothing_set() {
        let r = resolve_filter(None, None, false);
        assert_eq!(r.directive, "info");
        assert_eq!(r.source, "default");
    }
}
