//! Data Saver (macOS), for hotspots: while on, only DNS, the local network and the hosts in
//! `~/.agents-dont-sleep/datasaver-hosts.txt` are reachable, and macOS / App Store automatic
//! updates are off. macOS has no per-process firewall without a signed Network Extension, so
//! this allows by destination: anything talking to an allowed host gets through too.
//!
//! The work happens in `datasaver.sh`, installed root-owned; a sudoers rule lets the user run
//! exactly `<helper> on` and `<helper> off`. The host list goes in on stdin, never as a path.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use crate::settings::data_dir;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const SUPPORTED: bool = cfg!(target_os = "macos");

const SCRIPT: &str = include_str!("datasaver.sh");
const HELPER: &str = "/Library/PrivilegedHelperTools/agents-dont-sleep-datasaver";
const SUDOERS: &str = "/etc/sudoers.d/agents-dont-sleep-datasaver";

const DEFAULT_HOSTS: &str = "\
# Agents Don't Sleep: Data Saver allowlist.
# While Data Saver is on, only these hosts (plus DNS and your local network) are reachable.
# One host name, IP or CIDR per line. Changes apply within 5 minutes, or toggle Data Saver.
# Anything else talking to these hosts gets through too, so keep the list short.

# Claude Code
api.anthropic.com
claude.ai
console.anthropic.com
platform.claude.com
statsig.anthropic.com
mcp-proxy.anthropic.com

# Codex
api.openai.com
chatgpt.com
auth.openai.com
ab.chatgpt.com

# git and package installs during a task (delete to save more)
github.com
api.github.com
codeload.github.com
registry.npmjs.org
pypi.org
files.pythonhosted.org

# Off by default: these share addresses with GitHub release downloads, which is how many
# apps fetch their updates.
# raw.githubusercontent.com
# objects.githubusercontent.com

# Add your VPN gateway here if you use one, or it disconnects while Data Saver is on.
";

pub fn hosts_path() -> PathBuf {
    data_dir().join("datasaver-hosts.txt")
}

fn hosts() -> String {
    let p = hosts_path();
    if !p.exists() {
        let _ = crate::settings::write_atomic(&p, DEFAULT_HOSTS.as_bytes());
    }
    std::fs::read_to_string(p).unwrap_or_else(|_| DEFAULT_HOSTS.into())
}

/// The sudoers rule is in place and the installed helper is this build's (an update that
/// changes the script asks for the permission again).
pub fn has_grant() -> bool {
    SUPPORTED
        && std::fs::read_to_string(HELPER).is_ok_and(|s| s == SCRIPT)
        && crate::power::quiet("/usr/bin/sudo", &["-n", "-l", HELPER, "on"])
}

/// Switches Data Saver on (also refreshes the allowed addresses) or off.
pub fn apply(on: bool) -> Result<(), String> {
    let mut child = Command::new("/usr/bin/sudo")
        .args(["-n", HELPER, if on { "on" } else { "off" }])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if on {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(hosts().as_bytes());
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { "the Data Saver helper failed".into() } else { err })
}

/// Undo whatever a crash, reboot or older build left on. Any installed helper version will do.
pub fn restore() {
    if SUPPORTED && crate::power::quiet("/usr/bin/sudo", &["-n", "-l", HELPER, "off"]) {
        let _ = apply(false);
    }
}

/// Opens the allowlist in TextEdit.
pub fn edit_hosts() {
    let _ = hosts();
    let _ =
        Command::new("/usr/bin/open").arg("-t").arg(hosts_path()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// One admin prompt: installs the helper root-owned and a sudoers rule for exactly `on`/`off`.
#[cfg(target_os = "macos")]
pub fn install_grant() -> Result<(), String> {
    let user = crate::power::sudo_user()?;
    let dir = data_dir();
    let (script, rule) = (dir.join("datasaver.tmp"), dir.join("sudoers-datasaver.tmp"));
    let (script_s, rule_s) = (script.to_string_lossy().into_owned(), rule.to_string_lossy().into_owned());
    if script_s.contains('\'') {
        return Err("home path contains a quote".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    std::fs::write(
        &rule,
        format!(
            "# Agents Don't Sleep: lets {user} switch Data Saver on and off, nothing else.\n\
             {user} ALL=(root) NOPASSWD: {HELPER} on, {HELPER} off\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    // mkdir -p, not install -d: the folder usually exists and its mode must stay as macOS set it.
    let res = crate::power::admin_shell(
        &format!(
            "/usr/sbin/visudo -cf '{rule_s}' && /bin/mkdir -p /Library/PrivilegedHelperTools && \
             /usr/bin/install -m 0755 -o root -g wheel '{script_s}' {HELPER} && \
             /usr/bin/install -m 0440 -o root -g wheel '{rule_s}' {SUDOERS}"
        ),
        "Agents Don't Sleep needs permission to switch Data Saver on and off.",
    );
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&rule);
    res
}

#[cfg(not(target_os = "macos"))]
pub fn install_grant() -> Result<(), String> {
    Err("Data Saver is macOS only".into())
}

/// Switches it off as root first (works even if the sudoers rule is gone), then removes it all.
#[cfg(target_os = "macos")]
pub fn uninstall_grant() -> Result<(), String> {
    crate::power::admin_shell(
        &format!("[ -x {HELPER} ] && {HELPER} off; /bin/rm -f {HELPER} {HELPER}.state {SUDOERS}"),
        "Remove the Data Saver permission for Agents Don't Sleep.",
    )
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall_grant() -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn run(args: &[&str], input: &str) -> std::process::Output {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/src/datasaver.sh");
        let mut child = Command::new("/bin/sh")
            .arg(script)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    #[test]
    fn helper_only_passes_host_like_tokens() {
        let out = run(&["filter"], "api.anthropic.com # c\n-rf\n;rm -rf /\n10.0.0.0/8 github.com\r\n\n$(id)\n");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "api.anthropic.com\n10.0.0.0/8\ngithub.com\n");
        // The shipped defaults survive the filter intact.
        let defaults = String::from_utf8(run(&["filter"], DEFAULT_HOSTS).stdout).unwrap();
        assert!(defaults.lines().any(|l| l == "api.anthropic.com"));
        assert!(!defaults.lines().any(|l| l.contains("githubusercontent")));
    }

    #[test]
    fn helper_parses_and_refuses_unknown_commands() {
        assert!(Command::new("/bin/sh")
            .args(["-n", concat!(env!("CARGO_MANIFEST_DIR"), "/src/datasaver.sh")])
            .status()
            .unwrap()
            .success());
        assert_eq!(run(&["rm"], "").status.code(), Some(2));
    }
}
