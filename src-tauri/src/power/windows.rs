//! Windows. Lid-closed awake = the active power plan's lid action set to "Do nothing" (plus the
//! battery idle-sleep timeout off, which Modern Standby needs), and a "system required" power
//! request. A standard user may change their own plan, so no elevation. The original values are
//! saved to `restore.json` first, so a crash can never leave them changed.
use super::sound_files;
use crate::settings::{data_dir, write_atomic};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::*;
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const OS: &str = "windows";
pub const DEVICE: &str = "PC";
pub const NEEDS_GRANT: bool = false;

const SUB_BUTTONS: GUID = GUID::from_u128(0x4f971e89_eebd_4455_a8de_9e59040e7347);
const LID_ACTION: GUID = GUID::from_u128(0x5ca83367_6e45_459f_a27b_476b1d01c936);
const SUB_SLEEP: GUID = GUID::from_u128(0x238c9fa8_0aad_41ed_83f4_97be242c8f20);
const STANDBY_IDLE: GUID = GUID::from_u128(0x29f6c1db_86da_48c5_9fdb_f2b67b1f44da);
const LIDSWITCH_STATE_CHANGE: GUID = GUID::from_u128(0xba3e0f4d_b817_4094_a2d1_d56379e6a0f3);

static LID_CLOSED: AtomicBool = AtomicBool::new(false);

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
    lid_ac: u32,
    lid_dc: u32,
    standby_dc: u32,
}

fn restore_file() -> std::path::PathBuf {
    data_dir().join("restore.json")
}

/// Runs `f` with the active scheme GUID.
fn with_scheme<T>(f: impl FnOnce(*const GUID) -> T) -> Option<T> {
    unsafe {
        let mut scheme: *mut GUID = std::ptr::null_mut();
        if PowerGetActiveScheme(std::ptr::null_mut(), &mut scheme) != 0 || scheme.is_null() {
            return None;
        }
        let out = f(scheme);
        LocalFree(scheme as _);
        Some(out)
    }
}

fn read(scheme: *const GUID, sub: &GUID, setting: &GUID, ac: bool) -> Option<u32> {
    let mut v = 0u32;
    let err = unsafe {
        if ac {
            PowerReadACValueIndex(std::ptr::null_mut(), scheme, sub, setting, &mut v)
        } else {
            PowerReadDCValueIndex(std::ptr::null_mut(), scheme, sub, setting, &mut v)
        }
    };
    (err == 0).then_some(v)
}

fn write(scheme: *const GUID, sub: &GUID, setting: &GUID, ac: bool, v: u32) {
    unsafe {
        if ac {
            PowerWriteACValueIndex(std::ptr::null_mut(), scheme, sub, setting, v);
        } else {
            PowerWriteDCValueIndex(std::ptr::null_mut(), scheme, sub, setting, v);
        }
    }
}

fn apply(s: &Saved) {
    with_scheme(|scheme| unsafe {
        write(scheme, &SUB_BUTTONS, &LID_ACTION, true, s.lid_ac);
        write(scheme, &SUB_BUTTONS, &LID_ACTION, false, s.lid_dc);
        write(scheme, &SUB_SLEEP, &STANDBY_IDLE, false, s.standby_dc);
        PowerSetActiveScheme(std::ptr::null_mut(), scheme);
    });
}

/// Handle of the power request, stored as an integer so `Hold` is `Send`.
pub struct Hold {
    request: isize,
}

pub fn hold(_granted: bool) -> Hold {
    // Save the user's values once; a leftover file (crash) already holds the originals.
    if !restore_file().exists() {
        let saved = with_scheme(|scheme| {
            Some(Saved {
                lid_ac: read(scheme, &SUB_BUTTONS, &LID_ACTION, true)?,
                lid_dc: read(scheme, &SUB_BUTTONS, &LID_ACTION, false)?,
                standby_dc: read(scheme, &SUB_SLEEP, &STANDBY_IDLE, false)?,
            })
        })
        .flatten();
        if let Some(s) = saved {
            if let Ok(json) = serde_json::to_vec(&s) {
                let _ = write_atomic(&restore_file(), &json);
            }
        }
    }
    if restore_file().exists() {
        apply(&Saved { lid_ac: 0, lid_dc: 0, standby_dc: 0 });
    }

    let mut reason: Vec<u16> = "Coding agents are working".encode_utf16().chain([0]).collect();
    let ctx = REASON_CONTEXT {
        Version: 0, // POWER_REQUEST_CONTEXT_VERSION
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 { SimpleReasonString: reason.as_mut_ptr() },
    };
    let h = unsafe { PowerCreateRequest(&ctx) };
    let request = if h == INVALID_HANDLE_VALUE || h.is_null() {
        0
    } else {
        unsafe {
            PowerSetRequest(h, PowerRequestSystemRequired);
            PowerSetRequest(h, PowerRequestExecutionRequired);
        }
        h as isize
    };
    Hold { request }
}

