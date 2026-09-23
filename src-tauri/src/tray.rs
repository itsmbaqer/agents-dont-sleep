//! Tray menu, icon and label. The menu is described as a tree (`spec`) and only rebuilt when
//! its shape changes (sessions come or go, a state changes); timers and counters update in
//! place with `set_text`, so an open menu stays open while agents work.
use crate::agents::Session;
use crate::decide::Reason;
use crate::settings::{Settings, TrayLabel};
use crate::{power, Status};
use std::collections::HashMap;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, IconMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Wry,
};

#[cfg(target_os = "macos")]
const ICONS: [&[u8]; 4] = [
    include_bytes!("../icons/tray-idle.png"),
    include_bytes!("../icons/tray-awake.png"),
    include_bytes!("../icons/tray-attention.png"),
    include_bytes!("../icons/tray-off.png"),
];
#[cfg(not(target_os = "macos"))]
const ICONS: [&[u8]; 4] = [
    include_bytes!("../icons/tray-idle-color.png"),
    include_bytes!("../icons/tray-awake-color.png"),
    include_bytes!("../icons/tray-attention-color.png"),
    include_bytes!("../icons/tray-off-color.png"),
];
/// macOS menu-bar icons are black templates the system tints; other trays get colored icons.
pub const TEMPLATE_ICONS: bool = cfg!(target_os = "macos");

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Look {
    Idle,
    Awake,
    Attention,
    Off,
}

pub fn icon(look: Look) -> Image<'static> {
    Image::from_bytes(ICONS[look as usize]).expect("bundled tray icon")
}

/// Which icon the tray shows: someone waiting on you beats everything.
pub fn look(s: &Status) -> Look {
    if s.needs_you > 0 {
        Look::Attention
    } else if s.held {
        Look::Awake
    } else if !matches!(s.reason, Reason::NoAgents | Reason::Holding) {
        Look::Off
    } else {
        Look::Idle
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dot {
    Green,
    Amber,
    Gray,
    Red,
}

/// A 16px anti-aliased colored circle for the status row (drawn, so no image files).
fn dot_image(dot: Dot) -> Image<'static> {
    let rgb = match dot {
        Dot::Green => [34, 197, 94],
        Dot::Amber => [245, 158, 11],
        Dot::Gray => [148, 163, 184],
        Dot::Red => [239, 68, 68],
    };
    let n = 16usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 + 0.5 - 8.0).powi(2) + (y as f32 + 0.5 - 8.0).powi(2)).sqrt();
            let a = (5.5 - d).clamp(0.0, 1.0);
            let i = (y * n + x) * 4;
            px[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], (a * 255.0) as u8]);
        }
    }
    Image::new_owned(px, n as u32, n as u32)
}

/// One node of the menu. `live` text changes without a rebuild.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Item { id: String, text: String, enabled: bool, live: bool, accel: Option<String> },
    Status { id: String, text: String, dot: Dot },
    Check { id: String, text: String, checked: bool, accel: Option<String> },
    Sub { id: String, text: String, live: bool, children: Vec<Node> },
    Sep,
}

fn item(id: &str, text: impl Into<String>) -> Node {
    Node::Item { id: id.into(), text: text.into(), enabled: true, live: false, accel: None }
}
fn info(id: impl Into<String>, text: impl Into<String>) -> Node {
    Node::Item { id: id.into(), text: text.into(), enabled: false, live: true, accel: None }
}
fn header(id: &str, text: &str) -> Node {
    Node::Item { id: id.into(), text: text.into(), enabled: false, live: false, accel: None }
}

/// "45s", "12m", "1h 05m"
pub fn dur(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {:02}m", secs / 3600, secs / 60 % 60),
    }
}

fn error_label(kind: &str) -> String {
    match kind {
        "rate_limit" => "rate limited".into(),
        "overloaded" => "API overloaded".into(),
        "billing_error" => "billing problem".into(),
        "authentication_failed" => "signed out".into(),
        "max_output_tokens" => "hit the output limit".into(),
        "server_error" => "server error".into(),
        k => k.replace('_', " "),
    }
}

