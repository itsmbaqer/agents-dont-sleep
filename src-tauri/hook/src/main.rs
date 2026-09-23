//! `adshook <agent> <working|waiting|idle|end> [session-id] [cwd] [--event=Name] [--tool=Name]`
//!
//! Called by coding agents' lifecycle hooks. Keeps one small JSON record per session in
//! `~/.agents-dont-sleep/sessions/<agent>__<sid>` that the menu-bar app polls: state, folder,
//! agent pid, and activity metadata (event and tool *names*, model, counters, error *kind*).
//! Prompts, commands, file contents and tool output are never read out of the payload.
//! Never blocks, never fails the agent.
use serde::{Deserialize, Serialize};
use std::io::Read;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// The on-disk session record. Mirrored by `Record` in `src-tauri/src/agents.rs`.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
struct Record {
    state: String,
    cwd: String,
    pid: u32,
    event: String,
    tool: String,
    model: String,
    term: String,
    started: u64,
    turn_started: u64,
    last_turn_secs: u64,
    waiting_since: u64,
    last_event: u64,
    tools: u32,
    turns: u32,
    errors: u32,
    error_kind: String,
}

/// What one hook call tells us.
#[derive(Default)]
struct Event {
    state: String,
    event: String,
    tool: String,
    model: String,
    error_kind: String,
    cwd: String,
    term: String,
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let flag =
        |name: &str| raw.iter().find_map(|a| a.strip_prefix(&format!("--{name}="))).unwrap_or_default().to_string();
    let args: Vec<&String> = raw.iter().filter(|a| !a.starts_with("--")).collect();
    let arg = |i: usize| args.get(i).map(|s| s.to_string()).unwrap_or_default();
    let (agent, state, mut sid) = (arg(0), arg(1), arg(2));

    let mut ev = Event {
        state,
        event: name_only(&flag("event"), 40),
        tool: name_only(&flag("tool"), 80),
        cwd: arg(3),
        ..Default::default()
    };
    // The terminal app: its bundle id on macOS (tells Cursor from VS Code), else TERM_PROGRAM.
    let term = std::env::var("__CFBundleIdentifier").or_else(|_| std::env::var("TERM_PROGRAM"));
    ev.term = name_only(&term.unwrap_or_default(), 60);
    if sid.is_empty() {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let p = parse_payload(&input);
        sid = p.sid;
        if ev.cwd.is_empty() {
            ev.cwd = p.cwd;
        }
        if ev.event.is_empty() {
            ev.event = p.event;
        }
        if ev.tool.is_empty() {
            ev.tool = p.tool;
        }
        ev.model = p.model;
        ev.error_kind = p.error_kind;
    }
    let pid = agent_pid();
    if sid.is_empty() {
        sid = format!("pid-{pid}");
    }
    if !agent.is_empty() {
        let _ = record(&sanitize(&agent), &sanitize(&sid), &ev, pid);
    }
    // Agents that parse stdout get an explicit no-op.
    match agent.as_str() {
        "copilot" | "gemini" => print!("{{}}"),
        "cursor" => print!("{{\"continue\":true}}"),
        _ => {}
    }
}

#[derive(Default, Debug, PartialEq)]
struct Payload {
    sid: String,
    cwd: String,
    event: String,
    tool: String,
    model: String,
    error_kind: String,
}

/// Metadata from an agent's JSON hook payload. Only names and ids are kept, never free text.
fn parse_payload(input: &str) -> Payload {
    let v: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let get = |keys: &[&str]| {
        keys.iter().filter_map(|k| v.get(*k)?.as_str()).find(|s| !s.is_empty()).unwrap_or_default().to_string()
    };
    let event = get(&["hook_event_name", "hookEventName"]);
    // Claude's StopFailure `error` is an enum-like kind (rate_limit, overloaded, …); other
    // events' `error` fields carry tool output, which we don't keep.
    let error_kind = if event == "StopFailure" { name_only(&get(&["error"]), 40) } else { String::new() };
    Payload {
        sid: get(&["session_id", "sessionId", "conversation_id"]),
        cwd: get(&["cwd"]),
        tool: name_only(&get(&["tool_name", "toolName"]), 80),
        model: name_only(&get(&["model"]), 60),
        event: name_only(&event, 40),
        error_kind,
    }
}

/// A short identifier (tool, model, error kind) or nothing: no spaces, newlines or long text.
fn name_only(s: &str, max: usize) -> String {
    let ok = !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-:/@[]".contains(c));
    if ok {
        s.to_string()
    } else {
        String::new()
    }
}

