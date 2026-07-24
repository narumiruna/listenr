mod render;

use render::{auto as render_auto, json as render_json, legacy as render_legacy};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal, Write as _};
use std::process::Command;

#[cfg(test)]
use render::format_docker_services;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListenEntry {
    process: String,
    pid: u32,
    proto: String,
    host: String,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DockerServicePort {
    container_id: String,
    container_name: String,
    container_port: u16,
    proto: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerPortBinding {
    container_id: String,
    container_name: String,
    host_port: u16,
    container_port: u16,
    proto: String,
}

#[derive(Debug)]
struct CommandOutput {
    stdout: String,
    stderr: String,
    success: bool,
}

trait CommandRunner {
    fn run(&self, command: &str, args: &[&str]) -> Result<CommandOutput, String>;
}

struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &str, args: &[&str]) -> Result<CommandOutput, String> {
        run_command(command, args)
    }
}

#[derive(Debug)]
enum DockerEnrichment {
    Available {
        lookup: BTreeMap<(u16, String), BTreeSet<DockerServicePort>>,
        parse_failures: usize,
    },
    Skipped,
    Unavailable {
        error: String,
    },
}

#[derive(Debug)]
struct Snapshot {
    entries: Vec<ListenEntry>,
    listener_parse_failures: usize,
    docker: DockerEnrichment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum OutputFormat {
    #[default]
    Auto,
    Json,
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CliOptions {
    port: Option<u16>,
    no_docker: bool,
    details: bool,
    format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseOutcome {
    Run(CliOptions),
    Help,
    Version,
}

fn parse_cli<I, S>(args: I) -> Result<ParseOutcome, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut options = CliOptions::default();
    let mut args = args.into_iter();
    let mut positional_only = false;

    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        if !positional_only {
            match arg {
                "-h" | "--help" => return Ok(ParseOutcome::Help),
                "-V" | "--version" => return Ok(ParseOutcome::Version),
                "--no-docker" => {
                    options.no_docker = true;
                    continue;
                }
                "--details" => {
                    options.details = true;
                    continue;
                }
                "--" => {
                    positional_only = true;
                    continue;
                }
                "--format" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--format requires auto, json, or legacy".to_string())?;
                    options.format = parse_output_format(value.as_ref())?;
                    continue;
                }
                _ => {}
            }

            if let Some(value) = arg.strip_prefix("--format=") {
                options.format = parse_output_format(value)?;
                continue;
            }
            if arg.starts_with('-') {
                return Err(format!("unknown option `{arg}`; try `listenr --help`"));
            }
        }

        if options.port.is_some() {
            return Err("listenr accepts at most one port".to_string());
        }
        options.port = Some(parse_port(arg)?);
    }

    Ok(ParseOutcome::Run(options))
}

fn parse_output_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "auto" => Ok(OutputFormat::Auto),
        "json" => Ok(OutputFormat::Json),
        "legacy" => Ok(OutputFormat::Legacy),
        _ => Err(format!(
            "invalid format `{value}`; expected auto, json, or legacy"
        )),
    }
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| format!("invalid port `{value}`; expected a number between 1 and 65535"))
}

const HELP: &str = "listenr — show TCP listeners and their container ownership

Usage: listenr [OPTIONS] [PORT]

Arguments:
  [PORT]  Show only one host port (1-65535)

Options:
      --no-docker       Skip optional container lookup
      --details         Show container IDs and full port mappings
      --format <FORMAT> Output mode: auto, json, or legacy [default: auto]
  -h, --help            Show this help and exit
  -V, --version         Show the version and exit

Auto format uses a responsive layout in a terminal and preserves the legacy table when redirected.
";

fn main() {
    let outcome = match parse_cli(std::env::args().skip(1)) {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("error: {err}\n\nUsage: listenr [OPTIONS] [PORT]");
            std::process::exit(2);
        }
    };

    match outcome {
        ParseOutcome::Help => print!("{HELP}"),
        ParseOutcome::Version => println!("listenr {}", env!("CARGO_PKG_VERSION")),
        ParseOutcome::Run(options) => {
            if let Err(err) = run(&options) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
    }
}

