use super::{CliOptions, DockerEnrichment, DockerServicePort, ListenEntry, Snapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn auto(snapshot: &Snapshot, options: &CliOptions, width: usize) -> String {
    let width = width.max(40);
    if snapshot.entries.is_empty() {
        let message = match (options.port, snapshot.listener_parse_failures) {
            (Some(port), 0) => format!("Port {port} is not listening."),
            (Some(port), _) => {
                format!("Listener data is partial; port {port} was not found in parsed results.")
            }
            (None, 0) => "No TCP listeners found.".to_string(),
            (None, _) => {
                "Listener data is partial; no TCP listeners could be parsed reliably.".to_string()
            }
        };
        let mut output = String::new();
        append_wrapped_line(&mut output, "", "", &message, width);
        return output;
    }

    let matched = snapshot
        .entries
        .iter()
        .filter(|entry| services_for(snapshot, entry).is_some_and(|services| !services.is_empty()))
        .count();
    let listener_status = if snapshot.listener_parse_failures == 0 {
        format!("Listeners: {}", snapshot.entries.len())
    } else {
        format!("Listeners: {} (partial)", snapshot.entries.len())
    };
    let docker_status = match &snapshot.docker {
        DockerEnrichment::Available {
            parse_failures: 0, ..
        } => format!("Container mapping: available; {matched} matched"),
        DockerEnrichment::Available { .. } => {
            format!("Container mapping: partial; {matched} matched")
        }
        DockerEnrichment::Skipped => "Container mapping: not checked".to_string(),
        DockerEnrichment::Unavailable { .. } => "Container mapping: unavailable".to_string(),
    };

    let mut output = String::new();
    append_wrapped_line(
        &mut output,
        "",
        "",
        &format!("{listener_status} | {docker_status}"),
        width,
    );
    output.push('\n');

    let rows = snapshot
        .entries
        .iter()
        .map(|entry| {
            [
                entry.port.to_string(),
                entry.proto.clone(),
                entry.host.clone(),
                format!("{} (PID {})", entry.process, entry.pid),
                format_container_cell(snapshot, entry, options.details),
            ]
        })
        .collect::<Vec<_>>();
    let headers = ["PORT", "PROTOCOL", "BIND ADDRESS", "PROCESS", "CONTAINER"];
    let mut widths = headers.map(UnicodeWidthStr::width);
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }
    let table_width = widths.iter().sum::<usize>() + 2 * (widths.len() - 1);

    if table_width <= width {
        append_display_row(&mut output, &headers, &widths);
        for row in &rows {
            let cells = row.each_ref().map(String::as_str);
            append_display_row(&mut output, &cells, &widths);
        }
    } else {
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                output.push('\n');
            }
            append_wrapped_line(
                &mut output,
                "",
                "",
                &format!("Port {}/{}", row[0], row[1]),
                width,
            );
            append_wrapped_line(
                &mut output,
                "  Bind address: ",
                "                ",
                &row[2],
                width,
            );
            append_wrapped_line(&mut output, "  Process: ", "           ", &row[3], width);
            append_wrapped_line(
                &mut output,
                "  Container: ",
                "             ",
                &row[4],
                width,
            );
        }
    }

    output
}

fn append_display_row(output: &mut String, cells: &[&str; 5], widths: &[usize; 5]) {
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        output.push_str(cell);
        if index + 1 < cells.len() {
            let padding = widths[index].saturating_sub(UnicodeWidthStr::width(*cell));
            output.extend(std::iter::repeat_n(' ', padding));
        }
    }
    output.push('\n');
}

fn append_wrapped_line(
    output: &mut String,
    first_prefix: &str,
    continuation_prefix: &str,
    value: &str,
    width: usize,
) {
    let mut prefix = first_prefix;
    let mut remaining = value;
    loop {
        output.push_str(prefix);
        let available = width.saturating_sub(UnicodeWidthStr::width(prefix)).max(1);
        let (chunk, rest) = split_at_display_width(remaining, available);
        output.push_str(chunk);
        output.push('\n');
        if rest.is_empty() {
            break;
        }
        remaining = rest;
        prefix = continuation_prefix;
    }
}

fn split_at_display_width(value: &str, max_width: usize) -> (&str, &str) {
    let mut width = 0;
    let mut end = 0;
    for (index, character) in value.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > max_width && end > 0 {
            break;
        }
        width += character_width;
        end = index + character.len_utf8();
        if width >= max_width {
            break;
        }
    }
    if end == 0 && !value.is_empty() {
        end = value.chars().next().map_or(0, char::len_utf8);
    }
    if end < value.len()
        && let Some((space_index, whitespace)) = value[..end]
            .char_indices()
            .rev()
            .find(|(index, character)| *index > 0 && character.is_whitespace())
    {
        let rest =
            value[space_index + whitespace.len_utf8()..].trim_start_matches(char::is_whitespace);
        return (&value[..space_index], rest);
    }
    value.split_at(end)
}