fn is_tool_start(event: &str) -> bool {
    matches!(
        event,
        "PreToolUse"
            | "preToolUse"
            | "BeforeTool"
            | "pre_tool_call"
            | "tool"
            | "tool_execution_start"
            | "afterShellExecution"
            | "afterFileEdit"
            | "afterMCPExecution"
    )
}

/// Tool name for events that don't carry one in the payload (Cursor).
fn implied_tool(event: &str) -> &'static str {
    match event {
        "afterShellExecution" => "Shell",
        "afterFileEdit" => "Edit",
        "afterMCPExecution" => "MCP",
        _ => "",
    }
}

fn active(state: &str) -> bool {
    state == "working" || state == "waiting"
}

/// Next record from the previous one and this hook call. Pure, so the counting rules are tested.
/// ponytail: parallel tool hooks can lose a counter increment; counts are indicative, not billing.
fn update(prev: Option<Record>, ev: &Event, pid: u32, now: u64) -> Record {
    let was = prev.unwrap_or_default();
    let mut r = was.clone();
    r.state = if ev.state.is_empty() { "idle".into() } else { ev.state.clone() };
    r.pid = pid;
    r.last_event = now;
    r.event = ev.event.clone();
    if r.started == 0 {
        r.started = now;
    }
    for (field, new) in [(&mut r.cwd, &ev.cwd), (&mut r.model, &ev.model), (&mut r.term, &ev.term)] {
        if !new.is_empty() {
            *field = new.clone();
        }
    }
    if active(&r.state) && !active(&was.state) {
        r.turn_started = now;
        r.error_kind.clear();
    } else if active(&r.state) && r.turn_started == 0 {
        r.turn_started = now; // mid-turn record from an older helper: start the clock now
    }
    r.waiting_since = match (r.state.as_str(), was.state.as_str()) {
        ("waiting", "waiting") => was.waiting_since,
        ("waiting", _) => now,
        _ => 0,
    };
    if is_tool_start(&ev.event) {
        r.tools += 1;
        let tool = if ev.tool.is_empty() { implied_tool(&ev.event) } else { &ev.tool };
        if !tool.is_empty() {
            r.tool = tool.to_string();
        }
    }
    if ev.event.ends_with("Failure") || ev.event == "errorOccurred" {
        r.errors += 1;
    }
    if !ev.error_kind.is_empty() {
        r.error_kind = ev.error_kind.clone();
    }
    if r.state == "idle" {
        if active(&was.state) {
            r.turns += 1;
            r.last_turn_secs = now.saturating_sub(was.turn_started);
        }
        r.turn_started = 0;
        r.tool.clear();
    }
    r
}