fn run(options: &CliOptions) -> Result<(), String> {
    let snapshot = collect_snapshot(&SystemCommandRunner, options)?;
    emit_warnings(&snapshot);

    let stdout = io::stdout();
    let output = match options.format {
        OutputFormat::Json => render_json(&snapshot),
        OutputFormat::Legacy => render_legacy(&snapshot),
        OutputFormat::Auto if stdout.is_terminal() => {
            let width = terminal_size::terminal_size()
                .map_or(80, |(terminal_size::Width(width), _)| usize::from(width));
            render_auto(&snapshot, options, width)
        }
        OutputFormat::Auto => render_legacy(&snapshot),
    };

    let mut stdout = stdout.lock();
    match stdout.write_all(output.as_bytes()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(format!("failed to write output: {err}")),
    }
}

fn emit_warnings(snapshot: &Snapshot) {
    if snapshot.listener_parse_failures > 0 {
        eprintln!(
            "warning: listener data is partial: {} LISTEN row(s) could not be parsed",
            snapshot.listener_parse_failures
        );
    }
    match &snapshot.docker {
        DockerEnrichment::Available { parse_failures, .. } if *parse_failures > 0 => {
            eprintln!(
                "warning: container mapping is partial: {parse_failures} Docker port mapping(s) could not be parsed"
            );
        }
        DockerEnrichment::Unavailable { error } => {
            eprintln!(
                "warning: container mapping is unavailable: {error}; run `docker ps` to diagnose or use `--no-docker`"
            );
        }
        DockerEnrichment::Available { .. } | DockerEnrichment::Skipped => {}
    }
}

fn collect_snapshot(runner: &impl CommandRunner, options: &CliOptions) -> Result<Snapshot, String> {
    let (mut entries, listener_parse_failures) = collect_listen_entries_with(runner)?;
    if let Some(port) = options.port {
        entries.retain(|entry| entry.port == port);
    }

    let docker = if options.no_docker {
        DockerEnrichment::Skipped
    } else {
        match collect_docker_bindings_with(runner) {
            Ok((bindings, parse_failures)) => DockerEnrichment::Available {
                lookup: build_docker_lookup(bindings),
                parse_failures,
            },
            Err(error) => DockerEnrichment::Unavailable { error },
        }
    };

    Ok(Snapshot {
        entries,
        listener_parse_failures,
        docker,
    })
}

fn collect_listen_entries_with(
    runner: &impl CommandRunner,
) -> Result<(Vec<ListenEntry>, usize), String> {
    let output = runner
        .run("lsof", &["+c", "0", "-i", "-P", "-n"])
        .map_err(|err| {
            format!(
                "cannot inspect listeners: {err}; ensure `lsof` is installed and available on PATH"
            )
        })?;
    if !output.success {
        let stderr = output.stderr.trim();
        return Err(if stderr.is_empty() {
            "lsof failed; verify that `lsof` is installed and you have permission to inspect processes"
                .to_string()
        } else {
            format!("lsof failed: {stderr}")
        });
    }

    let mut entries = Vec::new();
    let mut parse_failures = 0;
    for line in output.stdout.lines() {
        if !line.contains("(LISTEN)") {
            continue;
        }
        if let Some(entry) = parse_lsof_line(line) {
            entries.push(entry);
        } else {
            parse_failures += 1;
        }
    }

    entries.sort_by(|a, b| {
        a.port
            .cmp(&b.port)
            .then(a.proto.cmp(&b.proto))
            .then(a.host.cmp(&b.host))
            .then(a.process.cmp(&b.process))
            .then(a.pid.cmp(&b.pid))
    });
    entries.dedup_by(|a, b| {
        a.port == b.port
            && a.proto == b.proto
            && a.host == b.host
            && a.process == b.process
            && a.pid == b.pid
    });
    Ok((entries, parse_failures))
}

fn collect_docker_bindings_with(
    runner: &impl CommandRunner,
) -> Result<(Vec<DockerPortBinding>, usize), String> {
    let output = runner.run(
        "docker",
        &["ps", "--format", "{{.ID}}\t{{.Names}}\t{{.Ports}}"],
    )?;
    if !output.success {
        let stderr = output.stderr.trim();
        return Err(if stderr.is_empty() {
            "docker ps failed".to_string()
        } else {
            format!("docker ps failed: {stderr}")
        });
    }

    let mut bindings = Vec::new();
    let mut parse_failures = 0;
    for line in output.stdout.lines() {
        let (parsed, failures) = parse_docker_ps_line_detailed(line);
        bindings.extend(parsed);
        parse_failures += failures;
    }
    Ok((bindings, parse_failures))
}

fn run_command(command: &str, args: &[&str]) -> Result<CommandOutput, String> {
    let output = Command::new(command).args(args).output().map_err(|err| {
        format!(
            "failed to run `{command} {}`: {err}",
            args.join(" ").trim_end()
        )
    })?;

    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    })
}

