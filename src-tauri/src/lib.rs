mod agents;
mod alerts;
mod decide;
mod power;
mod settings;
mod stats;
mod tray;
mod usage;

use decide::{decide, Inputs, Reason};
use serde::Serialize;
use settings::{now, DisplayOff, Settings};
use std::{
    collections::{HashMap, VecDeque},
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
use tauri::{
    menu::{Menu, MenuItem},
    AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

const TICK: Duration = Duration::from_secs(2);
/// Battery samples used for the time-to-cutoff estimate.
const BATTERY_WINDOW_SECS: u64 = 15 * 60;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    pub(crate) reason: Reason,
    pub(crate) held: bool,
    pub(crate) held_secs: u64,
    pub(crate) lid_proof: bool,
    pub(crate) battery: Option<u8>,
    pub(crate) on_ac: bool,
    pub(crate) thermal: Option<u8>,
    pub(crate) low_power: bool,
    pub(crate) lid_closed: bool,
    pub(crate) working: usize,
    /// Sessions waiting on a permission prompt (and still counted).
    pub(crate) needs_you: usize,
    /// Minutes until the battery reaches the cut-off at the current drain, when known.
    pub(crate) battery_eta_mins: Option<u32>,
    pub(crate) sessions: Vec<agents::Session>,
    pub(crate) process_agents: Vec<String>,
    pub(crate) paused_until: u64,
    /// "Keep awake" end time (0 = off, FOREVER = until turned off).
    pub(crate) manual_until: u64,
    /// One-shot "Sleep when agents finish".
    pub(crate) sleep_when_done: bool,
    /// Today so far: agent working seconds (all agents added up) and time kept awake.
    pub(crate) today_agent_secs: u64,
    pub(crate) today_held_secs: u64,
    pub(crate) platform: power::Platform,
    /// 5-hour and weekly limits per signed-in provider.
    pub(crate) usage: Vec<usage::Limit>,
}

struct Core {
    settings: Settings,
    held_since: Option<Instant>,
    hold: Option<power::Hold>,
    /// Whether the current hold also keeps the display on.
    hold_display: bool,
    granted: bool,
    lid_closed: bool,
    finished_at: Option<Instant>,
    status: Option<Status>,
    /// Sessions the user stopped counting, by key → the `last_event` they were dismissed at.
    dismissed: HashMap<String, u64>,
    /// (unix secs, percent) while on battery, for the time-to-cutoff estimate.
    battery_samples: VecDeque<(u64, u8)>,
    alerts: alerts::Memory,
    /// "Sleep when agents finish": one-shot, deliberately not saved.
    sleep_when_done: bool,
    stats: stats::Tracker,
    usage: Vec<usage::Limit>,
}

struct AppState {
    core: Mutex<Core>,
    /// Separate from `core`: menu calls hop to the main thread, which may want `core`.
    tray: Mutex<tray::TrayState>,
    kick: Mutex<mpsc::Sender<()>>,
}

impl AppState {
    fn core(&self) -> std::sync::MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Re-run the loop now instead of waiting for the next tick.
    fn kick(&self) {
        let _ = self.kick.lock().map(|k| k.send(()));
    }
}

fn release(c: &mut Core) {
    if let Some(h) = c.hold.take() {
        h.release();
    }
    c.held_since = None;
}

/// Body of `<exe> --watchdog <pid>` (see `main.rs`).
pub fn watchdog(pid: u32) {
    power::run_watchdog(pid);
}

/// One pass: gather inputs (no lock held — some of these hop to the main thread), decide,
/// apply transitions under the lock, then update UI.
fn tick(app: &AppHandle, sys: &mut System) {
    let running = agents::running(sys);
    let stale_secs = u64::from(app.state::<AppState>().core().settings.release_quiet_after_mins.max(10)) * 60;
    let sessions = agents::scan_sessions(&running, stale_secs);
    let (battery, on_ac) = power::battery();
    let thermal = power::thermal();
    let low_power = power::low_power();
    let lid = power::lid_closed();
    let monitors = app.available_monitors().map(|m| m.len()).unwrap_or(1);
    // With the lid shut the built-in panel is offline, so any monitor left is external.
    let external = monitors > usize::from(!lid);

    let st = app.state::<AppState>();
    let mut sessions = sessions;
    let (status, note) = {
        let mut c = st.core();
        let t = now();
        let mut manual_ended = false;
        if c.settings.manual_until > 0 && c.settings.manual_until <= t {
            c.settings.manual_until = 0;
            let _ = settings::save(&c.settings);
            manual_ended = true;
        }
        let s = c.settings.clone();
        // A dismissal lasts until the session's next event.
        c.dismissed.retain(|k, at| sessions.iter().any(|x| tray::key(x) == *k && x.last_event == *at));
        for x in &mut sessions {
            x.dismissed = c.dismissed.contains_key(&tray::key(x));
        }
        let process_agents: Vec<String> =
            s.process_agents.iter().filter(|n| running.names.contains(&n.to_lowercase())).cloned().collect();
        let counted = || sessions.iter().filter(|x| x.active() && !x.dismissed);
        let working = counted().count() + process_agents.len();
        let needs_you = counted().filter(|x| x.state == "waiting").count();

        match battery {
            Some(pct) if !on_ac => {
                if c.battery_samples.back().is_none_or(|&(at, _)| t.saturating_sub(at) >= 60) {
                    c.battery_samples.push_back((t, pct));
                }
                while c.battery_samples.front().is_some_and(|&(at, _)| t.saturating_sub(at) > BATTERY_WINDOW_SECS) {
                    c.battery_samples.pop_front();
                }
            }
            _ => c.battery_samples.clear(),
        }
        let battery_eta_mins = tray::minutes_to(s.battery_cutoff, c.battery_samples.make_contiguous());
        let session_alerts = alerts::due(&sessions, &mut c.alerts, &s);
        let announced_finish = session_alerts.iter().any(|a| a.finished);
        let reason = decide(&Inputs {
            enabled: s.enabled,
            paused: s.paused_until > now(),
            working,
            manual: s.manual_until > t,
            thermal: thermal.unwrap_or(0),
            thermal_limit: s.thermal_limit,
            low_power,
            respect_low_power: s.respect_low_power,
            only_plugged_in: s.only_plugged_in,
            on_ac,
            battery,
            cutoff: s.battery_cutoff,
        });
        let hold = reason == Reason::Holding;
        let mut note: Option<(&str, String)> = None;
        let mut sleep_after = false;
        let device = power::DEVICE;

        let keep_display = s.keep_display_on && s.display_off != DisplayOff::WhileAgentsRun;
        if hold && c.held_since.is_some() && c.hold_display != keep_display {
            // The screen setting changed mid-hold: swap the hold, keep the session going.
            if let Some(h) = c.hold.take() {
                h.release();
            }
            c.hold = Some(power::hold(c.granted, keep_display));
            c.hold_display = keep_display;
        }
        if hold && c.held_since.is_none() {
            c.hold = Some(power::hold(c.granted, keep_display));
            c.hold_display = keep_display;
            c.held_since = Some(Instant::now());
            c.finished_at = None;
            if s.display_off == DisplayOff::WhileAgentsRun {
                power::display_sleep_now();
            }
            if s.notify_engage {
                note = Some(("Keeping your computer awake", format!("{working} agent{} working.", plural(working))));
            }
        } else if !hold && c.held_since.is_some() {
            release(&mut c);
            c.finished_at = Some(Instant::now());
            note = match reason {
                Reason::Battery if s.notify_battery => Some((
                    "Battery limit reached",
                    format!("Stopped at {}%. Your {device} can sleep now.", battery.unwrap_or(0)),
                )),
                Reason::Thermal => Some(("Running hot", format!("Letting your {device} sleep until it cools down."))),
                Reason::NoAgents if manual_ended && s.notify_finish => {
                    Some(("Keep-awake ended", format!("Your {device} can sleep now.")))
                }
                // A per-session "finished" alert already said it.
                Reason::NoAgents if s.notify_finish && !announced_finish => {
                    Some(("Agents finished", format!("Your {device} can sleep now.")))
                }
                _ => None,
            };
            if reason == Reason::NoAgents && c.sleep_when_done {
                c.sleep_when_done = false;
                // Only when nobody is at the keyboard: never sleep under someone typing.
                if power::user_idle_secs() >= 60 {
                    sleep_after = true;
                    note = Some(("Agents finished", format!("Putting your {device} to sleep, as you asked.")));
                } else {
                    note = Some(("Agents finished", format!("Not sleeping: you're using your {device}.")));
                }
            }
            if lid && !external {
                power::sleep_now();
            }
        }

        // Lid just closed while we're holding: nothing else will lock or blank the screen.
        if lid && !c.lid_closed && c.held_since.is_some() && !external {
            if s.lock_on_lid_close {
                power::lock_screen();
            }
            if s.display_off == DisplayOff::OnLidClose {
                power::display_sleep_now();
            }
        }
        c.lid_closed = lid;

        if s.display_off == DisplayOff::AfterFinish {
            let wait = u64::from(s.display_off_after_secs);
            if c.finished_at.is_some_and(|t| t.elapsed().as_secs() >= wait) {
                c.finished_at = None;
                if power::user_idle_secs() >= wait {
                    power::display_sleep_now();
                }
            }
        }

        let status = Status {
            reason,
            held: c.held_since.is_some(),
            held_secs: c.held_since.map_or(0, |t| t.elapsed().as_secs()),
            lid_proof: c.granted,
            battery,
            on_ac,
            thermal,
            low_power,
            lid_closed: lid,
            working,
            needs_you,
            battery_eta_mins,
            sessions,
            process_agents,
            paused_until: s.paused_until,
            manual_until: s.manual_until,
            sleep_when_done: c.sleep_when_done,
            today_agent_secs: 0,
            today_held_secs: 0,
            platform: power::platform(),
            usage: c.usage.clone(),
        };
        let mut status = status;
        c.stats.tick(t, &stats::today(), &status.sessions, &status.process_agents, status.held, battery, on_ac);
        status.today_agent_secs = c.stats.day().agent_secs.values().sum();
        status.today_held_secs = c.stats.day().held_secs;
        c.status = Some(status.clone());
        (status, (note, session_alerts, sleep_after))
    };
    let (note, session_alerts, sleep_after) = note;

    let banners: Vec<(String, String)> = note
        .map(|(t, b)| (t.to_string(), b))
        .into_iter()
        .chain(session_alerts.into_iter().map(|a| (a.title, a.body)))
        .collect();
    if !banners.is_empty() {
        for (title, body) in &banners {
            let _ = app.notification().builder().title(title).body(body).show();
        }
        power::play_sound(&st.core().settings.sound);
    }
    if sleep_after {
        std::thread::sleep(Duration::from_secs(2)); // let the banner and sound land first
        power::sleep_now();
    }
    update_tray(app, &status);
    let _ = app.emit("status", &status);
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn update_tray(app: &AppHandle, s: &Status) {
    let st = app.state::<AppState>();
    let set = st.core().settings.clone();
    // Only look at agent configs when there's nothing to show (the "Connect your agents…" nudge).
    let connected = !s.sessions.is_empty()
        || !s.process_agents.is_empty()
        || agents::statuses().iter().any(|a| a.state == "installed");
    let mut t = st.tray.lock().unwrap_or_else(|e| e.into_inner());
    tray::update(app, &mut t, s, &set, connected);
}

fn update_settings(app: &AppHandle, f: impl FnOnce(&mut Settings)) {
    let st = app.state::<AppState>();
    let s = {
        let mut c = st.core();
        f(&mut c.settings);
        let _ = settings::save(&c.settings);
        c.settings.clone()
    };
    let _ = app.emit("settings", &s);
    st.kick();
}

fn open_settings(app: &AppHandle) {
    open_settings_at(app, "");
}

/// Opens Settings on a tab ("activity", "agents"; "" = where it was).
fn open_settings_at(app: &AppHandle, tab: &str) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        if !tab.is_empty() {
            let _ = app.emit_to("settings", "navigate", tab);
        }
        return;
    }
    let url = if tab.is_empty() { "index.html".to_string() } else { format!("index.html#{tab}") };
    let w = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(url.into()))
        .title("Agents Don't Sleep")
        .inner_size(720.0, 600.0)
        .min_inner_size(640.0, 480.0)
        .center()
        .build();
    if let Ok(w) = w {
        let _ = w.set_focus();
    }
}

