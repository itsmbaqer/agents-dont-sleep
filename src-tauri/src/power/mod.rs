//! Platform power control. Every OS module exposes the same API:
//!
//! - `OS`, `DEVICE`, `NEEDS_GRANT`, `has_grant`, `install_grant`, `uninstall_grant`
//! - `hold(granted, keep_display) -> Hold`, `Hold::release`, `restore` (undo anything a crash left behind)
//! - `battery`, `low_power`, `thermal`, `lid_closed`, `user_idle_secs`
//! - `lock_screen`, `display_sleep_now`, `sleep_now`
//! - `sounds`, `play_sound`, `default_sound`, `init`, `disable_app_nap`
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;

use serde::Serialize;
use std::process::{Command, Stdio};

/// What this OS supports, so the settings window only shows what works.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub os: &'static str,
    pub needs_grant: bool,
    pub thermal: bool,
}

pub fn platform() -> Platform {
    Platform { os: OS, needs_grant: NEEDS_GRANT, thermal: thermal().is_some() }
}

#[allow(dead_code)] // not every OS shells out
pub(crate) fn quiet(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[allow(dead_code)]
pub(crate) fn output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Sound names (file stems) in `dir` with extension `ext`, sorted.
#[allow(dead_code)]
pub(crate) fn sound_files(dir: &str, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(ext).map(String::from))
        .collect();
    v.sort();
    v
}

/// Opens a folder in Finder / Explorer / the file manager.
pub fn open_path(path: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if std::path::Path::new(path).is_dir() {
        let _ = Command::new(opener).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

/// Brings the agent's terminal app to the front (macOS): a bundle id (`com.microsoft.VSCode`)
/// or an app name. No-op elsewhere.
#[allow(unused_variables)]
pub fn activate_app(bundle_or_name: &str) {
    #[cfg(target_os = "macos")]
    {
        let flag = if bundle_or_name.contains('.') { "-b" } else { "-a" };
        let _ = Command::new("/usr/bin/open")
            .args([flag, bundle_or_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Re-launches this executable as `--watchdog <pid>`, detached. When the app dies for any
/// reason (even SIGKILL / End task), the watchdog runs `restore()` so a crash can't leave a
/// machine unable to sleep.
pub fn spawn_watchdog() {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = Command::new(exe);
    cmd.args(["--watchdog", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
    }
    let _ = cmd.spawn();
}

/// Body of the `--watchdog` process.
pub fn run_watchdog(pid: u32) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    loop {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
        if sys.process(pid).is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    restore();
}

/// (percent, on_ac) from `pmset -g batt` (macOS).
#[allow(dead_code)]
pub fn parse_pmset_batt(s: &str) -> (Option<u8>, bool) {
    let on_ac = s.contains("'AC Power'");
    let pct = s.split(|c: char| c.is_whitespace() || c == ';').find_map(|w| w.strip_suffix('%')?.parse().ok());
    (pct, on_ac)
}

/// One entry of /sys/class/power_supply (Linux).
#[allow(dead_code)]
pub struct Supply {
    pub kind: String,
    pub online: Option<String>,
    pub capacity: Option<String>,
    pub scope: Option<String>,
}

/// (percent, on_ac) from Linux power supplies. Peripheral batteries (mice, headsets) report
/// `scope=Device` and are ignored. No battery at all counts as "on power".
#[allow(dead_code)]
pub fn combine_supplies(items: &[Supply]) -> (Option<u8>, bool) {
    let system = |s: &&Supply| s.scope.as_deref().map(str::trim) != Some("Device");
    let batteries: Vec<u8> = items
        .iter()
        .filter(system)
        .filter(|s| s.kind.trim() == "Battery")
        .filter_map(|s| s.capacity.as_deref()?.trim().parse().ok())
        .collect();
    let mains = items
        .iter()
        .filter(system)
        .any(|s| s.kind.trim() != "Battery" && s.online.as_deref().map(str::trim) == Some("1"));
    let pct = (!batteries.is_empty())
        .then(|| (batteries.iter().map(|&b| u32::from(b)).sum::<u32>() / batteries.len() as u32) as u8);
    (pct, mains || pct.is_none())
}

/// A thermal zone: current temperature and its trip points (type, temperature), m°C.
#[allow(dead_code)]
pub struct Zone {
    pub temp: i64,
    pub trips: Vec<(String, i64)>,
}

/// NSProcessInfoThermalState-like level from Linux trip points: 2 = at a passive (throttling)
/// trip, 3 = at a hot/critical trip, 0 otherwise.
#[allow(dead_code)]
pub fn thermal_level(zones: &[Zone]) -> u8 {
    zones
        .iter()
        .flat_map(|z| z.trips.iter().filter(|(_, t)| *t > 0 && z.temp >= *t).map(|(kind, _)| kind.as_str()))
        .map(|kind| match kind {
            "hot" | "critical" => 3,
            "passive" => 2,
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmset_batt() {
        let on_batt = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=7077987)\t70%; discharging; 5:07 remaining present: true\n";
        assert_eq!(parse_pmset_batt(on_batt), (Some(70), false));
        let charging =
            "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(parse_pmset_batt(charging), (Some(100), true));
        assert_eq!(parse_pmset_batt("Now drawing from 'AC Power'\n"), (None, true));
    }

    fn s(kind: &str, online: Option<&str>, capacity: Option<&str>, scope: Option<&str>) -> Supply {
        Supply {
            kind: kind.into(),
            online: online.map(Into::into),
            capacity: capacity.map(Into::into),
            scope: scope.map(Into::into),
        }
    }

    #[test]
    fn linux_supplies() {
        let laptop = [s("Mains\n", Some("0\n"), None, None), s("Battery\n", None, Some("64\n"), None)];
        assert_eq!(combine_supplies(&laptop), (Some(64), false));
        let charging = [s("Mains", Some("1"), None, None), s("Battery", None, Some("64"), None)];
        assert_eq!(combine_supplies(&charging), (Some(64), true));
        let usb_c = [s("USB", Some("1"), None, None), s("Battery", None, Some("30"), None)];
        assert_eq!(combine_supplies(&usb_c), (Some(30), true));
        let mouse_only = [s("Battery", None, Some("5"), Some("Device"))];
        assert_eq!(combine_supplies(&mouse_only), (None, true));
        let two = [s("Battery", None, Some("80"), None), s("Battery", None, Some("40"), None)];
        assert_eq!(combine_supplies(&two), (Some(60), false));
    }

    #[test]
    fn linux_thermal() {
        let cool = Zone { temp: 55_000, trips: vec![("passive".into(), 90_000), ("critical".into(), 105_000)] };
        let throttling = Zone { temp: 91_000, trips: vec![("passive".into(), 90_000), ("critical".into(), 105_000)] };
        let critical = Zone { temp: 106_000, trips: vec![("passive".into(), 90_000), ("critical".into(), 105_000)] };
        let active_only = Zone { temp: 70_000, trips: vec![("active".into(), 60_000)] };
        assert_eq!(thermal_level(&[]), 0);
        assert_eq!(thermal_level(&[cool]), 0);
        assert_eq!(thermal_level(&[throttling]), 2);
        assert_eq!(thermal_level(&[critical]), 3);
        assert_eq!(thermal_level(&[active_only]), 0);
    }
}
