# listenr

`listenr` is a small, read-only CLI for finding TCP listeners, identifying their processes, and mapping Docker-published ports to containers.

## Requirements

- A Unix-like environment with `lsof` installed and available on `PATH` (Linux, macOS, or WSL).
- Docker is optional. If it is missing or unavailable, listener inspection still succeeds and container mappings are marked unknown.

Listener and Docker data are collected sequentially, so the result is a best-effort snapshot of a system that may be changing.

## Install

From crates.io:

```bash
cargo install listenr
```

From local source:

```bash
cargo install --path .
```

## Common workflows

Inspect all listeners:

```bash
listenr
```

Check one host port:

```bash
listenr 3000
```

Skip the optional Docker lookup:

```bash
listenr --no-docker
```

Show container IDs and complete host-to-container mappings:

```bash
listenr --details
```

Use structured output for automation:

```bash
listenr --format json
```

See every option without running a scan:

```bash
listenr --help
```

## Interactive output

In a terminal, `listenr` starts with the current listener and container-mapping state. It uses an aligned table when the content fits and stacked records on narrow terminals (designed for widths of 40 columns and above). Values wrap rather than being truncated.

```text
Listeners: 3 | Container mapping: available; 2 matched

PORT  PROTOCOL  BIND ADDRESS  PROCESS                 CONTAINER
3001  TCP       *             OrbStack (PID 11645)    uptime-kuma
6379  TCP       127.0.0.1     redis-ser (PID 547)     None
7751  TCP       *             OrbStack (PID 11645)    tabemap-app
```

Container values have explicit meanings:

- `None`: Docker was checked and no published-port match exists.
- `Unknown`: Docker data was unavailable or incomplete, so no reliable conclusion is possible.
- `Not checked`: Docker lookup was skipped with `--no-docker`.
- A container name: a published-port match was found.

Warnings are written to stderr with a recovery action. Docker failure remains a successful, degraded scan; required `lsof` failure is fatal and produces no partial stdout.

Output is text-only and ANSI-free: state never depends on color, animation, cursor movement, or symbols alone.

## Output formats and compatibility

`--format` accepts:

- `auto` (default): responsive terminal output; exact legacy table output when stdout is redirected.
- `json`: a versioned, machine-readable result with source completeness and mapping state.
- `legacy`: the original `PORT`, `PROTO`, `HOST`, `PROCESS`, and `DOCKER` table in every context.

Redirected default output remains compatible with the original interface:

```text
PORT   PROTO  HOST       PROCESS                  DOCKER
3001   TCP    *          OrbStack (pid 11645)    uptime-kuma (a2c2ce057b71) -> 3001/tcp
6379   TCP    127.0.0.1  redis-ser (pid 547)     -
7751   TCP    *          OrbStack (pid 11645)    tabemap-app (601f67a6c86c) -> 7751/tcp
```

JSON includes `schema_version: 1`, source status, parse-failure counts, listeners, processes, mapping state, and complete container records. Consumers should ignore unknown fields so compatible fields can be added later.

## Exit and cancellation behavior

- Exit `0`: listener inspection completed, including empty results or unavailable/skipped Docker enrichment.
- Exit `1`: required collection or output failed.
- Exit `2`: command-line arguments were invalid.
- Ctrl-C cancels the read-only scan without saving state or printing a completed result.

`listenr` has no configuration files, stored data, destructive actions, confirmation flow, or apply/save step.
