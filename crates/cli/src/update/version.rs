//! Version tag parsing, normalization, and downgrade detection.

pub(super) fn parse_version_tag(s: &str) -> Option<semver::Version> {
    s.strip_prefix('v')
        .and_then(|rest| semver::Version::parse(rest).ok())
}

pub(super) fn normalize_tag(s: &str) -> String {
    if s.starts_with('v') {
        return s.to_string();
    }
    if semver::Version::parse(s).is_ok() {
        return format!("v{s}");
    }
    s.to_string()
}

pub(super) fn is_stable_target(s: &str) -> bool {
    s == "stable" || parse_version_tag(s).is_some()
}

pub(super) fn should_warn_downgrade(channel: &str, target: &str, current_version: &str) -> bool {
    if channel != "stable" {
        return false;
    }
    let Some(target_ver) = parse_version_tag(target) else {
        return false;
    };
    let Ok(current_ver) = semver::Version::parse(current_version) else {
        return false;
    };
    target_ver.cmp_precedence(&current_ver) == std::cmp::Ordering::Less
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_tag_strips_v_prefix() {
        let v = parse_version_tag("v2.6.0").unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 6);
        assert_eq!(v.patch, 0);
    }

    #[test]
    fn parse_version_tag_rejects_branch_names() {
        assert!(parse_version_tag("feat/foo").is_none());
        assert!(parse_version_tag("vfoo").is_none());
        assert!(parse_version_tag("2.6.0").is_none());
        assert!(parse_version_tag("stable").is_none());
    }

    #[test]
    fn normalize_tag_adds_v_to_bare_semver() {
        assert_eq!(normalize_tag("2.4.0"), "v2.4.0");
    }

    #[test]
    fn normalize_tag_preserves_v_prefixed_semver() {
        assert_eq!(normalize_tag("v2.4.0"), "v2.4.0");
    }

    #[test]
    fn normalize_tag_passes_through_non_semver() {
        assert_eq!(normalize_tag("feat/foo"), "feat/foo");
        assert_eq!(normalize_tag("stable"), "stable");
    }

    #[test]
    fn is_stable_target_recognizes_stable_and_v_tags() {
        assert!(is_stable_target("stable"));
        assert!(is_stable_target("v2.6.0"));
        assert!(is_stable_target("v0.0.1"));
    }

    #[test]
    fn is_stable_target_rejects_branches() {
        assert!(!is_stable_target("feat/foo"));
        assert!(!is_stable_target("master"));
        assert!(!is_stable_target("v2.6"));
        assert!(!is_stable_target("2.6.0"));
    }

    #[test]
    fn warn_downgrade_fires_when_target_older() {
        assert!(should_warn_downgrade("stable", "v2.4.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_same() {
        assert!(!should_warn_downgrade("stable", "v2.6.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_newer() {
        assert!(!should_warn_downgrade("stable", "v2.7.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_source_is_branch() {
        assert!(!should_warn_downgrade("feat/foo", "v2.4.0", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_is_branch() {
        assert!(!should_warn_downgrade("stable", "feat/foo", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_target_is_literal_stable() {
        assert!(!should_warn_downgrade("stable", "stable", "2.6.0"));
    }

    #[test]
    fn warn_downgrade_silent_when_current_version_is_unparseable() {
        assert!(!should_warn_downgrade("stable", "v2.4.0", "unknown"));
    }
}
