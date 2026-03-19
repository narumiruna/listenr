# listenr

`listenr` is a small CLI that shows current listening ports and enriches Docker-exposed ports with container/service names.

## Install

From crates.io:

```bash
cargo install listenr
```

From local source (development):

```bash
cargo install --path .
```

## Usage

```bash
listenr
```

The tool:

- collects listeners from `lsof +c 0 -i -P -n` and keeps only `(LISTEN)` rows
- enriches matching host ports from `docker ps --format '{{.ID}}\t{{.Names}}\t{{.Ports}}'`
- prints a table with `PORT`, `PROTO`, `HOST`, `PROCESS`, and `DOCKER`

If Docker is unavailable, `listenr` still prints local listeners and shows a warning on stderr.

## Example Output

```text
PORT   PROTO  HOST       PROCESS                  DOCKER
3001   TCP    *          OrbStack (pid 11645)    uptime-kuma (a2c2ce057b71) -> 3001/tcp
6379   TCP    127.0.0.1  redis-ser (pid 547)     -
7751   TCP    *          OrbStack (pid 11645)    tabemap-app (601f67a6c86c) -> 7751/tcp
```
