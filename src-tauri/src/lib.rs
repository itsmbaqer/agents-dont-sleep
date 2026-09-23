mod agents;
mod decide;
mod license;
mod power;
mod settings;

use decide::{decide, Inputs, Reason};
use serde::Serialize;
use settings::{now, DisplayOff, Settings};
use std::{
    process::Child,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem},
    AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

const TICK: Duration = Duration::from_secs(2);
const ICON_AWAKE: &[u8] = include_bytes!("../icons/tray-awake.png");
const ICON_IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Status {
    reason: Reason,
    held: bool,
    held_secs: u64,
    lid_proof: bool,
    battery: Option<u8>,
    on_ac: bool,
    thermal: u8,
    low_power: bool,
    lid_closed: bool,
    working: usize,
    sessions: Vec<agents::Session>,
    process_agents: Vec<String>,
    paused_until: u64,
    license: license::LicenseInfo,
}

struct Core {
    settings: Settings,
    held_since: Option<Instant>,
    caff: Option<Child>,
    sudo_ok: bool,
    lid_closed: bool,
    finished_at: Option<Instant>,
    status: Option<Status>,
    menu_key: String,
}

struct AppState {
    core: Mutex<Core>,
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
    if c.sudo_ok {
        power::set_sleep_disabled(false);
    }
    if let Some(mut child) = c.caff.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    c.held_since = None;
}

