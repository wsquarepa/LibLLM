//! Authentication configuration: [`AuthKind`], [`Auth`], [`AuthOverrides`], and [`resolve_auth`].

use serde::{Deserialize, Serialize};

use super::Config;

/// Discriminator for `Auth` — used for labels and UI state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthKind {
    None,
    Basic,
    Bearer,
    Header,
    Query,
}

impl std::fmt::Display for AuthKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AuthKind::None => "None",
            AuthKind::Basic => "Basic",
            AuthKind::Bearer => "Bearer",
            AuthKind::Header => "Header",
            AuthKind::Query => "Query",
        };
        f.write_str(s)
    }
}

/// Outbound-request authentication configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
pub enum Auth {
    #[default]
    None,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
    },
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
        value: String,
    },
}

impl Auth {
    pub fn kind(&self) -> AuthKind {
        match self {
            Auth::None => AuthKind::None,
            Auth::Basic { .. } => AuthKind::Basic,
            Auth::Bearer { .. } => AuthKind::Bearer,
            Auth::Header { .. } => AuthKind::Header,
            Auth::Query { .. } => AuthKind::Query,
        }
    }

    pub fn basic_username(&self) -> String {
        match self {
            Auth::Basic { username, .. } => username.clone(),
            _ => String::new(),
        }
    }

    pub fn basic_password(&self) -> String {
        match self {
            Auth::Basic { password, .. } => password.clone(),
            _ => String::new(),
        }
    }

    pub fn bearer_token(&self) -> String {
        match self {
            Auth::Bearer { token } => token.clone(),
            _ => String::new(),
        }
    }