/// Where the agent runs, for "Show in …": a label for known terminals (macOS bundle ids first,
/// then TERM_PROGRAM values), a generic one for other bundle ids, nothing otherwise.
pub fn term_app(term: &str) -> Option<&'static str> {
    if !cfg!(target_os = "macos") || term.is_empty() {
        return None;
    }
    Some(match term {
        "com.microsoft.VSCode" | "vscode" => "VS Code",
        "com.todesktop.230313mzl4w4u92" => "Cursor",
        "com.exafunction.windsurf" => "Windsurf",
        "dev.zed.Zed" => "Zed",
        "com.googlecode.iterm2" | "iTerm.app" => "iTerm",
        "com.apple.Terminal" | "Apple_Terminal" => "Terminal",
        "dev.warp.Warp-Stable" | "WarpTerminal" => "Warp",
        "com.mitchellh.ghostty" | "ghostty" => "Ghostty",
        t if t.contains('.') => "its app",
        _ => return None,
    })
}

/// Stable key per session, also used in menu ids.
pub fn key(x: &Session) -> String {
    format!("{}__{}", x.agent, x.id)
}

#[derive(PartialEq, PartialOrd, Clone, Copy, Debug)]
enum Section {
    NeedsYou,
    Failed,
    Working,
    Idle,
}

fn section(x: &Session) -> Section {
    match x.state.as_str() {
        _ if x.dismissed => Section::Idle,
        "waiting" => Section::NeedsYou,
        "working" => Section::Working,
        _ if !x.error_kind.is_empty() => Section::Failed,
        _ => Section::Idle,
    }
}

fn row_text(x: &Session, stuck_mins: u32) -> String {
    let project = if x.project.is_empty() { String::new() } else { format!(" — {}", x.project) };
    let detail = match section(x) {
        _ if x.dismissed => "not counted".to_string(),
        Section::NeedsYou => format!("waiting {}", dur(x.waiting_secs)),
        Section::Working => {
            let mut d = dur(x.turn_secs);
            if !x.tool.is_empty() {
                d += &format!(" · {}", x.tool);
            }
            if stuck_mins > 0 && x.quiet_secs >= u64::from(stuck_mins) * 60 {
                d += &format!(" · ⚠ quiet {}", dur(x.quiet_secs));
            }
            d
        }
        Section::Failed => error_label(&x.error_kind),
        Section::Idle => "idle".to_string(),
    };
    let glyph = match section(x) {
        Section::NeedsYou => "◐",
        Section::Working => "●",
        Section::Failed => "⚠",
        Section::Idle => "○",
    };
    format!("{glyph} {}{project} · {detail}", x.name)
}

fn session_node(x: &Session, stuck_mins: u32) -> Node {
    let k = key(x);
    let state_line = match section(x) {
        Section::NeedsYou => format!("Waiting for you · {}", dur(x.waiting_secs)),
        Section::Working if x.tool.is_empty() => format!("Working for {}", dur(x.turn_secs)),
        Section::Working => format!("Working for {} · {}", dur(x.turn_secs), x.tool),
        Section::Failed => format!("Stopped: {}", error_label(&x.error_kind)),
        Section::Idle if x.last_turn_secs > 0 => format!("Idle · last turn took {}", dur(x.last_turn_secs)),
        Section::Idle => "Idle".into(),
    };
    let plural = |n: u32, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    let mut children = vec![
        info(format!("{k}:state"), state_line),
        info(format!("{k}:quiet"), format!("Last activity {} ago", dur(x.quiet_secs))),
        info(format!("{k}:model"), format!("Model: {}", if x.model.is_empty() { "—" } else { &x.model })),
        info(
            format!("{k}:counts"),
            format!("{} · {} · {}", plural(x.tools, "tool call"), plural(x.turns, "turn"), plural(x.errors, "error")),
        ),
        info(format!("{k}:started"), format!("Started {} ago", dur(crate::settings::now().saturating_sub(x.started)))),
        Node::Sep,
    ];
    if !x.cwd.is_empty() {
        children.push(item(&format!("open:{k}"), "Open project folder"));
    }
    if let Some(app) = term_app(&x.term) {
        children.push(item(&format!("term:{k}"), format!("Show in {app}")));
    }
    children.push(if x.dismissed {
        item(&format!("undismiss:{k}"), "Count this session again")
    } else {
        item(&format!("dismiss:{k}"), "Stop counting this session")
    });
    Node::Sub { id: format!("s:{k}"), text: row_text(x, stuck_mins), live: true, children }
}

