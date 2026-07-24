use std::path::{Component, Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use libllm_core::config::CliOverrides;
use libllm_core::session::Session;

use super::parser::is_safe_snapshot_name;
use super::{ApiSetup, DbSetup, Matcher, Scenario, Setup, Step};
use crate::harness::Harness;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RunMode {
    Check,
    Bless,
}

#[derive(Debug)]
pub struct Failure {
    pub step_index: usize,
    pub verb: String,
    pub detail: String,
    pub screen: String,
}

#[derive(Debug, Default)]
pub struct RunReport {
    pub failures: Vec<Failure>,
}

impl RunReport {
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Builds a harness from `scenario.setup`, executes each step in order, and collects
/// ALL assertion failures without stopping at the first. `golden_dir` is where
/// `snapshot` files are read/written; `scenario_stem` is the scenario file stem used
/// to name golden files (`{stem}.{name}.snap`). `mode` selects compare vs. bless.
pub async fn run_scenario(
    scenario: &Scenario,
    golden_dir: &Path,
    scenario_stem: &str,
    mode: RunMode,
) -> anyhow::Result<RunReport> {
    let mut session = Session::default();
    let mut harness = build_harness(&scenario.setup, &mut session, golden_dir).await?;
    let mut report = RunReport::default();
    let mock_enabled = matches!(scenario.setup.api, ApiSetup::Mock);
    for (i, step) in scenario.steps.iter().enumerate() {
        execute_step(
            &mut harness,
            step,
            i,
            golden_dir,
            scenario_stem,
            mode,
            mock_enabled,
            &mut report,
        )
        .await;
    }
    Ok(report)
}

async fn build_harness<'a>(
    setup: &Setup,
    session: &'a mut Session,
    golden_dir: &Path,
) -> anyhow::Result<Harness<'a>> {
    if setup.seed.is_some() {
        anyhow::bail!("seed not yet supported by the harness runner");
    }

    let mut builder = Harness::builder().size(setup.size.0, setup.size.1);

    builder = match &setup.db {
        DbSetup::None => builder.no_db(),
        DbSetup::Temp => builder.temp_db(),
        DbSetup::Encrypted(_) => {
            anyhow::bail!("encrypted db not yet supported by the harness runner");
        }
    };

    builder = match &setup.api {
        ApiSetup::None => builder.no_api(),
        ApiSetup::Mock => builder.mock_api(),
    };

    if !setup.overrides.is_empty() {
        let mut cli_overrides = CliOverrides::default();
        for name in &setup.overrides {
            apply_override(&mut cli_overrides, name)?;
        }
        builder = builder.overrides(cli_overrides);
    }

    let _ = golden_dir;
    builder.build(session).await
}

/// Sets the `CliOverrides` field corresponding to a named override string.
///
/// `"persona_readonly"` signals that the persona is CLI-locked by setting the
/// `persona` field to `Some("")`; the app treats any `Some` value as a lock.
/// `"system_readonly"` does the same for `system_prompt`.
fn apply_override(overrides: &mut CliOverrides, name: &str) -> anyhow::Result<()> {
    match name {
        "persona_readonly" => overrides.persona = Some(String::new()),
        "system_readonly" => overrides.system_prompt = Some(String::new()),
        other => anyhow::bail!("unknown override '{other}'"),
    }
    Ok(())
}

/// Parses a key name (optionally prefixed with `Ctrl+`, `Alt+`, `Shift+`) into a
/// `(KeyCode, KeyModifiers)` pair.
///
/// Supported names: `Enter`, `Esc`, `Tab`, `BackTab`, `Up`, `Down`, `Left`,
/// `Right`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, and any
/// single character (e.g. `a`, `S`). Modifiers may be combined, e.g. `Ctrl+Shift+S`.
pub fn parse_key(name: &str) -> anyhow::Result<(KeyCode, KeyModifiers)> {
    let mut mods = KeyModifiers::NONE;
    let mut remainder = name;

    loop {
        if let Some(rest) = remainder.strip_prefix("Ctrl+") {
            mods |= KeyModifiers::CONTROL;
            remainder = rest;
        } else if let Some(rest) = remainder.strip_prefix("Alt+") {
            mods |= KeyModifiers::ALT;
            remainder = rest;
        } else if let Some(rest) = remainder.strip_prefix("Shift+") {
            mods |= KeyModifiers::SHIFT;
            remainder = rest;
        } else {
            break;
        }
    }

    let code = match remainder {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        s if s.chars().count() == 1 => {
            let ch = s.chars().next().expect("counted one char");
            KeyCode::Char(ch)
        }
        other => anyhow::bail!("unknown key name '{other}'"),
    };

    Ok((code, mods))
}

