//! Run `skills-manager-cli` on another machine over SSH.
//!
//! A remote host is any POSIX machine the user can already reach with `ssh`
//! and that has Skills Manager (app or CLI) on it. The app spawns the system
//! `ssh` binary in batch mode (it never prompts), runs the CLI there with
//! `--json`, and parses what comes back. No daemon, no open port, no stored
//! credentials: the user's SSH agent, keys and `~/.ssh/config` do the work.
//!
//! Every function here blocks on a network round trip. Callers run them inside
//! `spawn_blocking`, the same way the Git backup commands do.

use semver::Version;
use serde_json::Value;
use std::process::{Command, Output};

use super::error::{AppError, ErrorKind, TargetConflictDetail};
use super::skill_store::RemoteHostRecord;

const CONNECT_TIMEOUT_SECS: u32 = 10;

/// Exit status of the resolver script when there is no CLI to run. Distinct
/// from the CLI's own `1`/`2` and from ssh's `255`.
const RESOLVE_EXIT: i32 = 3;

/// The "Resolve the CLI first" rules of the `manage-skills` skill, so the app
/// and the agents on that machine agree on which binary is authoritative: a
/// stamped copy published by the desktop app wins; an unstamped or half-copied
/// one is refused rather than worked around; otherwise whatever is on PATH.
/// The resolved binary is exec'd with the CLI arguments that follow.
const RESOLVE_AND_EXEC: &str = r#"D="$HOME/.skills-manager/bin"
B="$D/skills-manager-cli"; [ -e "$B" ] || B="$B.exe"
if [ -s "$D/.version" ] && [ -x "$B" ]; then :
elif [ -s "$D/.version" ] || [ -e "$B" ]; then echo BRIDGE_BROKEN >&2; exit 3
else B="$(command -v skills-manager-cli 2>/dev/null || true)"; [ -x "$B" ] || { echo CLI_NOT_FOUND >&2; exit 3; }
fi
exec "$B" "$@""#;

/// The version of this app, which the remote CLI must share a major with
/// before any write is sent its way.
pub fn app_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is valid semver")
}

pub fn is_compatible(remote: &Version) -> bool {
    remote.major == app_version().major
}

/// The remote CLI version, from `--version`.
pub fn version(host: &RemoteHostRecord) -> Result<Version, AppError> {
    let stdout = run_raw(host, &["--version"])?;
    parse_version_output(&stdout)
        .ok_or_else(|| AppError::internal(format!("unexpected --version output: {stdout:?}")))
}

/// Run a read-only CLI command with `--json` and return its parsed payload.
pub fn run(host: &RemoteHostRecord, args: &[&str]) -> Result<Value, AppError> {
    let mut full = Vec::with_capacity(args.len() + 1);
    full.push("--json");
    full.extend_from_slice(args);
    let stdout = run_raw(host, &full)?;
    serde_json::from_str(&stdout)
        .map_err(|e| AppError::internal(format!("remote CLI returned invalid JSON: {e}")))
}

/// Like [`run`], but refuses to proceed when the remote CLI's major version
/// differs from this app's. A write sent to a CLI that speaks a different
/// contract can do the wrong thing silently; a read at worst shows odd data.
pub fn run_write(host: &RemoteHostRecord, args: &[&str]) -> Result<Value, AppError> {
    let remote = version(host)?;
    if !is_compatible(&remote) {
        return Err(AppError::invalid_input(format!(
            "The Skills Manager CLI on {} is version {remote}, which is not compatible with this app ({}). Update Skills Manager on that host first.",
            host.name,
            app_version()
        )));
    }
    run(host, args)
}

fn run_raw(host: &RemoteHostRecord, cli_args: &[&str]) -> Result<String, AppError> {
    let output = ssh_command(host, cli_args)
        .output()
        .map_err(|e| AppError::io(format!("cannot start ssh: {e}")))?;
    interpret_output(host, output)
}