/// "Alt+Super+KeyL" → "⌥⌘L" on macOS, "Ctrl+Alt+Shift+L" elsewhere.
pub fn pretty_shortcut(s: &str, mac: bool) -> String {
    let parts = s.split('+').map(|t| match (t.to_lowercase().as_str(), mac) {
        ("alt" | "option", true) => "⌥".to_string(),
        ("super" | "cmd" | "command", true) => "⌘".into(),
        ("shift", true) => "⇧".into(),
        ("ctrl" | "control", true) => "⌃".into(),
        ("ctrl" | "control", false) => "Ctrl".into(),
        ("super" | "cmd" | "command", false) => "Win".into(),
        _ => t.trim_start_matches("Key").trim_start_matches("Digit").to_string(),
    });
    parts.collect::<Vec<_>>().join(if mac { "" } else { "+" })
}

fn low_power_name() -> &'static str {
    match power::OS {
        "macos" => "Low Power Mode",
        "windows" => "Battery saver",
        _ => "Power saver",
    }
}

pub fn status_line(s: &Status, set: &Settings) -> String {
    let now = crate::settings::now();
    match s.reason {
        Reason::Holding => {
            format!("Awake — {} · {}", if s.lid_proof { "lid-proof" } else { "lid open only" }, dur(s.held_secs))
        }
        Reason::NoAgents => format!("Idle — your {} can sleep normally", power::DEVICE),
        Reason::Paused if s.paused_until == u64::MAX => "Paused until you resume".into(),
        Reason::Paused => format!("Paused · {} left", dur(s.paused_until.saturating_sub(now))),
        Reason::Disabled => {
            format!("Off — press {} to turn on", pretty_shortcut(&set.shortcut, cfg!(target_os = "macos")))
        }
        Reason::Battery => format!("Held off — battery {}% (limit {}%)", s.battery.unwrap_or(0), set.battery_cutoff),
        Reason::Thermal => format!("Held off — your {} is running hot", power::DEVICE),
        Reason::LowPower => format!("Held off — {} is on", low_power_name()),
        Reason::NotPluggedIn => "Held off — not plugged in".into(),
    }
}

fn status_dot(s: &Status) -> Dot {
    match look(s) {
        Look::Attention => Dot::Amber,
        Look::Awake => Dot::Green,
        Look::Off => Dot::Red,
        Look::Idle => Dot::Gray,
    }
}

fn battery_line(s: &Status, set: &Settings) -> String {
    match (s.battery, s.on_ac, s.battery_eta_mins) {
        (None, _, _) => "On power adapter".into(),
        (Some(b), true, _) => format!("Battery {b}% · on power"),
        (Some(b), false, Some(m)) => {
            format!("Battery {b}% · ~{} to the {}% limit", dur(u64::from(m) * 60), set.battery_cutoff)
        }
        (Some(b), false, None) => format!("Battery {b}% · stops below {}%", set.battery_cutoff),
    }
}

