//! Replays a single `.scenario` file and prints assertion results. Run via
//! `cargo xtask scenario <file> [--bless]`.

use std::path::Path;

use libllm_tui::harness::scenario::{RunMode, parse, run_scenario};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let file = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: scenario_runner <file> [--bless]"))?;
    let bless = args.any(|a| a == "--bless");
    let mode = if bless {
        RunMode::Bless
    } else {
        RunMode::Check
    };

    let src = std::fs::read_to_string(&file)?;
    let scenario = parse(&src)?;
    let path = Path::new(&file);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scenario");

    let report = run_scenario(&scenario, dir, stem, mode).await?;
    for f in &report.failures {
        eprintln!(
            "FAIL step {} `{}`: {}\n--- screen ---\n{}",
            f.step_index, f.verb, f.detail, f.screen
        );
    }
    if report.ok() {
        println!("OK: {} ({} steps)", file, scenario.steps.len());
        Ok(())
    } else {
        std::process::exit(1);
    }
}