fn ssh_command(host: &RemoteHostRecord, cli_args: &[&str]) -> Command {
    let mut cmd = Command::new("ssh");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"))
        // Ends option parsing, so a target that starts with `-` cannot be
        // read as an ssh flag.
        .arg("--")
        .arg(&host.ssh_target)
        .arg(remote_command(host, cli_args));
    cmd
}

/// `skills-manager-cli CLI_ARGS` on the host, the way the live session runs
/// it: normally `serve --stdio`. `-T` keeps a terminal out of the byte
/// stream; the keepalives notice a dead link within about 45 s, since
/// requests themselves have no timeout.
pub fn cli_command(host: &RemoteHostRecord, cli_args: &[&str]) -> Command {
    let mut cmd = Command::new("ssh");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.arg("-T")
        .args(["-o", "BatchMode=yes"])
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"))
        .args(["-o", "ServerAliveInterval=15"])
        .args(["-o", "ServerAliveCountMax=3"])
        .arg("--")
        .arg(&host.ssh_target)
        .arg(remote_command(host, cli_args));
    cmd
}

/// The single string ssh hands to the remote login shell. It is wrapped in
/// `sh -c` so the resolver runs under POSIX `sh` whatever the login shell is;
/// on a non-POSIX remote `sh` itself is missing, which [`interpret_output`]
/// turns into an explicit "unsupported" error.
fn remote_command(host: &RemoteHostRecord, cli_args: &[&str]) -> String {
    // `sh -c SCRIPT NAME ARGS…` binds NAME to `$0` and ARGS to `$@`.
    let (script, program) = match host.cli_path.as_deref() {
        Some(path) => (r#"exec "$0" "$@""#, path),
        None => (RESOLVE_AND_EXEC, "sh"),
    };
    let mut parts = vec!["sh".to_string(), "-c".to_string(), shell_quote(script)];
    parts.push(shell_quote(program));
    parts.extend(cli_args.iter().map(|arg| shell_quote(arg)));
    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

fn interpret_output(host: &RemoteHostRecord, output: Output) -> Result<String, AppError> {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    Err(classify_failure(host, output.status.code(), &stderr))
}

/// Turn a failed ssh run into the error the UI should show. Ordered from the
/// transport outward: ssh itself, then the remote shell, then the resolver,
/// then the CLI's own JSON envelope.
pub(crate) fn classify_failure(
    host: &RemoteHostRecord,
    code: Option<i32>,
    stderr: &str,
) -> AppError {
    if code == Some(255) {
        return AppError::network(format!(
            "Cannot reach {} over ssh: {}",
            host.ssh_target,
            last_line(stderr)
        ));
    }
    if stderr.contains("is not recognized as an internal or external command") {
        return AppError::invalid_input(format!(
            "{} is not a POSIX host; Windows remote hosts are not supported.",
            host.name
        ));
    }
    if code == Some(RESOLVE_EXIT) {
        if stderr.contains("BRIDGE_BROKEN") {
            return AppError::not_found(format!(
                "The Skills Manager CLI on {} is incomplete. Open the Skills Manager app there once to republish it.",
                host.name
            ));
        }
        if stderr.contains("CLI_NOT_FOUND") {
            return AppError::not_found(format!(
                "Skills Manager CLI was not found on {}.",
                host.name
            ));
        }
    }
    match serde_json::from_str::<Value>(last_line(stderr)) {
        Ok(envelope) if envelope.get("ok") == Some(&Value::Bool(false)) => map_envelope(&envelope),
        _ => AppError::internal(format!(
            "Remote command failed (exit {}): {}",
            code.map_or("signal".to_string(), |c| c.to_string()),
            if stderr.is_empty() { "no output" } else { stderr }
        )),
    }
}

/// Map the CLI's `--json` failure envelope (`ok=false`, stable `code`,
/// `message`) onto the same `AppError` a local command would have produced,
/// so the frontend handles both alike. A deployment refusal keeps its paths.
fn map_envelope(envelope: &Value) -> AppError {
    let code = envelope["code"].as_str().unwrap_or("COMMAND_FAILED");
    let message = envelope["message"]
        .as_str()
        .or_else(|| envelope["error"].as_str())
        .unwrap_or("remote command failed")
        .to_string();
    match code {
        "TARGET_CONFLICT" => {
            let conflicts = envelope["details"]["conflicts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| TargetConflictDetail {
                    path: c["path"].as_str().unwrap_or_default().to_string(),
                    reason: c["reason"].as_str().unwrap_or_default().to_string(),
                })
                .collect();
            AppError::target_conflict(message, conflicts)
        }
        "INVALID_ARGUMENT" => AppError::invalid_input(message),
        _ => AppError {
            kind: match envelope["kind"].as_str() {
                Some("not_found") => ErrorKind::NotFound,
                Some("invalid_input") => ErrorKind::InvalidInput,
                Some("network") => ErrorKind::Network,
                Some("io") => ErrorKind::Io,
                Some("git") => ErrorKind::Git,
                _ => ErrorKind::Internal,
            },
            message,
            details: None,
        },
    }
}

/// `clap` prints `skills-manager-cli 1.40.0`; take the last token so a
/// differently named binary still parses.
fn parse_version_output(stdout: &str) -> Option<Version> {
    stdout
        .split_whitespace()
        .last()
        .and_then(|token| Version::parse(token).ok())
}

fn last_line(text: &str) -> &str {
    text.lines().last().unwrap_or("").trim()
}

/// Whether a session ended before its hello because the CLI there predates
/// `serve`: clap refuses an unknown subcommand with exit 2.
pub(crate) fn predates_serve(code: Option<i32>, stderr: &str) -> bool {
    code == Some(2) && stderr.contains("unrecognized subcommand 'serve'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(cli_path: Option<&str>) -> RemoteHostRecord {
        RemoteHostRecord {
            id: "h1".into(),
            name: "build box".into(),
            ssh_target: "me@build".into(),
            cli_path: cli_path.map(str::to_string),
            created_at: 0,
        }
    }

    #[test]
    fn remote_command_resolves_the_cli_unless_a_path_is_given() {
        let resolved = remote_command(&host(None), &["--json", "skills", "list"]);
        assert!(resolved.starts_with("sh -c '"));
        assert!(resolved.contains("BRIDGE_BROKEN"));
        assert!(resolved.ends_with("'sh' '--json' 'skills' 'list'"));

        let explicit = remote_command(&host(Some("/opt/sm/cli")), &["--version"]);
        assert_eq!(explicit, r#"sh -c 'exec "$0" "$@"' '/opt/sm/cli' '--version'"#);
    }

    #[test]
    fn arguments_are_single_quoted_for_the_remote_shell() {
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
        let cmd = remote_command(&host(Some("cli")), &["skills", "install", "https://x/y.git; rm -rf ~"]);
        assert!(cmd.ends_with("'https://x/y.git; rm -rf ~'"));
    }

    #[test]
    fn cli_command_keeps_the_link_alive_and_ends_options_before_the_target() {
        let cmd = cli_command(&host(Some("/opt/sm/cli")), &["serve", "--stdio"]);
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(cmd.get_program(), "ssh");
        assert_eq!(args[0], "-T");
        for option in [
            "BatchMode=yes",
            "ConnectTimeout=10",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=3",
        ] {
            assert!(args.iter().any(|a| a == option), "missing {option}");
        }
        let target = args.iter().position(|a| a == "me@build").unwrap();
        assert_eq!(args[target - 1], "--");
        assert_eq!(
            args[target + 1],
            r#"sh -c 'exec "$0" "$@"' '/opt/sm/cli' 'serve' '--stdio'"#
        );
    }

    #[test]
    fn parses_clap_version_output() {
        let v = parse_version_output("skills-manager-cli 1.40.0\n").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (1, 40, 0));
        assert!(parse_version_output("garbage").is_none());
    }

    #[test]
    fn a_deployment_refusal_keeps_its_paths() {
        let envelope: Value = serde_json::from_str(
            r#"{"ok":false,"code":"TARGET_CONFLICT","kind":"target_conflict",
                "message":"Refusing to deploy: 1 of 2 target(s) are not ours",
                "details":{"conflicts":[{"path":"/home/me/.claude/skills/db","reason":"is not a managed deployment"}]}}"#,
        )
        .unwrap();
        let err = map_envelope(&envelope);
        assert_eq!(err.kind, ErrorKind::TargetConflict);
        let details = err.details.unwrap();
        assert_eq!(details.conflicts.len(), 1);
        assert_eq!(details.conflicts[0].path, "/home/me/.claude/skills/db");
    }

    #[test]
    fn an_ordinary_failure_keeps_its_message() {
        let envelope: Value = serde_json::from_str(
            r#"{"ok":false,"code":"COMMAND_FAILED","message":"skill 'nope' not found","error":"skill 'nope' not found"}"#,
        )
        .unwrap();
        let err = map_envelope(&envelope);
        assert_eq!(err.kind, ErrorKind::Internal);
        assert_eq!(err.message, "skill 'nope' not found");

        let bad_args: Value =
            serde_json::from_str(r#"{"ok":false,"code":"INVALID_ARGUMENT","message":"missing --agent"}"#)
                .unwrap();
        assert_eq!(map_envelope(&bad_args).kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn only_a_refused_serve_marks_an_older_cli() {
        assert!(predates_serve(Some(2), "error: unrecognized subcommand 'serve'\n\nUsage: skills-manager-cli"));
        assert!(!predates_serve(Some(2), "error: unexpected argument '--stdio' found"));
        assert!(!predates_serve(Some(1), "error: unrecognized subcommand 'serve'"));
    }

    #[test]
    fn failures_are_classified_from_the_outside_in() {
        let h = host(None);
        let ssh = classify_failure(&h, Some(255), "ssh: connect to host build port 22: Connection refused");
        assert_eq!(ssh.kind, ErrorKind::Network);
        assert!(ssh.message.contains("Connection refused"));

        let windows = classify_failure(
            &h,
            Some(1),
            "'sh' is not recognized as an internal or external command,\r\noperable program or batch file.",
        );
        assert_eq!(windows.kind, ErrorKind::InvalidInput);
        assert!(windows.message.contains("not supported"));

        assert_eq!(classify_failure(&h, Some(3), "CLI_NOT_FOUND").kind, ErrorKind::NotFound);
        assert!(classify_failure(&h, Some(3), "BRIDGE_BROKEN").message.contains("republish"));

        let envelope = classify_failure(
            &h,
            Some(1),
            "warning: something\n{\"ok\":false,\"code\":\"COMMAND_FAILED\",\"message\":\"boom\"}",
        );
        assert_eq!(envelope.message, "boom");

        let opaque = classify_failure(&h, Some(1), "segfault");
        assert_eq!(opaque.kind, ErrorKind::Internal);
        assert!(opaque.message.contains("exit 1"));
    }

    /// Transport round trip against this machine. Needs `ssh localhost` to
    /// work non-interactively (key in `authorized_keys`, host key trusted);
    /// when it does not, the test reports itself as skipped rather than
    /// failing on a machine that simply has no such setup.
    #[test]
    fn localhost_round_trip() {
        let reachable = Command::new("ssh")
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "--", "localhost", "true"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !reachable {
            eprintln!("skipped: ssh localhost is not available");
            return;
        }
        let local = RemoteHostRecord {
            id: "local".into(),
            name: "localhost".into(),
            ssh_target: "localhost".into(),
            cli_path: None,
            created_at: 0,
        };
        match version(&local) {
            Ok(v) => assert!(v.major >= 1),
            // ssh works but no CLI is installed here: the resolver said so.
            Err(e) if e.kind == ErrorKind::NotFound => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