/// The whole menu, from current state. Pure, so layout rules are tested.
pub fn spec(s: &Status, set: &Settings, connected: bool) -> Vec<Node> {
    let mut v = vec![
        Node::Status { id: "status".into(), text: status_line(s, set), dot: status_dot(s) },
        info("battery", battery_line(s, set)),
        Node::Sep,
    ];
    let mut sessions: Vec<&Session> = s.sessions.iter().collect();
    sessions.sort_by(|a, b| section(a).partial_cmp(&section(b)).unwrap().then(b.turn_secs.cmp(&a.turn_secs)));
    let mut sections: Vec<Section> = sessions.iter().map(|x| section(x)).collect();
    sections.dedup();
    let headers = sections.len() > 1 || !s.process_agents.is_empty();
    let mut last = None;
    for x in &sessions {
        let sec = section(x);
        if headers && last != Some(sec) {
            let (id, title) = match sec {
                Section::NeedsYou => ("h-needs", "Needs you"),
                Section::Failed => ("h-failed", "Stopped with an error"),
                Section::Working => ("h-working", "Working"),
                Section::Idle => ("h-idle", "Idle"),
            };
            v.push(header(id, title));
            last = Some(sec);
        }
        v.push(session_node(x, set.alert_stuck_mins));
    }
    if !s.process_agents.is_empty() {
        v.push(header("h-procs", "Running (no hooks)"));
        for p in &s.process_agents {
            v.push(header(&format!("p:{p}"), &format!("● {p}")));
        }
    }
    if sessions.is_empty() && s.process_agents.is_empty() {
        v.push(if connected { header("none", "No agents running") } else { item("connect", "Connect your agents…") });
    }
    v.push(Node::Sep);
    v.push(Node::Check {
        id: "toggle".into(),
        text: "Enabled".into(),
        checked: set.enabled,
        accel: Some(set.shortcut.clone()),
    });
    let paused = s.reason == Reason::Paused;
    let mut pause = vec![item("pause30", "30 minutes"), item("pause60", "1 hour"), item("pauseinf", "Until I resume")];
    if paused {
        pause.extend([Node::Sep, item("resume", "Resume")]);
    }
    v.push(Node::Sub { id: "pause".into(), text: "Pause".into(), live: false, children: pause });
    if power::NEEDS_GRANT && !s.lid_proof {
        v.push(item("grant", "Allow lid-closed awake…"));
    }
    v.push(Node::Sep);
    v.push(Node::Item {
        id: "settings".into(),
        text: "Settings…".into(),
        enabled: true,
        live: false,
        accel: Some("CmdOrCtrl+,".into()),
    });
    v.push(Node::Item {
        id: "quit".into(),
        text: "Quit Agents Don't Sleep".into(),
        enabled: true,
        live: false,
        accel: Some("CmdOrCtrl+Q".into()),
    });
    v
}

/// The menu's shape: everything except live text. A new shape means a rebuild.
pub fn shape(nodes: &[Node]) -> String {
    fn walk(n: &Node, out: &mut String) {
        match n {
            Node::Item { id, text, enabled, live, .. } => {
                out.push_str(&format!("i{id}{enabled}{};", if *live { "" } else { text }))
            }
            Node::Status { id, dot, .. } => out.push_str(&format!("s{id}{dot:?};")),
            Node::Check { id, checked, .. } => out.push_str(&format!("c{id}{checked};")),
            Node::Sub { id, text, live, children } => {
                out.push_str(&format!("u{id}{}[", if *live { "" } else { text }));
                children.iter().for_each(|c| walk(c, out));
                out.push(']');
            }
            Node::Sep => out.push('-'),
        }
    }
    let mut out = String::new();
    nodes.iter().for_each(|n| walk(n, &mut out));
    out
}

fn live_texts(nodes: &[Node], out: &mut Vec<(String, String)>) {
    for n in nodes {
        match n {
            Node::Item { id, text, live: true, .. } | Node::Status { id, text, .. } => {
                out.push((id.clone(), text.clone()))
            }
            Node::Sub { id, text, live, children } => {
                if *live {
                    out.push((id.clone(), text.clone()));
                }
                live_texts(children, out);
            }
            _ => {}
        }
    }
}

enum Handle {
    Item(MenuItem<Wry>),
    Icon(IconMenuItem<Wry>),
    Sub(Submenu<Wry>),
}

#[derive(Default)]
pub struct TrayState {
    shape: String,
    handles: HashMap<String, Handle>,
    texts: HashMap<String, String>,
    look: Option<Look>,
    title: String,
}

