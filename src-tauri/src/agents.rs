//! Agent integrations. Every hook/plugin calls `adshook <agent> <state> [sid] [cwd]` (the
//! `src-tauri/hook` sidecar), which drops `sessions/<agent>__<sid>` (lines: state, cwd, agent
//! pid). The app only polls that folder.
use crate::settings::{data_dir, home, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Every entry we write names the helper, so uninstall only ever touches our own lines. The
/// name is 8.3-safe so it survives Windows short paths; `hook.sh` covers 0.1 pre-release installs.
const MARKERS: [&str; 2] = ["adshook", ".agents-dont-sleep/hook.sh"];

pub fn ours(s: &str) -> bool {
    let s = s.to_lowercase();
    MARKERS.iter().any(|m| s.contains(m))
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    /// Claude Code: nested, exec form (`command` + `args`, no shell, no quoting).
    Claude,
    /// Codex: `hooks.<Event>: [{hooks: [{type, command}]}]`
    Nested,
    /// Gemini CLI: same plus `matcher: "*"`, timeout in ms.
    Gemini,
    /// Cursor: `{version: 1, hooks: {<event>: [{command}]}}`
    Cursor,
    /// Copilot CLI drop-in file: `{version: 1, hooks: {<event>: [{type, bash, timeoutSec}]}}`
    Copilot,
}

enum Kind {
    Json { file: &'static str, shape: Shape, events: &'static [(&'static str, &'static str)] },
    Owned { file: &'static str, template: &'static str },
    Hermes,
}

pub struct Agent {
    pub id: &'static str,
    pub name: &'static str,
    /// Presence of `~/<dir>` means the agent is installed on this Mac.
    dir: &'static str,
    /// Process basenames (lowercase). A session is dropped once none of these run.
    pub procs: &'static [&'static str],
    kind: Kind,
    pub note: &'static str,
}

const HERMES_EVENTS: &[(&str, &str)] = &[
    ("on_session_start", "idle"),
    ("pre_llm_call", "working"),
    ("pre_tool_call", "working"),
    ("post_tool_call", "working"),
    ("on_session_end", "idle"), // fires at every turn end, despite the name
    ("on_session_reset", "end"),
    ("on_session_finalize", "end"),
];

