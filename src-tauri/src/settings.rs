use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// `~/.agents-dont-sleep` — no spaces on purpose: Hermes runs hook commands via shlex, no shell.
pub fn data_dir() -> PathBuf {
    home().join(".agents-dont-sleep")
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum DisplayOff {
    Never,
    OnLidClose,
    WhileAgentsRun,
    AfterFinish,
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
    pub license_key: String,
    pub activation_id: String,
    pub license_ok: bool,
    pub license_checked_at: u64,
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
            notify_engage: false,
            notify_finish: true,
            notify_battery: true,
            sound: "Glass".into(),
            shortcut: "Alt+Super+KeyL".into(),
            launch_at_login: true,
            process_agents: ["aider", "goose", "cline", "conductor"]
                .map(String::from)
                .to_vec(),
            first_run: 0,
            license_key: String::new(),
            activation_id: String::new(),
            license_ok: false,
            license_checked_at: 0,
        }
    }
}

fn path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn load() -> Settings {
    fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
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