fn build(
    app: &AppHandle,
    nodes: &[Node],
    handles: &mut HashMap<String, Handle>,
) -> tauri::Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    let mut out: Vec<Box<dyn IsMenuItem<Wry>>> = vec![];
    for n in nodes {
        match n {
            Node::Sep => out.push(Box::new(PredefinedMenuItem::separator(app)?)),
            Node::Item { id, text, enabled, accel, .. } => {
                let it = MenuItem::with_id(app, id, text, *enabled, accel.as_deref())
                    .or_else(|_| MenuItem::with_id(app, id, text, *enabled, None::<&str>))?;
                handles.insert(id.clone(), Handle::Item(it.clone()));
                out.push(Box::new(it));
            }
            Node::Status { id, text, dot } => {
                match IconMenuItem::with_id(app, id, text, false, Some(dot_image(*dot)), None::<&str>) {
                    Ok(it) => {
                        handles.insert(id.clone(), Handle::Icon(it.clone()));
                        out.push(Box::new(it));
                    }
                    // ponytail: menus without image support fall back to plain text.
                    Err(_) => {
                        let it = MenuItem::with_id(app, id, text, false, None::<&str>)?;
                        handles.insert(id.clone(), Handle::Item(it.clone()));
                        out.push(Box::new(it));
                    }
                }
            }
            Node::Check { id, text, checked, accel } => {
                let it = CheckMenuItem::with_id(app, id, text, true, *checked, accel.as_deref())
                    .or_else(|_| CheckMenuItem::with_id(app, id, text, true, *checked, None::<&str>))?;
                out.push(Box::new(it));
            }
            Node::Sub { id, text, children, .. } => {
                let kids = build(app, children, handles)?;
                let refs: Vec<&dyn IsMenuItem<Wry>> = kids.iter().map(|b| b.as_ref()).collect();
                let sub = Submenu::with_id_and_items(app, id, text, true, &refs)?;
                handles.insert(id.clone(), Handle::Sub(sub.clone()));
                out.push(Box::new(sub));
            }
        }
    }
    Ok(out)
}

/// Rebuilds the menu when its shape changed, otherwise patches live text; swaps icon/label
/// only when they change.
pub fn update(app: &AppHandle, st: &mut TrayState, s: &Status, set: &Settings, connected: bool) {
    let Some(tray) = app.tray_by_id("main") else { return };
    let nodes = spec(s, set, connected);
    let shape = shape(&nodes);
    let mut texts = vec![];
    live_texts(&nodes, &mut texts);
    if shape != st.shape {
        let mut handles = HashMap::new();
        match build(app, &nodes, &mut handles).and_then(|items| {
            let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
            Menu::with_items(app, &refs)
        }) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
                st.shape = shape;
                st.handles = handles;
                st.texts = texts.into_iter().collect();
            }
            Err(e) => eprintln!("tray menu: {e}"),
        }
    } else {
        for (id, text) in texts {
            if st.texts.get(&id) == Some(&text) {
                continue;
            }
            let _ = match st.handles.get(&id) {
                Some(Handle::Item(h)) => h.set_text(&text),
                Some(Handle::Icon(h)) => h.set_text(&text),
                Some(Handle::Sub(h)) => h.set_text(&text),
                None => Ok(()),
            };
            st.texts.insert(id, text);
        }
    }

    let look = look(s);
    if st.look != Some(look) {
        let _ = tray.set_icon_with_as_template(Some(icon(look)), TEMPLATE_ICONS);
        st.look = Some(look);
    }
    let title = title(&s.sessions, &s.process_agents, set.tray_label);
    let mut tooltip = status_line(s, set);
    // Windows can't show text beside a tray icon, so the counts go in the tooltip there.
    if cfg!(windows) && !title.is_empty() {
        tooltip = format!("{tooltip}\n{title}");
    }
    if title != st.title {
        let _ = tray.set_title(if title.is_empty() { None } else { Some(title.clone()) });
        st.title = title;
    }
    let _ = tray.set_tooltip(Some(tooltip));
}