fn grant_permission(app: &AppHandle) -> Result<(), String> {
    let res = power::install_grant();
    let st = app.state::<AppState>();
    st.core().granted = power::has_grant();
    st.kick();
    res
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "toggle" => update_settings(app, |s| s.enabled = !s.enabled),
        "pause30" => update_settings(app, |s| s.paused_until = now() + 30 * 60),
        "pause60" => update_settings(app, |s| s.paused_until = now() + 60 * 60),
        "pauseinf" => update_settings(app, |s| s.paused_until = settings::FOREVER),
        "keep30" => update_settings(app, |s| s.manual_until = now() + 30 * 60),
        "keep60" => update_settings(app, |s| s.manual_until = now() + 60 * 60),
        "keep120" => update_settings(app, |s| s.manual_until = now() + 2 * 60 * 60),
        "keepinf" => update_settings(app, |s| s.manual_until = settings::FOREVER),
        "keepstop" => update_settings(app, |s| s.manual_until = 0),
        "sleepdone" => {
            let st = app.state::<AppState>();
            let on = {
                let mut c = st.core();
                c.sleep_when_done = !c.sleep_when_done;
                c.sleep_when_done
            };
            let _ = app.emit("sleep-when-done", on);
            st.kick();
        }
        "resume" => update_settings(app, |s| s.paused_until = 0),
        "grant" => {
            let app = app.clone();
            std::thread::spawn(move || grant_permission(&app));
        }
        "settings" | "status" => open_settings(app),
        "connect" => open_settings_at(app, "agents"),
        "today" => open_settings_at(app, "activity"),
        "quit" => app.exit(0),
        _ => session_action(app, id),
    }
}

