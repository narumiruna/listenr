use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

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

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let listen_entries = collect_listen_entries()?;
    let docker_map = match collect_docker_bindings() {
        Ok(bindings) => build_docker_lookup(bindings),
        Err(err) => {
            eprintln!("warning: docker enrichment is unavailable: {err}");
            BTreeMap::new()
        }
    };

    print_rows(&listen_entries, &docker_map);
    Ok(())
}

fn collect_listen_entries() -> Result<Vec<ListenEntry>, String> {
    let output = run_command("lsof", &["-i", "-P", "-n"])?;
    if !output.success {
        let stderr = output.stderr.trim();
        return Err(if stderr.is_empty() {
            "lsof failed".to_string()
        } else {
            format!("lsof failed: {stderr}")
        });
    }

    let mut entries = Vec::new();
    for line in output.stdout.lines() {
        if !line.contains("(LISTEN)") {
            continue;
        }
        if let Some(entry) = parse_lsof_line(line) {
            entries.push(entry);
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
    Ok(entries)
}

fn collect_docker_bindings() -> Result<Vec<DockerPortBinding>, String> {
    let output = run_command(
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
    for line in output.stdout.lines() {
        bindings.extend(parse_docker_ps_line(line));
    }
    Ok(bindings)
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

    let process = fields[0].to_string();
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

fn parse_docker_ps_line(line: &str) -> Vec<DockerPortBinding> {
    let mut parts = line.splitn(3, '\t');
    let container_id = match parts.next() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return Vec::new(),
    };
    let container_name = match parts.next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => return Vec::new(),
    };
    let ports = parts.next().unwrap_or_default();
    if ports.is_empty() {
        return Vec::new();
    }

    let mut bindings = Vec::new();
    for raw_mapping in ports.split(", ") {
        let (host_mapping, container_mapping) = match raw_mapping.split_once("->") {
            Some(parts) => parts,
            None => continue,
        };

        let (_, host_port_text) = match host_mapping.rsplit_once(':') {
            Some(parts) => parts,
            None => continue,
        };
        let host_port = match host_port_text.parse::<u16>() {
            Ok(port) => port,
            Err(_) => continue,
        };

        let (container_port_text, proto) = match container_mapping.split_once('/') {
            Some(parts) => parts,
            None => continue,
        };
        let container_port = match container_port_text.parse::<u16>() {
            Ok(port) => port,
            Err(_) => continue,
        };

        bindings.push(DockerPortBinding {
            container_id: container_id.clone(),
            container_name: container_name.clone(),
            host_port,
            container_port,
            proto: proto.to_ascii_lowercase(),
        });
    }

    bindings
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

fn print_rows(
    entries: &[ListenEntry],
    docker_map: &BTreeMap<(u16, String), BTreeSet<DockerServicePort>>,
) {
    let mut rows = Vec::new();
    for entry in entries {
        let process = format!("{} (pid {})", entry.process, entry.pid);
        let docker =
            format_docker_services(docker_map.get(&(entry.port, entry.proto.to_ascii_lowercase())));
        rows.push([
            entry.port.to_string(),
            entry.proto.clone(),
            entry.host.clone(),
            process,
            docker,
        ]);
    }

    let headers = ["PORT", "PROTO", "HOST", "PROCESS", "DOCKER"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    print_row(&headers.map(ToString::to_string), &widths);
    for row in rows {
        print_row(&row, &widths);
    }
}

fn print_row(cells: &[String; 5], widths: &[usize; 5]) {
    println!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
        cells[0],
        cells[1],
        cells[2],
        cells[3],
        cells[4],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4]
    );
}

fn format_docker_services(services: Option<&BTreeSet<DockerServicePort>>) -> String {
    let Some(services) = services else {
        return "-".to_string();
    };
    if services.is_empty() {
        return "-".to_string();
    }

    services
        .iter()
        .map(|service| {
            let short_id = short_container_id(&service.container_id);
            format!(
                "{} ({}) -> {}/{}",
                service.container_name, short_id, service.container_port, service.proto
            )
        })
        .collect::<Vec<String>>()
        .join("; ")
}

fn short_container_id(container_id: &str) -> &str {
    const SHORT_LEN: usize = 12;
    if container_id.len() <= SHORT_LEN {
        return container_id;
    }
    &container_id[..SHORT_LEN]
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
    fn parse_docker_ports_with_ipv4_and_ipv6_mappings() {
        let line = "601f67a6c86c\ttabemap-app\t0.0.0.0:7751->7751/tcp, [::]:7751->7751/tcp";
        let bindings = parse_docker_ps_line(line);
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].host_port, 7751);
        assert_eq!(bindings[0].container_port, 7751);
        assert_eq!(bindings[0].proto, "tcp");
    }

    #[test]
    fn parse_docker_ports_ignores_internal_only_expose() {
        let line = "1b5d6f457360\timmich_redis\t6379/tcp";
        let bindings = parse_docker_ps_line(line);
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
}
