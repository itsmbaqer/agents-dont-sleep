//! `adshook <agent> <working|waiting|idle|end> [session-id] [cwd]`
//!
//! Called by coding agents' lifecycle hooks. Records the agent's state as a small file in
//! `~/.agents-dont-sleep/sessions/` (lines: state, cwd, agent pid) that the menu-bar app polls.
//! Only the session id and folder are read from the hook payload. Never blocks, never fails.
use std::io::Read;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let (agent, state, mut sid, mut cwd) = (arg(0), arg(1), arg(2), arg(3));
    if sid.is_empty() {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let (s, c) = parse_payload(&input);
        sid = s;
        if cwd.is_empty() {
            cwd = c;
        }
    }
    let pid = agent_pid();
    if sid.is_empty() {
        sid = format!("pid-{pid}");
    }
    if !agent.is_empty() {
        let _ = record(&sanitize(&agent), &state, &sanitize(&sid), &cwd, pid);
    }
    // Agents that parse stdout get an explicit no-op.
    match agent.as_str() {
        "copilot" | "gemini" => print!("{{}}"),
        "cursor" => print!("{{\"continue\":true}}"),
        _ => {}
    }
}

/// (session id, cwd) from an agent's JSON hook payload.
fn parse_payload(input: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let sid = ["session_id", "sessionId", "conversation_id"]
        .iter()
        .map(|k| get(k))
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    (sid, get("cwd"))
}

fn sanitize(sid: &str) -> String {
    sid.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect()
}

/// The agent's pid: our parent, or its parent when the agent ran us through a shell.
fn agent_pid() -> u32 {
    let mut sys = System::new();
    let parent_of = |sys: &mut System, pid: Pid| {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, ProcessRefreshKind::nothing());
        sys.process(pid).and_then(|p| p.parent())
    };
    let Ok(me) = sysinfo::get_current_pid() else { return 0 };
    let Some(parent) = parent_of(&mut sys, me) else { return 0 };
    let name = sys.process(parent).map(|p| p.name().to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = name.trim_end_matches(".exe");
    let shell = matches!(name, "sh" | "bash" | "zsh" | "dash" | "fish" | "cmd" | "pwsh" | "powershell")
        || name.ends_with("/sh");
    let pid = if shell { parent_of(&mut sys, parent).unwrap_or(parent) } else { parent };
    pid.as_u32()
}

fn record(agent: &str, state: &str, sid: &str, cwd: &str, pid: u32) -> std::io::Result<()> {
    let dir = std::env::home_dir().unwrap_or_default().join(".agents-dont-sleep").join("sessions");
    let file = dir.join(format!("{agent}__{sid}"));
    if state == "end" {
        return std::fs::remove_file(file);
    }
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".tmp.{}", std::process::id()));
    std::fs::write(&tmp, format!("{state}\n{cwd}\n{pid}\n"))?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads() {
        let claude = r#"{"session_id":"abc-123","cwd":"/Users/me/My Proj","hook_event_name":"PreToolUse","tool_input":{"x":"\"q"}}"#;
        assert_eq!(parse_payload(claude), ("abc-123".into(), "/Users/me/My Proj".into()));
        let copilot = r#"{"sessionId":"cop/1","timestamp":1,"cwd":"C:\\work"}"#;
        assert_eq!(parse_payload(copilot), ("cop/1".into(), "C:\\work".into()));
        let cursor = r#"{"conversation_id":"c9","workspace_roots":["/w"]}"#;
        assert_eq!(parse_payload(cursor), ("c9".into(), String::new()));
        assert_eq!(parse_payload("not json"), (String::new(), String::new()));
        assert_eq!(parse_payload(""), (String::new(), String::new()));
    }

    #[test]
    fn sanitizes_ids() {
        assert_eq!(sanitize("cop/1"), "cop_1");
        assert_eq!(sanitize("../../etc"), ".._.._etc");
        assert_eq!(sanitize("ses_f34-A.b"), "ses_f34-A.b");
    }
}