/// `open:<key>`, `term:<key>`, `dismiss:<key>`, `undismiss:<key>` from a session's submenu.
fn session_action(app: &AppHandle, id: &str) {
    let Some((action, key)) = id.split_once(':') else { return };
    let st = app.state::<AppState>();
    let mut c = st.core();
    let Some(x) = c.status.as_ref().and_then(|s| s.sessions.iter().find(|x| tray::key(x) == key)).cloned() else {
        return;
    };
    match action {
        "open" => power::open_path(&x.cwd),
        "term" => power::activate_app(&x.term),
        "dismiss" => {
            c.dismissed.insert(key.to_string(), x.last_event);
        }
        "undismiss" => {
            c.dismissed.remove(key);
        }
        _ => return,
    }
    drop(c);
    st.kick();
}

fn register_shortcut(app: &AppHandle, accel: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let sc: Shortcut = accel.parse().map_err(|e| format!("Invalid shortcut {accel:?}: {e}"))?;
    gs.on_shortcut(sc, |app, _, ev| {
        if ev.state() == ShortcutState::Pressed {
            update_settings(app, |s| s.enabled = !s.enabled);
        }
    })
    .map_err(|e| e.to_string())
}

fn sync_autostart(app: &AppHandle, on: bool) {
    // Dev builds would register the debug binary as a login item.
    if cfg!(debug_assertions) {
        return;
    }
    let al = app.autolaunch();
    let _ = if on { al.enable() } else { al.disable() };
}

