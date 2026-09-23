//! Everything that touches macOS power state. Lid-closed awake = the kernel SleepDisabled
//! flag (`pmset -a disablesleep 1`), which needs root — granted once via a sudoers rule that
//! allows exactly that command and nothing else.
use crate::settings::data_dir;
use std::process::{Child, Command, Stdio};

const PMSET: &str = "/usr/bin/pmset";
const SUDOERS: &str = "/etc/sudoers.d/agents-dont-sleep";

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Whether the sudoers rule is in place (`sudo -l <cmd>` checks without running it).
pub fn has_sudoers() -> bool {
    quiet(Command::new("/usr/bin/sudo").args(["-n", "-l", PMSET, "-a", "disablesleep", "1"]))
}

pub fn set_sleep_disabled(on: bool) -> bool {
    let v = if on { "1" } else { "0" };
    quiet(Command::new("/usr/bin/sudo").args(["-n", PMSET, "-a", "disablesleep", v]))
}

fn admin_shell(script: &str, prompt: &str) -> Result<(), String> {
    let apple = format!(
        "do shell script \"{}\" with administrator privileges with prompt \"{}\"",
        script.replace('\\', "\\\\").replace('"', "\\\""),
        prompt
    );
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", &apple])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// One admin prompt: validate the rule with visudo, then install it root-owned 0440.
pub fn install_sudoers() -> Result<(), String> {
    let user = std::env::var("USER").map_err(|e| e.to_string())?;
    // The name ends up in sudoers and in a shell line; refuse anything unusual.
    if user.is_empty() || !user.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err(format!("unsupported user name {user:?}"));
    }
    let tmp = data_dir().join("sudoers.tmp");
    let tmp_s = tmp.to_string_lossy().into_owned();
    if tmp_s.contains('\'') {
        return Err("home path contains a quote".into());
    }
    let rule = format!(
        "# Agents Don't Sleep: lets {user} toggle lid-closed awake, nothing else.\n\
         {user} ALL=(root) NOPASSWD: {PMSET} -a disablesleep 0, {PMSET} -a disablesleep 1\n"
    );
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, rule).map_err(|e| e.to_string())?;
    let res = admin_shell(
        &format!("/usr/sbin/visudo -cf '{tmp_s}' && /usr/bin/install -m 0440 -o root -g wheel '{tmp_s}' {SUDOERS}"),
        "Agents Don't Sleep needs permission to keep your Mac awake with the lid closed.",
    );
    let _ = std::fs::remove_file(&tmp);
    res
}

pub fn uninstall_sudoers() -> Result<(), String> {
    set_sleep_disabled(false);
    admin_shell(&format!("/bin/rm -f {SUDOERS}"), "Remove the lid-closed permission for Agents Don't Sleep.")
}

/// Detached shell in its own process group: when this app dies for any reason (even
/// SIGKILL), it clears SleepDisabled so a crashed app can't leave a Mac hot in a bag.
/// ponytail: only resets the flag; macOS idle sleep then puts a closed Mac to sleep.
pub fn spawn_watchdog() {
    use std::os::unix::process::CommandExt;
    let script = format!(
        "while kill -0 \"$1\" 2>/dev/null; do sleep 3; done; /usr/bin/sudo -n {PMSET} -a disablesleep 0"
    );
    let _ = Command::new("/bin/sh")
        .args(["-c", &script, "ads-watchdog", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}

/// A plain user-level assertion as well — covers idle sleep when the sudoers rule is missing,
/// and shows up in `pmset -g assertions`. Dies with us thanks to `-w`.
pub fn caffeinate() -> Option<Child> {
    Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// (percent, on_ac) from `pmset -g batt`. Percent is None on Macs without a battery.
pub fn battery() -> (Option<u8>, bool) {
    parse_batt(&output(PMSET, &["-g", "batt"]))
}

pub fn parse_batt(s: &str) -> (Option<u8>, bool) {
    let on_ac = s.contains("'AC Power'");
    let pct = s
        .split(|c: char| c.is_whitespace() || c == ';')
        .find_map(|w| w.strip_suffix('%')?.parse().ok());
    (pct, on_ac)
}

pub fn lid_closed() -> bool {
    output("/usr/sbin/ioreg", &["-r", "-k", "AppleClamshellState", "-d", "1"])
        .contains("\"AppleClamshellState\" = Yes")
}

pub fn thermal_state() -> u8 {
    objc2_foundation::NSProcessInfo::processInfo().thermalState().0 as u8
}

pub fn low_power_mode() -> bool {
    objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled()
}

/// Keeps App Nap from throttling the 2s poll loop while no window is visible.
/// ponytail: token is leaked on purpose — held for the process lifetime.
pub fn disable_app_nap() {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    let token = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
        &NSString::from_str("Watching coding agents"),
    );
    std::mem::forget(token);
}

/// Seconds since the last keyboard/mouse input.
pub fn user_idle_secs() -> u64 {
    output("/usr/sbin/ioreg", &["-c", "IOHIDSystem", "-d", "4"])
        .lines()
        .find_map(|l| l.split("\"HIDIdleTime\" = ").nth(1)?.trim().parse::<u64>().ok())
        .map_or(0, |ns| ns / 1_000_000_000)
}

pub fn display_sleep_now() {
    quiet(Command::new(PMSET).arg("displaysleepnow"));
}

pub fn sleep_now() {
    quiet(Command::new(PMSET).arg("sleepnow"));
}

/// Lock via the private login.framework call the menu-bar "Lock Screen" item uses;
/// falls back to display sleep (locks too when "require password immediately" is set).
pub fn lock_screen() {
    let lib = c"/System/Library/PrivateFrameworks/login.framework/Versions/A/login";
    unsafe {
        let h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY);
        let f = if h.is_null() { std::ptr::null_mut() } else { libc::dlsym(h, c"SACLockScreenImmediate".as_ptr()) };
        if f.is_null() {
            display_sleep_now();
        } else {
            let lock: extern "C" fn() -> i32 = std::mem::transmute(f);
            lock();
        }
    }
}

const SOUNDS_DIR: &str = "/System/Library/Sounds";

pub fn sounds() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(SOUNDS_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".aiff").map(String::from))
        .collect();
    v.sort();
    v
}

pub fn play_sound(name: &str) {
    // Only names that exist in the system folder — never a caller-supplied path.
    if !name.is_empty() && sounds().iter().any(|s| s == name) {
        let _ = Command::new("/usr/bin/afplay")
            .arg(format!("{SOUNDS_DIR}/{name}.aiff"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::parse_batt;

    #[test]
    fn batt() {
        let on_batt = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=7077987)\t70%; discharging; 5:07 remaining present: true\n";
        assert_eq!(parse_batt(on_batt), (Some(70), false));
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(parse_batt(charging), (Some(100), true));
        assert_eq!(parse_batt("Now drawing from 'AC Power'\n"), (None, true));
    }
}
