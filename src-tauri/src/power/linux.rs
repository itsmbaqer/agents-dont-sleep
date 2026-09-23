//! Linux (systemd). Lid-closed awake = a logind block inhibitor on sleep, idle and the lid
//! switch, held by a `systemd-inhibit` child that exits with us (`tail --pid`). Allowed for
//! any user in an active local session, so no root. KDE's PowerDevil ignores other apps' lid
//! inhibitors, so on KDE the user's own lid action is set to "do nothing" and restored after.
use super::{output, quiet, sound_files, Supply, Zone};
use crate::settings::{data_dir, write_atomic};
use serde::{Deserialize, Serialize};
use std::process::{Child, Command, Stdio};

pub const OS: &str = "linux";
pub const DEVICE: &str = "computer";
pub const NEEDS_GRANT: bool = false;

const SOUNDS_DIR: &str = "/usr/share/sounds/freedesktop/stereo";
/// powerdevilrc groups whose lid action we override on KDE Plasma 6.
const KDE_GROUPS: [&str; 3] = ["AC", "Battery", "LowBattery"];

pub fn has_grant() -> bool {
    true
}
pub fn install_grant() -> Result<(), String> {
    Ok(())
}
pub fn uninstall_grant() -> Result<(), String> {
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Saved {
    /// Original powerdevilrc LidAction per group ("" = key was unset).
    kde_lid: Vec<(String, String)>,
}

fn restore_file() -> std::path::PathBuf {
    data_dir().join("restore.json")
}

fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE"))
}

fn kde_set(group: &str, value: &str) {
    let mut args =
        vec!["--file", "powerdevilrc", "--group", group, "--group", "SuspendAndShutdown", "--key", "LidAction"];
    if value.is_empty() {
        args.push("--delete");
    } else {
        args.push(value);
    }
    quiet("kwriteconfig6", &args);
}

fn kde_reload() {
    // ponytail: best effort; PowerDevil also re-reads its config on the next session start.
    quiet(
        "busctl",
        &[
            "--user",
            "call",
            "org.kde.Solid.PowerManagement",
            "/org/kde/Solid/PowerManagement",
            "org.kde.Solid.PowerManagement",
            "reparseConfiguration",
        ],
    );
}

pub struct Hold {
    inhibit: Option<Child>,
}