/// Text beside the tray icon: active (working or waiting) sessions per agent, e.g.
/// "Claude 2 · Codex 1"; "3 agents · 5" when more would crowd the menu bar; "◐1 " in front
/// when someone is waiting for you.
pub fn title(sessions: &[Session], process_agents: &[String], mode: TrayLabel) -> String {
    let counted: Vec<&Session> = sessions.iter().filter(|x| x.active() && !x.dismissed).collect();
    let mut groups: Vec<(&str, usize)> = vec![];
    let names = counted
        .iter()
        .map(|x| x.name.split(' ').next().unwrap_or(&x.name)) // "Claude Code" → "Claude"
        .chain(process_agents.iter().map(String::as_str));
    for name in names {
        match groups.iter_mut().find(|(g, _)| *g == name) {
            Some((_, n)) => *n += 1,
            None => groups.push((name, 1)),
        }
    }
    let total: usize = groups.iter().map(|(_, n)| n).sum();
    let body = match (mode, groups.len()) {
        (TrayLabel::Off, _) | (_, 0) => return String::new(),
        (TrayLabel::Count, _) => total.to_string(),
        (TrayLabel::Full, 1 | 2) => groups.iter().map(|(g, n)| format!("{g} {n}")).collect::<Vec<_>>().join(" · "),
        (TrayLabel::Full, k) => format!("{k} agents · {total}"),
    };
    let waiting = counted.iter().filter(|x| x.state == "waiting").count();
    if waiting > 0 {
        format!("◐{waiting} {body}")
    } else {
        body
    }
}

