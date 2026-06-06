//! In-process verification harness for the TUI. Compiled only under the
//! `test-support` feature. Boots `App` against a `ratatui` `TestBackend`,
//! drives it with synthetic events, and exposes screen + state for assertions.

mod observe;

pub use observe::Observation;