fn parse_lsof_line(line: &str) -> Option<ListenEntry> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 10 {
        return None;
    }

    let process = decode_lsof_command(fields[0]);
    let pid = fields[1].parse::<u32>().ok()?;
    let proto_idx = fields
        .iter()
        .position(|field| field.eq_ignore_ascii_case("TCP"))?;
    let addr = fields.get(proto_idx + 1)?;
    let (host, port) = parse_host_and_port(addr)?;

    Some(ListenEntry {
        process,
        pid,
        proto: "TCP".to_string(),
        host,
        port,
    })
}

fn parse_host_and_port(addr: &str) -> Option<(String, u16)> {
    let (host, port_text) = addr.rsplit_once(':')?;
    let port = port_text.parse::<u16>().ok()?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    Some((host, port))
}

fn decode_lsof_command(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1] == b'x'
            && bytes[i + 2].is_ascii_hexdigit()
            && bytes[i + 3].is_ascii_hexdigit()
        {
            let hex = &raw[i + 2..i + 4];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value as char);
                i += 4;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn parse_docker_ps_line_detailed(line: &str) -> (Vec<DockerPortBinding>, usize) {
    if line.trim().is_empty() {
        return (Vec::new(), 0);
    }

    let mut parts = line.splitn(3, '\t');
    let container_id = match parts.next() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return (Vec::new(), 1),
    };
    let container_name = match parts.next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => return (Vec::new(), 1),
    };
    let Some(ports) = parts.next() else {
        return (Vec::new(), 1);
    };
    if ports.is_empty() {
        return (Vec::new(), 0);
    }

    let mut bindings = Vec::new();
    let mut parse_failures = 0;
    for raw_mapping in ports.split(", ") {
        let Some((host_mapping, container_mapping)) = raw_mapping.split_once("->") else {
            if !is_internal_only_port(raw_mapping) {
                parse_failures += 1;
            }
            continue;
        };

        let parsed = host_mapping
            .rsplit_once(':')
            .and_then(|(_, host_port)| host_port.parse::<u16>().ok())
            .zip(
                container_mapping
                    .split_once('/')
                    .and_then(|(container_port, proto)| {
                        container_port.parse::<u16>().ok().map(|port| (port, proto))
                    }),
            );

        let Some((host_port, (container_port, proto))) = parsed else {
            parse_failures += 1;
            continue;
        };

        bindings.push(DockerPortBinding {
            container_id: container_id.clone(),
            container_name: container_name.clone(),
            host_port,
            container_port,
            proto: proto.to_ascii_lowercase(),
        });
    }

    (bindings, parse_failures)
}

fn is_internal_only_port(value: &str) -> bool {
    value
        .split_once('/')
        .is_some_and(|(ports, proto)| is_port_or_range(ports) && !proto.is_empty())
}

fn is_port_or_range(value: &str) -> bool {
    if value.parse::<u16>().is_ok() {
        return true;
    }
    value.split_once('-').is_some_and(|(start, end)| {
        let Ok(start) = start.parse::<u16>() else {
            return false;
        };
        let Ok(end) = end.parse::<u16>() else {
            return false;
        };
        start <= end
    })
}

