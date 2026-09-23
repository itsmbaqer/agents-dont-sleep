//! Polar license keys (public customer-portal endpoints, no API token) + a local trial.
//! Licensing is off entirely until POLAR_ORG_ID is set, so a personal build never locks itself.
//! ponytail: the trial lives in settings.json and is trivially resettable — fine for a one-time-price app.
use crate::settings::{now, Settings};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

/// Your Polar organization id (Polar dashboard → Settings). Empty = licensing disabled.
pub const POLAR_ORG_ID: &str = "";
pub const BUY_URL: &str = "https://polar.sh";
const API: &str = "https://api.polar.sh/v1/customer-portal/license-keys";
const TRIAL_DAYS: u64 = 7;
const DAY: u64 = 86_400;
const REVALIDATE_EVERY: u64 = 7 * DAY;
/// Offline grace: a license that can't reach Polar keeps working this long after the last good check.
const OFFLINE_GRACE: u64 = 30 * DAY;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    pub configured: bool,
    /// "licensed" | "trial" | "expired" | "off"
    pub state: &'static str,
    pub trial_days_left: u64,
    pub buy_url: &'static str,
}

pub fn info(s: &Settings) -> LicenseInfo {
    let configured = !POLAR_ORG_ID.is_empty();
    let used = now().saturating_sub(s.first_run) / DAY;
    let trial_days_left = TRIAL_DAYS.saturating_sub(used);
    let state = if !configured {
        "off"
    } else if s.license_ok {
        "licensed"
    } else if trial_days_left > 0 {
        "trial"
    } else {
        "expired"
    };
    LicenseInfo { configured, state, trial_days_left, buy_url: BUY_URL }
}

pub fn allowed(s: &Settings) -> bool {
    info(s).state != "expired"
}

/// (http status, body). Status 0 = network failure.
fn post(endpoint: &str, body: Value) -> (u16, Value) {
    let child = Command::new("/usr/bin/curl")
        .args(["-sS", "-m", "15", "-X", "POST", "-H", "Content-Type: application/json"])
        .args(["--data-binary", "@-", "-w", "\n%{http_code}", &format!("{API}/{endpoint}")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return (0, Value::Null) };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.to_string().as_bytes());
    }
    let out = child.wait_with_output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let (json_part, code) = out.rsplit_once('\n').unwrap_or(("", "0"));
    (code.trim().parse().unwrap_or(0), serde_json::from_str(json_part).unwrap_or(Value::Null))
}

fn detail(v: &Value, fallback: &str) -> String {
    v.get("detail").and_then(Value::as_str).unwrap_or(fallback).to_string()
}

pub fn activate(s: &mut Settings, key: &str) -> Result<(), String> {
    let host = Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|_| "Mac".into());
    let (code, v) = post("activate", json!({ "key": key.trim(), "organization_id": POLAR_ORG_ID, "label": host }));
    match code {
        200 => {
            s.license_key = key.trim().into();
            s.activation_id = v["id"].as_str().unwrap_or_default().into();
            s.license_ok = true;
            s.license_checked_at = now();
            Ok(())
        }
        403 => Err(detail(&v, "This key is revoked, expired, or already on the maximum number of Macs.")),
        404 => Err("License key not found.".into()),
        0 => Err("Couldn't reach Polar. Check your connection.".into()),
        _ => Err(detail(&v, "Activation failed.")),
    }
}

pub fn deactivate(s: &mut Settings) {
    if !s.activation_id.is_empty() {
        post("deactivate", json!({ "key": s.license_key, "organization_id": POLAR_ORG_ID, "activation_id": s.activation_id }));
    }
    s.license_key.clear();
    s.activation_id.clear();
    s.license_ok = false;
}

/// Weekly re-check. Returns true when settings changed and should be saved.
pub fn revalidate(s: &mut Settings) -> bool {
    if POLAR_ORG_ID.is_empty() || !s.license_ok || now() - s.license_checked_at < REVALIDATE_EVERY {
        return false;
    }
    let (code, v) = post("validate", json!({ "key": s.license_key, "organization_id": POLAR_ORG_ID, "activation_id": s.activation_id }));
    match code {
        200 if v["status"] == "granted" => s.license_checked_at = now(),
        0 | 500.. => s.license_ok = now() - s.license_checked_at < OFFLINE_GRACE,
        _ => s.license_ok = false,
    }
    true
}