/// Renders a `serde_json::Value` to a string for use in `Matcher` comparisons.
///
/// JSON strings are unwrapped to their inner text. Other scalar types (number,
/// bool, null) are rendered via their JSON display form. Arrays and objects are
/// rendered via their JSON display form as well.
fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Builds `{stem}.{name}.snap` under `golden_dir`, rejecting names that would escape.
///
/// Parser already requires a single safe segment; this is defense in depth so a
/// hand-built `Step::Snapshot` cannot write outside `golden_dir`.
fn resolve_snapshot_path(golden_dir: &Path, stem: &str, name: &str) -> Result<PathBuf, String> {
    if !is_safe_snapshot_name(name) {
        return Err(format!("invalid snapshot name {name:?}"));
    }
    let file_name = format!("{stem}.{name}.snap");
    let path = golden_dir.join(&file_name);
    let Ok(rel) = path.strip_prefix(golden_dir) else {
        return Err("snapshot path escapes golden_dir".to_owned());
    };
    let mut comps = rel.components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => Ok(path),
        _ => Err("snapshot path escapes golden_dir".to_owned()),
    }
}

/// Computes a short diff-style detail string for a snapshot mismatch.
///
/// Reports the first line that differs, with its index, expected text, and actual
/// text. Falls back to showing the full expected and actual strings when short.
fn snapshot_diff_detail(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    for (i, (e, a)) in exp_lines.iter().zip(act_lines.iter()).enumerate() {
        if e != a {
            return format!("first diff at line {i}:\n  expected: {e:?}\n  actual:   {a:?}");
        }
    }
    if exp_lines.len() != act_lines.len() {
        return format!(
            "line count differs: expected {} lines, got {}",
            exp_lines.len(),
            act_lines.len()
        );
    }
    format!("expected:\n{expected}\nactual:\n{actual}")
}

