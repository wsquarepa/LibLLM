# Rust Codebase Standards

This document defines how this Rust codebase should be organized and maintained.
It is intentionally based on public Rust guidance and established Rust projects,
not on the current repository implementation.

## Source Basis

Normative references:

- [Cargo package layout](https://doc.rust-lang.org/cargo/guide/project-layout.html)
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Cargo targets and tests](https://doc.rust-lang.org/cargo/reference/cargo-targets.html)
- [The Rust Book: packages and crates](https://doc.rust-lang.org/book/ch07-01-packages-and-crates.html)
- [The Rust Reference: crates and source files](https://doc.rust-lang.org/reference/crates-and-source-files.html)
- [The Rust Style Guide](https://doc.rust-lang.org/style-guide/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Clippy documentation](https://doc.rust-lang.org/clippy/)
- [The Rust Reference: unsafety](https://doc.rust-lang.org/reference/unsafety.html)

Project exemplars:

- [rust-analyzer](https://github.com/rust-lang/rust-analyzer): large workspace, explicit API-boundary crates, `xtask`, architecture invariants.
- [Cargo](https://github.com/rust-lang/cargo): workspace dependencies, workspace lints, domain crates, credential crates, thin binary target.
- [Tokio](https://github.com/tokio-rs/tokio): public crates plus internal test, bench, stress, and integration packages in one workspace.
- [ripgrep](https://github.com/BurntSushi/ripgrep): command-line binary backed by focused reusable crates for search, matching, printing, and ignore handling.

## Organizing Principles

The codebase should optimize for discoverability, local reasoning, and stable
boundaries. A contributor should be able to answer these questions quickly:

- What crate owns this behavior?
- What module owns this type or function?
- Is this API public, crate-internal, or private implementation detail?
- Which tests prove the behavior?
- Which command verifies the change?

Code should be organized around stable concepts, not around incidental technical
tasks. A database crate, CLI crate, parser crate, UI crate, protocol crate, and
domain crate are meaningful. A broad `utils`, `common`, `helpers`, or `misc`
crate is not.

Default to private implementation details. Expose only the smallest API needed
by the next layer. Use `pub(crate)` for cross-module internals and `pub` only
when callers outside the crate should rely on the item.

## Workspace Layout

Use a Cargo workspace when the project has more than one independently
understandable package. The workspace root should contain shared metadata,
dependency versions, lint policy, profiles, and repository-level automation.

Recommended layout:

```text
.
├── Cargo.toml
├── Cargo.lock
├── README.md
├── STANDARDS.md
├── rustfmt.toml
├── crates/
│   ├── core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs
│   │       ├── model.rs
│   │       └── ...
│   ├── cli/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs
│   ├── storage/
│   ├── protocol/
│   └── test-support/
├── xtask/
│   ├── Cargo.toml
│   └── src/main.rs
├── tests/
├── examples/
└── benches/
```

Use `crates/<name>/` for workspace members unless there is a strong convention
for a different directory. Use `xtask/` for repository automation that would
otherwise become shell scripts: release preparation, code generation, fixture
updates, local install tasks, and cross-crate checks.

Keep the root `Cargo.toml` authoritative for:

- `[workspace]` membership and resolver.
- `[workspace.package]` shared edition, version, license, repository, and MSRV.
- `[workspace.dependencies]` shared dependency versions.
- `[workspace.lints]` shared lint policy.
- Shared `[profile.*]` settings.

Member crates should inherit workspace metadata and dependency versions wherever
possible. They should declare only crate-specific features, targets, and
dependencies.

## Crate Boundaries

Crate boundaries should reflect architecture boundaries.

Recommended crate roles:

- `core` or `domain`: pure business rules, domain models, validation, and
  transformations. No process I/O, no terminal rendering, no network calls, no
  database connections.
- `storage`: persistence schemas, migrations, repositories, transactions, and
  serialization formats for durable state.
- `protocol` or `api`: request and response models, wire-format translation,
  and external API client abstractions.
- `cli`: argument parsing, command dispatch, exit-code mapping, stdout/stderr
  policy, and process-level configuration.
- `tui` or `ui`: rendering, input handling, view state, and UI-specific
  orchestration.
- `test-support`: fixture builders, temp directory helpers, subprocess
  harnesses, and assertions shared by integration tests.
- `macros`: procedural macros only. Keep macro crates small and separately
  tested.
- `xtask`: repository automation only.

Avoid dependency cycles by making dependencies flow inward:

```text
cli/tui/bin -> application orchestration -> storage/api/ui adapters -> core/domain
```

The domain crate should not depend on the CLI, TUI, database, network client, or
process environment. Outer crates may depend on inner crates; inner crates should
not know about outer crates.

Create a new crate when a component has a distinct dependency set, needs an
independent public API boundary, compiles or tests better in isolation, or maps
to a stable domain concept. Do not create a crate only to move files around.

## Package and Target Placement

Follow Cargo target conventions:

- `src/lib.rs`: library crate root.
- `src/main.rs`: default binary crate root.
- `src/bin/*.rs`: additional single-file binaries.
- `src/bin/<name>/main.rs`: multi-file binary target.
- `tests/*.rs`: integration test crates.
- `tests/<name>/main.rs`: multi-file integration test crate.
- `examples/*.rs`: compileable examples.
- `benches/*.rs`: benchmark targets.

For a package with both a library and a binary, keep real behavior in the
library. The binary should parse process inputs, initialize infrastructure, call
library/application code, and translate success or failure into process output
and exit status.

For CLIs, prefer an explicit `[[bin]]` target only when the executable name or
binary path differs from Cargo defaults. Otherwise use the conventional
`src/main.rs`.

## Module Layout

Every source file is a module, and the crate root defines the module tree. Keep
module structure deliberate and shallow.

Recommended module rules:

- `lib.rs` contains crate-level docs, module declarations, and intentional
  re-exports. It should not contain substantial implementation logic.
- `main.rs` contains process wiring only.
- One module should have one main reason to change.
- A module with one principal type can live in `name.rs`.
- A module with several private collaborators should live as `name.rs` plus a
  `name/` directory of child modules.
- Prefer `name.rs` with `name/child.rs` for new code. Use `name/mod.rs` only
  when it is already the local convention or it materially improves clarity.
- Keep child modules private by default and re-export a small facade from the
  parent module when callers need a simpler path.

Example:

```text
src/
├── lib.rs
├── error.rs
├── config.rs
├── session.rs
├── session/
│   ├── branch.rs
│   ├── message.rs
│   └── tree.rs
├── storage.rs
└── storage/
    ├── migrations.rs
    ├── schema.rs
    └── transaction.rs
```

Avoid deep public paths such as `crate::a::b::c::d::Type`. Deep paths usually
mean the public facade is missing or the type is in the wrong layer.

## Naming

Use Rust naming conventions consistently:

- Packages and published crate names: `kebab-case`.
- Rust crate identifiers, modules, functions, methods, and variables:
  `snake_case`.
- Types, traits, and enum variants: `UpperCamelCase`.
- Constants and statics: `SCREAMING_SNAKE_CASE`.
- Macros: `snake_case!`.
- Lifetimes: short lowercase names such as `'a`, `'de`, or a descriptive name
  when the lifetime has domain meaning.

Use standard method names:

- `new` for the primary constructor.
- `with_<detail>` for constructors that need extra information.
- `from_<source>` for conversion constructors when `From` is not appropriate.
- `as_*`, `to_*`, and `into_*` according to Rust conversion conventions.
- `iter`, `iter_mut`, and `into_iter` for iterator-producing methods.

Name errors consistently in verb-object order: `ParseConfigError`,
`OpenDatabaseError`, `LoadSessionError`. Keep error names stable and specific.

Avoid all-caps acronyms in type names. Prefer `HttpClient`, `SqlStore`, and
`UuidParser` over `HTTPClient`, `SQLStore`, and `UUIDParser`.

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
- Keep trait definitions at real abstraction boundaries. Do not create a trait
  for one implementation unless tests or external callers need the abstraction.
- If a trait may be used as a trait object, design it for object safety from the
  start.

Use crate roots and parent modules as facades. Re-export the types callers need;
keep implementation modules private.

## Error Handling

Use explicit, typed, contextual errors.

Rules:

- Return `Result<T, E>` for recoverable failures.
- Use `Option<T>` only for expected absence, not for failure with lost context.
- Library crates expose specific error types. Binary crates may use broader
  report types at the process boundary.
- Include enough context to debug failures: operation, path or identifier,
  external request parameters where safe, status codes, and response bodies when
  relevant.
- Do not silently ignore errors. Either handle them deliberately or return them.
- Do not catch broad errors and replace them with generic messages.
- Do not use `unwrap` or `expect` in production paths unless the invariant is
  structurally guaranteed and the message documents the invariant precisely.
- Tests may use `unwrap` or `expect` when failure would make the test itself
  invalid.
- Panics are for bugs, violated invariants, and unrecoverable programmer errors,
  not normal user or environment failures.
- Destructors must not be the only place where fallible cleanup happens. Provide
  an explicit `close`, `finish`, `flush`, or `commit` method that returns a
  `Result`.

For external services, centralize retry policy. Retry only operations that are
safe to retry, log each retry with structured fields, and return the last error
with full context when retries are exhausted.

## Configuration and State

Configuration should be parsed once, validated once, and passed explicitly.

Rules:

- Represent configuration as typed structs.
- Keep raw environment variables, CLI strings, and config-file syntax at the
  process boundary.
- Convert raw inputs into validated domain types before they reach core logic.
- Do not read environment variables deep inside library code.
- Avoid global mutable state. If shared process state is required, isolate it in
  an application state type and pass references explicitly.
- Keep pure domain functions free of I/O, time, randomness, logging, and process
  state.

## Async and Concurrency

Concurrency policy belongs at orchestration boundaries.

Rules:

- Binary/application crates own runtime initialization.
- Library crates should not start a runtime.
- Spawn tasks only in orchestration code or in a component explicitly
  responsible for background work.
- Pass cancellation, timeout, and shutdown signals explicitly.
- Do not block inside async code. Use the runtime's blocking facilities for
  unavoidable blocking work.
- Protect shared mutable state behind the narrowest synchronization primitive
  that matches the access pattern.
- Prefer message passing or ownership transfer when it makes state changes
  easier to reason about.

## Observability

Use structured diagnostics.

Rules:

- Libraries use `tracing` or another structured facade, not `println!`.
- CLIs write intentional user output to stdout and diagnostics to stderr.
- Log dynamic values as fields, not by formatting them into the message string.
- Use levels consistently:
  - `trace`: per-event details useful only while debugging.
  - `debug`: internal state transitions and lifecycle events.
  - `info`: user-significant operations and durable state changes.
  - `warn`: recoverable problems and retries.
  - `error`: failed operations that prevent the requested work.

Do not log secrets, passphrases, tokens, private message bodies, or full request
payloads unless the payload is explicitly non-sensitive and the diagnostic mode
requires it.

## Unsafe Code

Safe Rust is the default. Unsafe code must be rare, isolated, and reviewable.

Rules:

- Prefer `#![forbid(unsafe_code)]` in crates that do not need unsafe code.
- When unsafe is necessary, isolate it in a small module or crate with a safe
  public wrapper.
- Every `unsafe fn`, unsafe trait, and unsafe block must have a documented
  safety invariant.
- Enable `unsafe_op_in_unsafe_fn` so unsafe operations remain explicit inside
  unsafe functions.
- Add tests that exercise the safe wrapper boundary. Use Miri or sanitizers when
  the unsafe code depends on aliasing, initialization, or pointer invariants.
- Do not use unsafe for convenience, to bypass the borrow checker, or to avoid a
  small amount of ordinary code.

## Formatting and Lints

Formatting is automated, not debated.

Rules:

- Use `rustfmt` and keep formatting configuration minimal.
- Follow default Rust style: spaces, 4-space indentation, and 100-character line
  width unless the project has an explicit reason to differ.
- Keep imports at the top of the file and let `rustfmt` organize them.
- Use workspace lint configuration so every crate inherits the same baseline.
- Treat Clippy correctness and performance warnings as defects.
- Prefer fixing lints over suppressing them.
- If a lint must be suppressed, make the suppression narrow and include a reason
  that explains the structural constraint.
- Do not use broad lint suppression at crate or workspace level to hide local
  problems.

Recommended verification commands:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

If features are mutually exclusive, replace `--all-features` with the explicit
feature matrix that CI supports.

## Dependencies

Dependencies should be centralized, intentional, and visible.

Rules:

- Define shared versions in `[workspace.dependencies]`.
- Use `dependency.workspace = true` in member crates.
- Disable default features when a dependency's defaults pull in unnecessary
  runtime, TLS, OS, or serialization support.
- Keep public dependencies stable if their types appear in public APIs.
- Prefer established crates with active maintenance, clear licensing, and small
  transitive dependency cost.
- Keep dev-only tools and fixtures in `dev-dependencies`.
- Keep build-time tools in `build-dependencies`.
- Avoid adding dependencies for trivial code.
- Audit new dependencies for license, maintenance, security posture, feature
  flags, and public API leakage before adoption.

## Tests

Testing should match the boundary being protected.

Use:

- Unit tests beside private logic when testing edge cases and invariants.
- Integration tests under `tests/` when testing the public API or process
  contract.
- Subprocess tests for CLI behavior, exit codes, stdout/stderr separation, and
  environment handling.
- Doctests for public examples that should stay compileable.
- Data-driven or golden tests for parsers, formatters, migrations, importers,
  exporters, and rendering output.
- Property tests for algorithms with broad input spaces.
- Benchmarks under `benches/` only for performance-sensitive code with stable
  measurement goals.

Rules:

- Test names should describe the behavior being proved.
- Tests must be deterministic and isolated.
- Use temporary directories for filesystem tests.
- Do not require network access in normal test runs.
- Store fixtures near the tests that use them, such as `tests/fixtures/` or a
  crate-specific `test_data/` directory.
- Shared integration-test helpers belong in `tests/common/mod.rs` or a
  dedicated `test-support` crate when multiple crates need them.
- Prefer testing public behavior over internal implementation details.

## Documentation

Documentation should live as close as possible to the API it explains.

Rules:

- `lib.rs` should explain the crate's purpose and show a minimal example when
  the crate exposes a public API.
- Public types, traits, functions, modules, and macros should have rustdoc.
- Public fallible functions should document meaningful error conditions.
- Public panicking behavior should be documented when callers can trigger it.
- Unsafe APIs must document caller or implementor obligations in a `# Safety`
  section.
- Examples should explain why an API is useful, not merely restate its syntax.
- Separate architecture documents should describe durable boundaries,
  invariants, and workflows that cannot be expressed clearly in code.
- Do not duplicate implementation details across README files, rustdoc, and
  separate docs.

## Review Checklist

Use this checklist when reviewing a Rust change:

- The file is in the crate and module that own the concept.
- The binary layer only wires process concerns.
- Domain logic is independent of I/O and process state.
- New public API is intentionally exposed and documented.
- Private implementation details remain private.
- Error types are specific and preserve debugging context.
- Dependencies are declared in the right manifest table and centralized when
  shared.
- New tests sit at the correct boundary and are deterministic.
- New configuration is typed, validated, and passed explicitly.
- Unsafe code is absent or isolated behind a documented safe wrapper.
- `cargo fmt`, `cargo clippy`, tests, and docs verification pass for the
  supported feature set.

## Anti-Patterns

Eliminate these patterns:

- Large files with unrelated responsibilities.
- `main.rs` containing business logic.
- Public modules that expose implementation structure.
- Broad `utils`, `common`, `misc`, or `helpers` modules.
- Error values represented as plain strings.
- `unwrap` or `expect` in production paths without a proven invariant.
- Boolean parameters that select different behaviors.
- Traits with only one implementation and no real abstraction boundary.
- Global mutable state.
- Environment-variable reads inside domain logic.
- Async runtimes started inside library crates.
- Network-dependent default tests.
- Lint suppression used to avoid fixing code.
- Unsafe code used for convenience rather than a documented necessity.