// ---- commands (the React settings window) ----

#[tauri::command]
fn get_settings(st: State<AppState>) -> Settings {
    st.core().settings.clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, st: State<AppState>, mut settings: Settings) -> Result<(), String> {
    let old = st.core().settings.clone();
    settings.first_run = old.first_run; // owned by Rust, never by the form
    settings.battery_cutoff = settings.battery_cutoff.clamp(5, 50);
    settings.thermal_limit = settings.thermal_limit.clamp(2, 3);
    if settings.shortcut != old.shortcut {
        if let Err(e) = register_shortcut(&app, &settings.shortcut) {
            let _ = register_shortcut(&app, &old.shortcut);
            return Err(e);
        }
    }
    if settings.launch_at_login != old.launch_at_login {
        sync_autostart(&app, settings.launch_at_login);
    }
    update_settings(&app, |s| *s = settings);
    Ok(())
}

#[tauri::command]
fn get_status(st: State<AppState>) -> Option<Status> {
    st.core().status.clone()
}

#[tauri::command]
fn agents_status() -> Vec<agents::AgentStatus> {
    agents::statuses()
}

#[tauri::command]
async fn install_agent(id: String) -> Result<(), String> {
    agents::install(&id)
}

#[tauri::command]
async fn uninstall_agent(id: String) -> Result<(), String> {
    agents::uninstall(&id)
}