/// ponytail: `keep_display` is ignored — screen blanking is the desktop's (GNOME/KDE screensaver
/// D-Bus inhibit), not logind's; add org.freedesktop.ScreenSaver.Inhibit when someone asks.
pub fn hold(_granted: bool, _keep_display: bool) -> Hold {
    if is_kde() && !restore_file().exists() {
        let kde_lid = KDE_GROUPS
            .iter()
            .map(|g| {
                let v = output(
                    "kreadconfig6",
                    &["--file", "powerdevilrc", "--group", g, "--group", "SuspendAndShutdown", "--key", "LidAction"],
                );
                (g.to_string(), v.trim().to_string())
            })
            .collect();
        if let Ok(json) = serde_json::to_vec(&Saved { kde_lid }) {
            if write_atomic(&restore_file(), &json).is_ok() {
                KDE_GROUPS.iter().for_each(|g| kde_set(g, "0"));
                kde_reload();
            }
        }
    }
    let inhibit = Command::new("systemd-inhibit")
        .args([
            "--what=sleep:idle:handle-lid-switch",
            "--mode=block",
            "--who=Agents Don't Sleep",
            "--why=Coding agents are working",
            "tail",
            &format!("--pid={}", std::process::id()),
            "-f",
            "/dev/null",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    Hold { inhibit }
}

impl Hold {
    pub fn release(mut self) {
        if let Some(mut c) = self.inhibit.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        restore();
    }
}

/// Puts KDE's lid actions back (startup, release, watchdog). The inhibitor itself dies with us.
pub fn restore() {
    let Ok(bytes) = std::fs::read(restore_file()) else { return };
    if let Ok(s) = serde_json::from_slice::<Saved>(&bytes) {
        s.kde_lid.iter().for_each(|(g, v)| kde_set(g, v));
        kde_reload();
    }
    let _ = std::fs::remove_file(restore_file());
}

pub fn init() {}

fn read_trim(path: std::path::PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn battery() -> (Option<u8>, bool) {
    let supplies: Vec<Supply> = std::fs::read_dir("/sys/class/power_supply")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let p = e.path();
            Supply {
                kind: read_trim(p.join("type")).unwrap_or_default(),
                online: read_trim(p.join("online")),
                capacity: read_trim(p.join("capacity")),
                scope: read_trim(p.join("scope")),
            }
        })
        .collect();
    super::combine_supplies(&supplies)
}

pub fn low_power() -> bool {
    output("powerprofilesctl", &["get"]).trim() == "power-saver"
}

pub fn thermal() -> Option<u8> {
    let zones: Vec<Zone> = std::fs::read_dir("/sys/class/thermal")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("thermal_zone"))
        .filter_map(|e| {
            let p = e.path();
            let temp = read_trim(p.join("temp"))?.parse().ok()?;
            let trips = (0..16)
                .map_while(|i| {
                    Some((
                        read_trim(p.join(format!("trip_point_{i}_type")))?,
                        read_trim(p.join(format!("trip_point_{i}_temp")))?,
                    ))
                })
                .filter_map(|(kind, t)| Some((kind, t.parse().ok()?)))
                .collect();
            Some(Zone { temp, trips })
        })
        .collect();
    (!zones.is_empty()).then(|| super::thermal_level(&zones))
}

pub fn lid_closed() -> bool {
    let logind = output(
        "busctl",
        &[
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "LidClosed",
        ],
    );
    if !logind.trim().is_empty() {
        return logind.trim() == "b true";
    }
    std::fs::read_dir("/proc/acpi/button/lid")
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| read_trim(e.path().join("state")).is_some_and(|s| s.contains("closed")))
}

pub fn disable_app_nap() {}

/// From logind's session idle hint (set by GNOME/KDE); 0 when unknown.
pub fn user_idle_secs() -> u64 {
    let get = |prop: &str| {
        output(
            "busctl",
            &[
                "get-property",
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                prop,
            ],
        )
    };
    if get("IdleHint").trim() != "b true" {
        return 0;
    }
    let since_us: u64 = get("IdleSinceHint").trim().trim_start_matches("t ").parse().unwrap_or(0);
    (crate::settings::now() * 1_000_000).saturating_sub(since_us) / 1_000_000
}

/// Best effort across X11, KDE Wayland and GNOME Wayland.
pub fn display_sleep_now() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !wayland && quiet("xset", &["dpms", "force", "off"]) {
        return;
    }
    if quiet("kscreen-doctor", &["--dpms", "off"]) {
        return;
    }
    quiet(
        "busctl",
        &[
            "--user",
            "set-property",
            "org.gnome.Mutter.DisplayConfig",
            "/org/gnome/Mutter/DisplayConfig",
            "org.gnome.Mutter.DisplayConfig",
            "PowerSaveMode",
            "i",
            "3",
        ],
    );
}

pub fn sleep_now() {
    quiet("systemctl", &["suspend"]);
}

pub fn lock_screen() {
    quiet("loginctl", &["lock-session"]);
}

pub fn sounds() -> Vec<String> {
    sound_files(SOUNDS_DIR, ".oga")
}

pub fn default_sound() -> &'static str {
    "complete"
}

pub fn play_sound(name: &str) {
    if !name.is_empty() && sounds().iter().any(|s| s == name) {
        let path = format!("{SOUNDS_DIR}/{name}.oga");
        let spawned = Command::new("paplay").arg(&path).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        if spawned.is_err() {
            let _ = Command::new("pw-play").arg(&path).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        }
    }
}