/// Minutes until the battery reaches `cutoff`, from `(unix secs, percent)` samples taken while
/// discharging. Needs ≥ 3 samples over ≥ 5 minutes; None when not draining.
pub fn minutes_to(cutoff: u8, samples: &[(u64, u8)]) -> Option<u32> {
    let (&(t0, p0), &(t1, p1)) = (samples.first()?, samples.last()?);
    if samples.len() < 3 || t1.saturating_sub(t0) < 300 || p1 >= p0 || p1 <= cutoff {
        return None;
    }
    let per_sec = f64::from(p0 - p1) / (t1 - t0) as f64;
    Some((f64::from(p1 - cutoff) / per_sec / 60.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(agent: &str, name: &str, state: &str) -> Session {
        Session {
            agent: agent.into(),
            id: format!("{agent}-{state}"),
            name: name.into(),
            state: state.into(),
            ..Default::default()
        }
    }

    fn status(sessions: Vec<Session>) -> Status {
        Status {
            reason: Reason::Holding,
            held: true,
            held_secs: 4380,
            lid_proof: true,
            battery: Some(64),
            on_ac: false,
            thermal: None,
            low_power: false,
            lid_closed: false,
            working: sessions.iter().filter(|x| x.active()).count(),
            needs_you: sessions.iter().filter(|x| x.state == "waiting").count(),
            battery_eta_mins: Some(80),
            sessions,
            process_agents: vec![],
            paused_until: 0,
            platform: power::platform(),
        }
    }

    fn ids(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match n {
                Node::Item { id, .. } | Node::Sub { id, .. } | Node::Status { id, .. } | Node::Check { id, .. } => {
                    Some(id.clone())
                }
                Node::Sep => None,
            })
            .collect()
    }

    #[test]
    fn needs_you_comes_first_with_headers() {
        let mut working = s("claude", "Claude Code", "working");
        working.tool = "Bash".into();
        working.turn_secs = 720;
        let st = status(vec![s("gemini", "Gemini CLI", "idle"), working, s("codex", "Codex", "waiting")]);
        let got = ids(&spec(&st, &Settings::default(), true));
        let order: Vec<&str> =
            got.iter().map(String::as_str).filter(|i| i.starts_with("h-") || i.starts_with("s:")).collect();
        assert_eq!(
            order,
            [
                "h-needs",
                "s:codex__codex-waiting",
                "h-working",
                "s:claude__claude-working",
                "h-idle",
                "s:gemini__gemini-idle"
            ]
        );
    }

    #[test]
    fn single_section_has_no_header() {
        let st = status(vec![s("claude", "Claude Code", "working")]);
        assert!(!ids(&spec(&st, &Settings::default(), true)).iter().any(|i| i.starts_with("h-")));
    }

    #[test]
    fn timers_dont_change_the_shape() {
        let mut a = s("claude", "Claude Code", "working");
        a.turn_secs = 60;
        let mut b = a.clone();
        b.turn_secs = 1260;
        b.quiet_secs = 30;
        let set = Settings::default();
        let (sa, sb) = (spec(&status(vec![a]), &set, true), spec(&status(vec![b.clone()]), &set, true));
        assert_eq!(shape(&sa), shape(&sb));
        b.state = "waiting".into();
        assert_ne!(shape(&sa), shape(&spec(&status(vec![b]), &set, true)));
    }

    #[test]
    fn row_texts() {
        let mut w = s("claude", "Claude Code", "working");
        w.project = "api".into();
        w.turn_secs = 754;
        w.tool = "Bash".into();
        assert_eq!(row_text(&w, 15), "● Claude Code — api · 12m · Bash");
        w.quiet_secs = 18 * 60;
        assert_eq!(row_text(&w, 15), "● Claude Code — api · 12m · Bash · ⚠ quiet 18m");
        let mut f = s("claude", "Claude Code", "idle");
        f.error_kind = "rate_limit".into();
        assert_eq!(row_text(&f, 15), "⚠ Claude Code · rate limited");
        let mut d = s("codex", "Codex", "waiting");
        d.dismissed = true;
        assert_eq!(row_text(&d, 15), "○ Codex · not counted");
    }

    #[test]
    fn empty_states() {
        let st = status(vec![]);
        assert!(ids(&spec(&st, &Settings::default(), false)).contains(&"connect".to_string()));
        assert!(ids(&spec(&st, &Settings::default(), true)).contains(&"none".to_string()));
    }

    #[test]
    fn durations() {
        assert_eq!(dur(45), "45s");
        assert_eq!(dur(754), "12m");
        assert_eq!(dur(3900), "1h 05m");
    }

    #[test]
    fn titles() {
        let two =
            [s("claude", "Claude Code", "working"), s("claude", "Claude Code", "waiting"), s("x", "Codex", "idle")];
        assert_eq!(title(&two, &[], TrayLabel::Full), "◐1 Claude 2");
        assert_eq!(title(&two, &[], TrayLabel::Count), "◐1 2");
        assert_eq!(title(&two, &[], TrayLabel::Off), "");
        let mixed = [s("claude", "Claude Code", "working"), s("codex", "Codex", "working")];
        assert_eq!(title(&mixed, &[], TrayLabel::Full), "Claude 1 · Codex 1");
        assert_eq!(title(&mixed, &["aider".into()], TrayLabel::Full), "3 agents · 3");
        assert_eq!(title(&[], &[], TrayLabel::Full), "");
        let mut dismissed = s("claude", "Claude Code", "working");
        dismissed.dismissed = true;
        assert_eq!(title(&[dismissed], &[], TrayLabel::Full), "");
    }

    #[test]
    fn battery_eta() {
        assert_eq!(minutes_to(15, &[(0, 64), (300, 62), (600, 60)]), Some(113)); // 45% at 4%/10min
        assert_eq!(minutes_to(15, &[(0, 64), (60, 63), (120, 62)]), None); // under 5 minutes
        assert_eq!(minutes_to(15, &[(0, 60), (300, 60), (600, 60)]), None); // not draining
        assert_eq!(minutes_to(15, &[(0, 60), (600, 50)]), None); // too few samples
    }

    #[test]
    fn shortcuts_read_per_os() {
        assert_eq!(pretty_shortcut("Alt+Super+KeyL", true), "⌥⌘L");
        assert_eq!(pretty_shortcut("Control+Alt+Shift+KeyL", false), "Ctrl+Alt+Shift+L");
    }

    #[test]
    fn term_apps() {
        if cfg!(target_os = "macos") {
            assert_eq!(term_app("com.todesktop.230313mzl4w4u92"), Some("Cursor"));
            assert_eq!(term_app("vscode"), Some("VS Code"));
            assert_eq!(term_app("com.example.Unknown"), Some("its app"));
            assert_eq!(term_app("xterm"), None);
        } else {
            assert_eq!(term_app("vscode"), None);
        }
    }
}