pub const AGENTS: &[Agent] = &[
    Agent {
        id: "claude",
        name: "Claude Code",
        dir: ".claude",
        procs: &["claude"],
        kind: Kind::Json {
            file: ".claude/settings.json",
            shape: Shape::Claude,
            events: &[
                ("SessionStart", "idle"),
                ("UserPromptSubmit", "working"),
                ("PreToolUse", "working"),
                ("PostToolUse", "working"),
                ("PostToolUseFailure", "working"),
                ("SubagentStart", "working"),
                ("SubagentStop", "working"),
                ("PermissionRequest", "waiting"),
                ("Stop", "idle"),
                ("StopFailure", "idle"),
                ("SessionEnd", "end"),
            ],
        },
        note: "",
    },
    Agent {
        id: "codex",
        name: "Codex",
        dir: ".codex",
        procs: &["codex"],
        kind: Kind::Json {
            file: ".codex/hooks.json",
            shape: Shape::Nested,
            events: &[
                ("SessionStart", "idle"),
                ("UserPromptSubmit", "working"),
                ("PreToolUse", "working"),
                ("PostToolUse", "working"),
                ("PermissionRequest", "waiting"),
                ("Stop", "idle"),
            ],
        },
        note: "Codex asks you to trust new hooks: run /hooks in Codex once after installing.",
    },
    Agent {
        id: "gemini",
        name: "Gemini CLI",
        dir: ".gemini",
        procs: &["gemini"],
        kind: Kind::Json {
            file: ".gemini/settings.json",
            shape: Shape::Gemini,
            events: &[
                ("SessionStart", "idle"),
                ("BeforeAgent", "working"),
                ("BeforeTool", "working"),
                ("AfterTool", "working"),
                ("Notification", "waiting"),
                ("AfterAgent", "idle"),
                ("SessionEnd", "end"),
            ],
        },
        note: "",
    },
    Agent {
        id: "cursor",
        name: "Cursor",
        dir: ".cursor",
        procs: &["cursor"],
        // Only observe-type events: no before-shell/MCP gates, so we never answer a permission.
        kind: Kind::Json {
            file: ".cursor/hooks.json",
            shape: Shape::Cursor,
            events: &[
                ("beforeSubmitPrompt", "working"),
                ("afterAgentThought", "working"),
                ("afterShellExecution", "working"),
                ("afterFileEdit", "working"),
                ("afterMCPExecution", "working"),
                ("afterAgentResponse", "idle"),
                ("stop", "idle"),
            ],
        },
        note: "",
    },
    Agent {
        id: "copilot",
        name: "Copilot CLI",
        dir: ".copilot",
        procs: &["copilot"],
        kind: Kind::Json {
            file: ".copilot/hooks/agents-dont-sleep.json",
            shape: Shape::Copilot,
            events: &[
                ("sessionStart", "idle"),
                ("userPromptSubmitted", "working"),
                ("preToolUse", "working"),
                ("postToolUse", "working"),
                ("permissionRequest", "waiting"),
                ("agentStop", "idle"),
                ("sessionEnd", "end"),
            ],
        },
        note: "",
    },
    Agent {
        id: "opencode",
        name: "OpenCode",
        dir: ".config/opencode",
        procs: &["opencode"],
        kind: Kind::Owned {
            file: ".config/opencode/plugins/agents-dont-sleep.js",
            template: include_str!("../resources/opencode-plugin.js"),
        },
        // ponytail: 1.x plugin API only; add a Plugin.define module when 2.x is in use.
        note: "Plugin for OpenCode 1.x. Restart OpenCode after installing.",
    },
    Agent {
        id: "pi",
        name: "Pi",
        dir: ".pi",
        procs: &["pi"],
        kind: Kind::Owned {
            file: ".pi/agent/extensions/agents-dont-sleep.ts",
            template: include_str!("../resources/pi-extension.ts"),
        },
        note: "Restart pi after installing.",
    },
    Agent {
        id: "hermes",
        name: "Hermes",
        dir: ".hermes",
        procs: &["hermes"],
        kind: Kind::Hermes,
        note: "Adds shell hooks to ~/.hermes/config.yaml, pre-approves them, and installs a gateway hook.",
    },
];

pub fn hook_path() -> PathBuf {
    data_dir().join("bin").join(if cfg!(windows) { "adshook.exe" } else { "adshook" })
}