impl Hold {
    pub fn release(self) {
        if self.request != 0 {
            let h = self.request as HANDLE;
            unsafe {
                PowerClearRequest(h, PowerRequestSystemRequired);
                PowerClearRequest(h, PowerRequestExecutionRequired);
                CloseHandle(h);
            }
        }
        restore();
    }
}

/// Puts the saved lid action and idle timeout back (startup, release, watchdog).
pub fn restore() {
    let Ok(bytes) = std::fs::read(restore_file()) else { return };
    if let Ok(s) = serde_json::from_slice::<Saved>(&bytes) {
        apply(&s);
    }
    let _ = std::fs::remove_file(restore_file());
}

/// Lid state arrives as a power-setting notification, so a message-only window listens for it.
/// Windows sends the current state right after registering.
pub fn init() {
    std::thread::spawn(|| unsafe {
        let class: Vec<u16> = "AgentsDontSleepLid".encode_utf16().chain([0]).collect();
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        RegisterPowerSettingNotification(hwnd, &LIDSWITCH_STATE_CHANGE, DEVICE_NOTIFY_WINDOW_HANDLE);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
    });
}

fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_POWERBROADCAST && wparam == PBT_POWERSETTINGCHANGE as usize && lparam != 0 {
        let setting = &*(lparam as *const POWERBROADCAST_SETTING);
        if same(&setting.PowerSetting, &LIDSWITCH_STATE_CHANGE) {
            LID_CLOSED.store(setting.Data[0] == 0, Ordering::Relaxed);
        }
        return 1;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn power_status() -> Option<SYSTEM_POWER_STATUS> {
    let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    (unsafe { GetSystemPowerStatus(&mut s) } != 0).then_some(s)
}

pub fn battery() -> (Option<u8>, bool) {
    match power_status() {
        // BatteryFlag 128 = no system battery; percent 255 = unknown.
        Some(s) => {
            ((s.BatteryFlag != 128 && s.BatteryLifePercent <= 100).then_some(s.BatteryLifePercent), s.ACLineStatus == 1)
        }
        None => (None, true),
    }
}

/// Battery saver / Energy saver.
pub fn low_power() -> bool {
    power_status().is_some_and(|s| s.SystemStatusFlag == 1)
}

/// No thermal-pressure API that works without admin on Windows.
pub fn thermal() -> Option<u8> {
    None
}

pub fn lid_closed() -> bool {
    LID_CLOSED.load(Ordering::Relaxed)
}

pub fn disable_app_nap() {}

pub fn user_idle_secs() -> u64 {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return 0;
    }
    u64::from(unsafe { GetTickCount() }.wrapping_sub(info.dwTime) / 1000)
}

pub fn display_sleep_now() {
    unsafe { PostMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER as usize, 2) };
}

/// Restoring the plan's idle timeout lets Windows sleep on its own; forcing it could hibernate.
pub fn sleep_now() {}

pub fn lock_screen() {
    unsafe { LockWorkStation() };
}

fn media_dir() -> String {
    format!("{}\\Media", std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
}

pub fn sounds() -> Vec<String> {
    sound_files(&media_dir(), ".wav")
}

pub fn default_sound() -> &'static str {
    "Windows Notify System Generic"
}

pub fn play_sound(name: &str) {
    if !name.is_empty() && sounds().iter().any(|s| s == name) {
        let path: Vec<u16> = format!("{}\\{name}.wav", media_dir()).encode_utf16().chain([0]).collect();
        unsafe { PlaySoundW(path.as_ptr(), std::ptr::null_mut(), SND_FILENAME | SND_ASYNC | SND_NODEFAULT) };
    }
}

/// 8.3 short form of `path` (no spaces, so it survives cmd.exe and PowerShell unquoted), or
/// the long path when 8.3 names are disabled on the volume.
pub fn short_path(path: &std::path::Path) -> String {
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let long: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetShortPathNameW(long.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return path.to_string_lossy().into_owned();
    }
    String::from_utf16_lossy(&buf[..n])
}