fn services_for<'a>(
    snapshot: &'a Snapshot,
    entry: &ListenEntry,
) -> Option<&'a BTreeSet<DockerServicePort>> {
    let DockerEnrichment::Available { lookup, .. } = &snapshot.docker else {
        return None;
    };
    lookup.get(&(entry.port, entry.proto.to_ascii_lowercase()))
}

fn format_container_cell(snapshot: &Snapshot, entry: &ListenEntry, details: bool) -> String {
    match &snapshot.docker {
        DockerEnrichment::Skipped => "Not checked".to_string(),
        DockerEnrichment::Unavailable { .. } => "Unknown".to_string(),
        DockerEnrichment::Available { parse_failures, .. } => {
            let Some(services) = services_for(snapshot, entry) else {
                return if *parse_failures == 0 {
                    "None".to_string()
                } else {
                    "Unknown".to_string()
                };
            };
            if services.is_empty() {
                return "None".to_string();
            }
            services
                .iter()
                .map(|service| {
                    if details {
                        format!(
                            "{} ({}) -> {}/{}",
                            service.container_name,
                            short_container_id(&service.container_id),
                            service.container_port,
                            service.proto
                        )
                    } else if service.container_port == entry.port
                        && service.proto.eq_ignore_ascii_case(&entry.proto)
                    {
                        service.container_name.clone()
                    } else {
                        format!(
                            "{} -> {}/{}",
                            service.container_name, service.container_port, service.proto
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("; ")
        }
    }
}

pub(super) fn legacy(snapshot: &Snapshot) -> String {
    let empty_lookup = BTreeMap::new();
    let docker_map = match &snapshot.docker {
        DockerEnrichment::Available { lookup, .. } => lookup,
        DockerEnrichment::Skipped | DockerEnrichment::Unavailable { .. } => &empty_lookup,
    };
    let mut rows = Vec::new();
    for entry in &snapshot.entries {
        rows.push([
            entry.port.to_string(),
            entry.proto.clone(),
            entry.host.clone(),
            format!("{} (pid {})", entry.process, entry.pid),
            format_docker_services(docker_map.get(&(entry.port, entry.proto.to_ascii_lowercase()))),
        ]);
    }

    let headers = ["PORT", "PROTO", "HOST", "PROCESS", "DOCKER"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }

    let mut output = String::new();
    append_legacy_row(&mut output, &headers.map(ToString::to_string), &widths);
    for row in &rows {
        append_legacy_row(&mut output, row, &widths);
    }
    output
}

fn append_legacy_row(output: &mut String, cells: &[String; 5], widths: &[usize; 5]) {
    writeln!(
        output,
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
    )
    .expect("writing to a string cannot fail");
}

pub(super) fn json(snapshot: &Snapshot) -> String {
    let listener_status = if snapshot.listener_parse_failures == 0 {
        "complete"
    } else {
        "partial"
    };
    let docker_status = match &snapshot.docker {
        DockerEnrichment::Available {
            parse_failures: 0, ..
        } => "available",
        DockerEnrichment::Available { .. } => "partial",
        DockerEnrichment::Skipped => "skipped",
        DockerEnrichment::Unavailable { .. } => "unavailable",
    };
    let listeners = snapshot
        .entries
        .iter()
        .map(|entry| {
            let (mapping_state, services) = match &snapshot.docker {
                DockerEnrichment::Skipped => ("not_checked", Vec::new()),
                DockerEnrichment::Unavailable { .. } => ("unknown", Vec::new()),
                DockerEnrichment::Available { parse_failures, .. } => {
                    match services_for(snapshot, entry) {
                        Some(services) if !services.is_empty() => {
                            ("matched", services.iter().collect())
                        }
                        _ if *parse_failures > 0 => ("unknown", Vec::new()),
                        _ => ("none", Vec::new()),
                    }
                }
            };
            let containers = services
                .into_iter()
                .map(|service| {
                    serde_json::json!({
                        "id": service.container_id,
                        "name": service.container_name,
                        "port": service.container_port,
                        "protocol": service.proto,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "port": entry.port,
                "protocol": entry.proto,
                "bind_address": entry.host,
                "process": {
                    "name": entry.process,
                    "pid": entry.pid,
                },
                "container_mapping": mapping_state,
                "containers": containers,
            })
        })
        .collect::<Vec<_>>();
    let docker_error = match &snapshot.docker {
        DockerEnrichment::Unavailable { error } => Some(error),
        _ => None,
    };
    let value = serde_json::json!({
        "schema_version": 1,
        "status": {
            "listeners": listener_status,
            "listener_parse_failures": snapshot.listener_parse_failures,
            "docker": docker_status,
            "docker_parse_failures": match &snapshot.docker {
                DockerEnrichment::Available { parse_failures, .. } => *parse_failures,
                _ => 0,
            },
            "docker_error": docker_error,
        },
        "listeners": listeners,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("snapshot JSON serialization cannot fail")
    )
}

pub(super) fn format_docker_services(services: Option<&BTreeSet<DockerServicePort>>) -> String {
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
