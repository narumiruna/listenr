# listenr Code Reading Guide

This guide explains how `listenr` works end-to-end and where to modify behavior safely.

## 1. Start From the Entry Path

Read the call flow in this order:

1. `main()`
2. `run()`
3. `collect_listen_entries()`
4. `collect_docker_bindings()`
5. `build_docker_lookup()`
6. `print_rows()`

This sequence is the entire runtime pipeline.

## 2. Runtime Pipeline (Data Flow)

`listenr` is a single-file pipeline:

1. Run `lsof +c 0 -i -P -n` and keep rows that contain `(LISTEN)`.
2. Parse each row into `ListenEntry`.
3. Run `docker ps --format '{{.ID}}\t{{.Names}}\t{{.Ports}}'`.
4. Parse Docker mappings into `DockerPortBinding`.
5. Build a lookup keyed by `(host_port, proto)` to get Docker service info.
6. Join listener rows with Docker info and print a fixed-width table.

If Docker is unavailable, the tool prints a warning to stderr and still shows listener rows.

## 3. Core Data Structures

- `ListenEntry`:
  Parsed listener record from `lsof` (`process`, `pid`, `proto`, `host`, `port`).
- `DockerPortBinding`:
  One parsed host->container port mapping from Docker output.
- `DockerServicePort`:
  Normalized service row used in the lookup map and display.
- `CommandOutput`:
  Minimal wrapper around external command execution (`stdout`, `stderr`, `success`).

## 4. Parsing Responsibilities

- `parse_lsof_line()`:
  Converts one `lsof` line into `ListenEntry`.
- `parse_host_and_port()`:
  Splits host/port and handles IPv6 bracket form.
- `decode_lsof_command()`:
  Decodes escaped command names such as `OrbStack\x20Helper`.
- `parse_docker_ps_line()`:
  Parses Docker `Ports` field and ignores entries that are not host-published (for example `6379/tcp` only).

If output format changes, these parser functions are the first place to inspect.

## 5. Join and Rendering

- `build_docker_lookup()`:
  Groups Docker bindings by `(host_port, proto)` and deduplicates with `BTreeSet`.
- `format_docker_services()`:
  Converts matched services to human-readable text.
- `print_rows()` + `print_row()`:
  Computes column widths and prints aligned output.

Display behavior is intentionally simple and deterministic (sorted entries, stable output).

## 6. Where to Change Specific Behavior

- Add/modify data source commands:
  `collect_listen_entries()` and `collect_docker_bindings()`.
- Change parser rules:
  `parse_lsof_line()` / `parse_docker_ps_line()`.
- Change matching strategy:
  `build_docker_lookup()` and lookup key usage in `print_rows()`.
- Change output format:
  `print_rows()`, `print_row()`, `format_docker_services()`.

## 7. Tests You Should Read First

The tests in `src/main.rs` document expected parser behavior:

- `parse_lsof_ipv4_wildcard`
- `parse_lsof_ipv6_loopback`
- `decode_lsof_command_unescapes_hex_bytes`
- `parse_docker_ports_with_ipv4_and_ipv6_mappings`
- `parse_docker_ports_ignores_internal_only_expose`
- `docker_lookup_deduplicates_dual_stack_bindings`
- `format_docker_outputs_fallback_for_missing_service`

When changing parsing/join logic, update these tests first, then implement.