/// Copies the bundled `adshook` sidecar (installed next to our executable) to a stable path
/// that survives app moves and uninstalls, on every launch so updates ship with the app. Written
/// as fresh bytes, so macOS quarantine flags aren't copied along. A copy that fails because a
/// hook is running right now is retried next launch.
pub fn install_hook_binary() -> std::io::Result<()> {
    fs::create_dir_all(data_dir().join("sessions"))?;
    let dest = hook_path();
    let src = std::env::current_exe()?.with_file_name(dest.file_name().unwrap_or_default());
    let bytes = fs::read(src)?;
    if fs::read(&dest).is_ok_and(|cur| cur == bytes) {
        return Ok(());
    }
    write_atomic(&dest, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// POSIX-shell quoting, only when needed (sh and Hermes' shlex both accept it).
#[cfg_attr(windows, allow(dead_code))]
pub fn quote_posix(p: &str) -> String {
    if !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+~".contains(c)) {
        p.to_string()
    } else {
        format!("'{}'", p.replace('\'', r"'\''"))
    }
}

/// The helper as a shell word. Windows: the 8.3 short path, because cmd.exe (Codex) breaks on
/// a quoted program path and PowerShell (Gemini) needs `&` for one.
/// ponytail: if 8.3 names are disabled and the profile path has spaces, shell-string hooks
/// (Codex/Gemini/Cursor) break on Windows; Claude/Copilot/plugins are unaffected.
fn program() -> String {
    #[cfg(windows)]
    return crate::power::short_path(&hook_path());
    #[cfg(not(windows))]
    return quote_posix(&hook_path().to_string_lossy());
}

/// Bump when generated hook entries change; connected agents are rewritten at startup.
pub const HOOKS_VERSION: u32 = 2;

/// `--event=` is baked in because Copilot and Cursor payloads don't name the event.
fn hook_cmd(agent: &str, state: &str, event: &str) -> String {
    format!("{} {agent} {state} --event={event}", program())
}

fn entry(shape: Shape, agent: &str, state: &str, event: &str) -> Value {
    let cmd = hook_cmd(agent, state, event);
    match shape {
        Shape::Claude => json!({ "hooks": [{
            "type": "command", "command": hook_path().to_string_lossy(),
            "args": [agent, state, format!("--event={event}")], "timeout": 5 }] }),
        Shape::Nested => json!({ "hooks": [{ "type": "command", "command": cmd, "timeout": 5 }] }),
        Shape::Gemini => json!({ "matcher": "*", "hooks": [{
            "name": "agents-dont-sleep", "type": "command", "command": cmd, "timeout": 5000 }] }),
        Shape::Cursor => json!({ "command": cmd }),
        Shape::Copilot if cfg!(windows) => json!({ "type": "command",
            "powershell": format!("& '{}' {agent} {state} --event={event}", hook_path().display()), "timeoutSec": 5 }),
        Shape::Copilot => json!({ "type": "command", "bash": cmd, "timeoutSec": 5 }),
    }
}

pub fn json_install(v: &mut Value, agent: &str, shape: Shape, events: &[(&str, &str)]) -> Result<(), String> {
    json_uninstall(v);
    if v.is_null() {
        *v = json!({});
    }
    let obj = v.as_object_mut().ok_or("config is not a JSON object")?;
    if matches!(shape, Shape::Cursor | Shape::Copilot) {
        obj.entry("version").or_insert(json!(1));
    }
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object; not touching it")?;
    for (event, state) in events {
        let list = hooks.entry(*event).or_insert_with(|| json!([]));
        list.as_array_mut()
            .ok_or_else(|| format!("hooks.{event} is not a list; not touching it"))?
            .push(entry(shape, agent, state, event));
    }
    Ok(())
}

/// Removes only our entries. Keys we emptied are dropped so an install+uninstall round-trip
/// leaves the file as it was.
pub fn json_uninstall(v: &mut Value) {
    let Some(hooks) = v.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    let mut emptied = vec![];
    for (event, list) in hooks.iter_mut() {
        if let Some(a) = list.as_array_mut() {
            let before = a.len();
            a.retain(|x| !ours(&x.to_string()));
            if a.len() != before && a.is_empty() {
                emptied.push(event.clone());
            }
        }
    }
    for e in &emptied {
        hooks.shift_remove(e);
    }
    if !emptied.is_empty() && hooks.is_empty() {
        v.as_object_mut().map(|o| o.shift_remove("hooks"));
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Err(_) => Ok(json!({})),
        Ok(s) if s.trim().is_empty() => Ok(json!({})),
        Ok(s) => {
            serde_json::from_str(&s).map_err(|e| format!("{} isn't plain JSON ({e}); not touching it", path.display()))
        }
    }
}

/// Backup, then atomic replace — a failed write can't truncate someone's config.
fn write_config(path: &Path, content: &str) -> Result<(), String> {
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        fs::copy(path, &bak).map_err(|e| e.to_string())?;
    }
    write_atomic(path, content.as_bytes()).map_err(|e| e.to_string())
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    write_config(path, &(serde_json::to_string_pretty(v).map_err(|e| e.to_string())? + "\n"))
}

/// Codex needs `[features] hooks = true`. Returns None when it's already set.
/// ponytail: line-based edit; add toml_edit if configs with inline tables show up.
pub fn codex_enable_hooks(toml: &str) -> Option<String> {
    let mut out: Vec<String> = vec![];
    let (mut in_features, mut saw_features, mut done) = (false, false, false);
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_features && !done {
                out.push("hooks = true".into());
                done = true;
            }
            in_features = t == "[features]";
            saw_features |= in_features;
        } else if in_features && !done && t.split('=').next().map(str::trim) == Some("hooks") {
            if t.split('=').nth(1).map(str::trim) == Some("true") {
                return None;
            }
            out.push("hooks = true".into());
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if in_features && !done {
        out.push("hooks = true".into());
    }
    if !saw_features {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[features]".into());
        out.push("hooks = true".into());
    }
    Some(out.join("\n") + "\n")
}

const BEGIN: &str = "# >>> agents-dont-sleep (managed block, remove from the app)";
const END: &str = "# <<< agents-dont-sleep";

