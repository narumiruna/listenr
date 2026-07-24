# listenr Code Reading Guide

This guide explains the CLI pipeline and where to modify it safely.

## 1. Runtime flow

Read the entry path in this order:

1. `main()` parses arguments and handles help, version, and argument errors without scanning.
2. `run()` collects a complete in-memory snapshot, selects an output mode, and writes stdout once.
3. `collect_snapshot()` coordinates required listener collection, optional port filtering, and Docker enrichment.
4. `collect_listen_entries_with()` and `collect_docker_bindings_with()` execute and parse their data sources.
5. `build_docker_lookup()` joins published host ports to normalized container records.
6. `src/render.rs` renders responsive terminal text, versioned JSON, or the compatibility table.

Stdout remains empty until collection and rendering finish. Warnings use stderr. A required `lsof` failure is fatal; Docker failure produces a degraded snapshot.

## 2. Source organization

- `src/main.rs`: CLI parsing, command execution, parser/join logic, source-state modeling, filtering, and orchestration.
- `src/render.rs`: responsive, JSON, and exact legacy rendering.
- `tests/cli.rs`: Unix CLI integration tests with isolated fake `lsof` and Docker commands.

The renderer is separate because responsive wrapping, explicit state presentation, JSON, and compatibility output form a distinct responsibility. Core collection remains in the entrypoint.

## 3. Core data structures

- `CliOptions`: optional port filter, Docker policy, detail level, and output format.
- `ListenEntry`: normalized process, PID, protocol, bind address, and host port.
- `DockerPortBinding`: one parsed published host-to-container mapping.
- `DockerServicePort`: normalized container record used for joining and display.
- `DockerEnrichment`: available (possibly partial), skipped, or unavailable Docker state.
- `Snapshot`: filtered listeners, listener completeness, and Docker enrichment state.
- `CommandOutput`: captured stdout, stderr, and success from an external command.

## 4. Parsing and collection

- `parse_cli()`: validates all supported arguments. Unknown options and invalid ports are errors.
- `parse_lsof_line()`: converts one TCP `(LISTEN)` row into a `ListenEntry`.
- `parse_host_and_port()`: handles ordinary and bracketed IPv6 addresses.
- `decode_lsof_command()`: decodes escaped command bytes such as `OrbStack\x20Helper`.
- `parse_docker_ps_line_detailed()`: returns valid published-port bindings and a failure count while accepting internal-only exposed ports.

Candidate rows that cannot be parsed increment a partial-state count rather than disappearing silently. Keep the distinction between a valid empty result, partial data, and command failure.

## 5. Rendering contracts

`src/render.rs` owns three formats:

- `auto()`: status summary plus a table that switches to wrapped stacked records when the computed display width does not fit. It uses Unicode display width and never truncates values.
- `json()`: schema version 1 with explicit source and per-listener mapping states.
- `legacy()`: the original dynamically padded five-column output, including its exact labels, casing, details, and fallback `-`.

Default `auto` selects responsive output only for a terminal. Redirected stdout selects `legacy()` to preserve existing shell workflows. `--format legacy` always forces that contract.

When changing output, preserve these meanings:

- `None`: reliable lookup, no mapping.
- `Unknown`: unavailable or partial lookup cannot establish absence.
- `Not checked`: lookup intentionally skipped.

## 6. Where to change behavior

- Add or modify CLI options: `CliOptions`, `parse_cli()`, `HELP`, and parsing tests in `src/main.rs`.
- Change external commands: `collect_listen_entries_with()` or `collect_docker_bindings_with()`.
- Change parser rules: `parse_lsof_line()` or `parse_docker_ps_line_detailed()`; add a focused regression test first.
- Change matching: `build_docker_lookup()` and the lookup key used by `src/render.rs`.
- Change presentation: `src/render.rs`; retain a golden legacy compatibility test.
- Change process-level behavior: `tests/cli.rs` for help, exits, cancellation, stdout/stderr separation, and fake command execution.

## 7. Validation

Run the same quality gates as CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Also build the release binary when command behavior changes:

```bash
cargo build --release
```
