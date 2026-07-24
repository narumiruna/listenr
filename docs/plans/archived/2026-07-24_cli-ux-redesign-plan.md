# CLI UX Redesign Plan

## Goal

Redesign `listenr` around fast listener diagnosis while preserving the existing no-argument collection behavior and exact legacy output for redirected or explicitly legacy output.

## Architecture

Keep one binary, with argument parsing, command collection, and result-state modeling in `src/main.rs` and the distinct responsive/JSON/compatibility presentation responsibility in `src/render.rs`. Buffer the complete result before writing stdout. Treat listener collection as required and Docker enrichment as available, skipped, unavailable, or partial.

## Non-Goals

- Interactive TUI, watch mode, mutation, confirmation, persistence, or configuration files.
- UDP support or changes to the underlying `lsof`/Docker matching semantics.
- Perfect simultaneity between sequential `lsof` and Docker snapshots.

## Plan

- [x] Add dependencies and red-first tests for CLI parsing (`PORT`, `--no-docker`, `--details`, `--format`, help/version, and invalid input), then implement deterministic parsing; verified with `cargo test parse_cli` (4 passing tests after the expected missing-symbol red state).
- [x] Add red-first tests for complete, empty, unavailable, skipped, and partial collection states, then refactor the command pipeline to preserve valid listener results and actionable diagnostics; verified with 4 focused collection tests after the expected missing-state-model red state.
- [x] Add red-first tests for wide, narrow, JSON, and exact legacy rendering—including long/Unicode values and explicit `None`, `Unknown`, and `Not checked` states—then implement buffered responsive rendering; verified with 6 focused renderer tests after the expected missing-renderer red state.
- [x] Add CLI integration tests for help/version without external commands, port filtering, degraded/fatal failures, redirected legacy compatibility, JSON, and cancellation before completion; verified with 7 passing tests in `tests/cli.rs`.
- [x] Update `README.md` and `docs/CODE_READING_GUIDE.md` for the final commands, states, compatibility, platform requirements, and snapshot limitations; verified help, version, legacy/degraded, JSON, wide TTY, and 40-column TTY output against the built binary.
- [x] Run formatting, Clippy with warnings denied, all tests, release build, and relevant manual smoke checks; `cargo fmt --all --check`, Clippy, 33 tests, release build, `git diff --check`, rust-analyzer diagnostics, and manual wide/narrow/JSON/degraded checks all passed.

## Completion Checklist

- [x] Primary all-listeners and single-port flows are clear and tested.
- [x] Docker success, no match, skipped, unavailable, and partial states are distinguishable.
- [x] Fatal failures emit no partial stdout and cancellation has no persistent or completed-output side effects.
- [x] Interactive output adapts without truncation, while redirected and `--format legacy` output preserve the old table.
- [x] Text output is linear, ANSI-free, color-independent, and tested at narrow/wide widths with Unicode data.
- [x] JSON provides a versioned machine-readable result and documents that consumers must ignore unknown future fields.
- [x] Help, documentation, and quality gates match the implemented behavior.