fn hermes_block() -> String {
    let mut s = format!("{BEGIN}\nhooks:\n");
    for (event, state) in HERMES_EVENTS {
        s += &format!("  {event}:\n    - command: \"{}\"\n      timeout: 5\n", hook_cmd("hermes", state, event));
    }
    s + END
}

pub fn hermes_strip(yaml: &str) -> String {
    let mut out = vec![];
    let mut skipping = false;
    for line in yaml.lines() {
        if line.starts_with(BEGIN) {
            skipping = true;
        } else if skipping && line.starts_with(END) {
            skipping = false;
        } else if !skipping {
            out.push(line);
        }
    }
    let s = out.join("\n");
    if s.trim().is_empty() {
        String::new()
    } else {
        s.trim_end().to_string() + "\n"
    }
}

/// Appends a managed `hooks:` block. If the user already has their own `hooks:` key we refuse
/// (a second key would be a YAML error) and hand them the snippet instead.
pub fn hermes_install_yaml(yaml: &str) -> Result<String, String> {
    let base = hermes_strip(yaml);
    if base.lines().any(|l| l.starts_with("hooks:")) {
        return Err(format!(
            "~/.hermes/config.yaml already has a hooks: section. Add these entries under it:\n\n{}",
            hermes_block().lines().skip(2).collect::<Vec<_>>().join("\n")
        ));
    }
    Ok(if base.is_empty() { hermes_block() + "\n" } else { format!("{base}\n{}\n", hermes_block()) })
}

fn hermes_allowlist(install: bool) -> Result<(), String> {
    let path = home().join(".hermes/shell-hooks-allowlist.json");
    let mut v = read_json(&path)?;
    let obj = v.as_object_mut().ok_or("allowlist is not a JSON object")?;
    let list = obj.entry("approvals").or_insert_with(|| json!([]));
    let list = list.as_array_mut().ok_or("allowlist approvals is not a list")?;
    list.retain(|x| !ours(&x.to_string()));
    if install {
        for (event, state) in HERMES_EVENTS {
            list.push(json!({ "event": event, "command": hook_cmd("hermes", state, event) }));
        }
    }
    write_json(&path, &v)
}

/// Templates contain `__HOOK__` where a string literal goes; a JSON string is valid in JS, TS
/// and Python and keeps Windows backslashes intact.
fn fill(template: &str) -> String {
    template.replace("__HOOK__", &json!(hook_path().to_string_lossy()).to_string())
}

fn find(id: &str) -> Result<&'static Agent, String> {
    AGENTS.iter().find(|a| a.id == id).ok_or_else(|| format!("unknown agent {id}"))
}

pub fn install(id: &str) -> Result<(), String> {
    let a = find(id)?;
    let _ = install_hook_binary();
    if !hook_path().exists() {
        return Err("The hook helper is missing. Reinstall Agents Don't Sleep.".into());
    }
    let h = home();
    match &a.kind {
        Kind::Json { file, shape, events } => {
            let path = h.join(file);
            let mut v = read_json(&path)?;
            json_install(&mut v, a.id, *shape, events)?;
            write_json(&path, &v)?;
            if a.id == "codex" {
                let cfg = h.join(".codex/config.toml");
                if let Some(new) = codex_enable_hooks(&fs::read_to_string(&cfg).unwrap_or_default()) {
                    write_config(&cfg, &new)?;
                }
            }
            Ok(())
        }
        Kind::Owned { file, template } => {
            write_atomic(&h.join(file), fill(template).as_bytes()).map_err(|e| e.to_string())
        }
        Kind::Hermes => {
            let cfg = h.join(".hermes/config.yaml");
            let new = hermes_install_yaml(&fs::read_to_string(&cfg).unwrap_or_default())?;
            write_config(&cfg, &new)?;
            hermes_allowlist(true)?;
            let dir = h.join(".hermes/hooks/agents-dont-sleep");
            write_atomic(&dir.join("HOOK.yaml"), include_str!("../resources/hermes-HOOK.yaml").as_bytes())
                .and_then(|_| {
                    write_atomic(
                        &dir.join("handler.py"),
                        fill(include_str!("../resources/hermes-handler.py")).as_bytes(),
                    )
                })
                .map_err(|e| e.to_string())
        }
    }
}