/// One pass: gather inputs (no lock held — some of these hop to the main thread), decide,
/// apply transitions under the lock, then update UI.
fn tick(app: &AppHandle, sys: &mut System) {
    let running = agents::running_names(sys);
    let sessions = agents::scan_sessions(&running);
    let (battery, on_ac) = power::battery();
    let thermal = power::thermal_state();
    let low_power = power::low_power_mode();
    let lid = power::lid_closed();
    let monitors = app.available_monitors().map(|m| m.len()).unwrap_or(1);
    // With the lid shut the built-in panel is offline, so any monitor left is external.
    let external = monitors > usize::from(!lid);

    let st = app.state::<AppState>();
    let (status, note) = {
        let mut c = st.core();
        let s = c.settings.clone();
        let process_agents: Vec<String> =
            s.process_agents.iter().filter(|n| running.contains(&n.to_lowercase())).cloned().collect();
        let working = sessions.iter().filter(|x| x.active()).count() + process_agents.len();
        let reason = decide(&Inputs {
            licensed: license::allowed(&s),
            enabled: s.enabled,
            paused: s.paused_until > now(),
            working,
            thermal,
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

        if hold && c.held_since.is_none() {
            if c.sudo_ok {
                power::set_sleep_disabled(true);
            }
            c.caff = power::caffeinate();
            c.held_since = Some(Instant::now());
            c.finished_at = None;
            if s.display_off == DisplayOff::WhileAgentsRun {
                power::display_sleep_now();
            }
            if s.notify_engage {
                note = Some(("Keeping your Mac awake", format!("{working} agent{} working.", plural(working))));
            }
        } else if !hold && c.held_since.is_some() {
            release(&mut c);
            c.finished_at = Some(Instant::now());
            note = match reason {
                Reason::Battery if s.notify_battery => Some((
                    "Battery limit reached",
                    format!("Stopped at {}%. Your Mac can sleep now.", battery.unwrap_or(0)),
                )),
                Reason::Thermal => Some(("Your Mac is running hot", "Letting it sleep until it cools down.".into())),
                Reason::NoAgents if s.notify_finish => Some(("Agents finished", "Your Mac can sleep now.".into())),
                _ => None,
            };
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
            lid_proof: c.sudo_ok,
            battery,
            on_ac,
            thermal,
            low_power,
            lid_closed: lid,
            working,
            sessions,
            process_agents,
            paused_until: s.paused_until,
            license: license::info(&s),
        };
        c.status = Some(status.clone());
        (status, note)
    };

    if let Some((title, body)) = note {
        let s = st.core().settings.clone();
        let _ = app.notification().builder().title(title).body(body).show();
        power::play_sound(&s.sound);
    }
    update_tray(app, &status);
    let _ = app.emit("status", &status);
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn hhmm(ts: u64) -> String {
    let t = ts as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// "Alt+Super+KeyL" → "⌥⌘L"
fn pretty_shortcut(s: &str) -> String {
    s.split('+')
        .map(|t| match t.to_lowercase().as_str() {
            "alt" | "option" => "⌥".to_string(),
            "super" | "cmd" | "command" => "⌘".into(),
            "shift" => "⇧".into(),
            "ctrl" | "control" => "⌃".into(),
            _ => t.trim_start_matches("Key").trim_start_matches("Digit").to_uppercase(),
        })
        .collect()
}

fn status_line(s: &Status, set: &Settings) -> String {
    match s.reason {
        Reason::Holding => format!(
            "Awake — {} · {}h {:02}m",
            if s.lid_proof { "lid-proof" } else { "lid open only" },
            s.held_secs / 3600,
            s.held_secs / 60 % 60
        ),
        Reason::NoAgents => "Idle — your Mac can sleep normally".into(),
        Reason::Paused => format!("Paused until {}", hhmm(s.paused_until)),
        Reason::Disabled => format!("Off — press {} to turn on", pretty_shortcut(&set.shortcut)),
        Reason::Battery => format!("Held off — battery {}% (limit {}%)", s.battery.unwrap_or(0), set.battery_cutoff),
        Reason::Thermal => "Held off — your Mac is running hot".into(),
        Reason::LowPower => "Held off — Low Power Mode is on".into(),
        Reason::NotPluggedIn => "Held off — not plugged in".into(),
        Reason::Unlicensed => "Trial ended — add a license in Settings".into(),
    }
}

/// Labels for the whole menu; the menu is rebuilt only when these change (≈ once a minute).
fn menu_labels(s: &Status, set: &Settings) -> Vec<(String, String, bool)> {
    let mut v: Vec<(String, String, bool)> = vec![];
    let mut info = |label: String| v.push((String::new(), label, false));
    info(status_line(s, set));
    info(match s.battery {
        Some(b) => format!("Battery {b}%{} · stops below {}%", if s.on_ac { " (on power)" } else { "" }, set.battery_cutoff),
        None => "On power adapter".into(),
    });
    info("-".into());
    info(match s.working {
        0 => "No agents working".into(),
        n => format!("Agents · {n} working"),
    });
    for x in &s.sessions {
        let dot = match x.state.as_str() {
            "working" => "●",
            "waiting" => "◐",
            _ => "○",
        };
        let project = if x.project.is_empty() { String::new() } else { format!(" — {}", x.project) };
        let waiting = if x.state == "waiting" { " (waiting for you)" } else { "" };
        info(format!("    {dot} {}{project}{waiting}", x.name));
    }
    for p in &s.process_agents {
        info(format!("    ● {p} (running)"));
    }
    v.push(("-".into(), "-".into(), false));
    v.push(("toggle".into(), format!("Enabled|{}", set.enabled), true));
    if s.reason == Reason::Paused {
        v.push(("resume".into(), "Resume".into(), true));
    } else {
        v.push(("pause30".into(), "Pause for 30 minutes".into(), true));
        v.push(("pause60".into(), "Pause for 1 hour".into(), true));
    }
    if !s.lid_proof {
        v.push(("grant".into(), "Allow lid-closed awake…".into(), true));
    }
    v.push(("-".into(), "-".into(), false));
    v.push(("settings".into(), "Settings…".into(), true));
    v.push(("quit".into(), "Quit Agents Don't Sleep".into(), true));
    v
}

fn build_menu(app: &AppHandle, labels: &[(String, String, bool)], set: &Settings) -> tauri::Result<Menu<Wry>> {
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = vec![];
    for (i, (id, label, enabled)) in labels.iter().enumerate() {
        if label == "-" {
            items.push(Box::new(PredefinedMenuItem::separator(app)?));
        } else if id == "toggle" {
            let accel = set.shortcut.replace("Super", "Cmd");
            let checked = set.enabled;
            let item = CheckMenuItem::with_id(app, "toggle", "Enabled", true, checked, Some(accel.as_str()))
                .or_else(|_| CheckMenuItem::with_id(app, "toggle", "Enabled", true, checked, None::<&str>))?;
            items.push(Box::new(item));
        } else {
            let accel = match id.as_str() {
                "settings" => Some("Cmd+,"),
                "quit" => Some("Cmd+Q"),
                _ => None,
            };
            let id = if id.is_empty() { format!("info{i}") } else { id.clone() };
            items.push(Box::new(MenuItem::with_id(app, id, label, *enabled, accel)?));
        }
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

fn update_tray(app: &AppHandle, s: &Status) {
    let st = app.state::<AppState>();
    let set = st.core().settings.clone();
    let labels = menu_labels(s, &set);
    let key = format!("{labels:?}{}", s.held);
    {
        let mut c = st.core();
        if c.menu_key == key {
            return;
        }
        c.menu_key = key;
    }
    let Some(tray) = app.tray_by_id("main") else { return };
    match build_menu(app, &labels, &set) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => eprintln!("tray menu: {e}"),
    }
    let icon = if s.held { ICON_AWAKE } else { ICON_IDLE };
    if let Ok(img) = Image::from_bytes(icon) {
        let _ = tray.set_icon_with_as_template(Some(img), true);
    }
    let _ = tray.set_tooltip(Some(status_line(s, &set)));
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
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let w = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
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
    let res = power::install_sudoers();
    let st = app.state::<AppState>();
    st.core().sudo_ok = power::has_sudoers();
    st.kick();
    res
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "toggle" => update_settings(app, |s| s.enabled = !s.enabled),
        "pause30" => update_settings(app, |s| s.paused_until = now() + 30 * 60),
        "pause60" => update_settings(app, |s| s.paused_until = now() + 60 * 60),
        "resume" => update_settings(app, |s| s.paused_until = 0),
        "grant" => {
            let app = app.clone();
            std::thread::spawn(move || grant_permission(&app));
        }
        "settings" => open_settings(app),
        "quit" => app.exit(0),
        _ => {}
    }
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
    // License state and trial start are owned by Rust, never by the form.
    settings.first_run = old.first_run;
    settings.license_key = old.license_key.clone();
    settings.activation_id = old.activation_id.clone();
    settings.license_ok = old.license_ok;
    settings.license_checked_at = old.license_checked_at;
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

#[tauri::command]
async fn install_sudoers(app: AppHandle) -> Result<(), String> {
    grant_permission(&app)
}

#[tauri::command]
async fn uninstall_sudoers(app: AppHandle) -> Result<(), String> {
    let st = app.state::<AppState>();
    release(&mut st.core());
    let res = power::uninstall_sudoers();
    st.core().sudo_ok = power::has_sudoers();
    st.kick();
    res
}

#[tauri::command]
fn sounds() -> Vec<String> {
    power::sounds()
}

#[tauri::command]
fn preview_sound(name: String) {
    power::play_sound(&name);
}

#[tauri::command]
async fn activate_license(app: AppHandle, key: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let mut s = st.core().settings.clone(); // network call happens without the lock
    license::activate(&mut s, &key)?;
    update_settings(&app, |cur| {
        cur.license_key = s.license_key;
        cur.activation_id = s.activation_id;
        cur.license_ok = s.license_ok;
        cur.license_checked_at = s.license_checked_at;
    });
    Ok(())
}

#[tauri::command]
async fn deactivate_license(app: AppHandle) {
    let st = app.state::<AppState>();
    let mut s = st.core().settings.clone();
    license::deactivate(&mut s);
    update_settings(&app, |cur| {
        cur.license_key.clear();
        cur.activation_id.clear();
        cur.license_ok = false;
    });
}

#[tauri::command]
fn open_url(url: String) {
    if url.starts_with("https://") {
        let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    }
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
            install_sudoers,
            uninstall_sudoers,
            sounds,
            preview_sound,
            activate_license,
            deactivate_license,
            open_url
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let _ = agents::install_hook_script();
            let sudo_ok = power::has_sudoers();
            if sudo_ok {
                power::set_sleep_disabled(false); // clear anything a crash or reboot left behind
            }
            power::spawn_watchdog();
            power::disable_app_nap();

            let mut s = settings::load();
            let first_launch = s.first_run == 0;
            if first_launch {
                s.first_run = now();
                let _ = settings::save(&s);
            }
            let shortcut = s.shortcut.clone();
            let autostart = s.launch_at_login;
            app.manage(AppState {
                core: Mutex::new(Core {
                    settings: s,
                    held_since: None,
                    caff: None,
                    sudo_ok,
                    lid_closed: power::lid_closed(),
                    finished_at: None,
                    status: None,
                    menu_key: String::new(),
                }),
                kick: Mutex::new(kick_tx),
            });

            let handle = app.handle().clone();
            tauri::tray::TrayIconBuilder::with_id("main")
                .icon(Image::from_bytes(ICON_IDLE)?)
                .icon_as_template(true)
                .show_menu_on_left_click(true)
                .menu(&Menu::with_items(&handle, &[&MenuItem::with_id(&handle, "settings", "Settings…", true, None::<&str>)?])?)
                .on_menu_event(|app, e| on_menu(app, e.id.as_ref()))
                .build(app)?;

            let _ = register_shortcut(&handle, &shortcut);
            sync_autostart(&handle, autostart);
            if first_launch {
                open_settings(&handle); // onboarding: grant permission, connect agents
            }

            std::thread::spawn(move || {
                {
                    let st = handle.state::<AppState>();
                    let mut s = st.core().settings.clone();
                    if license::revalidate(&mut s) {
                        update_settings(&handle, |cur| {
                            cur.license_ok = s.license_ok;
                            cur.license_checked_at = s.license_checked_at;
                        });
                    }
                }
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
        RunEvent::Exit => release(&mut app.state::<AppState>().core()),
        // Launching the app again while it runs (Finder, Spotlight) opens Settings.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_settings(app),
        _ => {}
    });
}
