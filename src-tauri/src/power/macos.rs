//! macOS. Lid-closed awake = the kernel SleepDisabled flag (`pmset -a disablesleep 1`), which
//! needs root — granted once via a sudoers rule that allows exactly that command and nothing else.
use super::{output, quiet, sound_files};
use crate::settings::data_dir;
use std::process::{Child, Command, Stdio};

pub const OS: &str = "macos";
pub const DEVICE: &str = "Mac";
pub const NEEDS_GRANT: bool = true;

const PMSET: &str = "/usr/bin/pmset";
const SUDOERS: &str = "/etc/sudoers.d/agents-dont-sleep";
const SOUNDS_DIR: &str = "/System/Library/Sounds";

/// Whether the sudoers rule is in place (`sudo -l <cmd>` checks without running it).
pub fn has_grant() -> bool {
    quiet("/usr/bin/sudo", &["-n", "-l", PMSET, "-a", "disablesleep", "1"])
}

fn set_sleep_disabled(on: bool) -> bool {
    quiet("/usr/bin/sudo", &["-n", PMSET, "-a", "disablesleep", if on { "1" } else { "0" }])
}

fn admin_shell(script: &str, prompt: &str) -> Result<(), String> {
    let apple = format!(
        "do shell script \"{}\" with administrator privileges with prompt \"{}\"",
        script.replace('\\', "\\\\").replace('"', "\\\""),
        prompt
    );
    let out = Command::new("/usr/bin/osascript").args(["-e", &apple]).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// One admin prompt: validate the rule with visudo, then install it root-owned 0440.
pub fn install_grant() -> Result<(), String> {
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

pub fn uninstall_grant() -> Result<(), String> {
    set_sleep_disabled(false);
    admin_shell(&format!("/bin/rm -f {SUDOERS}"), "Remove the lid-closed permission for Agents Don't Sleep.")
}

/// SleepDisabled when granted, plus a plain `caffeinate` assertion that covers idle sleep when
/// the rule is missing and shows up in `pmset -g assertions`. `-w` makes it die with us.
pub struct Hold {
    granted: bool,
    caffeinate: Option<Child>,
}

pub fn hold(granted: bool) -> Hold {
    if granted {
        set_sleep_disabled(true);
    }
    let caffeinate = Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    Hold { granted, caffeinate }
}

impl Hold {
    pub fn release(mut self) {
        if self.granted {
            set_sleep_disabled(false);
        }
        if let Some(mut c) = self.caffeinate.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub fn restore() {
    if has_grant() {
        set_sleep_disabled(false);
    }
}

pub fn init() {}

pub fn battery() -> (Option<u8>, bool) {
    super::parse_pmset_batt(&output(PMSET, &["-g", "batt"]))
}

pub fn low_power() -> bool {
    objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled()
}

pub fn thermal() -> Option<u8> {
    Some(objc2_foundation::NSProcessInfo::processInfo().thermalState().0 as u8)
}

pub fn lid_closed() -> bool {
    output("/usr/sbin/ioreg", &["-r", "-k", "AppleClamshellState", "-d", "1"]).contains("\"AppleClamshellState\" = Yes")
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
    quiet(PMSET, &["displaysleepnow"]);
}

pub fn sleep_now() {
    quiet(PMSET, &["sleepnow"]);
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

pub fn sounds() -> Vec<String> {
    sound_files(SOUNDS_DIR, ".aiff")
}

pub fn default_sound() -> &'static str {
    "Glass"
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
