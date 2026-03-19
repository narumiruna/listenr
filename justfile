set positional-arguments

# Show available recipes grouped by purpose.
[group('meta')]
default:
  @just --list

# Run all quality gates used by CI.
[group('quality')]
check: fmt-check lint test

# Format source files in-place.
[group('quality')]
fmt:
  cargo fmt --all

# Verify formatting without changing files.
[group('quality')]
fmt-check:
  cargo fmt --all --check

# Lint all targets and fail on warnings.
[group('quality')]
lint:
  cargo clippy --all-targets --all-features -- -D warnings

# Run the full test suite for all targets.
[group('quality')]
test:
  cargo test --all-targets --all-features

# Run the CLI locally. Extra args are forwarded to the binary.
[group('build')]
run *args:
  cargo run -- {{args}}

# Build and verify the package tarball for crates.io.
[group('build')]
package *args:
  cargo package {{args}}

# Publish the crate to crates.io (use --dry-run for verification).
[group('release')]
publish *args:
  cargo publish {{args}}