pub fn uninstall(id: &str) -> Result<(), String> {
    let a = find(id)?;
    let h = home();
    match &a.kind {
        Kind::Json { file, shape: Shape::Copilot, .. } => rm(&h.join(file)),
        Kind::Json { file, .. } => {
            let path = h.join(file);
            if !path.exists() {
                return Ok(());
            }
            let mut v = read_json(&path)?;
            json_uninstall(&mut v);
            write_json(&path, &v)
        }
        Kind::Owned { file, .. } => rm(&h.join(file)),
        Kind::Hermes => {
            let cfg = h.join(".hermes/config.yaml");
            if let Ok(s) = fs::read_to_string(&cfg) {
                if s.contains(BEGIN) {
                    write_config(&cfg, &hermes_strip(&s))?;
                }
            }
            hermes_allowlist(false)?;
            let _ = fs::remove_dir_all(h.join(".hermes/hooks/agents-dont-sleep"));
            Ok(())
        }
    }
}

fn rm(p: &Path) -> Result<(), String> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: &'static str,
    pub name: &'static str,
    /// "notDetected" | "available" | "installed"
    pub state: &'static str,
    pub note: &'static str,
}

/// The file whose content tells whether `a` is connected.
fn config_file(a: &Agent) -> PathBuf {
    match &a.kind {
        Kind::Json { file, .. } | Kind::Owned { file, .. } => home().join(file),
        Kind::Hermes => home().join(".hermes/config.yaml"),
    }
}

/// Rewrites connected agents' hook entries when an older app wrote them (`from_version` <
/// `HOOKS_VERSION`, or the pre-release `hook.sh` script), then removes that script. Runs at
/// startup; returns false if an agent couldn't be updated, so it's retried next launch.
pub fn refresh_integrations(from_version: u32) -> bool {
    let mut ok = true;
    for a in AGENTS {
        let Ok(s) = fs::read_to_string(config_file(a)) else { continue };
        if s.contains(".agents-dont-sleep/hook.sh") || (from_version < HOOKS_VERSION && ours(&s)) {
            ok &= install(a.id).is_ok();
        }
    }
    if ok {
        let _ = fs::remove_file(data_dir().join("hook.sh"));
    }
    ok
}

pub fn statuses() -> Vec<AgentStatus> {
    let h = home();
    AGENTS
        .iter()
        .map(|a| {
            let installed = fs::read_to_string(config_file(a)).is_ok_and(|s| ours(&s));
            let state = if installed {
                "installed"
            } else if h.join(a.dir).exists() {
                "available"
            } else {
                "notDetected"
            };
            AgentStatus { id: a.id, name: a.name, state, note: a.note }
        })
        .collect()
}

/// The session file written by `adshook` (mirrors `Record` in `src-tauri/hook/src/main.rs`).
#[derive(Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
struct Record {
    state: String,
    cwd: String,
    pid: u32,
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

/// JSON, or the 3-line format (state, cwd, pid) of helpers from 0.1 pre-releases.
fn parse_record(body: &str) -> Record {
    serde_json::from_str(body).unwrap_or_else(|_| {
        let mut lines = body.lines();
        Record {
            state: lines.next().unwrap_or("idle").trim().to_string(),
            cwd: lines.next().unwrap_or_default().to_string(),
            pid: lines.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0),
            ..Default::default()
        }
    })
}

#[derive(Serialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub agent: String,
    pub name: String,
    pub id: String,
    /// "working" | "waiting" | "idle"
    pub state: String,
    pub project: String,
    pub cwd: String,
    /// Current tool name while working ("Bash", "Edit", …), else empty.
    pub tool: String,
    pub model: String,
    /// TERM_PROGRAM of the agent's terminal ("vscode", "iTerm.app", …).
    pub term: String,
    pub started: u64,
    /// Seconds into the current turn (0 when idle).
    pub turn_secs: u64,
    pub last_turn_secs: u64,
    /// Seconds since the last hook event.
    pub quiet_secs: u64,
    /// Seconds spent waiting for the user (0 unless waiting).
    pub waiting_secs: u64,
    pub last_event: u64,
    pub tools: u32,
    pub turns: u32,
    pub errors: u32,
    /// Why the last turn failed (Claude StopFailure kind, e.g. "rate_limit"), else empty.
    pub error_kind: String,
    /// "Stop counting this session" from the tray, until its next event.
    pub dismissed: bool,
}

impl Session {
    /// Waiting on a permission prompt is still mid-turn, so it keeps holding.
    pub fn active(&self) -> bool {
        self.state != "idle"
    }
}

