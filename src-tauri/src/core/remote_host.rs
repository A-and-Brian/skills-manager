//! Reach `skills-manager-cli` on another machine over SSH.
//!
//! A remote host is any POSIX machine the user can already reach with `ssh`
//! and that has Skills Manager (app or CLI) on it. The app spawns the system
//! `ssh` binary in batch mode (it never prompts) and runs `serve --stdio`
//! there. No daemon, no open port, no stored credentials: the user's SSH
//! agent, keys and `~/.ssh/config` do the work.

use std::process::Command;

use super::error::AppError;
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
/// on a non-POSIX remote `sh` itself is missing, which [`classify_failure`]
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

/// Turn a session that ended before its hello into the error the UI should
/// show. Ordered from the transport outward: ssh itself, then the remote
/// shell, then the resolver, then whatever the CLI said.
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
    AppError::internal(format!(
        "Remote command failed (exit {}): {}",
        code.map_or("signal".to_string(), |c| c.to_string()),
        if stderr.is_empty() {
            "no output"
        } else {
            stderr
        }
    ))
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
    use crate::core::error::ErrorKind;

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
        assert_eq!(
            explicit,
            r#"sh -c 'exec "$0" "$@"' '/opt/sm/cli' '--version'"#
        );
    }

    #[test]
    fn arguments_are_single_quoted_for_the_remote_shell() {
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
        let cmd = remote_command(
            &host(Some("cli")),
            &["skills", "install", "https://x/y.git; rm -rf ~"],
        );
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
    fn only_a_refused_serve_marks_an_older_cli() {
        assert!(predates_serve(
            Some(2),
            "error: unrecognized subcommand 'serve'\n\nUsage: skills-manager-cli"
        ));
        assert!(!predates_serve(
            Some(2),
            "error: unexpected argument '--stdio' found"
        ));
        assert!(!predates_serve(
            Some(1),
            "error: unrecognized subcommand 'serve'"
        ));
    }

    #[test]
    fn failures_are_classified_from_the_outside_in() {
        let h = host(None);
        let ssh = classify_failure(
            &h,
            Some(255),
            "ssh: connect to host build port 22: Connection refused",
        );
        assert_eq!(ssh.kind, ErrorKind::Network);
        assert!(ssh.message.contains("Connection refused"));

        let windows = classify_failure(
            &h,
            Some(1),
            "'sh' is not recognized as an internal or external command,\r\noperable program or batch file.",
        );
        assert_eq!(windows.kind, ErrorKind::InvalidInput);
        assert!(windows.message.contains("not supported"));

        assert_eq!(
            classify_failure(&h, Some(3), "CLI_NOT_FOUND").kind,
            ErrorKind::NotFound
        );
        assert!(classify_failure(&h, Some(3), "BRIDGE_BROKEN")
            .message
            .contains("republish"));

        let opaque = classify_failure(&h, Some(1), "segfault");
        assert_eq!(opaque.kind, ErrorKind::Internal);
        assert!(opaque.message.contains("exit 1"));
    }
}
