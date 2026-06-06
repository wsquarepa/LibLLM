#![cfg(feature = "test-support")]

use std::path::Path;

use libllm_tui::harness::scenario::{RunMode, parse, run_scenario};

#[tokio::test]
async fn all_committed_scenarios_pass() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenarios");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("read scenarios dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("scenario"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .scenario files found under {dir:?}");

    let mut all_failures = Vec::new();
    for path in files {
        let src = std::fs::read_to_string(&path).expect("read scenario file");
        let scenario = parse(&src).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("scenario stem");
        let report = run_scenario(&scenario, &dir, stem, RunMode::Check)
            .await
            .expect("run scenario");
        for f in report.failures {
            all_failures.push(format!(
                "[{stem}] step {} `{}`: {}\n{}",
                f.step_index, f.verb, f.detail, f.screen
            ));
        }
    }
    assert!(
        all_failures.is_empty(),
        "scenario failures:\n\n{}",
        all_failures.join("\n\n")
    );
}