/// Disconnects every agent (the step before uninstalling the app).
#[tauri::command]
async fn remove_all_integrations() -> Result<(), String> {
    let errors: Vec<String> = agents::statuses()
        .iter()
        .filter(|a| a.state == "installed")
        .filter_map(|a| agents::uninstall(a.id).err().map(|e| format!("{}: {e}", a.name)))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[tauri::command]
async fn install_grant(app: AppHandle) -> Result<(), String> {
    grant_permission(&app)
}

#[tauri::command]
async fn uninstall_grant(app: AppHandle) -> Result<(), String> {
    let st = app.state::<AppState>();
    release(&mut st.core());
    let res = power::uninstall_grant();
    st.core().granted = power::has_grant();
    st.kick();
    res
}

/// The last `n` days of activity (oldest first), today from memory so it's current.
#[tauri::command]
fn stats_days(st: State<AppState>, n: i64) -> Vec<stats::Day> {
    let mut days = stats::days(n.clamp(1, 30));
    if let Some(last) = days.last_mut() {
        *last = st.core().stats.day().clone();
    }
    days
}

#[tauri::command]
fn sounds() -> Vec<String> {
    power::sounds()
}

#[tauri::command]
fn preview_sound(name: String) {
    power::play_sound(&name);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (kick_tx, kick_rx) = mpsc::channel::<()>();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_status,
            agents_status,
            install_agent,
            uninstall_agent,
            remove_all_integrations,
            install_grant,
            uninstall_grant,
            sounds,
            preview_sound,
            stats_days
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            power::restore(); // clear anything a crash or reboot left behind
            power::init();
            power::spawn_watchdog();
            power::disable_app_nap();

            let mut s = settings::load();
            let first_launch = s.first_run == 0;
            if first_launch {
                s.first_run = now();
            }
            stats::prune();
            let _ = agents::install_hook_binary();
            if agents::refresh_integrations(s.hooks_version) {
                s.hooks_version = agents::HOOKS_VERSION;
            }
            let _ = settings::save(&s);
            let shortcut = s.shortcut.clone();
            let autostart = s.launch_at_login;
            app.manage(AppState {
                core: Mutex::new(Core {
                    settings: s,
                    held_since: None,
                    hold: None,
                    hold_display: false,
                    granted: power::has_grant(),
                    lid_closed: power::lid_closed(),
                    finished_at: None,
                    status: None,
                    dismissed: HashMap::new(),
                    battery_samples: VecDeque::new(),
                    alerts: alerts::Memory::default(),
                    sleep_when_done: false,
                    stats: stats::Tracker::load(now()),
                    usage: vec![],
                }),
                tray: Mutex::new(tray::TrayState::default()),
                kick: Mutex::new(kick_tx),
            });

            let handle = app.handle().clone();
            tauri::tray::TrayIconBuilder::with_id("main")
                .icon(tray::icon(tray::Look::Idle))
                .icon_as_template(tray::TEMPLATE_ICONS)
                .show_menu_on_left_click(true)
                .menu(&Menu::with_items(
                    &handle,
                    &[&MenuItem::with_id(&handle, "settings", "Settings…", true, None::<&str>)?],
                )?)
                .on_menu_event(|app, e| on_menu(app, e.id.as_ref()))
                .build(app)?;

            let _ = register_shortcut(&handle, &shortcut);
            sync_autostart(&handle, autostart);
            if first_launch {
                open_settings(&handle); // onboarding: grant permission, connect agents
            }

            let usage_handle = handle.clone();
            std::thread::spawn(move || {
                // ponytail: fixed 5-minute poll; poll faster while agents work if it feels stale.
                let mut poller = usage::Poller::default();
                loop {
                    let u = poller.poll();
                    let st = usage_handle.state::<AppState>();
                    st.core().usage = u;
                    st.kick();
                    std::thread::sleep(Duration::from_secs(5 * 60));
                }
            });

            std::thread::spawn(move || {
                let mut sys = System::new();
                loop {
                    tick(&handle, &mut sys);
                    let _ = kick_rx.recv_timeout(TICK);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, ev| match ev {
        // Closing the settings window must not quit a menu-bar app.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            let st = app.state::<AppState>();
            let mut c = st.core();
            release(&mut c);
            c.stats.flush(now());
        }
        // Launching the app again while it runs (Finder, Spotlight) opens Settings.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_settings(app),
        _ => {}
    });
}