    pub fn header_name(&self) -> String {
        match self {
            Auth::Header { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    pub fn header_value(&self) -> String {
        match self {
            Auth::Header { value, .. } => value.clone(),
            _ => String::new(),
        }
    }

    pub fn query_name(&self) -> String {
        match self {
            Auth::Query { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    pub fn query_value(&self) -> String {
        match self {
            Auth::Query { value, .. } => value.clone(),
            _ => String::new(),
        }
    }
}

/// Plain-data bundle of CLI- and env-sourced overrides for the `Auth` config.
/// Populated by the `client` crate from `CliOverrides` and env vars.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthOverrides {
    pub auth_type: Option<AuthKind>,
    pub auth_basic_username: Option<String>,
    pub auth_basic_password: Option<String>,
    pub auth_bearer_token: Option<String>,
    pub auth_header_name: Option<String>,
    pub auth_header_value: Option<String>,
    pub auth_query_name: Option<String>,
    pub auth_query_value: Option<String>,
}

fn pick(override_value: &Option<String>, fallback: String) -> String {
    override_value.clone().unwrap_or(fallback)
}

/// Resolves the effective `Auth` by merging CLI/env overrides onto the on-disk config.
///
/// Precedence: CLI/env > on-disk config. Field accessors return empty strings when the
/// on-disk variant doesn't match the effective kind, so a CLI-set type can stand alone.
pub fn resolve_auth(config: &Config, overrides: &AuthOverrides) -> Auth {
    let kind = overrides.auth_type.unwrap_or_else(|| config.auth.kind());
    match kind {
        AuthKind::None => Auth::None,
        AuthKind::Basic => Auth::Basic {
            username: pick(&overrides.auth_basic_username, config.auth.basic_username()),
            password: pick(&overrides.auth_basic_password, config.auth.basic_password()),
        },
        AuthKind::Bearer => Auth::Bearer {
            token: pick(&overrides.auth_bearer_token, config.auth.bearer_token()),
        },
        AuthKind::Header => Auth::Header {
            name: pick(&overrides.auth_header_name, config.auth.header_name()),
            value: pick(&overrides.auth_header_value, config.auth.header_value()),
        },
        AuthKind::Query => Auth::Query {
            name: pick(&overrides.auth_query_name, config.auth.query_name()),
            value: pick(&overrides.auth_query_value, config.auth.query_value()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn auth_default_is_none() {
        let auth = Auth::default();
        assert_eq!(auth, Auth::None);
        assert_eq!(auth.kind(), AuthKind::None);
    }

    #[test]
    fn auth_round_trips_through_toml_none() {
        let cfg = Config {
            auth: Auth::None,
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::None);
    }

    #[test]
    fn auth_round_trips_through_toml_basic() {
        let cfg = Config {
            auth: Auth::Basic {
                username: "user".into(),
                password: "pw".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Basic {
                username: "user".into(),
                password: "pw".into()
            }
        );
    }

    #[test]
    fn auth_round_trips_through_toml_bearer() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "sk-xyz".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Bearer {
                token: "sk-xyz".into()
            }
        );
    }

    #[test]
    fn auth_round_trips_through_toml_header() {
        let cfg = Config {
            auth: Auth::Header {
                name: "X-Api-Key".into(),
                value: "abc".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Header {
                name: "X-Api-Key".into(),
                value: "abc".into()
            }
        );
    }

    #[test]
    fn auth_round_trips_through_toml_query() {
        let cfg = Config {
            auth: Auth::Query {
                name: "api_key".into(),
                value: "abc".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Query {
                name: "api_key".into(),
                value: "abc".into()
            }
        );
    }

    #[test]
    fn auth_defaults_when_missing_from_toml() {
        let toml_str = r#"
            api_url = "http://localhost:5001/v1"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.auth, Auth::None);
    }

    #[test]
    fn auth_kind_display() {
        assert_eq!(AuthKind::None.to_string(), "None");
        assert_eq!(AuthKind::Basic.to_string(), "Basic");
        assert_eq!(AuthKind::Bearer.to_string(), "Bearer");
        assert_eq!(AuthKind::Header.to_string(), "Header");
        assert_eq!(AuthKind::Query.to_string(), "Query");
    }

    #[test]
    fn auth_field_accessors_return_empty_when_variant_mismatches() {
        let b = Auth::Bearer { token: "t".into() };
        assert_eq!(b.basic_username(), "");
        assert_eq!(b.basic_password(), "");
        assert_eq!(b.bearer_token(), "t");
        assert_eq!(b.header_name(), "");
        assert_eq!(b.header_value(), "");
        assert_eq!(b.query_name(), "");
        assert_eq!(b.query_value(), "");
    }

    #[test]
    fn auth_field_accessors_for_basic() {
        let b = Auth::Basic {
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(b.basic_username(), "u");
        assert_eq!(b.basic_password(), "p");
    }

    #[test]
    fn auth_field_accessors_for_header_query() {
        let h = Auth::Header {
            name: "X".into(),
            value: "1".into(),
        };
        assert_eq!(h.header_name(), "X");
        assert_eq!(h.header_value(), "1");
        let q = Auth::Query {
            name: "k".into(),
            value: "v".into(),
        };
        assert_eq!(q.query_name(), "k");
        assert_eq!(q.query_value(), "v");
    }

    #[test]
    fn resolve_auth_uses_config_when_no_overrides() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "disk-token".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides::default();
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Bearer {
                token: "disk-token".into()
            }
        );
    }

    #[test]
    fn resolve_auth_cli_type_overrides_disk() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "disk-token".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_type: Some(AuthKind::None),
            ..Default::default()
        };
        assert_eq!(resolve_auth(&cfg, &overrides), Auth::None);
    }

    #[test]
    fn resolve_auth_env_secret_overrides_disk_token() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "disk-token".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_bearer_token: Some("env-token".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Bearer {
                token: "env-token".into()
            }
        );
    }

    #[test]
    fn resolve_auth_cli_type_with_no_disk_match_empty_fields() {
        let cfg = Config {
            auth: Auth::None,
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_type: Some(AuthKind::Basic),
            auth_basic_username: Some("u".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Basic {
                username: "u".into(),
                password: String::new()
            }
        );
    }

    #[test]
    fn resolve_auth_mixes_cli_env_and_disk() {
        let cfg = Config {
            auth: Auth::Header {
                name: "X-Disk".into(),
                value: "disk-val".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_header_name: Some("X-Cli".into()),
            auth_header_value: Some("env-val".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Header {
                name: "X-Cli".into(),
                value: "env-val".into()
            }
        );
    }

    #[test]
    fn resolve_auth_none_variant_ignores_other_fields() {
        let cfg = Config::default();
        let overrides = AuthOverrides {
            auth_type: Some(AuthKind::None),
            auth_bearer_token: Some("ignored".into()),
            ..Default::default()
        };
        assert_eq!(resolve_auth(&cfg, &overrides), Auth::None);
    }
}