pub struct Running {
    /// Lowercased basenames of every process name plus argv[0..2], so `node …/gemini` → "gemini".
    pub names: HashSet<String>,
    pub pids: HashSet<u32>,
}

pub fn running(sys: &mut System) -> Running {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let mut r = Running { names: HashSet::new(), pids: HashSet::new() };
    for (pid, p) in sys.processes() {
        r.pids.insert(pid.as_u32());
        r.names.insert(basename(p.name()));
        for arg in p.cmd().iter().take(2) {
            r.names.insert(basename(arg));
        }
    }
    r
}

pub fn basename(s: &OsStr) -> String {
    let b = s.to_string_lossy();
    let b = b.rsplit(['/', '\\']).next().unwrap_or(&b).to_lowercase();
    b.strip_suffix(".exe").map(String::from).unwrap_or(b)
}

/// Reads session files, deleting ones whose agent process is gone or that sent nothing for
/// `stale_secs` (a crashed agent, or a tool that ran silently longer than the user allows).
pub fn scan_sessions(running: &Running, stale_secs: u64) -> Vec<Session> {
    let now = crate::settings::now();
    let dir = data_dir().join("sessions");
    let mut out = vec![];
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let fname = e.file_name().to_string_lossy().into_owned();
        let Some((agent, id)) = fname.split_once("__") else { continue };
        let Some(def) = AGENTS.iter().find(|a| a.id == agent) else { continue };
        let r = parse_record(&fs::read_to_string(e.path()).unwrap_or_default());
        let mtime = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        let last_event = if r.last_event > 0 { r.last_event } else { mtime.map_or(0, |d| d.as_secs()) };
        let quiet_secs = now.saturating_sub(last_event);
        // Exact per-session liveness when the hook recorded the agent pid; agent-wide otherwise.
        let alive = match Some(r.pid).filter(|&p| p > 1) {
            Some(pid) => running.pids.contains(&pid),
            None => def.procs.iter().any(|p| running.names.contains(*p)),
        };
        if quiet_secs > stale_secs || !alive {
            let _ = fs::remove_file(e.path());
            continue;
        }
        let since = |t: u64| if t > 0 { now.saturating_sub(t) } else { 0 };
        out.push(Session {
            agent: agent.into(),
            name: def.name.into(),
            id: id.into(),
            project: Path::new(&r.cwd).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            turn_secs: since(r.turn_started),
            waiting_secs: since(r.waiting_since),
            quiet_secs,
            last_event,
            state: r.state,
            cwd: r.cwd,
            tool: r.tool,
            model: r.model,
            term: r.term,
            started: r.started,
            last_turn_secs: r.last_turn_secs,
            tools: r.tools,
            turns: r.turns,
            errors: r.errors,
            error_kind: r.error_kind,
            dismissed: false,
        });
    }
    out.sort_by(|a, b| (&a.agent, &a.id).cmp(&(&b.agent, &b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fake-home test swaps $HOME, which every hook path depends on.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn home_lock() -> std::sync::MutexGuard<'static, ()> {
        HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    const EXISTING: &str = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "echo hi" }] }],
    "Stop": [{ "matcher": "", "hooks": [{ "type": "command", "command": "~/.claude/hooks/journal.sh" }] }]
  },
  "permissions": { "allow": ["Bash(ls:*)"] }
}"#;

    fn claude_events() -> &'static [(&'static str, &'static str)] {
        match &AGENTS[0].kind {
            Kind::Json { events, .. } => events,
            _ => unreachable!(),
        }
    }

    #[test]
    fn json_round_trip_keeps_user_hooks() {
        let _g = home_lock();
        let original: Value = serde_json::from_str(EXISTING).unwrap();
        let mut v = original.clone();
        json_install(&mut v, "claude", Shape::Claude, claude_events()).unwrap();
        json_install(&mut v, "claude", Shape::Claude, claude_events()).unwrap(); // idempotent
        let s = v.to_string();
        assert_eq!(s.matches("adshook").count(), claude_events().len());
        assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["args"], json!(["claude", "idle", "--event=Stop"])); // exec form
        assert!(s.contains("echo hi") && s.contains("journal.sh") && s.contains("Bash(ls:*)"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        json_uninstall(&mut v);
        assert_eq!(v, original);
        // Key order preserved (serde_json preserve_order).
        assert_eq!(v.as_object().unwrap().keys().collect::<Vec<_>>(), ["model", "hooks", "permissions"]);
    }

    #[test]
    fn json_round_trip_from_empty() {
        let _g = home_lock();
        let mut v = json!({});
        json_install(&mut v, "cursor", Shape::Cursor, &[("stop", "idle")]).unwrap();
        assert_eq!(v["version"], 1);
        let cmd = v["hooks"]["stop"][0]["command"].as_str().unwrap();
        assert!(ours(cmd) && cmd.ends_with(" cursor idle --event=stop"), "{cmd}"); // adshook(.exe) cursor idle …
        json_uninstall(&mut v);
        assert_eq!(v, json!({ "version": 1 }));
    }

    #[test]
    fn json_refuses_odd_shapes() {
        let _g = home_lock();
        let mut v = json!({ "hooks": "nope" });
        assert!(json_install(&mut v, "claude", Shape::Nested, &[("Stop", "idle")]).is_err());
        assert_eq!(v, json!({ "hooks": "nope" }));
    }

    #[test]
    fn codex_feature_flag() {
        let _g = home_lock();
        assert_eq!(codex_enable_hooks(""), Some("[features]\nhooks = true\n".into()));
        assert_eq!(codex_enable_hooks("model = \"o5\"\n"), Some("model = \"o5\"\n\n[features]\nhooks = true\n".into()));
        assert_eq!(codex_enable_hooks("[features]\nhooks = true\n"), None);
        assert_eq!(
            codex_enable_hooks("[features]\nhooks = false\nweb = true\n[mcp]\nx = 1\n"),
            Some("[features]\nhooks = true\nweb = true\n[mcp]\nx = 1\n".into())
        );
        assert_eq!(
            codex_enable_hooks("[features]\nweb = true\n\n[mcp]\n"),
            Some("[features]\nweb = true\n\nhooks = true\n[mcp]\n".into())
        );
    }

    #[test]
    fn hermes_yaml() {
        let _g = home_lock();
        let user = "model: hermes-4\n# my comment\n";
        let installed = hermes_install_yaml(user).unwrap();
        assert!(installed.starts_with(user) && installed.contains("pre_llm_call:") && ours(&installed));
        assert_eq!(hermes_install_yaml(&installed).unwrap(), installed); // idempotent
        assert_eq!(hermes_strip(&installed), user);
        assert!(hermes_install_yaml("hooks:\n  x: []\n").is_err());
        assert_eq!(hermes_strip(&hermes_install_yaml("").unwrap()), "");
    }

    /// Installs every integration into a throwaway $HOME, then uninstalls and checks the
    /// user's own config came back. Set ADS_KEEP=1 to leave the installed files for manual e2e.
    #[test]
    fn install_uninstall_all_in_fake_home() {
        let _g = home_lock();
        let home = std::env::temp_dir().join(format!("ads-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        // home_dir() reads HOME on unix and USERPROFILE on Windows.
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let real_home = std::env::var_os(var);
        std::env::set_var(var, &home);
        struct Restore(&'static str, Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(h) = self.1.take() {
                    std::env::set_var(self.0, h);
                }
            }
        }
        let _restore = Restore(var, real_home);
        // The sidecar isn't next to the test binary; stand in for it.
        fs::create_dir_all(hook_path().parent().unwrap()).unwrap();
        fs::write(hook_path(), b"stub").unwrap();
        for a in AGENTS {
            fs::create_dir_all(home.join(a.dir)).unwrap();
        }
        // A pre-release (hook.sh) entry alongside the user's own hooks: migrated, then removed.
        let mut legacy: Value = serde_json::from_str(EXISTING).unwrap();
        legacy["hooks"]["Stop"].as_array_mut().unwrap().push(json!({ "hooks": [{
            "type": "command", "command": format!("{}/.agents-dont-sleep/hook.sh claude idle", home.display()) }] }));
        fs::write(home.join(".claude/settings.json"), legacy.to_string()).unwrap();
        fs::write(data_dir().join("hook.sh"), "#!/bin/sh\n").unwrap();
        assert!(refresh_integrations(HOOKS_VERSION));
        let migrated = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert!(!migrated.contains("hook.sh") && migrated.contains("adshook"), "{migrated}");
        assert!(!data_dir().join("hook.sh").exists());
        fs::write(home.join(".codex/config.toml"), "model = \"o5\"\n").unwrap();
        fs::write(home.join(".hermes/config.yaml"), "model: hermes-4\n").unwrap();

        for a in AGENTS {
            install(a.id).unwrap_or_else(|e| panic!("{}: {e}", a.id));
        }
        assert!(
            statuses().iter().all(|s| s.state == "installed"),
            "{:?}",
            statuses().iter().map(|s| (s.id, s.state)).collect::<Vec<_>>()
        );
        assert!(fs::read_to_string(home.join(".codex/config.toml")).unwrap().contains("[features]\nhooks = true"));
        let allow: Value =
            serde_json::from_str(&fs::read_to_string(home.join(".hermes/shell-hooks-allowlist.json")).unwrap())
                .unwrap();
        assert_eq!(allow["approvals"].as_array().unwrap().len(), HERMES_EVENTS.len());
        assert!(fs::read_to_string(home.join(".hermes/hooks/agents-dont-sleep/handler.py"))
            .unwrap()
            .contains(&json!(hook_path().to_string_lossy()).to_string()));
        assert!(!fs::read_to_string(home.join(".config/opencode/plugins/agents-dont-sleep.js"))
            .unwrap()
            .contains("__HOOK__"));

        // Entries written by an older app (no --event) are rewritten on a version bump.
        let old = json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command",
            "command": hook_path().to_string_lossy(), "args": ["claude", "idle"] }] }] } });
        fs::write(home.join(".claude/settings.json"), old.to_string()).unwrap();
        assert!(refresh_integrations(HOOKS_VERSION - 1));
        let refreshed = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert!(refreshed.contains("--event=Stop") && refreshed.contains("--event=PreToolUse"), "{refreshed}");
        assert_eq!(refreshed.matches("adshook").count(), claude_events().len());
        fs::write(home.join(".claude/settings.json"), EXISTING).unwrap();
        install("claude").unwrap();

        if std::env::var("ADS_KEEP").is_ok() {
            println!("kept installed files in {}", home.display());
            return;
        }
        for a in AGENTS {
            uninstall(a.id).unwrap();
        }
        assert!(statuses().iter().all(|s| s.state == "available"));
        let claude: Value =
            serde_json::from_str(&fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
        assert_eq!(claude, serde_json::from_str::<Value>(EXISTING).unwrap());
        assert_eq!(fs::read_to_string(home.join(".hermes/config.yaml")).unwrap(), "model: hermes-4\n");
        assert!(!home.join(".copilot/hooks/agents-dont-sleep.json").exists());
        assert!(!home.join(".hermes/hooks/agents-dont-sleep").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn session_records() {
        let _g = home_lock();
        let json = r#"{"state":"working","cwd":"/w/api","pid":9,"tool":"Bash","tools":4,"turns":1,"turn_started":100,"last_event":150}"#;
        let r = parse_record(json);
        assert_eq!((r.state.as_str(), r.tool.as_str(), r.pid, r.tools, r.turn_started), ("working", "Bash", 9, 4, 100));
        let legacy = parse_record("waiting\n/w/web\n77\n");
        assert_eq!((legacy.state.as_str(), legacy.cwd.as_str(), legacy.pid), ("waiting", "/w/web", 77));
        assert_eq!(parse_record("").state, "idle");
    }

    #[test]
    fn posix_quoting() {
        let _g = home_lock();
        assert_eq!(quote_posix("/Users/me/.agents-dont-sleep/bin/adshook"), "/Users/me/.agents-dont-sleep/bin/adshook");
        assert_eq!(quote_posix("/home/John Smith/x"), "'/home/John Smith/x'");
        assert_eq!(quote_posix("/home/o'neil/x"), r"'/home/o'\''neil/x'");
    }

    #[test]
    fn process_basenames() {
        let _g = home_lock();
        assert_eq!(basename(OsStr::new("/opt/homebrew/bin/gemini")), "gemini");
        assert_eq!(basename(OsStr::new("Cursor")), "cursor");
        assert_eq!(basename(OsStr::new("C:\\Program Files\\nodejs\\node.exe")), "node");
        assert_eq!(basename(OsStr::new("claude.exe")), "claude");
    }
}
