# Rust Codebase Standards

This document defines how this repository is organized and maintained. It
describes the actual conventions of this codebase and its target state. Where
`.agents/RULES.md` documents operational detail (build commands, test layout,
migrations, dialog keybindings), RULES.md is authoritative and this document
defers to it.

## Source Basis

Informed by public Rust guidance:

- [Cargo package layout](https://doc.rust-lang.org/cargo/guide/project-layout.html)
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Cargo targets and tests](https://doc.rust-lang.org/cargo/reference/cargo-targets.html)
- [The Rust Book: packages and crates](https://doc.rust-lang.org/book/ch07-01-packages-and-crates.html)
- [The Rust Reference: crates and source files](https://doc.rust-lang.org/reference/crates-and-source-files.html)
- [The Rust Style Guide](https://doc.rust-lang.org/style-guide/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Clippy documentation](https://doc.rust-lang.org/clippy/)
- [The Rust Reference: unsafety](https://doc.rust-lang.org/reference/unsafety.html)

Project exemplars with comparable structure:

- [rust-analyzer](https://github.com/rust-lang/rust-analyzer): large workspace, explicit API-boundary crates, `xtask`, architecture invariants.
- [Cargo](https://github.com/rust-lang/cargo): workspace dependencies, workspace lints, domain crates, thin binary target.
- [ripgrep](https://github.com/BurntSushi/ripgrep): command-line binary backed by focused reusable crates.

## Organizing Principles

The codebase optimizes for discoverability, local reasoning, and stable
boundaries. A contributor should be able to answer these questions quickly:

- What crate owns this behavior?
- What module owns this type or function?
- Is this API public, crate-internal, or private implementation detail?
- Which tests prove the behavior?
- Which command verifies the change? (Answer: `cargo xtask ci`.)

Code is organized around stable concepts, not incidental technical tasks.
The crates are a domain crate, a storage crate, a protocol crate, a config
crate, a diagnostics crate, a UI crate, a CLI crate, and a backup crate. A
broad `utils`, `common`, `helpers`, or `misc` crate or module is not
acceptable.

Default to private implementation details. Expose only the smallest API needed
by the next layer. Use `pub(crate)` (or `pub(super)`, as in `libllm-tui`) for
cross-module internals and `pub` only when callers outside the crate should
rely on the item.

## Workspace Layout

This is a Cargo workspace. The root `Cargo.toml` holds shared metadata,
dependency versions, lint policy, and membership.

Actual layout:

```text
.
├── Cargo.toml
├── Cargo.lock
├── README.md
├── rustfmt.toml
├── install.sh
├── AGENTS.md -> .agents/RULES.md
├── CLAUDE.md -> .agents/RULES.md
├── .agents/
│   ├── RULES.md        # agent operational guide (authoritative for build/test)
│   └── STANDARDS.md    # this document
├── assets/
├── docs/               # user-facing docs: cli, configuration, install, usage
├── crates/
│   ├── core/           # libllm-core
│   ├── config/         # libllm-config
│   ├── storage/        # libllm-storage
│   ├── protocol/       # libllm-protocol
│   ├── diagnostics/    # libllm-diagnostics
│   ├── tui/            # libllm-tui
│   ├── cli/            # libllm-cli (produces the `libllm` binary)
│   └── backup/         # libllm-backup
└── xtask/              # repository automation: ci, release, scenario
```

Workspace members live under `crates/<name>/`. `xtask/` owns repository
automation that would otherwise become shell scripts: the CI suite, release
version bumps, and scenario-test running.

The root `Cargo.toml` is authoritative for:

- `[workspace]` membership and resolver.
- `[workspace.package]` shared version and edition.
- `[workspace.dependencies]` shared dependency versions.
- `[workspace.lints]` shared lint policy (`unsafe_code = "forbid"`,
  `unsafe_op_in_unsafe_fn = "deny"`, `clippy::allow_attributes = "deny"`).

Member crates inherit workspace metadata and dependency versions. They declare
only crate-specific features, targets, and dependencies.

There is no `test-support` crate. Test support is a cargo *feature*
(`test-support`) on `libllm-core`, `libllm-config`, and `libllm-tui`; the TUI
test harness lives in `crates/tui/src/harness/` behind that feature. There is
no proc-macro crate.

## Crate Boundaries

Crate boundaries reflect architecture boundaries.

Crate roles:

- `libllm-core` (`crates/core`): pure domain — session tree, character,
  persona, world-info, presets, file-ingestion pipeline, crypto, config
  *types*. No database access, no network I/O, no async runtime, and no
  process-global state. Loading user data (character cards, presets, salts)
  from explicit `&Path` arguments is permitted; this crate is the data-model
  layer, not a no-filesystem layer. Logging/diagnostics *infrastructure*
  (subscriber setup, global state, sysinfo collection) does not belong here.
- `libllm-storage` (`crates/storage`): SQLCipher database access, migrations,
  repositories, FTS search.
- `libllm-protocol` (`crates/protocol`): llama.cpp HTTP API client, tokenizer,
  summarization orchestration.
- `libllm-config` (`crates/config`): process-boundary config — data-directory
  resolution, `config.toml` load/save. Config types live in core; the
  functions that touch the process environment live here.
- `libllm-diagnostics` (`crates/diagnostics`): diagnostics infrastructure —
  tracing-subscriber setup, the sanctioned diagnostics global state, log-file
  management, the `--timings` report, and the startup banner's sysinfo
  collection. The `timed_result!` macro stays in `libllm-core` because core
  itself uses it.
- `libllm-tui` (`crates/tui`): rendering, dialogs, input handling, view state,
  `FileSummarizer` orchestration.
- `libllm-cli` (`crates/cli`): argument parsing, command dispatch, subcommands,
  startup orchestration, exit-code mapping. Produces the `libllm` binary via an
  explicit `[[bin]]` target (the binary name differs from the package name).
- `libllm-backup` (`crates/backup`): backup and recovery library — snapshots,
  diff chains, retention, rekey, its own index-format migrations (distinct
  from storage's SQL migrations).

Dependencies flow inward, with no umbrella or facade crate:

```text
cli -> tui -> storage / protocol / config / diagnostics -> core
```

Core never depends on an outer crate. Outer crates depend on the concrete
library crates directly.

All workspace library crates use the `libllm-` package-name prefix.

Create a new crate when a component has a distinct dependency set, needs an
independent public API boundary, compiles or tests better in isolation, or maps
to a stable domain concept. Do not create a crate only to move files around.

## Package and Target Placement

Follow Cargo target conventions:

- `src/lib.rs`: library crate root.
- `src/main.rs`: binary crate root. In this repo the only binary is
  `crates/cli/src/main.rs`, which is a thin shim over `libllm_cli::app::run()`.
- `tests/*.rs`: integration test crates, per member crate.
- `examples/*.rs`: compileable examples (the TUI's `scenario_runner` is one,
  gated behind the `test-support` feature).

Real behavior lives in library code. The binary parses process inputs,
initializes infrastructure, calls library code, and translates success or
failure into process output and exit status.

## Module Layout

Every source file is a module, and the crate root defines the module tree. Keep
module structure deliberate and shallow.

Module rules:

- `lib.rs` contains crate-level docs, module declarations, and intentional
  re-exports. It should not contain substantial implementation logic, and it
  should curate: re-export the primary types callers need rather than only
  exposing bare `pub mod` lists.
- One module has one main reason to change. A module that accretes several
  unrelated responsibilities (data structure + container + utilities) must be
  split along those seams.
- A module with one principal type lives in `name.rs`. A module with several
  collaborators lives as a `name/` directory with a `name/mod.rs` root — this
  repo uses the `mod.rs` convention throughout (`db/mod.rs`, `dialogs/mod.rs`,
  `files/mod.rs`, `render/mod.rs`); follow it for new directories.
- Keep child modules private by default and re-export a small facade from the
  parent module when callers need a simpler path.

Avoid deep public paths such as `crate::a::b::c::d::Type`. Deep paths usually
mean the public facade is missing or the type is in the wrong layer.

Central mutable state must be grouped, not flat. A state struct accumulating
dozens of ungrouped fields across unrelated concerns is the module-level
"grab-bag" anti-pattern expressed as a type: group fields into cohesive
substructs per concern.

## Naming

Use Rust naming conventions consistently:

- Package names: `kebab-case` with the `libllm-` prefix for workspace
  libraries.
- Rust crate identifiers, modules, functions, methods, and variables:
  `snake_case`.
- Types, traits, and enum variants: `UpperCamelCase`.
- Constants and statics: `SCREAMING_SNAKE_CASE`.
- Macros: `snake_case!`.
- Lifetimes: short lowercase names such as `'a`, or a descriptive name when
  the lifetime has domain meaning.

Use standard method names: `new`, `with_<detail>`, `from_<source>`, and
`as_*`/`to_*`/`into_*` per Rust conversion conventions.

Error types are named `<Domain>Error` after the subsystem that produces them —
`DbError`, `ApiError`, `BackupError`, `FileError`, `CryptoError`,
`CharacterError` — not verb-object phrases. Keep error names stable and
specific.

Avoid all-caps acronyms in type names. Prefer `HttpClient` and `SqlStore` over
`HTTPClient` and `SQLStore`.

## Public API Design

Public APIs should be small, typed, documented, and hard to misuse.

Rules:

- Make invalid states unrepresentable with enums, newtypes, and validated
  constructors.
- Prefer explicit domain types over strings, booleans, tuples, or loose maps.
- Avoid boolean flag parameters that switch behavior. Split the function or use
  an enum with named variants.
- Prefer returning a value over mutating an input. If mutation is necessary,
  make ownership and side effects obvious in the type signature.
- Prefer concrete types in internal APIs. Use generics, trait objects, or
  `impl Trait` only when they buy real API flexibility or caller ergonomics.
- Do not expose dependency types in stable public APIs unless the dependency is
  intentionally part of the contract.
- Keep trait definitions at real abstraction boundaries. Many implementations
  behind one dispatch point (e.g. dialogs behind a handler trait) is a real
  boundary; a trait with one implementation and no external caller is not.
- If a trait may be used as a trait object, design it for object safety from
  the start.

Use crate roots and parent modules as facades. Re-export the types callers
need; keep implementation modules private.

## Error Handling

Use explicit, typed, contextual errors.

Rules:

- Return `Result<T, E>` for recoverable failures.
- Use `Option<T>` only for expected absence, not for failure with lost context.
- Library crates expose specific `thiserror` enums with `#[source]` chaining
  and a `pub type Result<T>` alias. The binary crate (`libllm-cli`) may use
  `anyhow::Result` at the process boundary.
- Include enough context to debug failures: operation, path or identifier,
  external request parameters where safe, status codes, and response bodies
  when relevant.
- Do not silently ignore errors. Either handle them deliberately or return
  them.
- Do not catch broad errors and replace them with generic messages.
- Do not use `unwrap` or `expect` in production paths unless the invariant is
  structurally guaranteed and the message documents the invariant precisely.
- Tests may use `unwrap` or `expect` when failure would make the test itself
  invalid.
- Panics are for bugs, violated invariants, and unrecoverable programmer
  errors, not normal user or environment failures.
- Destructors must not be the only place where fallible cleanup happens.
  Provide an explicit `close`, `finish`, `flush`, or `commit` method that
  returns a `Result`.

For external services, centralize retry policy. Retry only operations that are
safe to retry, log each retry with structured fields, and return the last error
with full context when retries are exhausted.

## Configuration and State

Configuration is parsed once, validated once, and passed explicitly.

Rules:

- Represent configuration as typed structs (`Config` and friends in
  `libllm-core::config`).
- Keep raw environment variables, CLI strings, and config-file syntax at the
  process boundary (`libllm-config` and `libllm-cli`).
- Convert raw inputs into validated domain types before they reach core logic.
- Do not read environment variables deep inside library code.
- Avoid global mutable state. The two sanctioned exceptions are: the
  `OnceLock`-based data-directory override in `libllm-config`, which confines
  process-global path state to the process-boundary crate (see RULES.md for
  its per-process test constraints); and the `OnceLock`-based subscriber and
  init state in `libllm-diagnostics`, which confines tracing initialization to
  the diagnostics crate. Core must hold no process-global state.
- Keep pure domain functions free of I/O, time, randomness, logging, and
  process state. Data-loading functions in core take explicit `&Path` inputs.

## Async and Concurrency

Concurrency policy belongs at orchestration boundaries.

Rules:

- The binary crate owns runtime initialization (tokio).
- `libllm-core` has no async runtime dependency at all.
- Spawn tasks only in orchestration code (`libllm-tui`, `libllm-cli`) or in a
  component explicitly responsible for background work (e.g. the
  `FileSummarizer`).
- Pass cancellation, timeout, and shutdown signals explicitly.
- Do not block inside async code. Use the runtime's blocking facilities for
  unavoidable blocking work.
- Protect shared mutable state behind the narrowest synchronization primitive
  that matches the access pattern.
- Prefer message passing or ownership transfer when it makes state changes
  easier to reason about.

## Observability

Use structured diagnostics via `tracing`. The authoritative level rubric and
filter configuration live in RULES.md ("Diagnostics authoring"); in brief:
`trace` for per-frame/per-keystroke events, `debug` for state transitions,
`info` for DB operations and API summaries, `warn` for retries and degraded
fallbacks, `error` for unrecoverable failures.

Rules:

- Libraries use `tracing`, not `println!`.
- The CLI writes intentional user output to stdout and diagnostics to stderr.
- Log dynamic values as fields, not by formatting them into the message string.
- For timed blocks, use spans (or `timed_result!`) so elapsed time feeds the
  `--timings` report; do not write elapsed fields manually.

Do not log secrets, passphrases, tokens, private message bodies, or full
request payloads unless the payload is explicitly non-sensitive and the
diagnostic mode requires it.

## Unsafe Code

Unsafe code is forbidden workspace-wide (`[workspace.lints.rust] unsafe_code =
"forbid"`). Do not introduce it. If a future need is genuinely unavoidable, it
requires loosening the workspace lint deliberately and isolating the unsafe
code behind a safe, documented wrapper — a decision for a human maintainer,
not something to do in passing.

## Formatting and Lints

Formatting is automated, not debated.

Rules:

- `rustfmt` with the minimal repo config (`rustfmt.toml`: edition 2024,
  `max_width = 100`).
- Keep imports at the top of the file and let `rustfmt` organize them.
- Workspace lint configuration in the root `Cargo.toml`; every crate inherits
  it via `[lints] workspace = true`.
- Clippy warnings are defects: the CI gate is `-D warnings` across the
  workspace and all targets.
- Never suppress warnings with `#[allow(...)]` — `clippy::allow_attributes`
  is denied workspace-wide. Fix the code instead. The only sanctioned
  mechanism for a documented structural false-positive is
  `#[expect(lint, reason = "...")]`, which self-verifies. See RULES.md
  ("No warning suppression").

Verification is one command:

```sh
cargo xtask ci
```

It runs, stopping at the first failure: `cargo fmt --all --check`, `cargo
clippy --workspace --all-targets -- -D warnings` (plus the `libllm-tui`
`test-support` feature pass), `cargo test --workspace` (plus the `test-support`
suite), and `cargo doc --workspace --no-deps`. Output is teed to a timestamped
log; read the log rather than re-running (see RULES.md "Build and Test").

## Dependencies

Dependencies are centralized, intentional, and visible.

Rules:

- Define shared versions in `[workspace.dependencies]`.
- Use `dependency.workspace = true` in member crates.
- Disable default features when a dependency's defaults pull in unnecessary
  runtime, TLS, OS, or serialization support (as done for `reqwest`,
  `minijinja`, `rustls`, `tracing-subscriber`).
- A crate's dependency list is part of its architecture contract: `libllm-core`
  must not grow dependencies that imply infrastructure concerns (runtimes,
  HTTP, database drivers, system introspection).
- Keep public dependencies stable if their types appear in public APIs.
- Prefer established crates with active maintenance, clear licensing, and small
  transitive dependency cost.
- Keep dev-only tools and fixtures in `dev-dependencies`; optional test
  machinery behind the `test-support` feature.
- Avoid adding dependencies for trivial code.
- Audit new dependencies for license, maintenance, security posture, feature
  flags, and public API leakage before adoption.

## Tests

Testing matches the boundary being protected. The authoritative test-suite
layout (file list, subprocess pattern, `OnceLock` constraint) is in RULES.md;
keep that list in sync when adding test binaries.

Use:

- Unit tests beside private logic (`#[cfg(test)]` modules) for edge cases and
  invariants.
- Integration tests under each crate's `tests/` for public API and process
  contracts.
- Subprocess tests (spawning the compiled `libllm` binary via
  `common::client_bin()`) for CLI behavior: exit codes, stdout/stderr
  separation, environment handling, multi-process safety.
- Scenario tests through the TUI harness (`crates/tui/src/harness/`,
  `tests/scenarios.rs`) for end-to-end TUI behavior with a mock API.
- Data-driven or golden tests for parsers, formatters, migrations, importers,
  and exporters (e.g. the cross-version migration tests in
  `crates/storage/src/db/migrations/mod.rs`).

Rules:

- Test names describe the behavior being proved.
- Tests are deterministic and isolated; use temporary directories for
  filesystem tests and `wiremock` for HTTP.
- No network access in normal test runs (the `update` subcommand is
  deliberately untested at the subprocess level for this reason).
- Shared integration-test helpers live in `tests/common/mod.rs`, with
  `#[expect(dead_code, reason = "...")]` on the per-binary `mod common;`
  declaration.
- Prefer testing public behavior over internal implementation details.

## Documentation

Documentation lives as close as possible to the API it explains.

Rules:

- `lib.rs` explains the crate's purpose and its architectural contract.
- Public types, traits, functions, modules, and macros have rustdoc.
- Public fallible functions document meaningful error conditions.
- Public panicking behavior is documented when callers can trigger it.
- Examples explain why an API is useful, not merely restate its syntax.
- Separate architecture documents (RULES.md, this file) describe durable
  boundaries, invariants, and workflows that cannot be expressed clearly in
  code — and must be corrected in the same change that invalidates them.
  A stated contract the code does not honor is worse than no contract.
- Do not duplicate detail across README, rustdoc, RULES.md, and this file;
  cross-reference instead.

## Review Checklist

Use this checklist when reviewing a Rust change:

- The file is in the crate and module that own the concept.
- `main.rs` only wires process concerns.
- Domain logic in core is independent of databases, the network, async
  runtimes, and process state.
- New public API is intentionally exposed and documented.
- Private implementation details remain private.
- Error types are specific and preserve debugging context.
- Dependencies are declared in the right manifest table and centralized when
  shared.
- New tests sit at the correct boundary and are deterministic; RULES.md's
  test-file list is updated if a binary was added.
- New configuration is typed, validated, and passed explicitly.
- New dialogs follow the keybinding contract in RULES.md.
- `cargo xtask ci` passes.

## Anti-Patterns

Eliminate these patterns:

- Large files with unrelated responsibilities.
- `main.rs` containing business logic.
- State structs with dozens of flat, ungrouped fields spanning unrelated
  concerns.
- Long `if x == Variant` dispatch chains where a match or a trait-based
  dispatch point belongs.
- Public modules that expose implementation structure with no curated facade.
- Broad `utils`, `common`, `misc`, or `helpers` modules.
- Error values represented as plain strings.
- `unwrap` or `expect` in production paths without a proven invariant.
- Boolean parameters that select different behaviors.
- Traits with only one implementation and no real abstraction boundary.
- Global mutable state outside the sanctioned `libllm-config` and `libllm-diagnostics` boundaries.
- Environment-variable reads inside domain logic.
- Async runtimes started inside library crates.
- Network-dependent default tests.
- `#[allow(...)]` or any other lint suppression used to avoid fixing code.
- Documentation that states a contract the code does not honor.