/// Reads the previous record, including the 3-line format (state, cwd, pid) of older helpers.
fn read_record(bytes: &str) -> Option<Record> {
    serde_json::from_str(bytes).ok().or_else(|| {
        let mut lines = bytes.lines();
        let state = lines.next()?.trim().to_string();
        let cwd = lines.next().unwrap_or_default().to_string();
        let pid = lines.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
        Some(Record { state, cwd, pid, ..Default::default() })
    })
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

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn record(agent: &str, sid: &str, ev: &Event, pid: u32) -> std::io::Result<()> {
    let dir = std::env::home_dir().unwrap_or_default().join(".agents-dont-sleep").join("sessions");
    let file = dir.join(format!("{agent}__{sid}"));
    if ev.state == "end" {
        return std::fs::remove_file(file);
    }
    std::fs::create_dir_all(&dir)?;
    let prev = std::fs::read_to_string(&file).ok().and_then(|s| read_record(&s));
    let next = update(prev, ev, pid, now());
    let tmp = dir.join(format!(".tmp.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(&next).unwrap_or_default())?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(state: &str, event: &str) -> Event {
        Event { state: state.into(), event: event.into(), ..Default::default() }
    }

    #[test]
    fn payloads_keep_names_only() {
        let claude = r#"{"session_id":"abc-123","cwd":"/Users/me/My Proj","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rm -rf x"}}"#;
        let p = parse_payload(claude);
        assert_eq!(
            (p.sid.as_str(), p.cwd.as_str(), p.event.as_str(), p.tool.as_str()),
            ("abc-123", "/Users/me/My Proj", "PreToolUse", "Bash")
        );
        let copilot = r#"{"sessionId":"cop/1","timestamp":1,"cwd":"C:\\work","toolName":"shell"}"#;
        assert_eq!(parse_payload(copilot).tool, "shell");
        let cursor = r#"{"conversation_id":"c9","workspace_roots":["/w"]}"#;
        assert_eq!(parse_payload(cursor).sid, "c9");
        let stop = r#"{"session_id":"s","hook_event_name":"StopFailure","error":"rate_limit"}"#;
        assert_eq!(parse_payload(stop).error_kind, "rate_limit");
        // Tool errors carry output text: never kept.
        let fail = r#"{"session_id":"s","hook_event_name":"PostToolUseFailure","error":"Exit code 1\nsecret"}"#;
        assert_eq!(parse_payload(fail).error_kind, "");
        let model = r#"{"session_id":"s","hook_event_name":"SessionStart","model":"claude-opus-5-5"}"#;
        assert_eq!(parse_payload(model).model, "claude-opus-5-5");
        assert_eq!(parse_payload("not json"), Payload::default());
    }

    #[test]
    fn names_reject_free_text() {
        assert_eq!(name_only("mcp__github__create_issue", 80), "mcp__github__create_issue");
        assert_eq!(name_only("two words", 80), "");
        assert_eq!(name_only("line\nbreak", 80), "");
        assert_eq!(name_only(&"x".repeat(81), 80), "");
    }

    #[test]
    fn a_turn_is_counted() {
        let r = update(None, &ev("idle", "SessionStart"), 7, 100);
        assert_eq!((r.state.as_str(), r.started, r.turns), ("idle", 100, 0));
        let r = update(Some(r), &ev("working", "UserPromptSubmit"), 7, 110);
        assert_eq!(r.turn_started, 110);
        let tool = Event { tool: "Bash".into(), ..ev("working", "PreToolUse") };
        let r = update(Some(r), &tool, 7, 120);
        assert_eq!((r.tools, r.tool.as_str()), (1, "Bash"));
        let r = update(Some(r), &ev("working", "PostToolUse"), 7, 150);
        assert_eq!((r.tools, r.tool.as_str()), (1, "Bash"));
        let r = update(Some(r), &ev("waiting", "PermissionRequest"), 7, 160);
        assert_eq!(r.waiting_since, 160);
        let r = update(Some(r), &ev("waiting", "Notification"), 7, 170);
        assert_eq!((r.waiting_since, r.turn_started), (160, 110)); // still the same wait and turn
        let r = update(Some(r), &ev("idle", "Stop"), 7, 200);
        assert_eq!((r.turns, r.last_turn_secs, r.turn_started, r.waiting_since), (1, 90, 0, 0));
        assert!(r.tool.is_empty());
        assert_eq!((r.started, r.last_event), (100, 200));
    }

    #[test]
    fn errors_and_kinds() {
        let r = update(None, &ev("working", "UserPromptSubmit"), 1, 10);
        let r = update(Some(r), &ev("working", "PostToolUseFailure"), 1, 11);
        assert_eq!(r.errors, 1);
        let stop = Event { error_kind: "overloaded".into(), ..ev("idle", "StopFailure") };
        let r = update(Some(r), &stop, 1, 12);
        assert_eq!((r.errors, r.error_kind.as_str(), r.turns), (2, "overloaded", 1));
        // The next turn starts clean.
        let r = update(Some(r), &ev("working", "UserPromptSubmit"), 1, 20);
        assert_eq!(r.error_kind, "");
    }

    #[test]
    fn upgraded_mid_turn_gets_a_clock() {
        let legacy = read_record("working\n/w\n5\n");
        let r = update(legacy, &ev("working", "PreToolUse"), 5, 300);
        assert_eq!(r.turn_started, 300);
    }

    #[test]
    fn cursor_tools_are_implied() {
        let r = update(None, &ev("working", "afterShellExecution"), 1, 1);
        assert_eq!((r.tools, r.tool.as_str()), (1, "Shell"));
    }

    #[test]
    fn legacy_three_line_records() {
        let r = read_record("working\n/Users/me/proj\n4242\n").unwrap();
        assert_eq!((r.state.as_str(), r.cwd.as_str(), r.pid), ("working", "/Users/me/proj", 4242));
        let r = read_record(&serde_json::to_string(&Record { tools: 3, ..Default::default() }).unwrap()).unwrap();
        assert_eq!(r.tools, 3);
    }

    #[test]
    fn sanitizes_ids() {
        assert_eq!(sanitize("cop/1"), "cop_1");
        assert_eq!(sanitize("../../etc"), ".._.._etc");
        assert_eq!(sanitize("ses_f34-A.b"), "ses_f34-A.b");
    }
}
