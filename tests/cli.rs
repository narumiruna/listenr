#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Sandbox {
    path: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("listenr-cli-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("sandbox should be created");
        Self { path }
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.path.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script should be written");
        let mut permissions = fs::metadata(&path)
            .expect("script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("script should be executable");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_listenr"));
        command.env("PATH", &self.path);
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("CLI output should be UTF-8")
}

fn run_with_sources(lsof: &str, docker: &str, args: &[&str]) -> Output {
    let sandbox = Sandbox::new();
    sandbox.script("lsof", lsof);
    sandbox.script("docker", docker);
    sandbox
        .command()
        .args(args)
        .output()
        .expect("listenr should run")
}

#[test]
fn help_and_version_do_not_require_external_commands() {
    let sandbox = Sandbox::new();

    let help = sandbox
        .command()
        .arg("--help")
        .output()
        .expect("help should run");
    let version = sandbox
        .command()
        .arg("--version")
        .output()
        .expect("version should run");

    assert!(help.status.success());
    assert!(text(&help.stdout).contains("Usage: listenr [OPTIONS] [PORT]"));
    assert!(help.stderr.is_empty());
    assert!(version.status.success());
    assert!(text(&version.stdout).starts_with("listenr "));
}

#[test]
fn redirected_auto_output_preserves_legacy_table_and_filters_by_port() {
    let output = run_with_sources(
        "printf 'node 42 user 1u IPv4 0 0t0 TCP *:3000 (LISTEN)\\nnode 43 user 1u IPv4 0 0t0 TCP *:4000 (LISTEN)\\n'",
        "printf 'abc123\\tweb\\t0.0.0.0:3000->80/tcp\\n'",
        &["3000"],
    );

    assert!(output.status.success());
    assert_eq!(
        text(&output.stdout),
        "PORT  PROTO  HOST  PROCESS        DOCKER                \n3000  TCP    *     node (pid 42)  web (abc123) -> 80/tcp\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn json_reports_degraded_docker_without_losing_listener_data() {
    let output = run_with_sources(
        "printf 'node 42 user 1u IPv4 0 0t0 TCP *:3000 (LISTEN)\\n'",
        "printf 'daemon stopped\\n' >&2; exit 1",
        &["--format", "json"],
    );

    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(value["status"]["docker"], "unavailable");
    assert_eq!(value["listeners"][0]["container_mapping"], "unknown");
    assert!(text(&output.stderr).contains("run `docker ps` to diagnose"));
}

#[test]
fn no_docker_skips_the_optional_command() {
    let output = run_with_sources(
        "printf 'node 42 user 1u IPv4 0 0t0 TCP *:3000 (LISTEN)\\n'",
        "printf 'docker must not run\\n' >&2; exit 99",
        &["--no-docker", "--format", "json"],
    );

    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(value["status"]["docker"], "skipped");
    assert_eq!(value["listeners"][0]["container_mapping"], "not_checked");
    assert!(output.stderr.is_empty());
}

#[test]
fn fatal_listener_failure_has_actionable_stderr_and_empty_stdout() {
    let output = run_with_sources("printf 'permission denied\\n' >&2; exit 1", "exit 0", &[]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("lsof failed: permission denied"));
}

#[test]
fn invalid_arguments_fail_before_external_commands_run() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .arg("--unknown")
        .output()
        .expect("listenr should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("unknown option"));
}

#[test]
fn interrupt_before_collection_finishes_emits_no_completed_stdout() {
    let sandbox = Sandbox::new();
    sandbox.script("lsof", "/bin/sleep 10");
    sandbox.script("docker", "exit 0");

    let mut command = sandbox.command();
    command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().expect("listenr should start");
    std::thread::sleep(Duration::from_millis(150));

    let kill_status = Command::new("/bin/kill")
        .args(["-INT", "--", &format!("-{}", child.id())])
        .status()
        .expect("process group should be interruptible");
    assert!(kill_status.success());

    let output = child.wait_with_output().expect("listenr should stop");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