#[expect(
    clippy::too_many_arguments,
    reason = "runner step context: 8 params; a RunContext struct would be the alternative"
)]
async fn execute_step(
    harness: &mut Harness<'_>,
    step: &Step,
    i: usize,
    golden_dir: &Path,
    stem: &str,
    mode: RunMode,
    mock_enabled: bool,
    report: &mut RunReport,
) {
    match step {
        Step::Key(name) => match parse_key(name) {
            Ok((code, mods)) => harness.chord(code, mods).await,
            Err(e) => {
                let screen = harness.screen();
                report.failures.push(Failure {
                    step_index: i,
                    verb: "key".to_owned(),
                    detail: format!("key parse error: {e}"),
                    screen,
                });
            }
        },

        Step::Type(text) => harness.type_text(text).await,

        Step::Paste(text) => harness.paste(text).await,

        Step::Resize(w, h) => harness.resize(*w, *h).await,

        Step::Pump => harness.pump().await,

        Step::Advance(d) => harness.advance(*d).await,

        Step::EnqueueCompletion(toks) => {
            if !mock_enabled {
                let screen = harness.screen();
                report.failures.push(Failure {
                    step_index: i,
                    verb: "enqueue completion".to_owned(),
                    detail: "enqueue requires api mock".to_owned(),
                    screen,
                });
                return;
            }
            harness.enqueue_completion(&toks.iter().map(String::as_str).collect::<Vec<_>>());
        }

        Step::EnqueueError(msg) => {
            if !mock_enabled {
                let screen = harness.screen();
                report.failures.push(Failure {
                    step_index: i,
                    verb: "enqueue error".to_owned(),
                    detail: "enqueue requires api mock".to_owned(),
                    screen,
                });
                return;
            }
            harness.enqueue_error(msg);
        }

        Step::ExpectScreenContains(text) => {
            let screen = harness.screen();
            if !screen.contains(text.as_str()) {
                report.failures.push(Failure {
                    step_index: i,
                    verb: "expect screen contains".to_owned(),
                    detail: format!("expected substring not found: {text:?}"),
                    screen,
                });
            }
        }

        Step::ExpectScreenExcludes(text) => {
            let screen = harness.screen();
            if screen.contains(text.as_str()) {
                report.failures.push(Failure {
                    step_index: i,
                    verb: "expect screen excludes".to_owned(),
                    detail: format!("excluded substring was found: {text:?}"),
                    screen,
                });
            }
        }

        Step::ExpectLine { n, matcher } => {
            let screen = harness.screen();
            let lines: Vec<&str> = screen.lines().collect();
            match lines.get(*n) {
                None => {
                    report.failures.push(Failure {
                        step_index: i,
                        verb: "expect line".to_owned(),
                        detail: format!(
                            "line {n} does not exist (screen has {} lines)",
                            lines.len()
                        ),
                        screen,
                    });
                }
                Some(line) => {
                    // TestBackend pads lines to the full terminal width with spaces;
                    // trim trailing whitespace so matchers work against logical content.
                    let trimmed = line.trim_end();
                    let pass = match matcher {
                        Matcher::Eq(expected) => trimmed == expected.as_str(),
                        Matcher::Contains(sub) => trimmed.contains(sub.as_str()),
                        Matcher::Null => trimmed.is_empty(),
                    };
                    if !pass {
                        let detail = match matcher {
                            Matcher::Eq(expected) => {
                                format!("line {n}: expected {expected:?}, got {trimmed:?}")
                            }
                            Matcher::Contains(sub) => {
                                format!("line {n}: expected to contain {sub:?}, got {trimmed:?}")
                            }
                            Matcher::Null => {
                                format!("line {n}: expected empty, got {trimmed:?}")
                            }
                        };
                        report.failures.push(Failure {
                            step_index: i,
                            verb: "expect line".to_owned(),
                            detail,
                            screen,
                        });
                    }
                }
            }
        }

        Step::ExpectState { probe, matcher } => {
            let obs = harness.observe();
            let screen = harness.screen();
            let value = match serde_json::to_value(&obs) {
                Ok(v) => v,
                Err(e) => {
                    report.failures.push(Failure {
                        step_index: i,
                        verb: "expect state".to_owned(),
                        detail: format!("failed to serialize observation: {e}"),
                        screen,
                    });
                    return;
                }
            };
            let field = match value.get(probe.as_str()) {
                Some(v) => v,
                None => {
                    report.failures.push(Failure {
                        step_index: i,
                        verb: "expect state".to_owned(),
                        detail: format!("unknown probe '{probe}'"),
                        screen,
                    });
                    return;
                }
            };
            let pass = match matcher {
                Matcher::Null => field.is_null(),
                Matcher::Eq(expected) => json_value_to_string(field) == expected.as_str(),
                Matcher::Contains(sub) => json_value_to_string(field).contains(sub.as_str()),
            };
            if !pass {
                let actual = json_value_to_string(field);
                let detail = match matcher {
                    Matcher::Null => format!("probe '{probe}': expected null, got {actual:?}"),
                    Matcher::Eq(expected) => {
                        format!("probe '{probe}': expected {expected:?}, got {actual:?}")
                    }
                    Matcher::Contains(sub) => {
                        format!("probe '{probe}': expected to contain {sub:?}, got {actual:?}")
                    }
                };
                report.failures.push(Failure {
                    step_index: i,
                    verb: "expect state".to_owned(),
                    detail,
                    screen,
                });
            }
        }

        Step::Snapshot(name) => {
            let actual = harness.screen();
            let golden_path = match resolve_snapshot_path(golden_dir, stem, name) {
                Ok(path) => path,
                Err(detail) => {
                    report.failures.push(Failure {
                        step_index: i,
                        verb: "snapshot".to_owned(),
                        detail,
                        screen: actual,
                    });
                    return;
                }
            };
            match mode {
                RunMode::Bless => {
                    // Parent is golden_dir itself after resolve_snapshot_path confinement.
                    if let Some(parent) = golden_path.parent()
                        && let Err(e) = std::fs::create_dir_all(parent)
                    {
                        report.failures.push(Failure {
                            step_index: i,
                            verb: "snapshot".to_owned(),
                            detail: format!("failed to create golden dir: {e}"),
                            screen: actual,
                        });
                        return;
                    }
                    if let Err(e) = std::fs::write(&golden_path, &actual) {
                        report.failures.push(Failure {
                            step_index: i,
                            verb: "snapshot".to_owned(),
                            detail: format!("failed to write golden file: {e}"),
                            screen: actual,
                        });
                    }
                }
                RunMode::Check => match std::fs::read_to_string(&golden_path) {
                    Err(_) => {
                        report.failures.push(Failure {
                            step_index: i,
                            verb: "snapshot".to_owned(),
                            detail: format!(
                                "missing golden file {}, run with --bless to create it",
                                golden_path.display()
                            ),
                            screen: actual,
                        });
                    }
                    Ok(expected) => {
                        if actual != expected {
                            let detail = snapshot_diff_detail(&expected, &actual);
                            report.failures.push(Failure {
                                step_index: i,
                                verb: "snapshot".to_owned(),
                                detail,
                                screen: actual,
                            });
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::scenario::{ApiSetup, DbSetup, Scenario, Setup, Step};

    fn default_setup_no_db_no_api() -> Setup {
        Setup {
            size: (100, 30),
            db: DbSetup::None,
            api: ApiSetup::None,
            overrides: Vec::new(),
            seed: None,
        }
    }

    #[test]
    fn resolve_snapshot_path_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_snapshot_path(dir.path(), "stem", "../../escape").is_err());
        assert!(resolve_snapshot_path(dir.path(), "stem", "a/b").is_err());
    }

    #[test]
    fn resolve_snapshot_path_accepts_normal_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = resolve_snapshot_path(dir.path(), "stem", "boot").unwrap();
        assert_eq!(path, dir.path().join("stem.boot.snap"));
        assert!(path.starts_with(dir.path()));
    }

    #[test]
    fn parse_key_enter() {
        let (code, mods) = parse_key("Enter").unwrap();
        assert_eq!(code, KeyCode::Enter);
        assert_eq!(mods, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_ctrl_s() {
        let (code, mods) = parse_key("Ctrl+S").unwrap();
        assert_eq!(code, KeyCode::Char('S'));
        assert_eq!(mods, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_key_bare_char() {
        let (code, mods) = parse_key("a").unwrap();
        assert_eq!(code, KeyCode::Char('a'));
        assert_eq!(mods, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_backtab() {
        let (code, mods) = parse_key("BackTab").unwrap();
        assert_eq!(code, KeyCode::BackTab);
        assert_eq!(mods, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_unknown_returns_err() {
        assert!(parse_key("Frobnicate").is_err());
    }

    #[test]
    fn parse_key_ctrl_shift_x() {
        let (code, mods) = parse_key("Ctrl+Shift+X").unwrap();
        assert_eq!(code, KeyCode::Char('X'));
        assert_eq!(mods, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[tokio::test]
    async fn passing_scenario_reports_ok() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![
                Step::ExpectScreenContains("Input".to_owned()),
                Step::ExpectState {
                    probe: "active_dialog".to_owned(),
                    matcher: Matcher::Null,
                },
            ],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(
            report.ok(),
            "expected no failures, got: {:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn failing_scenario_collects_failure() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::ExpectScreenContains(
                "this text is definitely not on screen xyzzy".to_owned(),
            )],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(!report.ok());
        assert_eq!(report.failures.len(), 1);
        assert!(!report.failures[0].screen.is_empty());
    }

    #[tokio::test]
    async fn streaming_scenario_with_mock_api() {
        let scenario = Scenario {
            setup: Setup {
                size: (100, 30),
                db: DbSetup::Temp,
                api: ApiSetup::Mock,
                overrides: Vec::new(),
                seed: None,
            },
            steps: vec![
                Step::EnqueueCompletion(vec!["Hi".to_owned()]),
                Step::Type("x".to_owned()),
                Step::Key("Enter".to_owned()),
                Step::Pump,
                Step::ExpectState {
                    probe: "is_streaming".to_owned(),
                    matcher: Matcher::Eq("false".to_owned()),
                },
                Step::ExpectState {
                    probe: "head_text".to_owned(),
                    matcher: Matcher::Contains("Hi".to_owned()),
                },
            ],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(
            report.ok(),
            "expected no failures, got: {:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn bless_then_check_snapshot_roundtrip() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::Snapshot("boot".to_owned())],
        };
        let golden_dir = tempfile::tempdir().unwrap();

        let bless_report = run_scenario(&scenario, golden_dir.path(), "stem", RunMode::Bless)
            .await
            .unwrap();
        assert!(
            bless_report.ok(),
            "bless failed: {:?}",
            bless_report.failures
        );

        let snap_path = golden_dir.path().join("stem.boot.snap");
        assert!(snap_path.exists(), "snap file was not written");

        let check_report = run_scenario(&scenario, golden_dir.path(), "stem", RunMode::Check)
            .await
            .unwrap();
        assert!(
            check_report.ok(),
            "check failed: {:?}",
            check_report.failures
        );
    }

    #[tokio::test]
    async fn enqueue_error_ends_streaming_cleanly() {
        let scenario = Scenario {
            setup: Setup {
                size: (100, 30),
                db: DbSetup::Temp,
                api: ApiSetup::Mock,
                overrides: Vec::new(),
                seed: None,
            },
            steps: vec![
                Step::EnqueueError("boom".to_owned()),
                Step::Type("x".to_owned()),
                Step::Key("Enter".to_owned()),
                Step::Pump,
                Step::ExpectState {
                    probe: "is_streaming".to_owned(),
                    matcher: Matcher::Eq("false".to_owned()),
                },
            ],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(
            report.ok(),
            "expected no failures, got: {:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn expect_screen_excludes_absent_text_passes() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::ExpectScreenExcludes(
                "zzz_definitely_absent".to_owned(),
            )],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(
            report.ok(),
            "expected no failures, got: {:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn expect_screen_excludes_present_text_fails() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::ExpectScreenExcludes("Input".to_owned())],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(!report.ok());
        assert_eq!(report.failures.len(), 1);
    }

    #[tokio::test]
    async fn snapshot_missing_golden_in_check_mode_reports_one_failure() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::Snapshot("nope".to_owned())],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "stem", RunMode::Check)
            .await
            .unwrap();
        assert!(!report.ok());
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].detail.contains("--bless"),
            "expected --bless hint in detail, got: {:?}",
            report.failures[0].detail
        );
    }

    #[tokio::test]
    async fn snapshot_content_mismatch_reports_one_failure() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::Snapshot("snap".to_owned())],
        };
        let golden_dir = tempfile::tempdir().unwrap();

        let bless_report = run_scenario(&scenario, golden_dir.path(), "stem", RunMode::Bless)
            .await
            .unwrap();
        assert!(
            bless_report.ok(),
            "bless failed: {:?}",
            bless_report.failures
        );

        let golden_path = golden_dir.path().join("stem.snap.snap");
        std::fs::write(&golden_path, "totally different content\n").unwrap();

        let check_report = run_scenario(&scenario, golden_dir.path(), "stem", RunMode::Check)
            .await
            .unwrap();
        assert!(!check_report.ok());
        assert_eq!(check_report.failures.len(), 1);
    }

    #[tokio::test]
    async fn advance_status_survives_short_advance_and_clears_after_long() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![
                Step::Type("/retry".to_owned()),
                Step::Key("Enter".to_owned()),
                Step::ExpectState {
                    probe: "status_message".to_owned(),
                    matcher: Matcher::Contains("No user message".to_owned()),
                },
                Step::Advance(std::time::Duration::from_secs(2)),
                Step::ExpectState {
                    probe: "status_message".to_owned(),
                    matcher: Matcher::Contains("No user message".to_owned()),
                },
                Step::Advance(std::time::Duration::from_secs(6)),
                Step::ExpectState {
                    probe: "status_message".to_owned(),
                    matcher: Matcher::Null,
                },
            ],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(
            report.ok(),
            "expected no failures, got: {:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn unknown_probe_reports_one_failure_with_hint() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::ExpectState {
                probe: "xyzzy_not_a_field".to_owned(),
                matcher: Matcher::Eq("x".to_owned()),
            }],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(!report.ok());
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].detail.contains("unknown probe"),
            "expected 'unknown probe' in detail, got: {:?}",
            report.failures[0].detail
        );
    }

    #[tokio::test]
    async fn expect_line_out_of_bounds_reports_one_failure() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::ExpectLine {
                n: 9999,
                matcher: Matcher::Contains(String::new()),
            }],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(!report.ok());
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].detail.contains("does not exist"),
            "expected 'does not exist' in detail, got: {:?}",
            report.failures[0].detail
        );
    }

    #[tokio::test]
    async fn expect_line_in_bounds_with_empty_substring_passes() {
        let scenario = Scenario {
            setup: default_setup_no_db_no_api(),
            steps: vec![Step::ExpectLine {
                n: 0,
                matcher: Matcher::Contains(String::new()),
            }],
        };
        let golden_dir = tempfile::tempdir().unwrap();
        let report = run_scenario(&scenario, golden_dir.path(), "test", RunMode::Check)
            .await
            .unwrap();
        assert!(
            report.ok(),
            "expected no failures, got: {:?}",
            report.failures
        );
    }
}