fn build_docker_lookup(
    bindings: Vec<DockerPortBinding>,
) -> BTreeMap<(u16, String), BTreeSet<DockerServicePort>> {
    let mut lookup: BTreeMap<(u16, String), BTreeSet<DockerServicePort>> = BTreeMap::new();
    for binding in bindings {
        lookup
            .entry((binding.host_port, binding.proto.clone()))
            .or_default()
            .insert(DockerServicePort {
                container_id: binding.container_id,
                container_name: binding.container_name,
                container_port: binding.container_port,
                proto: binding.proto,
            });
    }
    lookup
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsof_ipv4_wildcard() {
        let line =
            "OrbStack  11645 narumi  355u  IPv4 0xccefc3f372998c54      0t0  TCP *:3001 (LISTEN)";
        let entry = parse_lsof_line(line).expect("must parse");
        assert_eq!(entry.process, "OrbStack");
        assert_eq!(entry.pid, 11645);
        assert_eq!(entry.proto, "TCP");
        assert_eq!(entry.host, "*");
        assert_eq!(entry.port, 3001);
    }

    #[test]
    fn parse_lsof_ipv6_loopback() {
        let line = "redis-ser   547 narumi    7u  IPv6 0xdaccf0320f993419      0t0  TCP [::1]:6379 (LISTEN)";
        let entry = parse_lsof_line(line).expect("must parse");
        assert_eq!(entry.host, "::1");
        assert_eq!(entry.port, 6379);
    }

    #[test]
    fn decode_lsof_command_unescapes_hex_bytes() {
        let decoded = decode_lsof_command("OrbStack\\x20Helper");
        assert_eq!(decoded, "OrbStack Helper");
    }

    #[test]
    fn parse_docker_ports_with_ipv4_and_ipv6_mappings() {
        let line = "601f67a6c86c\ttabemap-app\t0.0.0.0:7751->7751/tcp, [::]:7751->7751/tcp";
        let (bindings, failures) = parse_docker_ps_line_detailed(line);
        assert_eq!(failures, 0);
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].host_port, 7751);
        assert_eq!(bindings[0].container_port, 7751);
        assert_eq!(bindings[0].proto, "tcp");
    }

    #[test]
    fn parse_docker_ports_ignores_internal_only_expose() {
        let line = "1b5d6f457360\timmich_redis\t6379/tcp";
        let (bindings, failures) = parse_docker_ps_line_detailed(line);
        assert_eq!(failures, 0);
        assert!(bindings.is_empty());
    }

    #[test]
    fn parse_docker_ports_accepts_internal_only_range() {
        let line = "1b5d6f457360\tservice\t8000-8005/tcp";
        let (bindings, failures) = parse_docker_ps_line_detailed(line);
        assert_eq!(failures, 0);
        assert!(bindings.is_empty());
    }

    #[test]
    fn docker_lookup_deduplicates_dual_stack_bindings() {
        let bindings = vec![
            DockerPortBinding {
                container_id: "601f67a6c86c".to_string(),
                container_name: "tabemap-app".to_string(),
                host_port: 7751,
                container_port: 7751,
                proto: "tcp".to_string(),
            },
            DockerPortBinding {
                container_id: "601f67a6c86c".to_string(),
                container_name: "tabemap-app".to_string(),
                host_port: 7751,
                container_port: 7751,
                proto: "tcp".to_string(),
            },
        ];

        let lookup = build_docker_lookup(bindings);
        let services = lookup
            .get(&(7751, "tcp".to_string()))
            .expect("service should exist");
        assert_eq!(services.len(), 1);
    }

    #[test]
    fn format_docker_outputs_fallback_for_missing_service() {
        let text = format_docker_services(None);
        assert_eq!(text, "-");
    }

    #[test]
    fn parse_cli_defaults_to_auto_all_listeners() {
        assert_eq!(
            parse_cli(Vec::<String>::new()).expect("arguments should parse"),
            ParseOutcome::Run(CliOptions::default())
        );
    }

    #[test]
    fn parse_cli_accepts_focused_and_advanced_options() {
        let outcome = parse_cli(["3000", "--no-docker", "--details", "--format=json"])
            .expect("arguments should parse");

        assert_eq!(
            outcome,
            ParseOutcome::Run(CliOptions {
                port: Some(3000),
                no_docker: true,
                details: true,
                format: OutputFormat::Json,
            })
        );
    }

    #[test]
    fn parse_cli_handles_help_without_running_a_scan() {
        assert_eq!(parse_cli(["--help"]).unwrap(), ParseOutcome::Help);
        assert_eq!(parse_cli(["-V"]).unwrap(), ParseOutcome::Version);
    }

    #[test]
    fn parse_cli_rejects_unknown_options_and_invalid_ports() {
        assert!(parse_cli(["--wat"]).unwrap_err().contains("unknown option"));
        assert!(
            parse_cli(["0"])
                .unwrap_err()
                .contains("between 1 and 65535")
        );
        assert!(
            parse_cli(["70000"])
                .unwrap_err()
                .contains("between 1 and 65535")
        );
        assert!(
            parse_cli(["3000", "4000"])
                .unwrap_err()
                .contains("one port")
        );
    }

    #[derive(Default)]
    struct FakeRunner {
        outputs: std::cell::RefCell<Vec<Result<CommandOutput, String>>>,
        commands: std::cell::RefCell<Vec<String>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<Result<CommandOutput, String>>) -> Self {
            Self {
                outputs: std::cell::RefCell::new(outputs.into_iter().rev().collect()),
                commands: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, command: &str, _args: &[&str]) -> Result<CommandOutput, String> {
            self.commands.borrow_mut().push(command.to_string());
            self.outputs
                .borrow_mut()
                .pop()
                .expect("fake output should exist")
        }
    }

    fn successful_output(stdout: &str) -> Result<CommandOutput, String> {
        Ok(CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            success: true,
        })
    }

    #[test]
    fn collect_snapshot_tracks_complete_and_empty_states() {
        let runner = FakeRunner::with_outputs(vec![
            successful_output("COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n"),
            successful_output(""),
        ]);

        let snapshot = collect_snapshot(&runner, &CliOptions::default()).expect("scan succeeds");

        assert!(snapshot.entries.is_empty());
        assert_eq!(snapshot.listener_parse_failures, 0);
        assert!(matches!(
            snapshot.docker,
            DockerEnrichment::Available {
                parse_failures: 0,
                ..
            }
        ));
    }

    #[test]
    fn collect_snapshot_skips_docker_when_disabled() {
        let runner = FakeRunner::with_outputs(vec![successful_output(
            "node 42 user 1u IPv4 0 0t0 TCP *:3000 (LISTEN)\n",
        )]);
        let options = CliOptions {
            no_docker: true,
            ..CliOptions::default()
        };

        let snapshot = collect_snapshot(&runner, &options).expect("scan succeeds");

        assert!(matches!(snapshot.docker, DockerEnrichment::Skipped));
        assert_eq!(runner.commands.borrow().as_slice(), ["lsof"]);
    }

    #[test]
    fn collect_snapshot_preserves_valid_rows_and_marks_partial_data() {
        let runner = FakeRunner::with_outputs(vec![
            successful_output(
                "node 42 user 1u IPv4 0 0t0 TCP *:3000 (LISTEN)\nmalformed (LISTEN)\n",
            ),
            successful_output("abc123\tweb\t0.0.0.0:3000->80/tcp\nbroken docker row\n"),
        ]);

        let snapshot = collect_snapshot(&runner, &CliOptions::default()).expect("scan succeeds");

        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.listener_parse_failures, 1);
        assert!(matches!(
            snapshot.docker,
            DockerEnrichment::Available {
                parse_failures: 1,
                ..
            }
        ));
    }

    #[test]
    fn collect_snapshot_keeps_listeners_when_docker_is_unavailable() {
        let runner = FakeRunner::with_outputs(vec![
            successful_output("node 42 user 1u IPv4 0 0t0 TCP *:3000 (LISTEN)\n"),
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: "daemon is stopped".to_string(),
                success: false,
            }),
        ]);

        let snapshot = collect_snapshot(&runner, &CliOptions::default()).expect("scan succeeds");

        assert_eq!(snapshot.entries.len(), 1);
        assert!(matches!(
            snapshot.docker,
            DockerEnrichment::Unavailable { ref error } if error.contains("daemon is stopped")
        ));
    }

    fn sample_entry() -> ListenEntry {
        ListenEntry {
            process: "node".to_string(),
            pid: 42,
            proto: "TCP".to_string(),
            host: "*".to_string(),
            port: 3000,
        }
    }

    fn available_snapshot(entries: Vec<ListenEntry>, services: Vec<DockerServicePort>) -> Snapshot {
        let mut lookup = BTreeMap::new();
        if !services.is_empty() {
            lookup.insert((3000, "tcp".to_string()), services.into_iter().collect());
        }
        Snapshot {
            entries,
            listener_parse_failures: 0,
            docker: DockerEnrichment::Available {
                lookup,
                parse_failures: 0,
            },
        }
    }

    #[test]
    fn render_auto_wide_prioritizes_state_and_explicit_no_match() {
        let output = render_auto(
            &available_snapshot(vec![sample_entry()], Vec::new()),
            &CliOptions::default(),
            120,
        );

        assert!(output.contains("Listeners: 1 | Container mapping: available; 0 matched"));
        assert!(output.contains("PORT"));
        assert!(output.contains("BIND ADDRESS"));
        assert!(output.contains("CONTAINER"));
        assert!(output.contains("None"));
        assert!(!output.contains("\u{1b}["));
    }

    #[test]
    fn render_auto_distinguishes_unavailable_skipped_and_partial_states() {
        let unavailable = Snapshot {
            entries: vec![sample_entry()],
            listener_parse_failures: 1,
            docker: DockerEnrichment::Unavailable {
                error: "stopped".to_string(),
            },
        };
        let skipped = Snapshot {
            entries: vec![sample_entry()],
            listener_parse_failures: 0,
            docker: DockerEnrichment::Skipped,
        };

        let unavailable_text = render_auto(&unavailable, &CliOptions::default(), 120);
        let skipped_text = render_auto(&skipped, &CliOptions::default(), 120);

        assert!(unavailable_text.contains("Listeners: 1 (partial)"));
        assert!(unavailable_text.contains("Container mapping: unavailable"));
        assert!(unavailable_text.contains("Unknown"));
        assert!(skipped_text.contains("Container mapping: not checked"));
        assert!(skipped_text.contains("Not checked"));
    }

    #[test]
    fn render_auto_narrow_wraps_without_truncating_unicode_data() {
        let mut entry = sample_entry();
        entry.process = "開発サーバー-with-a-very-long-unbroken-name".to_string();
        let output = render_auto(
            &available_snapshot(vec![entry], Vec::new()),
            &CliOptions::default(),
            40,
        );

        assert!(output.contains("Port 3000/TCP"));
        assert!(output.contains("開発サーバー"));
        assert!(
            output
                .lines()
                .all(|line| UnicodeWidthStr::width(line) <= 40)
        );
    }

    #[test]
    fn render_auto_narrow_prefers_word_boundaries() {
        let snapshot = Snapshot {
            entries: vec![sample_entry()],
            listener_parse_failures: 0,
            docker: DockerEnrichment::Skipped,
        };
        let output = render_auto(&snapshot, &CliOptions::default(), 40);

        assert!(output.contains("not"));
        assert!(output.contains("checked"));
        assert!(!output.contains("ch\necked"));
    }

    #[test]
    fn render_auto_has_explicit_empty_port_state() {
        let options = CliOptions {
            port: Some(3000),
            ..CliOptions::default()
        };
        let output = render_auto(&available_snapshot(Vec::new(), Vec::new()), &options, 80);
        let all_output = render_auto(
            &available_snapshot(Vec::new(), Vec::new()),
            &CliOptions::default(),
            80,
        );
        assert!(output.contains("Port 3000 is not listening."));
        assert!(all_output.contains("No TCP listeners found."));
    }

    #[test]
    fn render_auto_partial_empty_state_does_not_claim_reliable_absence() {
        let snapshot = Snapshot {
            entries: Vec::new(),
            listener_parse_failures: 1,
            docker: DockerEnrichment::Skipped,
        };
        let options = CliOptions {
            port: Some(3000),
            ..CliOptions::default()
        };
        let output = render_auto(&snapshot, &options, 80);

        assert!(output.contains("partial"));
        assert!(output.contains("port 3000 was not found"));
        assert!(!output.contains("Port 3000 is not listening."));
    }

    #[test]
    fn render_auto_progressively_discloses_container_details() {
        let service = DockerServicePort {
            container_id: "abcdef1234567890".to_string(),
            container_name: "web".to_string(),
            container_port: 3000,
            proto: "tcp".to_string(),
        };
        let snapshot = available_snapshot(vec![sample_entry()], vec![service]);
        let concise = render_auto(&snapshot, &CliOptions::default(), 120);
        let details = render_auto(
            &snapshot,
            &CliOptions {
                details: true,
                ..CliOptions::default()
            },
            120,
        );

        assert!(concise.contains("web"));
        assert!(!concise.contains("abcdef123456"));
        assert!(details.contains("web (abcdef123456) -> 3000/tcp"));
    }

    #[test]
    fn render_auto_treats_no_match_as_unknown_when_docker_data_is_partial() {
        let snapshot = Snapshot {
            entries: vec![sample_entry()],
            listener_parse_failures: 0,
            docker: DockerEnrichment::Available {
                lookup: BTreeMap::new(),
                parse_failures: 1,
            },
        };
        let output = render_auto(&snapshot, &CliOptions::default(), 120);

        assert!(output.contains("Container mapping: partial; 0 matched"));
        assert!(output.contains("Unknown"));
        assert!(!output.contains("None"));
    }

    #[test]
    fn render_legacy_preserves_existing_table_shape() {
        let output = render_legacy(&available_snapshot(vec![sample_entry()], Vec::new()));
        assert_eq!(
            output,
            "PORT  PROTO  HOST  PROCESS        DOCKER\n3000  TCP    *     node (pid 42)  -     \n"
        );
    }

    #[test]
    fn render_json_is_versioned_and_exposes_mapping_state() {
        let output = render_json(&available_snapshot(vec![sample_entry()], Vec::new()));
        let value: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["status"]["listeners"], "complete");
        assert_eq!(value["status"]["docker"], "available");
        assert_eq!(value["listeners"][0]["container_mapping"], "none");
        assert_eq!(value["listeners"][0]["process"]["pid"], 42);
    }
}
