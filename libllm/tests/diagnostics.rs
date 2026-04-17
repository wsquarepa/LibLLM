use std::path::Path;

use libllm::diagnostics::{self, BuildInfo, InitParams};

#[test]
fn banner_and_event_lines_are_rendered() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let log_path = tmp.path().join("banner.log");

    {
        let _guard = diagnostics::init(InitParams {
            debug_override: Some(&log_path),
            timings_path: None,
            run_mode: "test",
            cli_args: "test-runner".to_owned(),
            build: BuildInfo {
                version: "9.9.9",
                channel: "feat/example",
                commit: "abcdef0",
                dirty: false,
            },
            filter_flag: Some("info"),
            filter_env: None,
        })
        .expect("init");

        tracing::info!(answer = 42, "hello from test");
        tracing::warn!(reason = "synthetic", "warn from test");
    }

    assert_log_matches_shape(&log_path);
}

fn assert_log_matches_shape(path: &Path) {
    let contents = std::fs::read_to_string(path).expect("read log");
    let first = contents.lines().next().expect("empty log");
    assert!(first.starts_with("================"), "first line was {first:?}");
    assert!(contents.contains("LibLLM version 9.9.9 (-abcdef0)"));
    assert!(contents.contains("Run mode      test"));
    assert!(contents.contains("Filter        info  (source: --log-filter)"));

    let mut saw_info = false;
    let mut saw_warn = false;
    for line in contents.lines() {
        if !line.starts_with("[+") {
            continue;
        }
        assert_eq!(line[1..14].matches(':').count(), 2, "bad offset in {line:?}");
        assert_eq!(&line[14..15], "]");
        assert_eq!(&line[15..16], " ");
        let level = &line[16..21];
        assert!(
            ["TRACE", "DEBUG", "INFO ", "WARN ", "ERROR"].contains(&level),
            "bad level {level:?} in {line:?}"
        );
        if level == "INFO " {
            saw_info = true;
        }
        if level == "WARN " {
            saw_warn = true;
        }
    }
    assert!(saw_info, "no INFO event line in {contents}");
    assert!(saw_warn, "no WARN event line in {contents}");
}
