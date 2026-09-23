use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// `~/.agents-dont-sleep` — no spaces on purpose: Hermes runs hook commands via shlex, no shell.
pub fn data_dir() -> PathBuf {
    home().join(".agents-dont-sleep")
}

pub fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(std::env::temp_dir)
}

/// "Until turned off" for pause and keep-awake end times. Far future, but still exact as a
/// JavaScript number, so the settings window can round-trip it (u64::MAX can't).
pub const FOREVER: u64 = 9_999_999_999;

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum DisplayOff {
    Never,
    OnLidClose,
    WhileAgentsRun,
    AfterFinish,
}

/// Text beside the tray icon.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum TrayLabel {
    /// "Claude 2 · Codex 1"
    Full,
    /// "3"
    Count,
    Off,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub enabled: bool,
    pub paused_until: u64,
    pub battery_cutoff: u8,
    pub only_plugged_in: bool,
    pub respect_low_power: bool,
    /// NSProcessInfoThermalState raw value to stop at: 2 = serious, 3 = critical.
    pub thermal_limit: u8,
    pub display_off: DisplayOff,
    pub display_off_after_secs: u32,
    pub lock_on_lid_close: bool,
    /// Stop the display from dimming and sleeping while agents work (lid open).
    pub keep_display_on: bool,
    pub notify_engage: bool,
    pub notify_finish: bool,
    pub notify_battery: bool,
    /// macOS system sound name ("" = silent).
    pub sound: String,
    pub shortcut: String,
    pub launch_at_login: bool,
    /// Agents without hooks: counted as working while a process with this name runs.
    pub process_agents: Vec<String>,
    pub first_run: u64,
    /// `agents::HOOKS_VERSION` that connected agents' hooks were last written with.
    pub hooks_version: u32,
    pub tray_label: TrayLabel,
    /// A working session with no events for this long is flagged (0 = never).
    pub alert_stuck_mins: u32,
    /// Notify when a session has waited on you this long (0 = right away).
    pub alert_waiting: bool,
    pub alert_waiting_mins: u32,
    /// Notify when a turn at least this long finishes (0 = never).
    pub alert_long_turn_mins: u32,
    /// Notify when a turn stops with an error (rate limit, overloaded, …).
    pub alert_errors: bool,
    /// Stop counting a session after this long without any event.
    pub release_quiet_after_mins: u32,
    /// "Keep awake" from the tray: unix time it ends (0 = off, FOREVER = until turned off).
    pub manual_until: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            paused_until: 0,
            battery_cutoff: 15,
            only_plugged_in: false,
            respect_low_power: true,
            thermal_limit: 2,
            display_off: DisplayOff::OnLidClose,
            display_off_after_secs: 30,
            lock_on_lid_close: true,
            keep_display_on: true,
            notify_engage: false,
            notify_finish: true,
            notify_battery: true,
            sound: crate::power::default_sound().into(),
            // Win+L and Ctrl+Alt+L already lock the screen on Windows / most Linux desktops.
            shortcut: if cfg!(target_os = "macos") { "Alt+Super+KeyL" } else { "Control+Alt+Shift+KeyL" }.into(),
            launch_at_login: true,
            process_agents: ["aider", "goose", "cline", "conductor"].map(String::from).to_vec(),
            first_run: 0,
            hooks_version: 0,
            tray_label: TrayLabel::Full,
            alert_stuck_mins: 15,
            alert_waiting: true,
            alert_waiting_mins: 1,
            alert_long_turn_mins: 5,
            alert_errors: true,
            release_quiet_after_mins: 120,
            manual_until: 0,
        }
    }
}

fn path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn load() -> Settings {
    fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) -> std::io::Result<()> {
    write_atomic(&path(), serde_json::to_string_pretty(s)?.as_bytes())
}

pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ads-tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}
